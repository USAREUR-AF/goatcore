#!/usr/bin/env bash
# units/nodesec.sh — node security: after a full federation, each peering edge host and
# each core router host answers on exactly the ports its declared profile names, seen from
# the partner side of the tunnel and from the sovereign's internal side. Sourced by
# lib/unit.sh.
#
# Scenarios that read this unit (scenarios/catalog.yaml): node-ports-declared,
# node-firewall-profile, node-firewall-three-party.
#
# THE DECLARED SET. When the ref under test carries a host firewall profile (found by
# fw_profile_find below), that profile is the declaration. When it carries none, the
# declaration is the operator's baseline for the peering edge and the core router:
#   edge, partner side    udp: the tunnel port of that peering, on the boundary address
#                         tcp: the edge router port (7448), on the tunnel address only
#   edge, internal side   tcp 22 from the declared management source, and nothing else:
#                         under ADR 1219 only platform admins reach a peering edge, on 22
#   core, internal side   tcp 7447 (fabric TLS), tcp 22 from the management source, and
#                         the mesh port (none here: the lab runs no mesh daemon)
#   core, partner side    nothing
# The management source in the lab is a scanner container on the sovereign's internal
# network ($NB.<octet>.30). Two further lists are allowed at their exact ports, never
# silently: each is printed with the scan and named in the check's reason.
#   SPIRE forwards        what the ref's peering roles open for SPIRE federation (phase 13):
#                         the bundle port on the tunnel address, the outbound forward on
#                         the edge's internal address
#   drill additions       what only this harness runs: the core's REST port 8000 and the
#                         SPIRE server as the engine runs it (8443) on the internal network,
#                         and the probe sessions' REST ports on the edge's internal side
#
# EACH SCAN. A python scanner (lib/portscan.py, piped into the scanning host: the host
# image has no nmap) tries every TCP port and the UDP ports in NS_UDP. A UDP port alone
# cannot tell open from dropped, so the target holds counters (table inet fe2e_scan, after
# every other filter) for the datagrams the scan delivered. lib/portjudge.py decides, and
# the full record goes to RECORD/<step>.json.

NS_UDP="53,123,500,3478,4500,5353,7447,7448,8443,9310,18443,18444,51820,51821,51822,51823"
export NS_UDP

