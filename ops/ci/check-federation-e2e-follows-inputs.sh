#!/usr/bin/env bash
# check-federation-e2e-follows-inputs.sh — the four cases of docs/design/ci-results-follow-inputs.md
# for the two-sovereign end-to-end job (.github/workflows/federation-e2e.yml).
#
#   ops/ci/check-federation-e2e-follows-inputs.sh            the static checks and the key cases
#   ops/ci/check-federation-e2e-follows-inputs.sh --run BIN_DIR
#                                                            also case 1 measured: run.sh, all
#                                                            four scenarios, with every host
#                                                            denied the distribution mirrors and
#                                                            PyPI (BIN_DIR holds a built goat and
#                                                            goatbuilder)
#
# Each case prints PASS or FAIL with what it read. Case 2 and the reuse of case 3 are shown
# by named CI runs (the workflow's header comment); here they are checked for the shape that
# makes them possible. Exit 0 only when every case run here passes.
set -uo pipefail
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
WF="$REPO/.github/workflows/federation-e2e.yml"
KEY="$REPO/ops/ci/federation-e2e-key.sh"
RUN_BIN=""
[ "${1:-}" = --run ] && RUN_BIN="${2:?--run needs a directory with goat and goatbuilder}"
FAILS=0
pass() { echo "PASS  $*"; }
fail() { echo "FAIL  $*"; FAILS=$((FAILS + 1)); }

# --- case 1, rule 1: inputs are fixed --------------------------------------------------------
# Static: nothing in the job or its hosts is fetched by a floating name while it runs.
apt_out="$(python3 - "$WF" <<'PY'
import re, sys, yaml
wf = yaml.safe_load(open(sys.argv[1]))
for j, job in wf.get("jobs", {}).items():
    for st in job.get("steps", []):
        if re.search(r"(^|[^-])apt-get( |$)", st.get("run", "")) and "declared input" not in st.get("name", ""):
            print(f"{j}: {st.get('name', '(unnamed step)')}")
PY
)"
if [ -n "$apt_out" ]; then fail "case 1: apt-get in a step that is not a declared input: $(echo "$apt_out" | head -1)"
else pass "case 1: apt-get only in a step named as a declared input"; fi
if grep -nE 'pip[0-9]? install' "$WF" | grep -v -- '--require-hashes' >/dev/null; then
  fail "case 1: a pip install without --require-hashes: $(grep -nE 'pip[0-9]? install' "$WF" | grep -v -- '--require-hashes' | head -1)"
else pass "case 1: every pip install in the workflow requires hashes"; fi
floating="$(grep -nE '^\s*-?\s*uses:\s*[^ ]+@' "$WF" | grep -vE '@[0-9a-f]{40}(\s|$)' || true)"
if [ -n "$floating" ]; then fail "case 1: an action not pinned to a commit: $(echo "$floating" | head -1 | sed 's/^ *//')"
else pass "case 1: every action is pinned to a commit"; fi
if grep -qE "toolchain:\s*'?[0-9]+\.[0-9]+\.[0-9]+'?" "$WF" && ! grep -qE 'toolchain:\s*.?stable|rust-toolchain@stable' "$WF"; then
  pass "case 1: the Rust toolchain is an exact version ($(grep -oE "toolchain:\s*'?[0-9]+\.[0-9]+\.[0-9]+'?" "$WF" | head -1))"
else fail "case 1: no exact Rust toolchain version in the workflow"; fi
if grep -qE '^FROM [^ ]+@sha256:[0-9a-f]{64}' "$REPO/ops/federation-e2e/host/Dockerfile"; then
  pass "case 1: the host image's base is named by digest"
else fail "case 1: the host image's base is not named by digest"; fi
pkgs="$(sed -n 's/.*FE2E_HOST_PACKAGES:-\([^}]*\)}.*/\1/p' "$REPO/ops/federation-e2e/lib/engine.sh" | head -1)"
missing=""
for p in $pkgs; do grep -qE "(^|[[:space:]])$p([[:space:]]|\\\\|$)" "$REPO/ops/federation-e2e/host/Dockerfile" || missing="$missing $p"; done
if [ -n "$missing" ]; then fail "case 1: the host image lacks what a scenario needs at run time:$missing"
else pass "case 1: the host image carries every package a scenario needs ($pkgs)"; fi
if grep -q 'FE2E_HOST_APT:-0' "$REPO/ops/federation-e2e/lib/engine.sh"; then
  pass "case 1: run-time apt in a host only when a lab asks for it (FE2E_HOST_APT=1)"
else fail "case 1: a host may fetch from the mirror at run time"; fi
# Measured: the whole job's run.sh, all four scenarios, with the hosts denied every
# distribution mirror and PyPI.
if [ -n "$RUN_BIN" ]; then
  W="$(mktemp -d "${TMPDIR:-/tmp}/fe2e-proof.XXXXXX")"
  FE2E_DENY_MIRROR=1 "$REPO/ops/federation-e2e/run.sh" --goat "$RUN_BIN/goat" --goatbuilder "$RUN_BIN/goatbuilder" \
    --work "$W/work" --record "$W/report" > "$W/run.log" 2>&1
  rc=$?
  if [ "$rc" = 0 ]; then pass "case 1 measured: all four scenarios pass with the hosts denied the mirrors (record $W/report)"
  else fail "case 1 measured: run.sh exit $rc with the hosts denied the mirrors (log $W/run.log)"; fi
