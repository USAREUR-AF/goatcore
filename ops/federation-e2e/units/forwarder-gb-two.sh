#!/usr/bin/env bash
# units/forwarder-gb-two.sh — two peerings relayed by ONE forwarder, both applied by
# `goatbuilder federate` (docs/design/peering-forwarder.md §7's full gate). Scenario:
# forwarder-two-peerings-by-goatbuilder.
#
# units/forwarder-gb.sh with a third sovereign. {B}, behind its NAT and listening through
# the forwarder it operates, declares its peering with {A} from nothing and brings it up
# by the one command; then it adds a peering with {C} beside it (NH_ADD_PEERING), again
# with `forwarder:` naming the same forwarder, and {C} declares its side from nothing.
# allocate keeps the first peering's values and gives the second its own public port,
# carrier interface, carrier ports and carrier /31; phase 2 of the second peering's run
# applies the forwarder play, which then relays both.
#
# Each pair is driven as nothing-hand-written drives one (units/nohand.sh): the pair's
# two sovereigns are {A} and {B} while it runs, so the pair helpers (`other`, oc_drive,
# the acceptance check) work on it. The second pair's steps are prefixed nh2 (NH_STEP,
# OC_PREFIX). After both are up, the checks below read both peerings: each carrier, each
# path through the forwarder, each tunnel both ways, and what the forwarder holds.
# Sourced by lib/unit.sh.
# NH_ADMIT_TREE, NH_STEP and NH_DECLARED are units/nohand.sh's, read there.
# shellcheck disable=SC2034,SC2153
# shellcheck source=/dev/null
. "$HERE/units/forwarder-gb.sh"

# The forwarder machine is stood up and declared once, at {B}'s first peering.
eval "$(declare -f nh_operator_more | sed '1s/^nh_operator_more /fg2_operator_first /')"
nh_operator_more() { # $1 us, $2 them
  if [ "$1" = "$NATED" ] && [ -n "${FG2_FWD_UP:-}" ]; then return 0; fi
  fg2_operator_first "$@"
  if [ "$1" = "$NATED" ]; then FG2_FWD_UP=1; fi
  return 0
}

# One allocated value of {B}'s peering with $1, from the allocation record allocate wrote.
fg2_value() { # $1 partner, $2 field
  python3 - "$WORK/$NATED/clone/ops/goatbuilder/peering-values.yaml" "$NATED/$1" "$2" <<'PY'
import sys, yaml
rec, key, field = sys.argv[1:4]
v = (yaml.safe_load(open(rec)) or {}).get("peerings", {}).get(key, {}).get(field)
print("" if v is None else v)
PY
}
# A carrier /31's two addresses (forwarder.rs carrier_pair): the forwarder's is the network
# address, the edge's the next one.
fg2_fwd_carrier_ip() { python3 -c 'import ipaddress,sys; print(ipaddress.ip_network(sys.argv[1]).network_address)' "$1"; }
fg2_edge_carrier_ip() { python3 -c 'import ipaddress,sys; print(ipaddress.ip_network(sys.argv[1]).network_address + 1)' "$1"; }