# fw_profile_find: whether the ref under test declares a host firewall profile for the
# edge and core hosts, printing each hit as "<path>: <what matched>". The search, in full:
#   1. a role under ops/ansible/roles whose directory name contains firewall, host-fw,
#      hostfw, nodesec or node-security;
#   2. a variable in any role's defaults/main.yml whose name contains firewall, host_fw,
#      hostfw or node_fw (e.g. goat_host_firewall_profile);
#   3. a template in any role named *firewall*.j2, *host-fw*.j2 or *hostfw*.j2.
# The peering tunnel's own filter (goat-peering-wg's peering-wg.nft.j2, variables
# peering_wg_*) matches none of these and is not a host profile: it filters the tunnel
# interface only. Exit 0 when anything matched.
fw_profile_find() {
  local roles="$SRC/ops/ansible/roles" hit=1 d f
  for d in "$roles"/*; do
    case "$(basename "$d")" in
      *firewall*|*host-fw*|*hostfw*|*nodesec*|*node-security*) echo "${d#"$SRC"/}: role name"; hit=0 ;;
    esac
  done
  for f in "$roles"/*/defaults/main.yml; do
    [ -f "$f" ] || continue
    if grep -qE '^[a-z0-9_]*(firewall|host_fw|hostfw|node_fw)[a-z0-9_]*:' "$f"; then
      echo "${f#"$SRC"/}: $(grep -oE '^[a-z0-9_]*(firewall|host_fw|hostfw|node_fw)[a-z0-9_]*' "$f" | tr '\n' ' ')"; hit=0
    fi
  done
  for f in "$roles"/*/templates/*firewall*.j2 "$roles"/*/templates/*host-fw*.j2 "$roles"/*/templates/*hostfw*.j2; do
    [ -f "$f" ] && { echo "${f#"$SRC"/}: template name"; hit=0; }
  done
  return "$hit"
}

# fw_profile_ports: the declared set from the ref's own profile, as portjudge's
# "declared" object for $1 role (edge|core) and $2 side (boundary|tunnel|internal), or
# nothing when this harness cannot read the profile. No profile exists on main yet, so its
# variable names and shape are unknown: this reads nothing until they are, and the
# operator's baseline (ns_baseline) stands in, which the check's reason states.
fw_profile_ports() { return 1; }

ns_baseline() { # $1 role, $2 side, $3 us, $4 the partner on that side -> {"tcp":[..],"udp":[..]}
  case "$1:$2" in
    edge:boundary) echo "{\"tcp\": [], \"udp\": [$(wgport "$3" "$4")]}" ;;
    edge:tunnel) echo '{"tcp": [7448], "udp": []}' ;;
    edge:core) echo '{"tcp": [], "udp": []}' ;;
    edge:mgmt) echo '{"tcp": [22], "udp": []}' ;;
    core:mgmt) echo '{"tcp": [22, 7447], "udp": []}' ;;
    core:partner) echo '{"tcp": [], "udp": []}' ;;
  esac
}

# The SPIRE forwards and drill surfaces at their exact ports, for $1 role, $2 side, $3 us.
ns_spire() {
  local t l=""
  case "$1:$2" in
    edge:tunnel) echo '{"tcp": [8443], "udp": []}' ;;
    edge:core|edge:mgmt) for t in $(partners "$3"); do l="$l${l:+, }$(spire_out "$3" "$t")"; done; echo "{\"tcp\": [$l], \"udp\": []}" ;;
    *) echo '{"tcp": [], "udp": []}' ;;
  esac
}
ns_drill() {
  local t l=""
  case "$1:$2" in
    edge:core|edge:mgmt)
      for t in $(partners "$3"); do
        l="$l${l:+, }$(probe_port "$3" "$t" 8101), $(probe_port "$3" "$t" 8102), $(probe_port "$3" "$t" 8103)"
      done
      echo "{\"tcp\": [$l], \"udp\": []}" ;;
    core:mgmt) echo '{"tcp": [8000, 8443], "udp": []}' ;;
    *) echo '{"tcp": [], "udp": []}' ;;
  esac
}

ns_expect() { # $1 role, $2 side, $3 us, $4 partner -> portjudge's EXPECT object
  local decl src notes='[]'
  if decl="$(fw_profile_ports "$1" "$2" "$3" "$4")" && [ -n "$decl" ]; then
    src="the ref's host firewall profile"
  else
    decl="$(ns_baseline "$1" "$2" "$3" "$4")"
    if [ -s "$RECORD/nodesec-profile.txt" ]; then
      src="operator baseline: the ref declares a host firewall profile ($(head -1 "$RECORD/nodesec-profile.txt" | cut -d: -f1)) this harness does not read yet"
    else
      src="operator baseline: the ref declares no host firewall profile"
    fi
  fi
  [ "$1:$2" = core:mgmt ] && notes='["the core mesh port is not in the set: the lab runs no mesh daemon"]'
  python3 -c 'import json,sys; print(json.dumps({"declared": json.loads(sys.argv[1]), "spire": json.loads(sys.argv[2]), "drill": json.loads(sys.argv[3]),
    "spire_why": "SPIRE federation forwards the peering roles open", "drill_why": "drill additions (core REST 8000, SPIRE server as the engine runs it, probe REST ports)",
    "source": sys.argv[4], "notes": json.loads(sys.argv[5])}))' "$decl" "$(ns_spire "$1" "$2" "$3")" "$(ns_drill "$1" "$2" "$3")" "$src" "$notes"
}

# Delivery counters on the target for the UDP ports, from one source address.
ns_counters_on() { # $1 target host container, $2 source address
  local p rules=""
  for p in $(echo "$NS_UDP" | tr ',' ' '); do
    rules="$rules    ip saddr $2 udp dport $p counter
"
  done
  local fwd; fwd="$(echo "$rules" | sed 's/udp dport/meta l4proto udp ct original proto-dst/')"
  docker exec -i "$1" nft -f - <<NFT
table inet fe2e_scan
delete table inet fe2e_scan
table inet fe2e_scan {
  chain scan_in {
    type filter hook input priority 500; policy accept;
$rules  }
  chain scan_fwd {
    type filter hook forward priority 500; policy accept;
$fwd
  }
}
NFT
}
ns_counters_off() { # $1 target, $2 file: save the counters, then remove the table
  docker exec "$1" nft list table inet fe2e_scan > "$2" 2>&1 || true
  docker exec "$1" nft delete table inet fe2e_scan >/dev/null 2>&1 || true
}

# A partner's own edge filters what it sends onto the tunnel (goat-peering-wg's
# onto_tunnel). The question is what OUR edge answers to a partner that does not filter,
# so that chain is opened for the scan and closed after.
ns_lift_egress() { # $1 the scanning party, $2 the target party
  local i t; i="$(iface "$1" "$2")"; t="goat_peering_$(echo "$i" | tr -c 'a-zA-Z0-9_\n' _)"
  docker exec "$HP-$1-edge" nft insert rule inet "$t" onto_tunnel oifname "$i" accept comment '"fe2e-scan"'
}
ns_restore_egress() {
  local i t h; i="$(iface "$1" "$2")"; t="goat_peering_$(echo "$i" | tr -c 'a-zA-Z0-9_\n' _)"
  for h in $(docker exec "$HP-$1-edge" nft -a list chain inet "$t" onto_tunnel 2>/dev/null | grep fe2e-scan | sed -n 's/.*# handle \([0-9]*\).*/\1/p'); do
    docker exec "$HP-$1-edge" nft delete rule inet "$t" onto_tunnel handle "$h"
  done
}

