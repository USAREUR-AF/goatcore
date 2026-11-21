#!/usr/bin/env bash
# parity-teardown.sh — the parity gate for `goatbuilder teardown` against
# ops/scripts/goat-teardown.sh (ADR 1201). Both plan the same site's teardown
# from ops/goatbuilder/teardown.yaml — the full one (refused while a reached
# step holds data) and the services-only one from phase 12 — and every line is
# compared after the repository and home paths are masked and whitespace folded.
#
# usage: parity-teardown.sh <goatbuilder binary> <env>:<site> [<env>:<site> ...]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
BIN="${1:?}"; shift
[ $# -gt 0 ] || { echo "usage: parity-teardown.sh <binary> <env>:<site> ..."; exit 2; }

norm() {
  sed -E 's#\x1b\[[0-9;]*m##g' \
  | sed -E 's#'"$REPO"'#REPO#g; s#'"$HOME"'#HOME#g' \
  | grep -E '^(=== |    )' \
  | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//; s/[[:space:]]+/ /g'
}
script_side() { local site="$1"; shift; "$REPO/ops/scripts/goat-teardown.sh" "$site" --plan "$@" 2>&1 | norm || true; }
binary_side() {
  local env="$1" site="$2"; shift 2
  # The rows from -o json: the human table prints phase rows as headings.
  "$BIN" -C "$REPO" -o json teardown "$env" --site "$site" --dry-run "$@" 2>/dev/null \
  | python3 -c 'import json,sys
for c in json.load(sys.stdin)["checks"]:
    if c["name"] in ("phase",): print(c["detail"])
    elif c["name"] in ("cmd", "note"): print("    " + c["detail"])' \
  | norm || true
}

fail=0
for pair in "$@"; do
  env="${pair%%:*}"; site="${pair#*:}"
  for variant in "" "--services --from 12"; do
    # shellcheck disable=SC2086  # the variant is two or three words on purpose
    a="$(script_side "$site" $variant)"; b="$(binary_side "$env" "$site" $variant)"
    label="teardown $env:$site${variant:+ $variant}"
    if [ -z "$a" ]; then fail=1; echo "PARITY FAIL: $label — the script produced no plan"; continue; fi
    if [ "$a" != "$b" ]; then
      fail=1; echo "PARITY FAIL: $label"
      python3 "$HERE/parity-line-diff.py" <(printf '%s\n' "$a") <(printf '%s\n' "$b") script goatbuilder | sed 's/^/    /'
    else
      echo "parity ok: $label ($(printf '%s\n' "$a" | grep -c '^=== Teardown of') steps, $(printf '%s\n' "$a" | grep -c .) lines identical)"
    fi
  done
done
exit $fail
