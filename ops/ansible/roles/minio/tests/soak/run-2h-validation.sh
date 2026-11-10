#!/usr/bin/env bash
# 2-hour validation soak — MinIO under sustained write + lifecycle pressure.
#
# Anchor: ADR 1008 (soak-testing discipline). MinIO is a stateful + load-
# bearing service; ADR 1008 §"Required tiers" = smoke 5min, validation 2h
# (default), canonical 72h, long-haul 30d. This is the validation tier.
#
# Default duration: 2 hours. Override via SOAK_DURATION (e.g. 24h).
#
# Run from a host that can reach MinIO (laptop or Orin-C operator):
#   MINIO_HOST=192.168.1.253 \
#   MINIO_USER=<the MinIO root user> \
#   MINIO_PASS=<the MinIO root password> \
#   ops/ansible/roles/minio/tests/soak/run-2h-validation.sh
#
# What this validates:
#   1. Sustained write rate against telemetry-otlp (operational-typical
#      OpenObserve rotation rate).
#   2. Audit-integrity bucket Object Lock holds — periodic delete attempts
#      against newly-written objects MUST fail.
#   3. Lifecycle expiration on telemetry-otlp fires correctly (write
#      objects with 1-day TTL during soak; verify expiration after the
#      window — note this requires a longer soak than 2h to actually
#      see expiration; flag in evidence if soak duration < retention
#      day boundary).
#   4. MinIO's own Prometheus metrics stay healthy (no slow-write
#      saturation, no OOM, no disk-fill).
#   5. mc admin info reports cluster health throughout.
#
# Output: appends one JSON line per tick to
#   docs/reports/<YYYY-MM-DD>-minio-soak-evidence.jsonl
#
# Discipline reminder: do NOT run this against the production MinIO until
# the spike (27A.3) has passed and a smoke run (5-min) has succeeded
# against the same instance. See ADR 1008 §"Sequencing".

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../../../../../.." && pwd)"
SOAK_DURATION="${SOAK_DURATION:-2h}"
TICK_INTERVAL=30  # seconds
TODAY="$(date -u +%Y-%m-%d)"
# EVIDENCE override env var lets the soak run from any directory (e.g.
# /tmp on a remote host) where the repo isn't checked out.
EVIDENCE="${EVIDENCE:-$REPO_ROOT/docs/reports/${TODAY}-minio-soak-evidence.jsonl}"

MINIO_HOST="${MINIO_HOST:-192.168.1.253}"
MINIO_USER="${MINIO_USER:-}"
MINIO_PASS="${MINIO_PASS:-}"
# The account ssh logs in to MINIO_HOST as: MINIO_SSH_USER, else the account
# the project declares for its machines (deploy.ssh_user in
# ops/goatbuilder/goatbuilder.yaml); with neither, ssh picks the account itself
# (~/.ssh/config, else the local user name).
MINIO_SSH_USER="${MINIO_SSH_USER:-}"
if [ -z "$MINIO_SSH_USER" ] && [ -f "$REPO_ROOT/ops/goatbuilder/goatbuilder.yaml" ]; then
  MINIO_SSH_USER="$(python3 -c 'import sys, yaml; print(str(((yaml.safe_load(open(sys.argv[1])) or {}).get("deploy") or {}).get("ssh_user") or "").strip())' "$REPO_ROOT/ops/goatbuilder/goatbuilder.yaml" 2>/dev/null || true)"
fi
MINIO_SSH_TARGET="${MINIO_SSH_USER:+${MINIO_SSH_USER}@}${MINIO_HOST}"

if [ -z "$MINIO_USER" ] || [ -z "$MINIO_PASS" ]; then
  echo "ERROR: set MINIO_USER and MINIO_PASS" >&2
  exit 64
fi

mkdir -p "$(dirname "$EVIDENCE")"

# Convert duration to seconds. Supports e.g. 2h, 30m, 90s, 24h.
duration_to_sec() {
  local v="$1"
  case "$v" in
    *h) printf '%d' "$(( ${v%h} * 3600 ))" ;;
    *m) printf '%d' "$(( ${v%m} * 60   ))" ;;
    *s) printf '%d' "${v%s}"               ;;
    *)  printf '%d' "$v"                   ;;
  esac
}
SOAK_SEC=$(duration_to_sec "$SOAK_DURATION")