link_ip() { # $1 party, $2 partner: the party's own address on that peering's tunnel
  docker exec "$HP-$1-edge" ip -o -4 addr show dev "$(iface "$1" "$2")" 2>/dev/null | awk '{print $4}' | cut -d/ -f1
}

# One scan, judged and recorded. $1 step, $2 how to run the scanner (scanning host
# container, or "scanner:<party>"), $3 scanning source address, $4 target host container,
# $5 target address, $6 role, $7 side, $8 us, $9 partner.
ns_scan() {
  local step="$1" from="$2" saddr="$3" tgt="$4" taddr="$5" role="$6" side="$7" us="$8" them="$9"
  local scan="$RECORD/$step.scan" rc reason
  ns_expect "$role" "$side" "$us" "$them" > "$WORK/$step.expect.json"
  docker exec "$tgt" ss -Htulnp > "$RECORD/$step.listening.txt" 2>&1 || true
  ns_counters_on "$tgt" "$saddr" > "$RECORD/$step.counters-setup.txt" 2>&1 || true
  case "$from" in
    scanner:*)
      local p="${from#scanner:}"
      docker rm -f "$HP-$p-scanner" >/dev/null 2>&1 || true
      docker run --rm -i --name "$HP-$p-scanner" "${LABEL[@]}" --network "$NP-$p" --ip "$saddr" \
        --entrypoint python3 "$HOST_IMAGE" - "$taddr" --udp "$NS_UDP" < "$HERE/lib/portscan.py" > "$scan" 2>"$scan.err" || true ;;
    *) docker exec -i "$from" python3 - "$taddr" --udp "$NS_UDP" < "$HERE/lib/portscan.py" > "$scan" 2>"$scan.err" || true ;;
  esac
  ns_counters_off "$tgt" "$RECORD/$step.counters.txt"
  rc=0
  python3 "$HERE/lib/portjudge.py" --scan "$scan" --counters "$RECORD/$step.counters.txt" --expect "$WORK/$step.expect.json" \
    --label "$step (from $saddr)" --out "$RECORD/$step.json" > "$RECORD/$step.txt" 2>&1 || rc=$?
  reason="$(sed -n 's/^reason: //p' "$RECORD/$step.txt")"
  if [ "$rc" = 0 ]; then
    rec "$step" -- cat "$RECORD/$step.txt"
  else
    # The judge's own line becomes the check's reason: what answered beyond the set,
    # and the separately listed additions.
    rec "$step" --expect-not-in "${reason:-not exactly the declared ports}" -- cat "$RECORD/$step.txt"
  fi
}

# Every scan of one party's edge and core. $1 step prefix, $2 us.
ns_scan_party() {
  local pfx="$1" s="$2" t ip o; o="$(octet "$s")"
  for t in $(partners "$s"); do
    ns_lift_egress "$t" "$s" || true
    ns_scan "$pfx-$s-edge-from-$t-edge-boundary" "$HP-$t-edge" "$(boundary_ip "$t")" "$HP-$s-edge" "$(boundary_ip "$s")" edge boundary "$s" "$t"
    ip="$(link_ip "$s" "$t")"
    if [ -n "$ip" ]; then
      ns_scan "$pfx-$s-edge-from-$t-edge-tunnel" "$HP-$t-edge" "$(link_ip "$t" "$s")" "$HP-$s-edge" "$ip" edge tunnel "$s" "$t"
    else
      rec "$pfx-$s-edge-from-$t-edge-tunnel" -- bash -c "echo 'no address on $(iface "$s" "$t") of $s: the tunnel is not up'; exit 1"
    fi
    ns_scan "$pfx-$s-core-from-$t-edge" "$HP-$t-edge" "$(edge_ip "$t")" "$HP-$s-core" "$(core_ip "$s")" core partner "$s" "$t"
    ns_restore_egress "$t" "$s" || true
  done
  t="$(partners "$s" | awk '{print $1}')"
  ns_scan "$pfx-$s-edge-from-$s-core" "$HP-$s-core" "$(core_ip "$s")" "$HP-$s-edge" "$(edge_ip "$s")" edge core "$s" "$t"
  ns_scan "$pfx-$s-edge-from-$s-scanner" "scanner:$s" "$NB.$o.30" "$HP-$s-edge" "$(edge_ip "$s")" edge mgmt "$s" "$t"
  ns_scan "$pfx-$s-core-from-$s-scanner" "scanner:$s" "$NB.$o.30" "$HP-$s-core" "$(core_ip "$s")" core mgmt "$s" "$t"
}

