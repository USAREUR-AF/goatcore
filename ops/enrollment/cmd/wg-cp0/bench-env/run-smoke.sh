#!/usr/bin/env bash
# wg-cp0 orchestrator bench-env smoke (Block 22R.2 Session A).
#
# Drives the wg-cp0 binary end-to-end against a representative
# fixture peerset.json + a synthetic mock live-state. Exercises the
# full subcommand surface (validate, list, show, diff, reconcile)
# in dry-run AND --apply paths and asserts:
#
#   - validate exits 0 on the canon fixture
#   - list prints the expected canonical peers
#   - show emits the expected detail for one peer
#   - diff exits 0 in offline mode (no relays in fixture so no drift)
#   - reconcile --dry-run prints the PLAN header
#   - reconcile --apply writes one audit-record JSONL row
#   - the audit-record schema matches the Block 22R.2 contract
#
# What this test does NOT cover (deferred):
#
#   - Real-relay docker integration (the existing
#     ops/wg-cp0-mesh-reach/ harness brings up wg-cp0 on alpine
#     containers but does not run sshd; the orchestrator's live-
#     state reader uses SSH-over-wg-cp0). Wiring an sshd-equipped
#     bench-env or a docker-exec-based LiveStateReader is a
#     follow-on track. The unit + apply tests in this directory
#     already cover the apply pipeline against MockLiveStateReader
#     + MockRelayMutator with realistic peer / relay / rotation /
#     unreachable shapes.
#
# Exit 0 = PASS, non-zero = FAIL (also tee'd to stderr with a hint).

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="$(mktemp -d -t wg-cp0-bench-XXXXXX)"
trap 'rm -rf "${WORK}"' EXIT

cd "${HERE}/../../.." # -> ops/enrollment

step() { printf '\033[1;34m[bench-env]\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31m[bench-env FAIL]\033[0m %s\n' "$*" >&2; exit 1; }
ok()   { printf '\033[1;32m[bench-env OK]\033[0m %s\n' "$*"; }

# ---------------------------------------------------------------------
# Phase 0: build
# ---------------------------------------------------------------------
step "Building wg-cp0 binary"
go build -o "${WORK}/wg-cp0" ./cmd/wg-cp0
WGCP0="${WORK}/wg-cp0"

FIXTURE="$(pwd)/peerset/testdata/v1-flat.json"
[[ -f "${FIXTURE}" ]] || fail "fixture missing at ${FIXTURE}"

# ---------------------------------------------------------------------
# Phase 1: validate
# ---------------------------------------------------------------------
step "Phase 1 — validate"
out=$("${WGCP0}" validate --path "${FIXTURE}")
echo "${out}"
echo "${out}" | grep -q "PASS" || fail "validate did not report PASS"
ok "validate PASS"

# ---------------------------------------------------------------------
# Phase 2: list (table) + list --json
# ---------------------------------------------------------------------
step "Phase 2 — list (table)"
out=$("${WGCP0}" list --path "${FIXTURE}")
for canon in lab-a lab-b prov-rla prov-conus op-laptop; do
    echo "${out}" | grep -q "${canon}" || fail "list missing canonical peer ${canon}"
done
ok "list table contains canonical peers"

step "Phase 2b — list --json (parseable)"
"${WGCP0}" list --path "${FIXTURE}" --json > "${WORK}/list.json"
peer_count=$(python3 -c "import json; print(len(json.load(open('${WORK}/list.json'))['peers']))")
[[ "${peer_count}" -eq 11 ]] || fail "list --json reports ${peer_count} peers, want 11"
ok "list --json has 11 peers"

# ---------------------------------------------------------------------
# Phase 3: show
# ---------------------------------------------------------------------
step "Phase 3 — show lab-a"
out=$("${WGCP0}" show lab-a --path "${FIXTURE}")
echo "${out}" | grep -q "198.18.0.1" || fail "show didn't print expected address"
ok "show lab-a contains expected address"

step "Phase 3b — show ghost (expected exit 1)"
if "${WGCP0}" show ghost --path "${FIXTURE}" 2>/dev/null; then
    fail "show ghost should have exited non-zero"
fi
ok "show ghost correctly exits non-zero"

# ---------------------------------------------------------------------
# Phase 4: diff (offline mode — no relays in fixture so empty plan)
# ---------------------------------------------------------------------
step "Phase 4 — diff (offline mode)"
WG_CP0_OFFLINE=1 "${WGCP0}" diff --path "${FIXTURE}" > "${WORK}/diff.out"
ok "diff exits 0 with offline mode + zero relays in fixture"

# ---------------------------------------------------------------------
# Phase 5: reconcile --dry-run
# ---------------------------------------------------------------------
step "Phase 5 — reconcile --dry-run"
WG_CP0_OFFLINE=1 "${WGCP0}" reconcile --path "${FIXTURE}" > "${WORK}/reconcile-dryrun.out"
grep -q "PLAN" "${WORK}/reconcile-dryrun.out" || fail "reconcile dry-run missing PLAN header"
ok "reconcile --dry-run prints PLAN header"

# ---------------------------------------------------------------------
# Phase 6: reconcile --apply (offline; exercises the audit-emit path)
# ---------------------------------------------------------------------
step "Phase 6 — reconcile --apply (offline; audit emit)"
AUDIT="${WORK}/audit.jsonl"
WG_CP0_AUDIT_PATH="${AUDIT}" WG_CP0_OFFLINE=1 \
    "${WGCP0}" reconcile --path "${FIXTURE}" --apply > "${WORK}/apply.out"
grep -q "apply: ok" "${WORK}/apply.out" || fail "reconcile --apply did not report 'apply: ok'"
[[ -s "${AUDIT}" ]] || fail "audit-emit did not write to ${AUDIT}"
audit_lines=$(wc -l < "${AUDIT}")
[[ "${audit_lines}" -eq 1 ]] || fail "expected 1 audit row, got ${audit_lines}"

# Schema-spot-check: the audit row has the required fields.
python3 - "${AUDIT}" <<'PY'
import json, sys
row = json.loads(open(sys.argv[1]).readline())
required = ["ts", "operator", "command", "intent_kind", "correlation_id",
            "plan_summary", "outcome", "per_step"]
missing = [k for k in required if k not in row]
if missing:
    print(f"audit row missing fields: {missing}", file=sys.stderr)
    sys.exit(1)
if row["intent_kind"] != "reconcile":
    print(f"intent_kind={row['intent_kind']!r}, want 'reconcile'", file=sys.stderr)
    sys.exit(1)
if row["outcome"] != "ok":
    print(f"outcome={row['outcome']!r}, want 'ok'", file=sys.stderr)
    sys.exit(1)
PY
ok "audit row schema PASS (intent_kind=reconcile, outcome=ok)"

# ---------------------------------------------------------------------
# Phase 7: idempotency — re-running --apply appends one more audit row
# ---------------------------------------------------------------------
step "Phase 7 — re-apply is idempotent + audit-grows-by-one"
WG_CP0_AUDIT_PATH="${AUDIT}" WG_CP0_OFFLINE=1 \
    "${WGCP0}" reconcile --path "${FIXTURE}" --apply > /dev/null
audit_lines=$(wc -l < "${AUDIT}")
[[ "${audit_lines}" -eq 2 ]] || fail "expected 2 audit rows after re-apply, got ${audit_lines}"
ok "re-apply appends one row (audit grew 1 → 2)"

# ---------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------
echo
ok "ALL PHASES PASS — wg-cp0 orchestrator bench-env smoke green"
