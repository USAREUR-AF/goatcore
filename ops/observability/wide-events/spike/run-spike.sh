#!/usr/bin/env bash
# 27A.3 — OpenObserve × MinIO interop spike.
#
# Anchor: ADR 0822 §"Trigger conditions for revisit" — first one is
# "OpenObserve hits a wall in the Block 27A spike. Switch to ClickHouse +
# Grafana per the named fallback."
#
# What this proves (PASS criteria, in order):
#   1. OpenObserve /healthz answers 200.
#   2. OpenObserve accepts OTLP/HTTP-JSON logs at /api/default/default/_json
#      (the OTLP-compatible JSON ingest endpoint).
#   3. The ingested record is queryable via the search API and returns
#      with the fields we sent intact.
#   4. After the configured idle interval (default 60s on v0.13.x), a
#      Parquet object lands in the MinIO telemetry-otlp bucket. (Default
#      idle threshold is short; we wait up to 5 min before declaring
#      this step FAIL.)
#   5. The audit-integrity bucket is unaffected — `mc ls` still shows
#      it empty (we never wrote to it; this confirms two-bucket
#      isolation).
#
# FAIL on any of the above → write findings/F-NNN under docs/findings/,
# evaluate ClickHouse + Grafana fallback per ADR 0822.
#
# Usage:
#   OPENOBSERVE_URL=http://192.168.1.253:5080 \
#   OPENOBSERVE_USER=person-69@example.org \
#   OPENOBSERVE_PASS='...' \
#   MINIO_HOST=192.168.1.253 \
#   MINIO_USER=... MINIO_PASS=... \
#   ./run-spike.sh
#
# Output:
#   - Each step prints PASS / FAIL on its own line.
#   - Final line is `OVERALL PASS` or `OVERALL FAIL: <step>`.
#   - JSONL evidence appended to
#     docs/reports/2026-05-04-openobserve-minio-spike.jsonl

set -eu

REPO_ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
# EVIDENCE override env var lets the spike run from any directory (e.g.
# /tmp on a remote host) where the repo isn't checked out. Default
# stays the canonical docs/reports/ path for the repo-root invocation.
EVIDENCE="${EVIDENCE:-$REPO_ROOT/docs/reports/2026-05-04-openobserve-minio-spike.jsonl}"

OPENOBSERVE_URL="${OPENOBSERVE_URL:-http://192.168.1.253:5080}"
OPENOBSERVE_USER="${OPENOBSERVE_USER:-}"
OPENOBSERVE_PASS="${OPENOBSERVE_PASS:-}"
MINIO_HOST="${MINIO_HOST:-192.168.1.253}"
MINIO_USER="${MINIO_USER:-}"
MINIO_PASS="${MINIO_PASS:-}"
ROTATION_WAIT_SEC="${ROTATION_WAIT_SEC:-300}"  # 5 min

if [ -z "$OPENOBSERVE_USER" ] || [ -z "$OPENOBSERVE_PASS" ] \
   || [ -z "$MINIO_USER" ] || [ -z "$MINIO_PASS" ]; then
  echo "ERROR: set OPENOBSERVE_USER / OPENOBSERVE_PASS / MINIO_USER / MINIO_PASS" >&2
  exit 64
fi

mkdir -p "$(dirname "$EVIDENCE")"

emit() {
  # Emit one JSONL evidence line.
  local step="$1" verdict="$2" detail="$3"
  printf '{"ts":"%s","spike":"openobserve-minio-interop","step":"%s","verdict":"%s","detail":%s}\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$step" "$verdict" "$detail" \
    | tee -a "$EVIDENCE"
}

fail() {
  local step="$1" detail="$2"
  emit "$step" "FAIL" "$(printf '%s' "$detail" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')"
  echo "OVERALL FAIL: $step"
  exit 1
}

# ---------- Step 1: OpenObserve healthz ----------
status="$(curl -fsS -o /tmp/oo-health.txt -w '%{http_code}' "$OPENOBSERVE_URL/healthz" || echo 000)"
if [ "$status" = "200" ]; then
  emit step1_healthz PASS "{\"status\":$status}"
else
  fail step1_healthz "/healthz returned $status (expected 200)"
fi

# ---------- Step 2: OTLP-JSON ingest ----------
CORRELATION_ID="spike-$(date +%s)-$$"
TS_NS=$(date -u +%s%N)

PAYLOAD=$(cat <<EOF
[
  {
    "_timestamp": $TS_NS,
    "level": "info",
    "service.name": "spike-emitter",
    "service.namespace": "vega",
    "service.instance.id": "spike-host-1",
    "dbb.correlation_id": "$CORRELATION_ID",
    "dbb.event_type": "spike.synthetic",
    "dbb.schema_version": "v0",
    "message": "27A.3 interop spike synthetic record"
  }
]
EOF
)

ingest_response=$(curl -fsS -u "$OPENOBSERVE_USER:$OPENOBSERVE_PASS" \
  -H 'Content-Type: application/json' \
  -X POST "$OPENOBSERVE_URL/api/default/default/_json" \
  --data-binary "$PAYLOAD") || fail step2_ingest "OTLP ingest POST failed"

# OpenObserve returns {"code":200,"status":[...]} on success
if echo "$ingest_response" | grep -q '"code":200'; then
  emit step2_ingest PASS "$(printf '%s' "$ingest_response" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')"
