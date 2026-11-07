#!/usr/bin/env bash
# test-standup-waves.sh — goat-standup.sh's phase-12 wave executor (--parallel),
# exercised without a host: the functions are lifted from the script itself, so
# the test fails if the script's own code stops doing what it says.
#   1. independent steps run together (two 1 s sleeps finish in under 2 s);
#   2. a step waits for the steps it needs (it sees their marker files);
#   3. a failure stops the run after its wave and names the step.
# Bash 3.2 portable, like the script.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
SCRIPT="$REPO/ops/scripts/goat-standup.sh"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
fail=0

lift() { sed -n "/^$1() {/,/^}/p" "$SCRIPT"; }
lift dag_flush > "$T/fns.sh"
[ -s "$T/fns.sh" ] || { echo "FAIL: dag_flush not found in goat-standup.sh"; exit 1; }

run_case() {  # run_case <name> ; Q_* set by the caller via a here-doc file
  # shellcheck disable=SC2034  # read by the lifted dag_flush
  # GOAT_SECRET_STORE=env: the steps here read no secret, so no value store is
  # loaded around them (standup-phase-run loads one for env-file/sops/git-crypt).
  ( STATE_DIR="$T"; SITE=zagros; PROFILE=lan; export GOAT_SECRET_STORE=env
    # shellcheck disable=SC1090,SC1091
    . "$T/fns.sh"; . "$T/case.sh"; dag_flush ) >"$T/$1.out" 2>&1
}

# 1 + 2: a and b together, c after both.
cat > "$T/case.sh" <<EOF
Q_N=3
Q_ID[0]=a; Q_NEEDS[0]=''; Q_LABEL[0]=a; Q_CMD[0]='sleep 1; touch $T/a.done'
Q_ID[1]=b; Q_NEEDS[1]=''; Q_LABEL[1]=b; Q_CMD[1]='sleep 1; touch $T/b.done'
Q_ID[2]=c; Q_NEEDS[2]='a b'; Q_LABEL[2]=c; Q_CMD[2]='[ -f $T/a.done ] && [ -f $T/b.done ]'
EOF
t0=$(date +%s); run_case ok; rc=$?; dt=$(( $(date +%s) - t0 ))
if [ "$rc" = 0 ] && [ "$dt" -lt 3 ] && grep -q 'wave 1 of 2: a b' "$T/ok.out" && grep -q 'wave 2 of 2: c' "$T/ok.out"; then
  echo "ok: independent steps ran together (${dt}s) and c ran after a and b"
else
  fail=1; echo "FAIL: waves (rc $rc, ${dt}s)"; sed 's/^/    /' "$T/ok.out"
fi

# 3: a fails, c (needing a) never runs, the failure is named.
rm -f "$T"/*.done
cat > "$T/case.sh" <<EOF
Q_N=3
Q_ID[0]=a; Q_NEEDS[0]=''; Q_LABEL[0]=a; Q_CMD[0]='exit 3'
Q_ID[1]=b; Q_NEEDS[1]=''; Q_LABEL[1]=b; Q_CMD[1]='true'
Q_ID[2]=c; Q_NEEDS[2]='a'; Q_LABEL[2]=c; Q_CMD[2]='touch $T/c.ran'
EOF
run_case bad; rc=$?
if [ "$rc" != 0 ] && [ ! -f "$T/c.ran" ] && grep -q 'wave 1 of 2 FAILED: a' "$T/bad.out"; then
  echo "ok: a failed wave stops the run and names the step"
else
  fail=1; echo "FAIL: failure handling (rc $rc)"; sed 's/^/    /' "$T/bad.out"
fi
exit $fail
