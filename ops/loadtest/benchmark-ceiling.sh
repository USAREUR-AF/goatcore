#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# benchmark-ceiling.sh — reusable isolated-sink ceiling benchmark for the
# goat Zenoh fabric. Run ON the router host (nova-a). One command:
#
#   1. stands up an ISOLATED durable sink (dedicated loadtest/** audit-subscriber
#      → throwaway, quota-capped MinIO bucket with a header-safe writer user),
#   2. PAUSES the production audit-subscriber for the window (so the firehose
#      cannot flood-drop real-vendor audit — zenoh key-exprs can't express
#      "all-except-loadtest", so a clean pause beats chaotic shared drops),
#   3. sweeps a rate ladder, measuring rocksdb AND MinIO SEPARATELY at each rate
#      (rocksdb via router REST GET-back — bounded by store-rate, not offered;
#      MinIO via the dedicated subscriber's per-run metric) + router CPU + put
#      latency, into a `ceil2` evidence JSONL,
#   4. RESTARTS the production audit-subscriber + tears the isolated sink down,
#   5. renders the rich HTML report (summarize.py) — hypotheses scored
#      validated/rejected, per-sink.
#
# Idempotent + re-runnable. Honors the wipe contract in lib-loadtest.sh (only
# ever touches the dedicated loadtest subscriber + a loadtest-* bucket + the
# throwaway user; never a real bucket).
#
# Design: docs/design/zenoh-loadtest.md  ·  First runs: M-052 / M-053.
#
# Prereqs on the host: the loadtest namespace provisioned (cert at
# $LOADTEST_CERT etc. + loadtest/** ACL + rocksdb storage — see
# zenoh-loadtest.yml), the goat-loadgen image present, sudo docker, jq, curl.
#
# Usage:
#   benchmark-ceiling.sh                                  # default ladder
#   benchmark-ceiling.sh --ladder 2000:8:15,20000:16:12,200000:16:10,0:16:10
#   benchmark-ceiling.sh --no-pause-prod                 # skip pausing prod sub (UNSAFE at high rate)
#
#   rate:sessions:duration  — rate 0 = unbounded (max). Sweeps left→right.

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-loadtest.sh
source "${HERE}/lib-loadtest.sh"

# ---- Host-specific knobs (nova-a defaults; override via env) -------------------
: "${BENCH_MINIO_CONTAINER:=minio}"
: "${BENCH_ROUTER_CONTAINER:=goat-zenoh-router}"
: "${BENCH_PROD_SUBSCRIBER:=audit-subscriber}"
: "${BENCH_LOADTEST_SUBSCRIBER:=audit-subscriber-loadtest}"
: "${BENCH_AUDIT_IMAGE:=ghcr.io/usareur-af/goat-audit-subscriber:sha-51d9a8b}"
: "${BENCH_AUDIT_CERT_DIR:=/opt/audit-subscriber/certs}"   # reused: universal-read covers loadtest/**
: "${BENCH_CA_ROOT:=/etc/goat-zenoh/tls/ca-root-cert.pem}"
: "${BENCH_S3_ENDPOINT:=http://100.127.0.186:9000}"
: "${BENCH_SUB_METRICS_PORT:=9305}"
: "${BENCH_LOADGEN_IMAGE:=${LOADTEST_LOADGEN_IMAGE:-goat-loadgen:local}}"
: "${BENCH_LADDER:=2000:8:15,20000:16:12,200000:16:10,0:16:10}"
: "${BENCH_PAUSE_PROD:=1}"
: "${BENCH_SETTLE:=10}"
: "${BENCH_OUT:=${LOADTEST_OUT_DIR}}"

LADDER="$BENCH_LADDER"; PAUSE_PROD="$BENCH_PAUSE_PROD"
for a in "$@"; do case "$a" in
  --ladder) _next=ladder ;; --no-pause-prod) PAUSE_PROD=0 ;;
  -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  *) [ "${_next:-}" = ladder ] && { LADDER="$a"; _next=; } ;;
esac; done

D() { sudo docker "$@"; }
MC() { sudo docker exec "$BENCH_MINIO_CONTAINER" mc "$@"; }
ROUTER_IP="$(D inspect "$BENCH_ROUTER_CONTAINER" --format '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' 2>/dev/null)"
REST="http://${ROUTER_IP}:${goat_zenoh_rest_port:-8000}"
SUBM="http://127.0.0.1:${BENCH_SUB_METRICS_PORT}/metrics"
LGM="http://127.0.0.1:9304/metrics"
RID="ceil$(date +%Y%m%d-%H%M%S)"
mkdir -p "$BENCH_OUT"; EV="${BENCH_OUT}/$(date +%Y-%m-%d)-zenoh-fabric-ceiling-${RID}.jsonl"; : > "$EV"

