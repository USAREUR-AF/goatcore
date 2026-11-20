#!/usr/bin/env bash
# units/forwarder-gb.sh — the peering forwarder applied by `goatbuilder federate` itself, from
# the peering's `forwarder:` declaration (docs/design/peering-forwarder.md §7, "Applied by the
# driver from the peering declaration"). Scenario: forwarder-by-goatbuilder.
#
# nothing-hand-written (units/nohand.sh), with {B} behind a NAT router that forwards nothing
# inbound and listening through a peering forwarder it operates, the network of
# units/forwarder.sh. {B}'s operator input is nothing-hand-written's, with one more part: the
# peering entry's `forwarder:`, its `endpoint` and `ansible_host` (NH_FORWARDER_<side>) and the
# forwarder machine's own values (nh_operator_more below), `ansible_user`, `public_interface`
# and `ssh_sources`: the user it is reached as, its public interface, and the address it
# admits SSH from. No group_vars file is written by hand. Nobody runs a play by hand:
# `goatbuilder federate allocate` renders the forwarder's inventory group, its group_vars and
# the offer of its public port, and phase 2 of `goatbuilder federate {A}` on {B} runs
# peering-prepare.yml, peering-carrier-keys.yml, peering-forwarder.yml, the keys play again
# and peering-prepare.yml again (federate/forwarder.rs, splice). units/forwarder.sh still
# brings its forwarder up with the operator's three plays: it is the hand route this one
# replaces.
#
# One peering. Two peerings relayed by one forwarder (§7's full gate) need nohand.sh to
# declare a second peering of one side from nothing (nh_side_from_nothing writes exactly one
# `peerings:` entry) and a third sovereign driven by the one command beside the first.
# Sourced by lib/unit.sh.
# shellcheck source=/dev/null
. "$HERE/units/nohand.sh"
# The NAT router, the forwarder host, the probes and the forwarder's key search. Its own
# unit_main is replaced below.
# shellcheck source=/dev/null
. "$HERE/units/forwarder.sh"

# {B} is the side behind the NAT: nothing-hand-written's listening side. Its NAT router and
# forwarder are named for it (units/forwarder.sh named them for {A} when it was sourced).
NATED="$B"; PARTNER="$A"
HOSTS=("${HOSTS[@]:0:${#HOSTS[@]}-2}")
FWD_HOST="$HP-$B-fwd"; NAT_HOST="$HP-$B-nat"
HOSTS+=("$NAT_HOST" "$FWD_HOST")
export NATED PARTNER FWD_HOST NAT_HOST

# A value staged on one core arrives at the other: each core's own REST surface, on the core
# host itself (a goat-zenoh role core binds loopback, so lib/engine.sh's round_trip, which
# reads the core from this machine, does not reach it; units/offer.sh reads it the same way).
fgb_arrives() { # $1 sender, $2 receiver, $3 seconds
  local n t0 got; n="fgb$(date +%s)$RANDOM"; t0="$(date +%s)"
  while [ $(($(date +%s) - t0)) -lt "$3" ]; do
    docker exec "$HP-$1-core" curl -s -m 8 -X PUT -d "$n" "http://127.0.0.1:8000/release/all/fgb/$n" -o /dev/null
    got="$(docker exec "$HP-$2-core" curl -s -m 5 "http://127.0.0.1:8000/from/$1/all/fgb/$n" || true)"
    case "$got" in *"from/$1/all/fgb/$n"*) echo "PASS: from/$1/all/fgb/$n stored at $2 after $(($(date +%s) - t0))s"; return 0 ;; esac
    sleep 3
  done
  echo "from/$1/all/fgb/$n not stored at $2 within $3s"; return 1
}
export -f fgb_arrives