fi

# --- case 2, rule 2: stages keep their results ---------------------------------------------------
# The shape that lets "run failed jobs" repeat only the failed scenario: a build job, a job per
# scenario that needs it, and the required check as a job that needs every scenario.
python3 - "$WF" <<'PY' || FAILS=$((FAILS + 1))
import sys, yaml
wf = yaml.safe_load(open(sys.argv[1])); jobs = wf.get("jobs", {})
need = lambda j: [jobs[j].get("needs")] if isinstance(jobs[j].get("needs"), str) else (jobs[j].get("needs") or [])
required = "two sovereigns through the real driver, proved from the receiving side"
gate = [j for j, v in jobs.items() if v.get("name") == required]
builds = [j for j in jobs if not need(j) and j not in gate]
scen = [j for j in jobs if j not in gate and need(j)]
def legs(j):  # a matrix job is one job per leg
    m = (jobs[j].get("strategy") or {}).get("matrix") or {}
    n = 1
    for k, v in m.items():
        if isinstance(v, list) and k not in ("include", "exclude"): n *= len(v)
    return n
nscen = sum(legs(j) for j in scen)
nofail = all(((jobs[j].get("strategy") or {}).get("fail-fast") is False) for j in scen if legs(j) > 1)
ok = len(gate) == 1 and builds and nscen >= 2 and nofail and all(set(need(s)) & set(builds) for s in scen) \
     and set(scen) <= set(need(gate[0])) and jobs[gate[0]].get("if", "").replace(" ", "") in ("always()", "${{always()}}")
if ok:
    print(f"PASS  case 2: build job(s) {builds}, {nscen} scenario jobs (fail-fast off), the required check needs them all and always runs")
else:
    print(f"FAIL  case 2: the job is not split into a build, one job per scenario and the required check over them "
          f"(jobs: {list(jobs)})"); sys.exit(1)
PY

# --- cases 3 and 4, rule 3: the key ---------------------------------------------------------------
if [ ! -x "$KEY" ]; then
  fail "case 3: no key computation ($KEY)"
  fail "case 4: no key computation ($KEY)"
else
  T="$(mktemp -d "${TMPDIR:-/tmp}/fe2e-key.XXXXXX")"
  git -C "$REPO" worktree add -q --detach "$T/tree" HEAD >/dev/null 2>&1 || { fail "case 3: cannot make a scratch tree"; exit 1; }
  k1="$("$KEY" "$T/tree" scenario-2)"; k2="$("$KEY" "$T/tree" scenario-2)"
  if [ -n "$k1" ] && [ "$k1" = "$k2" ]; then pass "case 3: identical inputs give one key ($k1)"
  else fail "case 3: identical inputs gave two keys ($k1, $k2)"; fi
  # Each declared class, changed alone, must give a new key.
  for f in .github/workflows/federation-e2e.yml ops/federation-e2e/run.sh ops/ci/check-federation-e2e-prepare.sh \
           ops/federation/standup/acceptance.sh crates/Cargo.lock ops/federation-e2e/host/Dockerfile \
           ops/federation-e2e/requirements.lock \
           crates/goatbuilder/src/config.rs ops/ansible/roles/rollback-ledger/tasks/main.yml; do
    echo "# proof" >> "$T/tree/$f"
    k="$("$KEY" "$T/tree" scenario-2)"
    git -C "$T/tree" checkout -q -- "$f"
    if [ "$k" != "$k1" ]; then pass "case 4: a declared input changed ($f): a new key, a full run"
    else fail "case 4: a declared input changed ($f) and the key did not"; fi
  done
  sed -i "s/toolchain: '\([0-9.]*\)'/toolchain: '0.0.1'/" "$T/tree/.github/workflows/federation-e2e.yml"
  k="$("$KEY" "$T/tree" scenario-2)"
  git -C "$T/tree" checkout -q -- .github/workflows/federation-e2e.yml
  if [ "$k" != "$k1" ]; then pass "case 4: the toolchain changed: a new key, a full run"
  else fail "case 4: the toolchain changed and the key did not"; fi
  echo "proof" >> "$T/tree/README.md"
  k4="$("$KEY" "$T/tree" scenario-2)"
  if [ "$k4" = "$k1" ]; then pass "case 4: a path outside the declaration changed (README.md): the same key, the pass is reused"
  else fail "case 4: a path outside the declaration changed the key"; fi
  git -C "$T/tree" checkout -q -- README.md
  git -C "$REPO" worktree remove "$T/tree" >/dev/null 2>&1 || true
fi

echo
[ "$FAILS" = 0 ] && { echo "ok: the federation e2e job follows its inputs (cases run here)"; exit 0; }
echo "the federation e2e job does not yet follow its inputs: $FAILS case check(s) failed"
exit 1
