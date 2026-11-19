#!/usr/bin/env bash
# units/join-site.sh — a joining site's declared service set against join-node's plan
# (gate row I3), with no host and no container. Sourced by lib/unit.sh. Scenario:
# join-site-plan (scenarios/catalog.yaml).
#
# The tree is the tree under test's own fixture ops/goatbuilder/fixtures/join-node-third-site
# (the synthetic goatnet zagros: founder alpha, joiner beta, and gamma, a single-host site
# whose node gamma-core-1 is not enrolled yet), with the tree's real end-state model, the
# gates join-node's dry run calls and the plays whose presence it reads. The unit then:
#   1. models gamma with the tree's endstate.py (the declared set);
#   2. prints join-node's plan for gamma-core-1 with --dry-run, through the tree's goatbuilder
#      ($GB), with stand-in secret values in the environment that the plan must not print;
#   3. judges the plan against the model (lib/joinsite.py): every service declared on the
#      node is built by a command of the plan or named under "does not run yet".
# Nothing runs on a host; the measured join of two sites on hosts is a later unit.
#
# A tree whose join-node leaves a declared service unbuilt and unsaid fails the judgement,
# naming each such service: that is the gap gate row I3 measures, not a harness fault.

JS_SITE=gamma; JS_NODE=gamma-core-1; JS_ADDR=192.0.2.30
# Stand-in secret values: the plan must print none of them.
JS_PG="js-stand-in-pg-$$"; JS_NBK="js-stand-in-enckey-$$"; JS_BKP="js-stand-in-bkp-$$"

# shellcheck source=../lib/joinsite-tree.sh
. "$HERE/lib/joinsite-tree.sh"

js_tree() { # $1 destination: the fixture with the tree's real model, gates and plays
  local d="$1"
  rm -rf "$d"
  cp -a "$SRC/ops/goatbuilder/fixtures/join-node-third-site" "$d"
  rm -f "$d/README.md"
  js_real_files "$d" || return 1
  echo "join-site: tree for $JS_SITE at $d"
}

js_plan() { # $1 tree, $2 plan.json: join-node's dry-run plan, as JSON, printed and kept
  env -i PATH="/usr/local/bin:/usr/bin:/bin" HOME="$WORK" GOAT_SECRET_STORE=env GOAT_NO_FUN=1 \
    OWNER_EMAIL=owner@example.org GAMMA_PG_PASS="$JS_PG" GAMMA_NB_ENCKEY="$JS_NBK" GAMMA_BKP_KEY="$JS_BKP" \
    "$GB" -C "$1" -o json join-node zagros --site "$JS_SITE" --node "$JS_NODE" --class core \
    --address "$JS_ADDR" --dry-run > "$2"
  local rc=$?
  cat "$2"
  return "$rc"
}
export -f js_tree js_plan js_real_files
export JS_SITE JS_NODE JS_ADDR JS_PG JS_NBK JS_BKP

unit_main() {
  local t="$WORK/join-site/checkout"
  mkdir -p "$WORK/join-site"
  rec "js-tree" --expect-in "join-site: tree for $JS_SITE" -- bash -c "js_tree '$t'"
  rec "js-model" -- bash -c "python3 '$t/ops/goatbuilder/endstate/endstate.py' model '$JS_SITE' --root '$t' > '$WORK/join-site/model.json' && cat '$WORK/join-site/model.json'"
  rec "js-plan" --expect-in '"verb"' \
    --expect-not-in "$JS_PG" --expect-not-in "$JS_NBK" --expect-not-in "$JS_BKP" \
    -- bash -c "js_plan '$t' '$WORK/join-site/plan.json'"
  rec "js-declared-built-or-named" --expect-in " silent or contradictory" -- \
    python3 "$HERE/lib/joinsite.py" "$WORK/join-site/model.json" "$WORK/join-site/plan.json" "$JS_NODE"
}