# The forwarder holds no peering key of any party: every WireGuard and TLS key either side
# of either peering works with is searched for across its filesystem (as fwd_holds_nothing
# does for one peering); no PEM private key outside the host's own SSH keys; no peering
# interface; its configuration is exactly the two carriers' keys and environments and its
# table; and neither carrier key is any edge's peering key.
fg2_holds_nothing() {
  local s f p="$WORK/fg2-patterns.txt" bad=0 hits pem ifs keys fk c pk
  : > "$p"
  for s in "$A" "$B" "$C"; do
    docker exec "$HP-$s-edge" sh -c 'cat /etc/goat-peering-wg/*.key' >> "$p"
    docker exec "$HP-$s-edge" sh -c 'for f in /var/lib/goat-federation/*/pki/*key*.pem; do sed -n 2p "$f"; done' >> "$p"
    for f in ca-key.pem signing.pem core-key.pem; do sed -n 2p "$WORK/$s/keys/$f" >> "$p"; done
    cat "$WORK/$s/keys/signing.key" >> "$p"; echo >> "$p"
  done
  sed -i '/^[[:space:]]*$/d' "$p"
  echo "searching for $(wc -l < "$p" | tr -d ' ') key fragments of the three parties"
  hits="$(docker exec -i "$FWD_HOST" sh -c 'cat > /dev/shm/fe2e-p; grep -rlF -f /dev/shm/fe2e-p / --exclude-dir=proc --exclude-dir=sys --exclude-dir=dev 2>/dev/null; rm -f /dev/shm/fe2e-p' < "$p")"
  [ -z "$hits" ] || { echo "peering key material found on the forwarder: $hits"; bad=1; }
  pem="$(docker exec "$FWD_HOST" sh -c "grep -rlE -- '-----BEGIN ([A-Z]+ )?PRIVATE KEY-----' /etc /root /home /var /tmp /opt /srv /run 2>/dev/null | grep -v '^/etc/ssh/ssh_host_'" || true)"
  [ -z "$pem" ] || { echo "PEM private keys on the forwarder: $pem"; bad=1; }
  ifs="$(docker exec "$FWD_HOST" ls /sys/class/net | tr '\n' ' ')"; echo "interfaces: $ifs"
  case " $ifs " in *" wg-fed"*) echo "the forwarder has a peering interface"; bad=1 ;; esac
  keys="$(docker exec "$FWD_HOST" sh -c 'ls /etc/goat-peering-forwarder' | tr '\n' ' ')"; echo "config: $keys"
  [ "$keys" = "forwarder.nft wg-carry0.env wg-carry0.key wg-carry1.env wg-carry1.key " ] || { echo "the forwarder's config is not exactly the two carriers' keys, environments and its table"; bad=1; }
  for c in wg-carry0 wg-carry1; do
    fk="$(docker exec "$FWD_HOST" sh -c "wg pubkey < /etc/goat-peering-forwarder/$c.key")"
    for s in "$A" "$B" "$C"; do
      for pk in $(docker exec "$HP-$s-edge" sh -c 'for i in $(wg show interfaces); do case "$i" in wg-fed*) wg show "$i" public-key ;; esac; done'); do
        [ "$fk" = "$pk" ] && { echo "the forwarder's $c key is a peering key of $s"; bad=1; }
      done
    done
  done
  [ "$bad" = 0 ] && echo "the forwarder holds no peering key and no TLS key of any party"
  return "$bad"
}
export -f fg2_holds_nothing

# The digest of the agreement between $1 and {B}, as each of the two holds it: one line
# per side, "<side> <sha256>" (empty when a side holds none).
fg2_digests() { # $1 {B}'s partner
  local s w
  for s in "$1" "$NATED"; do
    if [ "$s" = "$NATED" ]; then w="/var/lib/goat-federation/$1"; else w="/var/lib/goat-federation/$NATED"; fi
    printf '%s %s\n' "$s" "$(docker exec "$(drv_host "$s")" sha256sum "$w/agreement.json" 2>/dev/null | cut -d' ' -f1)"
  done
}

# One pair, as nh_main drives it (units/nohand.sh): {A} and {B} are the pair's sovereigns
# for the length of the call, {B} the listener behind the forwarder.
fg2_pair() { # $1 the partner ({A}, then {C}), $2 1 for the second peering of {B}
  # Exported for the length of the pair, so a step's child shell reads the pair's {A}.
  local -x A="$1"
  NH_ADD_PEERING=0 nh_side_from_nothing "$A" "$B" false
  NH_ADD_PEERING="$2" nh_side_from_nothing "$B" "$A" true
  host_exec "$HP-$A-core" sh -c 'cat > /usr/local/bin/fed-responder.py' < "$SRC/ops/federation/roundtrip/responder.py"
  host_exec "$HP-$B-core" sh -c 'cat > /usr/local/bin/fed-responder.py' < "$SRC/ops/federation/roundtrip/responder.py"
  docker exec -d "$HP-$A-core" python3 /usr/local/bin/fed-responder.py "from/$B" "release/all"
  docker exec -d "$HP-$B-core" python3 /usr/local/bin/fed-responder.py "from/$A" "release/all"
  oc_drive "$B" "$A"
  oc_status "$A" "$B"; oc_status "$B" "$A"
  oc_checks "$A" "$B"
  oc_acceptance_checks "$A" "$B"
  oc_defect_checks "$A" "$B"
}

