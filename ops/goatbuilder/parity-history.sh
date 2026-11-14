#!/usr/bin/env bash
# parity-history.sh — the parity gate for `goatbuilder history` against
# `ops/scripts/rollback.sh --list` (goatbuilder-conformance.md §The parity gate;
# ADR 1201: both avenues stay functional).
#
# Both print a component's deploy ledger newest first, every row in the same
# shape. Where no host ledger is reachable — CI — both fall back to the role's
# in-repo deploy-history.yml, and the two are compared line for line for every
# component that carries one. On a laptop with reach, `history` reads every host
# of the named goatnet while `rollback --list` reads the first reachable host of
# any goatnet, so the gate is meaningful on the fallback path, which is the one
# it pins. `history` adds an action/prior/operator line under each on-host row;
# those lines are not part of the comparison.
#
# usage: parity-history.sh <goatbuilder binary> <goatnet> [component ...]
#        parity-history.sh --self-test
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
# goatbuilder prints each refusal's catalog code on a line of its own after it,
# `  (GB-NNN — goatbuilder explain GB-NNN)`; rollback.sh has no codes. That one
# line is taken off goatbuilder's side, and everything else is compared as it is.
binary_filter() { sed -E 's/^goatbuilder: //; s/^(ok  |FAIL)  history  //; /^history: (ok|refused|not converged)/d; /^      action=/d; /^[[:space:]]*\(GB-[0-9]+ — goatbuilder explain GB-[0-9]+\)$/d' | sed -E 's/[[:space:]]+$//'; }
# The self-test: the code line is the only thing taken off goatbuilder's side, so
# a real difference still fails. Each case is script output, goatbuilder output,
# and whether the two must compare equal.
code_line='  (GB-260 — goatbuilder explain GB-260)'
self_test() {
  local bad=0 a b want got
  check() {
    a="$(printf '%s\n' "$2" | sed -E 's/[[:space:]]+$//')"; b="$(printf '%s\n' "$3" | binary_filter)"
    want="$4"; if [ "$a" = "$b" ]; then got=same; else got=differ; fi
    if [ "$got" != "$want" ]; then bad=$((bad+1)); echo "SELF-TEST FAILED: $1: expected $want, got $got"; fi
  }
  check "the refusal with its code line" "deploy-history.yml has no hosts entries" \
    "goatbuilder: deploy-history.yml has no hosts entries
$code_line" same
  check "a listing row that differs, code line or not" "  1  2026-01-02  v1.2.3  a deploy" \
    "  1  2026-01-02  v1.2.4  a deploy
$code_line" differ
  check "a refusal in other words" "deploy-history.yml has no hosts entries" \
    "goatbuilder: deploy-history.yml has no host entries
$code_line" differ
  check "a parenthesised line that is not the code line" "deploy-history.yml has no hosts entries" \
    "goatbuilder: deploy-history.yml has no hosts entries
  (GB-260 — see the ledger)" differ
  check "the code line's words inside a row" "deploy-history.yml has no hosts entries" \
    "goatbuilder: deploy-history.yml has no hosts entries (GB-260 — goatbuilder explain GB-260)" differ
  if [ "$bad" = 0 ]; then echo "parity-history: self-test ok, 5 cases"; else echo "parity-history: self-test failed, $bad of 5 cases"; return 1; fi
}
if [ "${1:-}" = "--self-test" ]; then self_test; exit $?; fi
BIN="${1:?usage: parity-history.sh <goatbuilder binary> <goatnet> [component ...]}"; shift
ENV="${1:?usage: parity-history.sh <goatbuilder binary> <goatnet> [component ...]}"; shift
COMPONENTS=("$@")
if [ ${#COMPONENTS[@]} -eq 0 ]; then
  for f in "$REPO"/ops/ansible/roles/*/deploy-history.yml; do COMPONENTS+=("$(basename "$(dirname "$f")")"); done
fi
command -v yq >/dev/null || { echo "parity-history: yq required by the script side (brew install yq / apt-get install yq)"; exit 2; }

script_side() { "$REPO/ops/scripts/rollback.sh" "$1" --list 2>&1 | sed -E 's/^rollback\.sh: //' | sed -E 's/[[:space:]]+$//'; }
binary_side() { "$BIN" -C "$REPO" history "$1" --env "$ENV" 2>&1 | binary_filter; }

fail=0; n=0
for c in "${COMPONENTS[@]}"; do
  n=$((n+1))
  a="$(script_side "$c" || true)"; b="$(binary_side "$c" || true)"
  if [ "$a" != "$b" ]; then
    fail=$((fail+1)); echo "PARITY FAIL: history $c"
    { diff <(printf '%s\n' "$a") <(printf '%s\n' "$b") || true; } | sed 's/^/    /' | head -12
  fi
done
[ "${n:-0}" -gt 0 ] || { echo "parity FAIL: nothing was compared (no inputs read — is PyYAML importable by python3?)"; exit 1; }
if [ "$fail" = 0 ]; then echo "parity ok: $n components list identically (rollback.sh --list vs goatbuilder history --env $ENV)"; else echo "parity FAIL: $fail of $n differ"; exit 1; fi
