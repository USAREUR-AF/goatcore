#!/usr/bin/env bash
# parity-pins-opt-in.sh — an opt-in component (`opt_in: true`) on both avenues.
# It runs only on a goatnet that names it in its row's `opt_in:` list, and
# ops/pins/generate-pins.py and `goatbuilder pins` resolve it the same way.
#
# Two sets of cases, each on scratch copies of the tree (nothing committed moves):
#
#   1. ops/goatbuilder/fixtures/pins-opt-in/*.yaml, each a whole pin table for the
#      synthetic goatnet `zagros` with an `# expect: <exit> [needle]` line and an
#      optional `# has: <text>` line (text zagros_a_hosts' pins.yml must hold).
#      Both avenues exit as expected, print the needle, and on exit 0 write the
#      same pins.yml for every member and the same intent components.
#   2. Nothing else moves: for each of those cases at exit 0 where no goatnet
#      lists an opt-in component, and for each existing fixture goatnet at exit 0
#      (ops/goatbuilder/fixtures/pins-member-over/) with an opt-in component added,
#      every member's pins.yml and the intent's `components` are byte-identical to
#      the table with no opt-in component at all. Only the intent's `sources`
#      (the table's own hash) and its serial move.
#
# usage: parity-pins-opt-in.sh <goatbuilder binary>
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
BIN="${1:?usage: parity-pins-opt-in.sh <goatbuilder binary>}"
CASES="$REPO/ops/goatbuilder/fixtures/pins-opt-in"
EXISTING="$REPO/ops/goatbuilder/fixtures/pins-member-over"
SCRATCH="$(mktemp -d)"; trap 'rm -rf "$SCRATCH"' EXIT
MEMBERS="zagros_a_hosts zagros_b_hosts"

tree() { # $1 destination, $2 table
  mkdir -p "$1"
  ( cd "$REPO" && for d in ops/pins ops/ansible/group_vars ops/ansible/roles ops/consistency ops/enrollment/public-keys ops/release; do [ -e "$d" ] && printf "%s\n" "$d"; done | tar cf - -T - ) | ( cd "$1" && tar xf - )
  for m in $MEMBERS; do mkdir -p "$1/ops/ansible/group_vars/$m"; done
  cp "$2" "$1/ops/pins/fleet-pins.yaml"
}
# $1 table in, $2 table out: every opt-in component and every opt_in: list removed.
strip_opt_in() {
  python3 - "$1" "$2" <<'PY'
import sys, yaml
t = yaml.safe_load(open(sys.argv[1]))
t["components"] = {c: s for c, s in t["components"].items() if not (s or {}).get("opt_in")}
for g in (t.get("goatnets") or {}).values():
    (g or {}).pop("opt_in", None)
yaml.safe_dump(t, open(sys.argv[2], "w"), sort_keys=False)
PY
}
# $1 table in, $2 table out: an opt-in component no goatnet lists, added.
add_opt_in() {
  python3 - "$1" "$2" <<'PY'
import sys, yaml
t = yaml.safe_load(open(sys.argv[1]))
t["components"]["example-map"] = {
    "owns_state": False, "rollback": "pin-revert", "opt_in": True, "var": "example_map_image",
    "fleet": "ghcr.io/example/example-map@sha256:" + "d" * 64}
yaml.safe_dump(t, open(sys.argv[2], "w"), sort_keys=False)
PY
}
# The generated files that say what each member resolves: every pins.yml, and the
# intent's components, canonical.
resolved() { # $1 tree
  for m in $MEMBERS; do echo "== $m"; cat "$1/ops/ansible/group_vars/$m/pins.yml" 2>/dev/null || echo "(none)"; done
  echo "== intent components"
  python3 - "$1/ops/consistency/intent/zagros/intent.json" <<'PY'
import json, sys
try:
    print(json.dumps(json.load(open(sys.argv[1]))["components"], sort_keys=True, indent=1))
except (OSError, ValueError, KeyError) as e:
    print("(no intent: %s)" % type(e).__name__)
PY
}
# The one line each avenue refuses with, its own prefix taken off.
refusal() { sed -E 's/^(ok    pins|FAIL  pins)  //; s/^(ERROR: |goatbuilder: |Error: )//' | grep -E 'opt-in|opt_in' | head -1 || true; }
run() { # $1 tree, $2 py|rs; prints output, returns exit
  if [ "$2" = py ]; then ( cd "$1" && python3 ops/pins/generate-pins.py 2>&1 ); else "$BIN" -C "$1" pins --yes 2>&1; fi
}

