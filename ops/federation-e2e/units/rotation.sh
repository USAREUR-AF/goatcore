#!/usr/bin/env bash
# units/rotation.sh — gate row B3 (docs/design/goatbuilder-v1-gate.md), the proving scenario of
# docs/design/federation-key-rotation.md §6: each key rotated in turn on a running peering,
# the command run again on both sides, and a continuous flow of samples throughout.
#
# The peering is units/forwarder.sh's ({A} behind NAT, listening through its forwarder, so
# the carrier key is covered). Then a flow of one fresh value a second runs each way
# (lib/flow.sh) for the whole scenario. For each key in turn (leaves, carrier, transport,
# signing), {A} rotates it and then {B}: the material is set aside, `goatbuilder federate
# rotate <partner> <key>` runs on the rotating side, and the checks of §6 read what changed,
# what did not, and what the flow lost.
#
# Nothing of `rotate` is built (§7), so the first rotation fails today and the unit stops
# there: every later check reads BLOCKED. The partner's step for transport and signing waits
# on §5's operator decision; the unit names it where it is reached.
# shellcheck source=/dev/null
. "$HERE/units/pair.sh"
# shellcheck source=/dev/null
. "$HERE/units/forwarder.sh"
eval "$(declare -f unit_main | sed '1s/^unit_main /rot_peering_up /')"
# shellcheck source=/dev/null
. "$HERE/lib/flow.sh"

ROT_KEYS="leaves carrier transport signing"

# Every stop goes through here: the flows are stopped first, so none keeps releasing.
rot_die() {
  local l; for l in a2b b2a; do [ -e "$RECORD/flow-$l.pid" ] && touch "$RECORD/flow-$l.stop"; done
  die "$@"
}

# The material each key is, as the checks compare it. $1 side, $2 key.
rot_material() {
  local s="$1" k="$2" them h; them="$(other "$s")"; h="$(drv_host "$s")"
  case "$k" in
    leaves) docker exec "$h" sh -c "for f in /var/lib/goat-federation/$them/pki/peering-*.pem /var/lib/goat-federation/$them/pki/republisher.pem; do case \$f in *-key.pem) ;; *) [ -f \$f ] && printf '%s %s\n' \"\$(basename \$f)\" \"\$(openssl x509 -in \$f -noout -serial)\" ;; esac; done" ;;
    transport) docker exec "$HP-$s-edge" wg show "$(iface "$s" "$them")" public-key ;;
    carrier) if [ "$s" = "$NATED" ]; then docker exec "$HP-$s-edge" wg show wg-carry0 public-key; else echo "no carrier on $s"; fi ;;
    signing) docker exec "$h" sh -c "cat /etc/fed-e2e/signing.pub 2>/dev/null; python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); print(p.get(\"signing_key\") or p.get(\"verifying_key\") or \"\")' /var/lib/goat-federation/$them/ours.canonical.json 2>/dev/null" ;;
  esac
}
export -f rot_material

rot_all_material() { # $1 side, $2 file
  local k; for k in $ROT_KEYS; do echo "## $k"; rot_material "$1" "$k"; done > "$2" 2>&1
}

rot_one() { # $1 key, $2 side
  local k="$1" s="$2" them h out; them="$(other "$s")"; h="$(drv_host "$s")"
  rot_all_material "$s" "$RECORD/rot-$k-$s-before.txt"
  rot_all_material "$them" "$RECORD/rot-$k-$them-before.txt"
  out="$RECORD/rot-$k-$s-rotate.out"
  local f0="$FAILS"
  rec "rot-rotate-$k-$s-ran" -- bash -c \
    "docker exec $h goatbuilder -C /opt/goat --yes federate rotate $them $k --as $s > '$out' 2>&1; rc=\$?; tail -30 '$out'; exit \$rc"
  [ "$FAILS" = "$f0" ] || rot_die "\`goatbuilder federate rotate $them $k\` did not run on $s: rotation is not built (docs/design/federation-key-rotation.md §7)"
  rot_all_material "$s" "$RECORD/rot-$k-$s-after.txt"
  rec "rot-rotated-$k-$s" -- bash -c \
    "diff <(sed -n '/^## $k\$/,/^## /p' '$RECORD/rot-$k-$s-before.txt') <(sed -n '/^## $k\$/,/^## /p' '$RECORD/rot-$k-$s-after.txt') && { echo 'the $k material did not change'; exit 1; } || exit 0"
  rec "rot-kept-$k-$s" -- bash -c \
    "diff <(grep -v -A9999 '^## $k\$' '$RECORD/rot-$k-$s-before.txt' | grep -v '^## $k') <(grep -v -A9999 '^## $k\$' '$RECORD/rot-$k-$s-after.txt' | grep -v '^## $k') ; python3 - '$RECORD/rot-$k-$s-before.txt' '$RECORD/rot-$k-$s-after.txt' '$k' <<'PY'
import sys
def parts(p):
    out, cur = {}, None
    for l in open(p):
        if l.startswith('## '): cur = l[3:].strip(); out[cur] = []
        elif cur: out[cur].append(l)
    return out
a, b, k = parts(sys.argv[1]), parts(sys.argv[2]), sys.argv[3]
changed = [x for x in a if x != k and a.get(x) != b.get(x)]
print('unchanged besides', k, ':', sorted(x for x in a if x != k and x not in changed), '; changed:', changed)
sys.exit(1 if changed else 0)
PY"
  case "$k" in
    transport|signing) rot_die "the partner's step after a $k rotation waits on the operator's decision in docs/design/federation-key-rotation.md §5" ;;
  esac
  for x in "$s" "$them"; do
    rec "rot-acceptance-$k-$x" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" \
      --expect-not-in "ACCEPTANCE FAILED" --expect-not-in "ACCEPTANCE INCOMPLETE" -- bash -c "accept_as '$x' '$(other "$x")' '$AVENUE'"
  done
}

unit_main() {
  rot_peering_up
  say "rotation: a flow each way for the whole scenario, then each key in turn on each side"
  flow_start "$NATED" "$PARTNER" a2b
  flow_start "$PARTNER" "$NATED" b2a
  local k s
  for k in $ROT_KEYS; do
    printf '%s\tstart\n' "$(date +%s.%N)" >> "$RECORD/rot-$k.window"
    for s in "$NATED" "$PARTNER"; do rot_one "$k" "$s"; done
    printf '%s\tend\n' "$(date +%s.%N)" >> "$RECORD/rot-$k.window"
  done
  flow_stop a2b; flow_stop b2a
  rec "rot-samples" -- bash -c "flow_report a2b; flow_report b2a"
  rec "rot-no-pending" -- bash -c "for s in $NATED $PARTNER; do docker exec \$(drv_host \$s) sh -c 'find /var/lib/goat-federation -name .rotation-pending' | grep . && exit 1; done; echo none"
}
