#!/usr/bin/env bash
# parity-standup.sh — the parity gate for `goatbuilder standup` against
# ops/scripts/goat-standup.sh (goatbuilder-conformance.md §The parity gate).
#
# Both sides plan the same site — the script with --plan, the binary with
# --dry-run — and every phase header, every command, every obs-leg line and
# every rescue-class skip is compared in order after normalisation: the
# repository path and the home directory are masked, runs of whitespace are
# collapsed (the script's multi-line command strings fold their continuation
# indents into the command), and the resume instruction each side prints in
# its manual steps is reduced to its phase number (the script names itself,
# the binary names itself). Banners and the plan footer are not compared.
#
# usage: parity-standup.sh <goatbuilder binary> <env>:<site> [<env>:<site> ...]
#        e.g. parity-standup.sh crates/target/debug/goatbuilder <env>:<site>
#        parity-standup.sh --self-test
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"

# The self-test runs this gate against stand-in binaries on one site and checks
# what it reports when goatbuilder does not print a plan: a refusal is a
# PARITY FAIL that names goatbuilder's code and message, output that is not JSON
# is a PARITY FAIL that says so, and a plan that differs is a PARITY FAIL with
# the diff. None of them may print a Python traceback.
self_test() {
  local bad=0 n=0 t out rc
  t="$(mktemp -d "${TMPDIR:-/tmp}/parity-standup-self-test.XXXXXX")"
  # shellcheck disable=SC2064  # the path is fixed now, on purpose
  trap "rm -rf '$t'" EXIT
  stand_in() { printf '#!/usr/bin/env bash\ncat <<'"'"'EOF'"'"'\n%s\nEOF\n' "$2" >"$t/$1"; chmod +x "$t/$1"; }
  stand_in refuses '{"schema": "goatbuilder.error/v1", "error": "cannot read config ./ops/goatbuilder/goatbuilder.yaml: No such file or directory (os error 2)", "code": "GB-244"}'
  stand_in prints-text 'goatbuilder: something went wrong'
  stand_in differs '{"schema": "goatbuilder.output/v1", "verb": "standup", "ok": true, "exit": 0, "checks": [{"name": "phase", "ok": true, "detail": "=== Phase 1 [GATE] not the script'"'"'s phase ==="}]}'
  case_() {
    n=$((n+1))
    rc=0; out="$("$0" "$t/$1" zagros_dev:zagros-dev 2>&1)" || rc=$?
    if [ "$rc" != 1 ]; then bad=$((bad+1)); echo "SELF-TEST FAILED: $1: exit $rc, expected 1"; fi
    if printf '%s\n' "$out" | grep -q 'Traceback'; then bad=$((bad+1)); echo "SELF-TEST FAILED: $1: printed a Python traceback"; fi
    if ! printf '%s\n' "$out" | grep -qF -- "$2"; then bad=$((bad+1)); echo "SELF-TEST FAILED: $1: no line containing: $2"; printf '%s\n' "$out" | sed 's/^/    /' | head -8; fi
  }
  case_ refuses 'PARITY FAIL: standup zagros_dev:zagros-dev — goatbuilder refused before planning: GB-244 cannot read config ./ops/goatbuilder/goatbuilder.yaml'
  case_ prints-text 'PARITY FAIL: standup zagros_dev:zagros-dev — goatbuilder printed no JSON plan'
  case_ differs 'PARITY FAIL: standup zagros_dev:zagros-dev'
  # A difference past the 220th character, where the diff used to be cut, is
  # named by its column, with the text around it on both sides.
  n=$((n+1))
  printf 'same\n%s\n' "$(printf 'x%.0s' $(seq 229))A tail" >"$t/left"
  printf 'same\n%s\n' "$(printf 'x%.0s' $(seq 229))B tail" >"$t/right"
  out="$(python3 "$HERE/parity-line-diff.py" "$t/left" "$t/right" script goatbuilder 2>&1)" || true
  for want in 'line 2, column 230' 'script:' 'goatbuilder:' 'xA tail' 'xB tail'; do
    if ! printf '%s\n' "$out" | grep -qF -- "$want"; then bad=$((bad+1)); echo "SELF-TEST FAILED: late difference: no line containing: $want"; printf '%s\n' "$out" | sed 's/^/    /' | head -6; fi
  done
  if [ "$bad" = 0 ]; then echo "parity-standup: self-test ok, $n cases"; else echo "parity-standup: self-test failed, $bad check(s) in $n cases"; return 1; fi
}
if [ "${1:-}" = "--self-test" ]; then self_test; exit $?; fi