# The forwarder machine's own values, the operator's input for a machine only they run, on
# the peering entry's `forwarder:` beside its endpoint: the user the play reaches it as (the
# rendered inventory names only its address), the interface its partners reach it on, and
# the one address it admits SSH from ({B}'s NAT, which the edge's SSH leaves through).
# `goatbuilder federate allocate` renders them into the forwarder group's
# peering-forwarder.yml (docs/design/peering-forwarder.md §7). Then the forwarder host itself,
# keyed for {B}'s operator. Called by nh_side_from_nothing before allocate.
nh_operator_more() { # $1 us, $2 them
  [ "$1" = "$NATED" ] || return 0
  fwd_up
  nh_forwarder_values "$1" "$2"
}
# The forwarder machine's own values on $1's peering entry with $2 (each peering a
# forwarder relays names them on its own entry, docs/design/peering-forwarder.md §7).
nh_forwarder_values() { # $1 us, $2 them
  local c="$WORK/$1/clone"
  python3 - "$c/ops/goatbuilder/goatbuilder.yaml" "$2" goat "$(ifname_of "$FWD_HOST" "$FWD_PUB")" "$(boundary_ip "$1")" <<'PY'
import sys, yaml
p, them, user, iface, source = sys.argv[1:6]
text = open(p).read()
cfg = yaml.safe_load(text)
fwd = next(e for e in cfg["peerings"] if e["with"] == them)["forwarder"]
fwd.update({"ansible_user": user, "public_interface": iface, "ssh_sources": [source]})
open(p, "w").write(text.split("\nschema:")[0] + "\n" + yaml.safe_dump(cfg, sort_keys=False))
PY
  cp "$c/ops/goatbuilder/goatbuilder.yaml" "$RECORD/operator-input/$1/"
  nh_push "$1: the forwarder machine it operates, on its peering's forwarder: entry with $2"
}