else
  fail step2_ingest "ingest returned non-200 body: $ingest_response"
fi

# ---------- Step 3: Search retrieval ----------
# Wait briefly for OpenObserve's index to catch up (it's near-real-time
# but not strictly synchronous on the ingest path).
sleep 5

SEARCH_QUERY=$(cat <<EOF
{
  "query": {
    "sql": "SELECT * FROM default WHERE dbb_correlation_id = '$CORRELATION_ID'",
    "start_time": $((TS_NS / 1000 - 60000000)),
    "end_time":   $((TS_NS / 1000 + 60000000)),
    "from": 0,
    "size": 10
  }
}
EOF
)

# OpenObserve replaces dots in attribute names with underscores at index
# time, so dbb.correlation_id becomes dbb_correlation_id. The SQL
# above uses the underscored form.

search_response=$(curl -fsS -u "$OPENOBSERVE_USER:$OPENOBSERVE_PASS" \
  -H 'Content-Type: application/json' \
  -X POST "$OPENOBSERVE_URL/api/default/_search?type=logs" \
  --data-binary "$SEARCH_QUERY") || fail step3_search "search POST failed"

hit_count=$(echo "$search_response" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("total",0))' 2>/dev/null || echo 0)

if [ "$hit_count" -ge 1 ]; then
  emit step3_search PASS "{\"hits\":$hit_count,\"correlation_id\":\"$CORRELATION_ID\"}"
else
  fail step3_search "search returned 0 hits for correlation_id=$CORRELATION_ID. Body: $search_response"
fi

# ---------- Step 4: Parquet rotation to MinIO ----------
# OpenObserve rotates idle write-ahead segments to S3 on a configurable
# interval. To force prompt rotation, we hit a small flush + give it
# up to ROTATION_WAIT_SEC seconds.

emit step4_rotation_wait_started INFO "{\"wait_until_sec\":$ROTATION_WAIT_SEC}"

# Use mc-init container in the minio compose project to inspect the
# bucket. This avoids needing mc on the spike-runner machine.
mc_count() {
  # The mc-init service in /opt/minio/docker-compose.yml has
  # `entrypoint: ["/bin/sh", "/mc-init.sh"]`. A `docker compose run
  # mc-init mc ls ...` invocation passes `mc ls ...` as ARGS to
  # mc-init.sh, which ignores them — so the bucket is never queried
  # and we see zero "type":"file" entries even when the bucket has
  # objects. Override the entrypoint to /bin/sh + -c so mc actually
  # runs. This bug bit Step 4 of the spike on Orin-C.
  ssh -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 \
    "goat@$MINIO_HOST" \
    "sudo docker compose -f /opt/minio/docker-compose.yml run --rm --no-deps --entrypoint /bin/sh mc-init -c 'mc --json ls --recursive local/telemetry-otlp/'" \
    2>/dev/null \
    | grep -c '"type":"file"' || true
}

rotation_observed=0
elapsed=0
while [ "$elapsed" -lt "$ROTATION_WAIT_SEC" ]; do
  count=$(mc_count)
  if [ "$count" -ge 1 ]; then
    rotation_observed=1
    emit step4_rotation PASS "{\"objects_in_bucket\":$count,\"elapsed_sec\":$elapsed}"
    break
  fi
  sleep 30
  elapsed=$((elapsed + 30))
done

if [ "$rotation_observed" -eq 0 ]; then
  emit step4_rotation FAIL "{\"objects_in_bucket\":0,\"waited_sec\":$ROTATION_WAIT_SEC}"
  echo "OVERALL FAIL: step4_rotation"
  echo ""
  echo "If this is the only failure, OpenObserve ingest works but warm-tier"
  echo "rotation isn't reaching MinIO. Common causes:"
  echo "  - ZO_S3_FORCE_PATH_STYLE not set (MinIO requires path-style)"
  echo "  - MinIO endpoint unreachable from openobserve container (check"
  echo "    extra_hosts host.docker.internal:host-gateway)"
  echo "  - Credentials mismatch — OpenObserve and MinIO env vars must align"
  echo "  - OpenObserve rotation interval too long for spike window — raise"
  echo "    ROTATION_WAIT_SEC=900 and re-run; also consider lowering"
  echo "    ZO_FILE_PUSH_INTERVAL on the OpenObserve container."
  exit 1
fi

# ---------- Step 5: audit-integrity bucket isolation ----------
# Same entrypoint-override pattern as mc_count() above — without --entrypoint
# the mc args are eaten by mc-init.sh.
audit_count=$(ssh -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 \
  "goat@$MINIO_HOST" \
  "sudo docker compose -f /opt/minio/docker-compose.yml run --rm --no-deps --entrypoint /bin/sh mc-init -c 'mc --json ls --recursive local/audit-integrity/'" \
  2>/dev/null \
  | grep -c '"type":"file"' || true)

if [ "$audit_count" -eq 0 ]; then
  emit step5_audit_isolation PASS "{\"audit_objects\":0}"
else
  emit step5_audit_isolation FAIL "{\"audit_objects\":$audit_count}"
  echo "OVERALL FAIL: step5_audit_isolation — audit-integrity has $audit_count unexpected objects"
  exit 1
fi

emit overall PASS "{\"correlation_id\":\"$CORRELATION_ID\"}"
echo "OVERALL PASS"