fail=0; n=0
for case in "$CASES"/*.yaml; do
  name="$(basename "$case" .yaml)"; n=$((n + 1))
  read -r want needle < <(sed -n 's/^# expect: //p' "$case")
  has="$(sed -n 's/^# has: //p' "$case")"
  bad=""
  for av in py rs; do
    tree "$SCRATCH/$name/$av" "$case"
    set +e; out="$(run "$SCRATCH/$name/$av" "$av")"; rc=$?; set -e
    printf '%s\n' "$out" > "$SCRATCH/$name/$av.out"
    [ "$rc" = "$want" ] || bad="$bad $av exit $rc (want $want);"
    if [ -n "${needle:-}" ]; then
      grep -q -- "$needle" "$SCRATCH/$name/$av.out" || bad="$bad $av says no '$needle';"
    fi
  done
  [ "$(refusal < "$SCRATCH/$name/py.out")" = "$(refusal < "$SCRATCH/$name/rs.out")" ] || bad="$bad the two avenues refuse in different words;"
  if [ "$want" = 0 ]; then
    [ "$(resolved "$SCRATCH/$name/py")" = "$(resolved "$SCRATCH/$name/rs")" ] || bad="$bad the two avenues resolve differently;"
    if [ -n "$has" ]; then
      grep -q -- "$has" "$SCRATCH/$name/py/ops/ansible/group_vars/zagros_a_hosts/pins.yml" || bad="$bad zagros_a_hosts/pins.yml holds no '$has';"
    fi
    if ! grep -q '^    opt_in:' "$case"; then
      strip_opt_in "$case" "$SCRATCH/$name.base.yaml"
      for av in py rs; do
        tree "$SCRATCH/$name/base-$av" "$SCRATCH/$name.base.yaml"
        run "$SCRATCH/$name/base-$av" "$av" > /dev/null || bad="$bad $av fails on the table with no opt-in component;"
        [ "$(resolved "$SCRATCH/$name/$av")" = "$(resolved "$SCRATCH/$name/base-$av")" ] || bad="$bad $av: an opt-in component no goatnet lists changed what zagros resolves;"
      done
    fi
  fi
  if [ -n "$bad" ]; then
    fail=1; echo "FAIL $name:$bad"
    printf '  py: %s\n' "$(refusal < "$SCRATCH/$name/py.out")"
    printf '  rs: %s\n' "$(refusal < "$SCRATCH/$name/rs.out")"
  else
    echo "ok   $name (exit $want${needle:+, $needle})"
  fi
done
[ "$n" -gt 0 ] || { echo "FAIL: no case under $CASES"; exit 1; }

# The existing fixture goatnets: an added opt-in component no goatnet lists moves
# nothing they resolve.
for case in "$EXISTING"/*.yaml; do
  read -r want _ < <(sed -n 's/^# expect: //p' "$case")
  [ "$want" = 0 ] || continue
  name="existing-$(basename "$case" .yaml)"; n=$((n + 1)); bad=""
  add_opt_in "$case" "$SCRATCH/$name.yaml"
  for av in py rs; do
    tree "$SCRATCH/$name/before-$av" "$case"; tree "$SCRATCH/$name/after-$av" "$SCRATCH/$name.yaml"
    run "$SCRATCH/$name/before-$av" "$av" > /dev/null || bad="$bad $av fails before;"
    run "$SCRATCH/$name/after-$av" "$av" > /dev/null || bad="$bad $av fails after;"
    [ "$(resolved "$SCRATCH/$name/before-$av")" = "$(resolved "$SCRATCH/$name/after-$av")" ] || bad="$bad $av: what zagros resolves changed;"
  done
  if [ -n "$bad" ]; then fail=1; echo "FAIL $name:$bad"; else echo "ok   $name (an added opt-in component moves nothing zagros resolves)"; fi
done

if [ "$fail" != 0 ]; then echo "parity-pins-opt-in: FAILED"; exit 1; fi
echo "parity-pins-opt-in: $n cases, both avenues agree"