fgb_main() {
  local h c t i gen
  eval "NH_FORWARDER_$NATED=$FWD_PUB"; export "NH_FORWARDER_$NATED"
  eval "NH_MTU_$NATED=1300"; export "NH_MTU_$NATED"
  nh_main
  h="$(drv_host "$NATED")"; c="$WORK/$NATED/clone"
  gen="# GENERATED by \`goatbuilder federate allocate\`"

  say "forwarder-by-goatbuilder: what allocate rendered for the forwarder"
  # The peering's own inventory file: ops/ansible/inventory/peering-edges.ini stays the
  # deployment's own while not every peering declares hosts:, and allocate renders this
  # peering's groups, the forwarder's among them, into peering-<partner>_edge.ini.
  rec "fgb-$NATED-inventory-names-the-forwarder" --expect-in "[peering_forwarders]" --expect-in "ansible_host=$FWD_PUB" -- bash -c \
    "f='$c/ops/ansible/inventory/peering-${PARTNER}_edge.ini'; cat \"\$f\"; head -1 \"\$f\" | grep -qF '$gen'"
  rec "fgb-$NATED-forwarder-values-from-the-command" --expect-in "name: \"$PARTNER\"" --expect-in "peering_forwarder_enabled: true" \
    --expect-in 'ansible_user: "goat"' --expect-in "peering_forwarder_public_interface: \"$(ifname_of "$FWD_HOST" "$FWD_PUB")\"" \
    --expect-in "peering_forwarder_ssh_sources:" -- bash -c \
    "f='$c/ops/ansible/group_vars/peering_forwarders/peering-forwarder.yml'; cat \"\$f\"; head -1 \"\$f\" | grep -qF '$gen'"
  rec "fgb-$NATED-no-hand-written-forwarder-file" -- bash -c \
    "ls '$c/ops/ansible/group_vars/peering_forwarders/'; [ \"\$(ls '$c/ops/ansible/group_vars/peering_forwarders/')\" = peering-forwarder.yml ]"
  rec "fgb-$NATED-federation-toml-offers-the-forwarder" --expect-in "endpoint             = \"$FWD_PUB:" --expect-in "listener             = true" -- \
    cat "$c/ops/federation/$NATED/peerings/$PARTNER/federation.toml"

  say "forwarder-by-goatbuilder: phase 2 on $NATED ran the forwarder"
  # The transcripts of every run of the command on {B}: phase 2's forwarder play, its recap,
  # and the forwarder's carrier key that the keys play copied to the driver.
  rec "fgb-$NATED-phase-2-ran-the-forwarder-play" --expect-in "behind the forwarder $FWD_PUB" --expect-in "peering-forwarder.yml" -- bash -c \
    "t=\$(ls '$RECORD'/$OC_PREFIX-$NATED-*.log 2>/dev/null); [ -n \"\$t\" ] || { echo 'no run of the command on $NATED'; exit 1; }
     sed 's/\x1b\[[0-9;]*m//g' \$t | tr -d '\r' | grep -E 'behind the forwarder|peering-forwarder.yml|peering-carrier-keys.yml|forwarder-carrier' | sort -u | head -20
     docker exec $h test -s /var/lib/goat-federation/$PARTNER/pki/forwarder-carrier.pub && echo 'the forwarder carrier key is on the driver'"
  rec "fgb-forwarder-carrier-and-table-in-place" --expect-in '"holds_peering_material": false' --expect-in "dnat ip to 169.254.30.1:" -- \
    docker exec "$FWD_HOST" sh -c 'systemctl is-active goat-peering-forwarder@wg-carry0 && nft list table inet goat_peering_forwarder && cat /var/log/goat-peering-forwarder-state.json'
  rec "fgb-$NATED-carrier-handshake" -- bash -c \
    "for _ in \$(seq 1 30); do t=\$(docker exec $h wg show wg-carry0 latest-handshakes 2>/dev/null | cut -f2); [ \"\${t:-0}\" != 0 ] && break; sleep 2; done; docker exec $h wg show wg-carry0; [ \"\${t:-0}\" != 0 ]"

  say "forwarder-by-goatbuilder: what the path is, and what the forwarder holds"
  i="$(iface "$PARTNER" "$NATED")"
  rec "fgb-$PARTNER-endpoint-is-the-forwarder" --expect-in "$FWD_PUB:" --expect-not-in "$(boundary_ip "$NATED"):" -- \
    docker exec "$HP-$PARTNER-edge" wg show "$i" endpoints
  rec "fgb-$NATED-sees-$PARTNER-at-the-carrier" --expect-in "169.254.30.0:" -- \
    docker exec "$h" wg show "$(iface "$NATED" "$PARTNER")" endpoints
  for t in "$NATED" "$PARTNER"; do
    rec "fgb-$t-tunnel-carried-it" -- bash -c \
      "docker exec $HP-$t-edge wg show $(iface "$t" "$(other "$t")") transfer | awk '{print; if (\$2 < 1 || \$3 < 1) exit 1}'"
  done
  rec "fgb-$PARTNER-cannot-reach-$NATED-edge-through-its-nat" --expect-exit 1 -- bash -c probe_nated_edge
  nat_load open
  rec "fgb-the-same-probe-reaches-$NATED-edge-with-the-nat-open" -- bash -c probe_nated_edge
  nat_load closed
  rec "fgb-round-trip-$PARTNER-to-$NATED" --expect-in "PASS" -- bash -c "fgb_arrives '$PARTNER' '$NATED' 90"
  rec "fgb-round-trip-$NATED-to-$PARTNER" --expect-in "PASS" -- bash -c "fgb_arrives '$NATED' '$PARTNER' 90"
  rec "fgb-forwarder-holds-no-peering-key" -- bash -c fwd_holds_nothing
  docker exec "$FWD_HOST" sh -c 'wg show; nft list ruleset' > "$RECORD/fgb-forwarder-state.txt" 2>&1 || true
  docker exec "$h" sh -c 'wg show; ip route; nft list ruleset' > "$RECORD/fgb-$NATED-edge-state.txt" 2>&1 || true
}

unit_main() { fgb_main; }