fg2_main() {
  local s h c g l p0 p1 port args=()
  : > "$HAND"; : > "$OUTSIDE"
  for s in "$A" "$B" "$C"; do eval "FE2E_OVERLAY_ARGS_$s=--no-peering"; export "FE2E_OVERLAY_ARGS_$s"; done
  FE2E_CORE=role; export FE2E_CORE
  set_partners "$B" "$A $C"; set_partners "$A" "$B"; set_partners "$C" "$B"; export NODIR=""
  eval "NH_FORWARDER_$NATED=$FWD_PUB"; export "NH_FORWARDER_$NATED"
  eval "NH_MTU_$NATED=1300"; export "NH_MTU_$NATED"
  export_helpers
  export -f oc_passed
  export OC_PREFIX HP ZENOH_IMAGE GB
  say "forwarder-two-peerings-by-goatbuilder: $B behind its NAT relays $A and $C through one forwarder"
  setup_hosts fg2
  for s in "$A" "$B" "$C"; do oc_core_staging "$s"; done
  # As nh_main: a tree whose admission works a goat-zenoh role core declares only site_id.
  if has_text ops/errors/catalog.yaml "code: GB-237"; then
    NH_ADMIT_TREE=1
    for s in "$A" "$B" "$C"; do
      printf '%s\n' "# $s's core router site (the goat-zenoh role's mandatory site_id)." "site_id: $s" > "$WORK/$s/core-site.yml"
      core_declare "$s" ops/ansible/group_vars/core_router/site.yml "$WORK/$s/core-site.yml" "$s: the core router's site"
    done
    NH_WITHHELD="$NH_WITHHELD $NH_ADMIT"
  else
    die "the tree under test does not admit on a goat-zenoh role core (no GB-237); this scenario needs it"
  fi
  NH_DECLARES=0
  has_text ops/goatbuilder/schema/goatbuilder-v1.schema.json '"peering_directory"' && NH_DECLARES=1
  [ "$NH_DECLARES" = 1 ] && NH_WITHHELD="$NH_WITHHELD $NH_DECLARED"
  hand "values FED_THEIRS_REST, FED_SPIRE_THEIRS, FED_PROBE_LAUNCHER and the drill's reach into the partner's network (phase 14's counterparty surfaces): given only to the separate acceptance step; lab-only: only a drill owns both sides; a production operator holds only their own, so phase 14 inside \`goatbuilder federate $B\` is BLOCKED, exit 3, without them (no counterparty surface (FED_THEIRS_REST)), the designed reading (docs/design/federation-acceptance.md); production proves the partner's half through the bilateral session exchanged by file"

  say "forwarder-two-peerings-by-goatbuilder: the first peering, $B with $A"
  NH_STEP=nh; OC_PREFIX=nh; export OC_PREFIX
  fg2_pair "$A" 0
  # The first agreement as both sides hold it before the second peering is added.
  fg2_digests "$A" > "$WORK/fg2-first-agreement-before.txt"
  say "forwarder-two-peerings-by-goatbuilder: the second peering, $B with $C, beside the first"
  NH_STEP=nh2; OC_PREFIX=nh2; export OC_PREFIX
  fg2_pair "$C" 1

  h="$(drv_host "$NATED")"; c="$WORK/$NATED/clone"
  g="$c/ops/ansible/group_vars/peering_forwarders/peering-forwarder.yml"

  # Adding the second peering re-renders the first's files (allocate renders every
  # peering); what runs must not change: the first agreement on both sides, and its
  # acceptance from {A} run again now.
  say "forwarder-two-peerings-by-goatbuilder: the first peering after the second was added"
  fg2_digests "$A" > "$WORK/fg2-first-agreement-after.txt"
  rec "fg2-$A-$B-agreement-unchanged-by-the-second-peering" -- bash -c \
    "echo before:; cat '$WORK/fg2-first-agreement-before.txt'; echo after:; cat '$WORK/fg2-first-agreement-after.txt'
     [ -s '$WORK/fg2-first-agreement-before.txt' ] && [ \"\$(grep -cE ' [0-9a-f]{64}\$' '$WORK/fg2-first-agreement-before.txt')\" = 2 ] && cmp -s '$WORK/fg2-first-agreement-before.txt' '$WORK/fg2-first-agreement-after.txt'"
  OC_PREFIX=nh; export OC_PREFIX
  local out="$RECORD/acceptance-as-$A-after-the-second-peering.out" arc=0
  oc_accept "$A" "$B" > "$out" 2>&1 || arc=$?
  rec "fg2-acceptance-as-$A-with-$B-after-the-second-peering" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" \
    --expect-not-in "ACCEPTANCE FAILED" --expect-not-in "ACCEPTANCE INCOMPLETE" -- bash -c "cat '$out'; exit $arc"
  say "forwarder-two-peerings-by-goatbuilder: what allocate rendered for both"
  rec "fg2-$NATED-forwarder-values-list-both" --expect-in "name: \"$A\"" --expect-in "name: \"$C\"" \
    --expect-in 'interface: "wg-carry0"' --expect-in 'interface: "wg-carry1"' -- bash -c \
    "cat '$g'; head -1 '$g' | grep -qF '# GENERATED by \`goatbuilder federate allocate\`'"
  rec "fg2-$NATED-$C-phase-2-ran-the-forwarder-play" --expect-in "behind the forwarder $FWD_PUB" --expect-in "peering-forwarder.yml" -- bash -c \
    "t=\$(ls '$RECORD'/nh2-$NATED-*.log 2>/dev/null); [ -n \"\$t\" ] || { echo 'no run of the command on $NATED for $C'; exit 1; }
     sed 's/\x1b\[[0-9;]*m//g' \$t | tr -d '\r' | grep -E 'behind the forwarder|peering-forwarder.yml|peering-carrier-keys.yml|forwarder-carrier' | sort -u | head -20
     docker exec $h test -s /var/lib/goat-federation/$C/pki/forwarder-carrier.pub && echo 'the second forwarder carrier key is on the driver'"

  say "forwarder-two-peerings-by-goatbuilder: the forwarder relays both"
  rec "fg2-forwarder-carries-both" --expect-in '"holds_peering_material": false' \
    --expect-in "dnat ip to $(fg2_edge_carrier_ip "$(fg2_value "$A" carrier_subnet)"):" \
    --expect-in "dnat ip to $(fg2_edge_carrier_ip "$(fg2_value "$C" carrier_subnet)"):" -- \
    docker exec "$FWD_HOST" sh -c 'systemctl is-active goat-peering-forwarder@wg-carry0 goat-peering-forwarder@wg-carry1 && nft list table inet goat_peering_forwarder && cat /var/log/goat-peering-forwarder-state.json'
  for l in wg-carry0 wg-carry1; do
    rec "fg2-$NATED-$l-handshake" -- bash -c \
      "for _ in \$(seq 1 30); do t=\$(docker exec $h wg show $l latest-handshakes 2>/dev/null | cut -f2); [ \"\${t:-0}\" != 0 ] && break; sleep 2; done; docker exec $h wg show $l; [ \"\${t:-0}\" != 0 ]"
  done

  say "forwarder-two-peerings-by-goatbuilder: each path, each tunnel, each way"
  p0="$(fg2_value "$A" public_port)"; p1="$(fg2_value "$C" public_port)"
  echo "public ports on the forwarder: $A $p0, $C $p1"
  for s in "$A" "$C"; do
    if [ "$s" = "$A" ]; then port="$p0"; else port="$p1"; fi
    rec "fg2-$s-endpoint-is-the-forwarder" --expect-in "$FWD_PUB:${port:-none}" --expect-not-in "$(boundary_ip "$NATED"):" -- \
      docker exec "$HP-$s-edge" wg show "$(iface "$s" "$NATED")" endpoints
    rec "fg2-$NATED-sees-$s-at-its-carrier" --expect-in "$(fg2_fwd_carrier_ip "$(fg2_value "$s" carrier_subnet)"):" -- \
      docker exec "$h" wg show "$(iface "$NATED" "$s")" endpoints
    rec "fg2-$s-$NATED-tunnel-carried-it" -- bash -c \
      "docker exec $HP-$s-edge wg show $(iface "$s" "$NATED") transfer | awk '{print; if (\$2 < 1 || \$3 < 1) exit 1}'"
    rec "fg2-$NATED-$s-tunnel-carried-it" -- bash -c \
      "docker exec $h wg show $(iface "$NATED" "$s") transfer | awk '{print; if (\$2 < 1 || \$3 < 1) exit 1}'"
    rec "fg2-arrives-$s-to-$NATED" --expect-in "PASS" -- bash -c "fgb_arrives '$s' '$NATED' 90"
    rec "fg2-arrives-$NATED-to-$s" --expect-in "PASS" -- bash -c "fgb_arrives '$NATED' '$s' 90"
  done
  rec "fg2-forwarder-holds-no-peering-key" -- bash -c fg2_holds_nothing
  docker exec "$FWD_HOST" sh -c 'wg show; nft list ruleset' > "$RECORD/fg2-forwarder-state.txt" 2>&1 || true
  docker exec "$h" sh -c 'wg show; ip route; nft list ruleset' > "$RECORD/fg2-$NATED-edge-state.txt" 2>&1 || true

  # As nh_main: every line a person wrote besides the operator input; the lab-only lines
  # are nh_lab_only's reviewed set.
  sed "s/^/step (outside the command): /" "$OUTSIDE" | grep -E "\(($A|$B|$C)\)" >> "$HAND" || true
  while IFS= read -r l; do
    [ -n "$l" ] || continue
    nh_lab_only "$l" || args+=(--expect-not-in "$l")
  done < "$HAND"
  rec "nh-nothing-hand-written" ${args[@]+"${args[@]}"} -- bash -c "echo 'written by a person besides the operator input:'; cat '$HAND'; [ -s '$HAND' ] || echo none"
  collect fg2
}

unit_main() { fg2_main; }