for t in jq curl; do command -v "$t" >/dev/null || loadtest::die "missing tool: $t"; done
loadtest::assert_ephemeral_bucket "$LOADTEST_BUCKET"   # refuse if misconfigured to a real bucket

cleanup_done=0
teardown() {
  [ "$cleanup_done" = 1 ] && return; cleanup_done=1
  loadtest::log "teardown: restoring production audit-subscriber + removing isolated sink"
  [ "$PAUSE_PROD" = 1 ] && D start "$BENCH_PROD_SUBSCRIBER" >/dev/null 2>&1 || true
  D rm -f "$BENCH_LOADTEST_SUBSCRIBER" >/dev/null 2>&1 || true
  loadtest::assert_ephemeral_bucket "$LOADTEST_BUCKET"
  MC rm --recursive --force "local/${LOADTEST_BUCKET}" >/dev/null 2>&1 || true
  MC rb --force "local/${LOADTEST_BUCKET}" >/dev/null 2>&1 || true
  MC admin user remove local loadtest-writer >/dev/null 2>&1 || true
  D rm -f "glg-bench" >/dev/null 2>&1 || true
}
trap teardown EXIT INT TERM

# ---- 1. isolated sink: bucket + header-safe writer + dedicated subscriber ----
loadtest::log "standing up isolated sink (${LOADTEST_BUCKET} + ${BENCH_LOADTEST_SUBSCRIBER})"
MC mb --ignore-existing "local/${LOADTEST_BUCKET}" >/dev/null 2>&1 || true
MC quota set "local/${LOADTEST_BUCKET}" --size 30gib >/dev/null 2>&1 || true
SECRET="$(LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 40)"   # header-safe (SigV4) secret
MC admin user add local loadtest-writer "$SECRET" >/dev/null 2>&1 || true
MC admin policy attach local readwrite --user loadtest-writer >/dev/null 2>&1 || true
D rm -f "$BENCH_LOADTEST_SUBSCRIBER" >/dev/null 2>&1 || true
D run -d --name "$BENCH_LOADTEST_SUBSCRIBER" --network "$LOADTEST_DOCKER_NETWORK" -p "127.0.0.1:${BENCH_SUB_METRICS_PORT}:9303" \
  -v "${BENCH_AUDIT_CERT_DIR}:/certs:ro" -v "${BENCH_CA_ROOT}:/ca/ca-root.pem:ro" \
  -e AUDIT_SUBSCRIBER_ENDPOINT="$LOADTEST_ZENOH_ENDPOINT" -e AUDIT_SUBSCRIBER_KEY="${LOADTEST_KEY_PREFIX}/**" -e AUDIT_SUBSCRIBER_ID=loadtest \
  -e AUDIT_SUBSCRIBER_METRICS_BIND=0.0.0.0:9303 \
  -e AUDIT_SUBSCRIBER_CERT=/certs/audit-subscriber-cert.pem -e AUDIT_SUBSCRIBER_KEY_FILE=/certs/audit-subscriber-key.pem -e AUDIT_SUBSCRIBER_ROOT_CA=/ca/ca-root.pem \
  -e AUDIT_SUBSCRIBER_S3_ENDPOINT="$BENCH_S3_ENDPOINT" -e AUDIT_SUBSCRIBER_S3_BUCKET="$LOADTEST_BUCKET" -e AUDIT_SUBSCRIBER_S3_REGION=us-east-1 \
  -e AUDIT_SUBSCRIBER_S3_ACCESS_KEY=loadtest-writer -e AUDIT_SUBSCRIBER_S3_SECRET_KEY="$SECRET" \
  "$BENCH_AUDIT_IMAGE" >/dev/null || loadtest::die "failed to launch loadtest subscriber"
sleep 6
[ "$(curl -fsS "$SUBM" 2>/dev/null | awk '/subscriber_session_state/ && !/^#/{print $2}')" = 1 ] \
  || loadtest::die "loadtest subscriber did not declare its subscription"

# ---- 2. pause production subscriber (clean window) ---------------------------
if [ "$PAUSE_PROD" = 1 ]; then
  loadtest::log "pausing production subscriber '$BENCH_PROD_SUBSCRIBER' (audit-window start $(date -u +%H:%M:%SZ))"
  D stop "$BENCH_PROD_SUBSCRIBER" >/dev/null 2>&1 || true
fi
emit_meta() { printf '%s\n' "$1" >> "$EV"; }
emit_meta "{\"event\":\"bench_start\",\"run_id\":\"${RID}\",\"ladder\":\"${LADDER}\",\"prod_paused\":${PAUSE_PROD},\"ts_utc\":\"$(date -u +%H:%M:%SZ)\"}"