BIN="${1:?}"; shift
[ $# -gt 0 ] || { echo "usage: parity-standup.sh <binary> <env>:<site> ..."; exit 2; }

norm() {
  sed -E 's#\x1b\[[0-9;]*m##g' \
  | sed -E 's#'"$REPO"'#REPO#g; s#'"$HOME"'#HOME#g' \
  | sed -E 's#ops/scripts/goat-standup\.sh [a-z0-9-]+ --from#RESUME --from#g; s#goatbuilder standup [a-z0-9_]+ --site [a-z0-9-]+ --from#RESUME --from#g' \
  | sed -E 's#ops/scripts/goat-standup\.sh [a-z0-9-]+ --obs-only#RESUME --obs-only#g' \
  | grep -E '^(=== |    )' \
  | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//; s/[[:space:]]+/ /g'
}
script_side() { "$REPO/ops/scripts/goat-standup.sh" "$1" --plan 2>&1 | norm; }
binary_side() {
  # The rows from -o json: the human table prints phase rows as headings.
  # A refusal prints goatbuilder's error envelope instead of rows; the gate
  # reports that by its code (exit 3), and output that is not JSON by exit 4.
  local json rc=0
  json="$("$BIN" -C "$REPO" -o json standup "$1" --site "$2" --dry-run 2>/dev/null || true)"
  printf '%s' "$json" | python3 -c 'import json,sys
try:
    doc = json.load(sys.stdin)
except ValueError:
    sys.exit(4)
if not isinstance(doc, dict) or not isinstance(doc.get("checks"), list):
    if isinstance(doc, dict) and doc.get("error"):
        print(" ".join(x for x in (doc.get("code"), doc.get("error")) if x))
        sys.exit(3)
    sys.exit(4)
for c in doc["checks"]:
    if c["name"] in ("phase", "leg"): print(c["detail"])
    elif c["name"] in ("cmd", "note", "log"): print("    " + c["detail"])' || rc=$?
  return "$rc"
}

fail=0
for pair in "$@"; do
  env="${pair%%:*}"; site="${pair#*:}"
  a="$(script_side "$site" || true)"
  rc=0; raw="$(binary_side "$env" "$site")" || rc=$?
  if [ -z "$a" ]; then
    fail=1; echo "PARITY FAIL: standup $site — the script produced no phases (run it by hand with --plan to see why)"; continue
  fi
  case "$rc" in
    0) b="$(printf '%s\n' "$raw" | norm || true)" ;;
    3) fail=1; echo "PARITY FAIL: standup $env:$site — goatbuilder refused before planning: $raw"; continue ;;
    *) fail=1; echo "PARITY FAIL: standup $env:$site — goatbuilder printed no JSON plan (run it by hand: goatbuilder -o json standup $env --site $site --dry-run)"; continue ;;
  esac
  if [ "$a" != "$b" ]; then
    fail=1; echo "PARITY FAIL: standup $env:$site"
    # awk, not head: head closing the pipe early kills the gate under pipefail.
    { diff <(printf '%s\n' "$a") <(printf '%s\n' "$b") || true; } | sed 's/^/    /' | cut -c1-220 | awk 'NR <= 40'
  else
    echo "parity ok: standup $env:$site --dry-run ($(printf '%s\n' "$a" | grep -c '^=== ') phases, $(printf '%s\n' "$a" | grep -c .) lines identical)"
  fi
done
exit $fail
