#!/usr/bin/env bash
# units/join-site-hosts.sh — the two-site lab of gate row I3: a founder stood up with
# goatbuilder standup and a single-host joining site added with goatbuilder join-node, each
# on a host of its own. Sourced by lib/unit.sh. Scenario: join-site-two-hosts
# (scenarios/catalog.yaml).
#
# The lab is laid out from the tree under test's fixture join-node-third-site
# (lib/joinsite_lab.py layout): the founder alpha, its node mgmt-alpha-01, and the joiner
# gamma, its node gamma-core-1, at two addresses of one site network ($NB.6.0/24; the
# federation units use $NB.1 to $NB.5). The unit then prints both plans with --dry-run
# against that inventory and checks that every command reaches only the lab's two hosts
# (lib/joinsite_lab.py reach), and that the joiner's plan builds or names every service the
# joiner declares (lib/joinsite.py).
#
# Built so far: the layout and the dry runs. The runs on hosts (both hosts started from the
# fed-e2e host image, standup phases 1 to 14 at the founder, join-node phases 1 to 15 at the
# joiner, the end state graded at both sites) are not built; their checks are marked
# BLOCKED with that reason.

JSH_FOUNDER_NODE=mgmt-alpha-01; JSH_JOINER_NODE=gamma-core-1
JSH_FOUNDER_IP="${NB:-10.77}.6.10"; JSH_JOINER_IP="${NB:-10.77}.6.20"
JSH_PG="jsh-stand-in-pg-$$"; JSH_NBK="jsh-stand-in-enckey-$$"; JSH_BKP="jsh-stand-in-bkp-$$"

# shellcheck source=../lib/joinsite-tree.sh
. "$HERE/lib/joinsite-tree.sh"

jsh_tree() { # $1 destination
  python3 "$JSH_LIB/joinsite_lab.py" layout "$SRC/ops/goatbuilder/fixtures/join-node-third-site" "$1" \
    "$JSH_FOUNDER_IP" "$JSH_JOINER_IP" && js_real_files "$1"
}

jsh_gb() { # $1 tree, then goatbuilder's arguments: the tree's binary, with stand-in secrets only
  local t="$1"; shift
  env -i PATH="/usr/local/bin:/usr/bin:/bin" HOME="$WORK" GOAT_SECRET_STORE=env GOAT_NO_FUN=1 \
    OWNER_EMAIL=owner@example.org \
    ALPHA_PG_PASS="$JSH_PG" ALPHA_NB_ENCKEY="$JSH_NBK" ALPHA_BKP_KEY="$JSH_BKP" \
    GAMMA_PG_PASS="$JSH_PG" GAMMA_NB_ENCKEY="$JSH_NBK" GAMMA_BKP_KEY="$JSH_BKP" \
    "$GB" -C "$t" -o json "$@"
}

jsh_plan() { # $1 tree, $2 plan.json, then the verb's arguments: printed and kept
  local t="$1" out="$2" rc; shift 2
  jsh_gb "$t" "$@" --dry-run > "$out"; rc=$?
  cat "$out"
  return "$rc"
}
export -f jsh_tree jsh_gb jsh_plan js_real_files
JSH_LIB="$HERE/lib"
export JSH_LIB JSH_FOUNDER_NODE JSH_JOINER_NODE JSH_FOUNDER_IP JSH_JOINER_IP JSH_PG JSH_NBK JSH_BKP

unit_main() {
  local w="$WORK/join-site-hosts" t
  t="$w/tree"
  mkdir -p "$w"
  local lab=("$JSH_FOUNDER_NODE=$JSH_FOUNDER_IP" "$JSH_JOINER_NODE=$JSH_JOINER_IP" alpha_hosts gamma_hosts)
  rec "jsh-layout" --expect-in "join-site lab: $JSH_FOUNDER_NODE at $JSH_FOUNDER_IP" -- bash -c "jsh_tree '$t'"
  rec "jsh-founder-plan" --expect-in '"verb"' \
    --expect-not-in "$JSH_PG" --expect-not-in "$JSH_NBK" --expect-not-in "$JSH_BKP" \
    -- bash -c "jsh_plan '$t' '$w/founder.json' standup zagros --site alpha"
  rec "jsh-founder-plan-reaches-only-the-lab" --expect-in " 0 reach outside the lab" -- \
    python3 "$HERE/lib/joinsite_lab.py" reach "$w/founder.json" "${lab[@]}"
  rec "jsh-joiner-plan" --expect-in '"verb"' --expect-in "$JSH_JOINER_IP" \
    --expect-not-in "$JSH_PG" --expect-not-in "$JSH_NBK" --expect-not-in "$JSH_BKP" \
    -- bash -c "jsh_plan '$t' '$w/joiner.json' join-node zagros --site gamma --node '$JSH_JOINER_NODE' --class core --address '$JSH_JOINER_IP'"
  rec "jsh-joiner-plan-reaches-only-the-lab" --expect-in " 0 reach outside the lab" -- \
    python3 "$HERE/lib/joinsite_lab.py" reach "$w/joiner.json" "${lab[@]}"
  rec "jsh-joiner-model" -- bash -c "python3 '$t/ops/goatbuilder/endstate/endstate.py' model gamma --root '$t' > '$w/model.json' && cat '$w/model.json'"
  rec "jsh-joiner-declared-built-or-named" --expect-in " silent or contradictory" -- \
    python3 "$HERE/lib/joinsite.py" "$w/model.json" "$w/joiner.json" "$JSH_JOINER_NODE"
  local why="not built: the runs on hosts (standup at the founder, join-node at the joiner, both sites graded) are designed, not built"
  for c in founder-standup joiner-join founder-graded joiner-graded declared-equals-deployed; do
    mark "join-site-two-hosts.$c" BLOCKED "$why"
  done
}
