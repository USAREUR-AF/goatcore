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
    leaves) docker exec "$h" sh -c "for f in /var/lib/goat-federation/$them/pki/peering-*.pem /var/lib/goat-federation/$them/pki/republisher.pem /var/lib/goat-federation/$them/pki/core.pem; do case \$f in *-key.pem) ;; *) [ -f \$f ] && printf '%s %s\n' \"\$(basename \$f)\" \"\$(openssl x509 -in \$f -noout -serial)\" ;; esac; done" ;;
    transport) docker exec "$HP-$s-edge" wg show "$(iface "$s" "$them")" public-key ;;
    carrier) if [ "$s" = "$NATED" ]; then docker exec "$HP-$s-edge" wg show wg-carry0 public-key; else echo "no carrier on $s"; fi ;;
    signing)
      # What a signing rotation changes: the seed this side signs for this peering with (this
      # peering's edge group's goat_federation_signing_key, else FED_SIGNING_KEY as the drive
      # passes it), by digest,
      # and this party's verifying key as the partner's keyring holds it (spec §5.1).
      docker exec "$h" sh -c "k=\$(grep -hoE '^goat_federation_signing_key:[[:space:]]*\\S+' /opt/goat/ops/ansible/group_vars/${them}_edge/*.yml 2>/dev/null | head -1 | sed 's/^[^:]*:[[:space:]]*//; s/[\"\x27]//g'); k=\${k:-/etc/fed-e2e/signing.key}; printf 'seed %s %s\n' \"\$k\" \"\$(sha256sum \"\$k\" | cut -d' ' -f1)\""
      docker exec "$(drv_host "$them")" sh -c "grep -E '^$s=' /var/lib/goat-federation/$s/keys.txt | sed 's/^/in the partner keyring: /'" ;;
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
  if [ "$k" = signing ]; then
    # Signing: the seed must change here; the partner's keyring is its own check below.
    rec "rot-rotated-$k-$s" -- bash -c \
      "a=\$(grep '^seed ' '$RECORD/rot-$k-$s-before.txt'); b=\$(grep '^seed ' '$RECORD/rot-$k-$s-after.txt'); echo \"before: \$a\"; echo \"after:  \$b\"; [ -n \"\$a\" ] && [ -n \"\$b\" ] && [ \"\$a\" != \"\$b\" ]"
  else
  rec "rot-rotated-$k-$s" -- bash -c \
    "diff <(sed -n '/^## $k\$/,/^## /p' '$RECORD/rot-$k-$s-before.txt') <(sed -n '/^## $k\$/,/^## /p' '$RECORD/rot-$k-$s-after.txt') && { echo 'the $k material did not change'; exit 1; } || exit 0"
  fi
  rec "rot-kept-$k-$s" -- python3 - "$RECORD/rot-$k-$s-before.txt" "$RECORD/rot-$k-$s-after.txt" "$k" <<'PY'
import re, sys
def parts(p):
    out, cur = {}, None
    for l in open(p):
        if l.startswith("## "): cur = l[3:].strip(); out[cur] = []
        elif cur: out[cur].append(l)
    return out
a, b, k = parts(sys.argv[1]), parts(sys.argv[2]), sys.argv[3]
bad = re.compile(r"error|no such|not found|unable|cannot|permission denied|usage:", re.I)
problems = []
for x in a:
    if x == k: continue
    body = "".join(a[x]).strip()
    if not body or bad.search(body): problems.append(f"{x}: no material read before ({body[:80]!r})")
    elif a.get(x) != b.get(x): problems.append(f"{x}: changed")
print("other keys:", sorted(x for x in a if x != k), "; problems:", problems or "none")
sys.exit(1 if problems else 0)
PY
  rot_all_material "$them" "$RECORD/rot-$k-$them-after.txt"
  rec "rot-partner-kept-$k-$s" -- diff "$RECORD/rot-$k-$them-before.txt" "$RECORD/rot-$k-$them-after.txt"
  if [ "$k" = signing ]; then
    # Our new verifying key in the partner's keyring is the partner's step (§5), not built yet.
    local ord; ord="$([ "$s" = "$NATED" ] && echo first || echo second)"
    mark "key-rotation.partner-keyring-$ord" BLOCKED "the partner's step that takes our new verifying key into its keyring waits on the operator's decision in docs/design/federation-key-rotation.md §5"
  fi
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
  sleep 20 # values after the last rotation, before the flows stop
  flow_stop a2b; flow_stop b2a
  { flow_report a2b; flow_report b2a; } > "$RECORD/rot-samples.json"
  # Decidable now: data resumes. Values are stored inside and after every rotation's window,
  # and the last five of each flow are stored.
  rec "rot-samples-resume" -- python3 - "$RECORD" $ROT_KEYS <<'PY'
import json, os, sys
d, keys = sys.argv[1], sys.argv[2:]
def rows(l): return [r.rstrip("\n").split("\t") for r in open(os.path.join(d, f"flow-{l}.read")) if r.strip()]
def window(k):
    t = [float(r.split("\t")[0]) for r in open(os.path.join(d, f"rot-{k}.window")) if r.strip()]
    return min(t), max(t)
bad = []
for l in ("a2b", "b2a"):
    rs = rows(l); ok = [r[3] == "stored" for r in rs]; t = [float(r[0]) for r in rs]
    if len(ok) < 5 or not all(ok[-5:]): bad.append(f"{l}: the last five values are not all stored")
    for k in keys:
        lo, hi = window(k)
        inside = [o for o, x in zip(ok, t) if lo <= x <= hi]; after = [o for o, x in zip(ok, t) if x > hi]
        if not any(inside): bad.append(f"{l}: nothing stored during the {k} rotation")
        if not any(after): bad.append(f"{l}: nothing stored after the {k} rotation")
print(open(os.path.join(d, "rot-samples.json")).read().strip()); print("problems:", bad or "none")
sys.exit(1 if bad else 0)
PY
  # Not decidable yet: how much may be lost across a rotation (the same question as B4's).
  mark "key-rotation.samples-loss" BLOCKED "loss tolerance across a rotation awaits the operator's decision (as B4); the flow's figures are in rot-samples.json"
  rec "rot-no-pending" -- bash -c "for s in $NATED $PARTNER; do o=\$(docker exec \$(drv_host \$s) find /var/lib/goat-federation -name .rotation-pending) || { echo \"could not search \$s\"; exit 1; }; [ -z \"\$o\" ] || { echo \"\$o\"; exit 1; }; done; echo none"
}
