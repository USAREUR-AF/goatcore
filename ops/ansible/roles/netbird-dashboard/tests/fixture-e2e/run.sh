#!/usr/bin/env bash
# ops/ansible/roles/netbird-dashboard/tests/fixture-e2e/run.sh
#
# Entry point for the Block 21D.1 dashboard E2E + perf harness.
#
# Three modes:
#
#   1. local  — bring up the full stack via docker-compose, drive Playwright against it.
#               (~3-5 min wall, ~3 GB RAM. Suitable for CI.)
#
#   2. live   — drive Playwright against a live deployment (vega-A via SSH tunnel
#               or a staged dashboard URL). Skips fixture bring-up.
#               Requires the operator to have a working tunnel + /etc/hosts entries
#               + the dev-CA root trusted in their browser keychain.
#
#   3. perf   — same as live but runs N times (default 5), aggregates per-phase
#               percentiles, emits a single summary JSONL for trend tracking.
#
# Usage:
#   run.sh local
#   run.sh live  https://dashboard.vega.netbird-prod.goat.example  user@example.com  Passw0rd
#   run.sh perf  https://dashboard.vega.netbird-prod.goat.example  user@example.com  Passw0rd  10
#
# Reads E2E_* env vars per ../playwright/e2e.mjs.

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
PLAYWRIGHT_DIR="$HERE/playwright"

mode="${1:?usage: run.sh local|live|perf [args...]}"
shift

# ----- Mode: local (fixture stack) ---------------------------------------------------------------
if [[ "$mode" == "local" ]]; then
  if ! [[ -f "$HERE/docker-compose.yml" ]]; then
    echo "FATAL: $HERE/docker-compose.yml not present yet — local fixture is documented in README.md but not yet authored." >&2
    echo "       Use 'live' mode against an existing deployment, or contribute the fixture compose file." >&2
    exit 64
  fi
  export DASHBOARD_BASE_URL="${DASHBOARD_BASE_URL:-https://dashboard.localhost.test}"
  export E2E_OPERATOR_EMAIL="${E2E_OPERATOR_EMAIL:-e2e-test@localhost.test}"
  export E2E_OPERATOR_PASSWORD="${E2E_OPERATOR_PASSWORD:-e2e-test-password-fixed-1234}"
  # docker-compose.yml pulls netbird-mgmt from the image namespace GHCR_ORG.
  # Same rule as every deploy script: the caller's GHCR_ORG wins, else ghcr_org
  # from ops/ansible/group_vars/all.yml, the namespace this checkout deploys from.
  if [[ -z "${GHCR_ORG:-}" ]]; then
    GHCR_ORG="$(sed -nE "s/^ghcr_org:.*[\"' ]([A-Za-z0-9][A-Za-z0-9._-]*)[\"']?[ }\"]*\$/\\1/p" \
      "$HERE/../../../../group_vars/all.yml" 2>/dev/null | head -1 || true)"
  fi
  if [[ -z "${GHCR_ORG:-}" ]]; then
    echo "FATAL: no image namespace. Export GHCR_ORG, or set ghcr_org in ops/ansible/group_vars/all.yml." >&2
    exit 64
  fi
  export GHCR_ORG
  # Container-log capture lives in EXIT trap so it survives the
  # `docker compose up -d --wait` failure path. compose --wait fails
  # fast when a service exits before its healthcheck (kratos config
  # parse error class — exits in <1s), and previously the trap was
  # only registered AFTER compose up, so on early failure no logs
  # were captured. Trap registers FIRST now; cleanup is no-op-safe
  # if compose hasn't started yet.
  log_dump_dir="/tmp/dashboard-e2e-container-logs"
  rm -rf "$log_dump_dir"
  mkdir -p "$log_dump_dir"
  cleanup_fixture() {
    local rc=$?
    echo "[run] capturing container logs (rc=$rc)..."
    (
      cd "$HERE" || exit 0
      docker compose ps -a > "$log_dump_dir/ps.txt" 2>&1 || true
      for svc in postgres kratos-migrate kratos hydra-migrate hydra selfservice-ui netbird-mgmt dashboard traefik; do
        docker compose logs --no-color --timestamps "$svc" > "$log_dump_dir/$svc.log" 2>&1 || true
      done
      echo "[run] tearing down fixture stack..."
      docker compose down -v || true
    )
    return $rc
  }
  trap cleanup_fixture EXIT
  # F-085 ordering fix: seed.sh writes config files (kratos.yaml,
  # hydra.yaml, identity-schema.json, init-databases.sql, traefik-
  # dynamic.yml, TLS cert) that the compose containers bind-mount.
  # Phase 1 of seed.sh (config + cert) MUST run before compose up
  # or postgres exits with code 1 trying to bind-mount a not-yet-
  # existing init-databases.sql. Phase 2 (Hydra OIDC client +
  # Kratos identity) runs AFTER services are up.
  echo "[run] seed phase 1 (config + cert) ..."
  cd "$HERE" && SEED_PHASE=config bash "$HERE/seed.sh"
  echo "[run] bringing up fixture stack..."
  cd "$HERE" && docker compose up -d --wait
  echo "[run] seed phase 2 (runtime — Hydra client + Kratos identity) ..."
  cd "$HERE" && SEED_PHASE=runtime bash "$HERE/seed.sh"
  echo "[run] running Playwright e2e..."
  cd "$PLAYWRIGHT_DIR" && npm install --silent && node e2e.mjs
  exit $?