subabs() { curl -fsS "$SUBM" 2>/dev/null | awk -v m="$1" '$0 ~ ("goat_audit_" m) && $0 !~ /^#/ {s+=$2} END{print s+0}'; }
rcpu() { sudo docker stats --no-stream --format '{{.Name}} {{.CPUPerc}}' "$1" 2>/dev/null | awk '{print $2}'; }

# ---- 3. sweep ---------------------------------------------------------------
IFS=',' read -ra STEPS <<< "$LADDER"
for spec in "${STEPS[@]}"; do
  R=${spec%%:*}; rest=${spec#*:}; S=${rest%%:*}; DUR=${rest##*:}
  PRID="${RID}-r${R}"; CN="glg-bench"
  loadtest::log "rate=${R} sessions=${S} dur=${DUR}s (run ${PRID})"
  d0=$(subabs drop_total); w0=$(subabs write_errors_total)
  D rm -f "$CN" >/dev/null 2>&1 || true
  D run -d --name "$CN" --network "$LOADTEST_DOCKER_NETWORK" -p 127.0.0.1:9304:9304 \
    -v "${LOADTEST_CERT}:/certs/cert.pem:ro" -v "${LOADTEST_KEY}:/certs/key.pem:ro" -v "${LOADTEST_ROOT_CA}:/certs/ca.pem:ro" \
    -e LOADGEN_ENDPOINT="$LOADTEST_ZENOH_ENDPOINT" -e LOADGEN_KEY_PREFIX="$LOADTEST_KEY_PREFIX" -e LOADGEN_RUN_ID="$PRID" \
    -e LOADGEN_RATE="$R" -e LOADGEN_PAYLOAD_BYTES=100 -e LOADGEN_DURATION_SECS="$DUR" -e LOADGEN_SESSIONS="$S" -e LOADGEN_PROGRESS_SECS=0 \
    -e LOADGEN_CERT=/certs/cert.pem -e LOADGEN_KEY=/certs/key.pem -e LOADGEN_ROOT_CA=/certs/ca.pem "$BENCH_LOADGEN_IMAGE" >/dev/null
  sleep "$(( DUR/2 ))"
  RCPU=$(rcpu "$BENCH_ROUTER_CONTAINER"); SCPU=$(rcpu "$BENCH_LOADTEST_SUBSCRIBER")
  PUTHIST=$(curl -fsS "$LGM" 2>/dev/null | grep '^loadgen_put_latency_seconds_bucket' | jq -R -s -c 'split("\n")|map(select(length>0))')
  while D ps --format '{{.Names}}' | grep -q "^${CN}$"; do sleep 2; done
  SENT=$(D logs "$CN" 2>&1 | grep -o '"event":"final".*' | tail -1 | jq -r .sent 2>/dev/null)
  sleep "$BENCH_SETTLE"
  RDB=$(curl -fsS "${REST}/${LOADTEST_KEY_PREFIX}/${PRID}/**" 2>/dev/null | jq 'length' 2>/dev/null); [ -n "$RDB" ] || RDB=0
  MIO=$(curl -fsS "$SUBM" 2>/dev/null | awk -v p="$PRID" '$0 ~ ("key_pattern=\"" "'"$LOADTEST_KEY_PREFIX"'" "/" p "/\\*\"") && $0 !~ /^#/ {print $2}'); [ -n "$MIO" ] || MIO=0
  d1=$(subabs drop_total); w1=$(subabs write_errors_total)
  printf '{"event":"ceil2","run_id":"%s","rate_target":%s,"sessions":%s,"dur":%s,"sent":%s,"router_cpu":"%s","sub_cpu":"%s","rocksdb_stored":%s,"minio_captured":%s,"minio_drops":%s,"minio_write_errors":%s,"put_hist":%s}\n' \
    "$PRID" "$R" "$S" "$DUR" "${SENT:-0}" "$RCPU" "$SCPU" "$RDB" "$MIO" "$(( d1 - d0 ))" "$(( w1 - w0 ))" "${PUTHIST:-[]}" >> "$EV"
  D rm -f "$CN" >/dev/null 2>&1 || true
  sleep 4
done
emit_meta "{\"event\":\"bench_end\",\"run_id\":\"${RID}\",\"ts_utc\":\"$(date -u +%H:%M:%SZ)\"}"

# ---- 4. teardown (trap) + 5. report ----------------------------------------
teardown
loadtest::log "evidence: $EV"
REPORT="${EV%.jsonl}.report.html"
if command -v python3 >/dev/null; then
  python3 "${HERE}/summarize.py" "$EV" >/dev/null 2>&1 && mv "${EV}.report.html" "$REPORT" 2>/dev/null \
    && loadtest::log "report: $REPORT"
fi
loadtest::log "ceiling benchmark complete."