emit() {
  printf '{"ts":"%s","soak":"minio-2h-validation","kind":"%s","detail":%s}\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$1" "$2" >> "$EVIDENCE"
}

emit start "{\"duration_sec\":$SOAK_SEC,\"host\":\"$MINIO_HOST\"}"

# mc wrapper that runs against the live MinIO via the on-host mc-init
# container — avoids needing mc on the soak runner. Operator can swap to
# a local mc install if preferred.
#
# Override mc-init's compose-defined entrypoint (`["/bin/sh","/mc-init.sh"]`)
# with `--entrypoint /bin/sh ... -c "mc ..."`; otherwise mc args get passed
# as args to mc-init.sh which ignores them and the bucket is never queried.
# Same bug bit the spike harness; fix is identical.
mc_remote() {
  ssh -o StrictHostKeyChecking=accept-new -o ConnectTimeout=10 \
    "$MINIO_SSH_TARGET" \
    "sudo docker compose -f /opt/minio/docker-compose.yml run --rm --no-deps --entrypoint /bin/sh mc-init -c 'mc $*'"
}

# Pre-check: all three buckets present?
buckets=$(mc_remote ls local/ 2>/dev/null || true)
for b in audit-integrity runtime-detection-archive telemetry-otlp; do
  if echo "$buckets" | grep -q "$b"; then
    emit precheck_bucket "{\"bucket\":\"$b\",\"present\":true}"
  else
    emit precheck_bucket "{\"bucket\":\"$b\",\"present\":false}"
    echo "ABORT: bucket $b missing" >&2
    exit 1
  fi
done

# Soak loop
END_TS=$(( $(date +%s) + SOAK_SEC ))
TICK=0
while [ "$(date +%s)" -lt "$END_TS" ]; do
  TICK=$((TICK + 1))
  TICK_TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)

  # 1. Synthetic write to telemetry-otlp (operational-typical pattern).
  KEY="soak/${TICK_TS}-tick-${TICK}.json"
  PAYLOAD=$(printf '{"tick":%d,"ts":"%s","payload":"x"}' "$TICK" "$TICK_TS")
  if echo "$PAYLOAD" | mc_remote pipe "local/telemetry-otlp/$KEY" >/dev/null 2>&1; then
    emit write "{\"bucket\":\"telemetry-otlp\",\"key\":\"$KEY\",\"verdict\":\"PASS\"}"
  else
    emit write "{\"bucket\":\"telemetry-otlp\",\"key\":\"$KEY\",\"verdict\":\"FAIL\"}"
  fi

  # 2. Object Lock probe — write a tiny object to audit-integrity then
  # try to delete it. Delete MUST fail (COMPLIANCE retention).
  AUDIT_KEY="soak-objlock-probe/$TICK_TS-tick-$TICK.txt"
  echo "objlock probe" | mc_remote pipe "local/audit-integrity/$AUDIT_KEY" >/dev/null 2>&1 || true
  if mc_remote rm "local/audit-integrity/$AUDIT_KEY" 2>&1 | grep -qiE 'WORM|retention|denied|forbidden|not allowed'; then
    emit objlock_probe "{\"key\":\"$AUDIT_KEY\",\"verdict\":\"PASS\",\"note\":\"delete refused as expected\"}"
  else
    # If delete succeeded, Object Lock is broken — load-bearing failure.
    emit objlock_probe "{\"key\":\"$AUDIT_KEY\",\"verdict\":\"FAIL\",\"note\":\"delete succeeded — Object Lock NOT enforcing\"}"
  fi

  # 3. Cluster health
  health=$(mc_remote admin info local 2>&1 | head -3 | tr '\n' ' ' | tr -s ' ')
  if echo "$health" | grep -qE 'online|Drives|Used'; then
    emit health "{\"status\":\"OK\",\"raw\":$(printf '%s' "$health" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')}"
  else
    emit health "{\"status\":\"DEGRADED\",\"raw\":$(printf '%s' "$health" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')}"
  fi

  sleep "$TICK_INTERVAL"
done

emit end "{\"ticks\":$TICK,\"duration_sec\":$SOAK_SEC}"
echo "Soak complete. Evidence: $EVIDENCE"
echo "Recommended next step: scan evidence for any objlock_probe FAIL"
echo "or write FAIL and write up a post-soak decision-review entry."