fi

# ----- Mode: live (drive against existing URL) ---------------------------------------------------
if [[ "$mode" == "live" ]]; then
  url="${1:?live mode requires URL as 1st arg}"
  email="${2:?live mode requires operator email as 2nd arg}"
  password="${3:?live mode requires operator password as 3rd arg}"
  export DASHBOARD_BASE_URL="$url"
  export E2E_OPERATOR_EMAIL="$email"
  export E2E_OPERATOR_PASSWORD="$password"
  cd "$PLAYWRIGHT_DIR" && npm install --silent && node e2e.mjs
  exit $?
fi

# ----- Mode: perf (N-run aggregator) -------------------------------------------------------------
if [[ "$mode" == "perf" ]]; then
  url="${1:?perf mode requires URL as 1st arg}"
  email="${2:?perf mode requires operator email as 2nd arg}"
  password="${3:?perf mode requires operator password as 3rd arg}"
  n="${4:-5}"
  export DASHBOARD_BASE_URL="$url"
  export E2E_OPERATOR_EMAIL="$email"
  export E2E_OPERATOR_PASSWORD="$password"
  cd "$PLAYWRIGHT_DIR" && npm install --silent
  agg="/tmp/dashboard-perf-$(date -u +%Y%m%dT%H%M%SZ).jsonl"
  echo "[perf] running $n iterations; aggregate -> $agg"
  for i in $(seq 1 "$n"); do
    echo "[perf] iteration $i/$n"
    out="/tmp/dashboard-perf-iter-$i.jsonl"
    E2E_OUT_JSONL="$out" node "$PLAYWRIGHT_DIR/e2e.mjs" || true
    cat "$out" >> "$agg"
    sleep 2
  done
  echo "[perf] aggregate: $agg"
  echo "[perf] computing percentiles..."
  python3 - <<PY
import json
from pathlib import Path
agg = Path("$agg")
phases = {}
totals = []
for line in agg.read_text().splitlines():
    if not line.strip(): continue
    e = json.loads(line)
    if e.get("event") == "phase_summary":
        totals.append(e["total_duration_ms"])
        for name, info in e["phases"].items():
            phases.setdefault(name, []).append(info["duration_ms"])
def pct(xs, p):
    xs = sorted(xs); k = int(round((len(xs)-1) * p/100))
    return xs[k] if xs else None
print(f"  N runs: {len(totals)}")
print(f"  total p50/p95/p99: {pct(totals,50)}/{pct(totals,95)}/{pct(totals,99)} ms")
for name, durs in phases.items():
    print(f"  {name}: p50/p95/p99 = {pct(durs,50)}/{pct(durs,95)}/{pct(durs,99)} ms")
PY
  exit 0
fi

echo "FATAL: unknown mode '$mode'. usage: run.sh local|live|perf [args...]" >&2
exit 64
