#!/usr/bin/env bash
# parity-pins-member-over.sh — ADR 1256 on both avenues: a member deviation
# names the goatnet value it overrides (`over:`), and a stale or missing one
# is refused the same way by ops/pins/generate-pins.py and `goatbuilder pins`.
#
# Each case in ops/goatbuilder/fixtures/pins-member-over/*.yaml is a whole pin
# table for the synthetic goatnet `zagros`, with an `# expect: <exit> [needle]`
# line. For each case, on two scratch copies of the tree (nothing committed
# moves), both avenues regenerate. The case passes when:
#   - both exit with the expected code, and the message carries the needle
#     (an error code such as GB-357, or WARN for a warning);
#   - both print the same refusal or warning lines;
#   - on exit 0, every generated group_vars/<member>/pins.yml is byte-identical.
#
# usage: parity-pins-member-over.sh <goatbuilder binary>
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
BIN="${1:?usage: parity-pins-member-over.sh <goatbuilder binary>}"
CASES="$REPO/ops/goatbuilder/fixtures/pins-member-over"
SCRATCH="$(mktemp -d)"; trap 'rm -rf "$SCRATCH"' EXIT

tree() { # $1 destination, $2 case table
  mkdir -p "$1"
  ( cd "$REPO" && for d in ops/pins ops/ansible/group_vars ops/ansible/roles ops/consistency ops/enrollment/public-keys ops/release; do [ -e "$d" ] && printf "%s\n" "$d"; done | tar cf - -T - ) | ( cd "$1" && tar xf - )
  mkdir -p "$1/ops/ansible/group_vars/zagros_a_hosts" "$1/ops/ansible/group_vars/zagros_b_hosts"
  cp "$2" "$1/ops/pins/fleet-pins.yaml"
}
# The refusal or warning lines, with each avenue's own prefix taken off.
lines() { sed -E 's/^(ERROR: |goatbuilder: )//' | grep -E '^(GB-35[78]|WARN)' || true; }

fail=0; n=0
for case in "$CASES"/*.yaml; do
  name="$(basename "$case" .yaml)"; n=$((n + 1))
  read -r want needle < <(sed -n 's/^# expect: //p' "$case")
  tree "$SCRATCH/$name/py" "$case"; tree "$SCRATCH/$name/rs" "$case"
  set +e
  py="$( cd "$SCRATCH/$name/py" && python3 ops/pins/generate-pins.py 2>&1 )"; py_rc=$?
  rs="$( "$BIN" -C "$SCRATCH/$name/rs" pins --yes 2>&1 )"; rs_rc=$?
  set -e
  bad=""
  [ "$py_rc" = "$want" ] || bad="$bad generate-pins.py exit $py_rc (want $want);"
  [ "$rs_rc" = "$want" ] || bad="$bad goatbuilder exit $rs_rc (want $want);"
  if [ -n "${needle:-}" ]; then
    printf '%s\n' "$py" | grep -q -- "$needle" || bad="$bad generate-pins.py says no $needle;"
    printf '%s\n' "$rs" | grep -q -- "$needle" || bad="$bad goatbuilder says no $needle;"
  fi
  [ "$(printf '%s\n' "$py" | lines)" = "$(printf '%s\n' "$rs" | lines)" ] || bad="$bad the two avenues print different lines;"
  if [ "$want" = 0 ]; then
    for m in zagros_a_hosts zagros_b_hosts; do
      cmp -s "$SCRATCH/$name/py/ops/ansible/group_vars/$m/pins.yml" "$SCRATCH/$name/rs/ops/ansible/group_vars/$m/pins.yml" \
        || bad="$bad $m/pins.yml differs;"
    done
  fi
  if [ -n "$bad" ]; then
    fail=1; echo "FAIL $name:$bad"
    printf '  py: %s\n' "$(printf '%s\n' "$py" | lines | head -3)"
    printf '  rs: %s\n' "$(printf '%s\n' "$rs" | lines | head -3)"
  else
    echo "ok   $name (exit $want${needle:+, $needle})"
  fi
done
[ "$n" -gt 0 ] || { echo "FAIL: no case under $CASES"; exit 1; }
exit $fail