ns_tunnel_up() { # $1 us, $2 them: a handshake on the peering, or die
  local t
  for _ in $(seq 1 30); do
    t="$(docker exec "$HP-$1-edge" wg show "$(iface "$1" "$2")" latest-handshakes 2>/dev/null | cut -f2)"
    [ "${t:-0}" != 0 ] && return 0
    sleep 2
  done
  die "the $1-$2 peering has no WireGuard handshake: the federation the scans need did not come up"
}

# The checks that need a profile, when the ref has none, or one this unit cannot drive.
ns_profile_blocked() { # $1 reason
  local c
  for c in a-edge-boundary a-edge-tunnel a-core-partner a-edge-core a-edge-mgmt a-core-mgmt \
           b-edge-boundary b-edge-tunnel b-core-partner b-edge-core b-edge-mgmt b-core-mgmt \
           override-a-edge-core override-a-edge-mgmt override-b-edge-core override-b-edge-mgmt; do
    mark "node-firewall-profile.$c" BLOCKED "$1"
  done
  for c in ab-acceptance ac-acceptance b-receives-own c-receives-own a-edge-from-b-boundary a-edge-from-c-boundary \
           a-edge-from-b-tunnel a-edge-from-c-tunnel a-edge-core a-edge-mgmt a-core-mgmt; do
    mark "node-firewall-three-party.$c" BLOCKED "$1"
  done
}

# How to tighten the ref's profile: SSH on the edge from one management address only.
# Unknown until the profile exists (its override variable is not named on main); returns 1
# so that the override checks read BLOCKED with that reason.
fw_profile_override() { return 1; }

unit_main() {
  set_partners "$A" "$B"; set_partners "$B" "$A"; set_partners "$C" ""; NODIR=""
  export_helpers
  local found=0
  fw_profile_find > "$RECORD/nodesec-profile.txt" 2>&1 && found=1
  setup_hosts 1
  peer_up 1 "$A" "$B" "$AVENUE"
  ns_tunnel_up "$A" "$B"

  say "node security: every port each edge and core answers on, from each side"
  ns_scan_party ns "$A"
  ns_scan_party ns "$B"

  if [ "$found" = 0 ]; then
    ns_profile_blocked "the ref declares no host firewall profile (searched: fw_profile_find in units/nodesec.sh)"
    return 0
  fi
  local where; where="$(head -1 "$RECORD/nodesec-profile.txt")"
  if ! fw_profile_ports edge boundary "$A" "$B" >/dev/null; then
    ns_profile_blocked "the ref declares a host firewall profile ($where), but this unit does not yet read its declared set (fw_profile_ports in units/nodesec.sh)"
    return 0
  fi
  # The roles the driver ran applied the profile: the baseline scans, under their own names.
  say "node security: the baseline profile on"
  ns_scan_party nsp "$A"
  ns_scan_party nsp "$B"
  say "node security: one tightening override"
  if fw_profile_override; then
    local s; for s in "$A" "$B"; do
      ns_scan "nso-$s-edge-from-$s-core" "$HP-$s-core" "$(core_ip "$s")" "$HP-$s-edge" "$(edge_ip "$s")" edge core "$s" "$(other "$s")"
      ns_scan "nso-$s-edge-from-$s-scanner" "scanner:$s" "$NB.$(octet "$s").30" "$HP-$s-edge" "$(edge_ip "$s")" edge mgmt "$s" "$(other "$s")"
    done
  else
    for c in override-a-edge-core override-a-edge-mgmt override-b-edge-core override-b-edge-mgmt; do
      mark "node-firewall-profile.$c" BLOCKED "the ref's host firewall profile ($where) names no override this unit can set yet (fw_profile_override in units/nodesec.sh)"
    done
  fi

  say "node security: three parties on one edge and one core, baseline on"
  if [ "$AVENUE" = goatbuilder ]; then scenario3 goatbuilder script; else scenario3 script script; fi
  ns_scan_party ns3 "$A"
}
