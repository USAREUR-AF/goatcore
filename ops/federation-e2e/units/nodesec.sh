#!/usr/bin/env bash
# units/nodesec.sh — node security: after a full federation, each peering edge host and
# each core router host answers on exactly the ports its declared profile names, seen from
# the partner side of the tunnel and from the sovereign's internal side. Sourced by
# lib/unit.sh.
#
# Scenarios that read this unit (scenarios/catalog.yaml): node-ports-declared,
# node-firewall-profile, node-firewall-three-party.
#
# THE DECLARED SET. When the host carries the ref's host firewall baseline (the
# goat-host-firewall role, found by fw_profile_find and switched on by fw_profile_on), the
# declaration is the rule set that role loaded on that host, read by fw_profile_ports: the
# ports its accept rules admit from the scanning source on the interface it arrives on.
# Otherwise (a ref with no such role, or a host where the switch is still off) the
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
# Under the ref's rule set those two lists are empty: the rule set is the whole
# declaration, and the drill's own surfaces are in it only as the deployment's named,
# reasoned loosenings (fw_profile_vars), which the scan's notes print.
#
# EACH SCAN. A python scanner (lib/portscan.py, piped into the scanning host: the host
# image has no nmap) tries every TCP port and the UDP ports in NS_UDP. A UDP port alone
# cannot tell open from dropped, so the target holds counters (table inet fe2e_scan, after
# every other filter) for the datagrams the scan delivered. lib/portjudge.py decides, and
# the full record goes to RECORD/<step>.json.

NS_UDP="53,123,500,3478,4500,5353,7447,7448,8443,9310,18443,18444,51820,51821,51822,51823"
export NS_UDP

# The role this unit drives, in the tree under test.
FW_ROLE="ops/ansible/roles/goat-host-firewall"

# fw_profile_find: whether the ref under test carries the goat-host-firewall role, printing
# what was found as "<path>: <what>". The first line names the role; the next lines name
# each play that includes it (the driver runs peering-wg.yml in phase 11 and
# peering-republisher.yml in phase 12) and its renderer. Exit 0 when the role is there with
# its switch (goat_host_firewall_baseline) and its renderer.
#
# On a ref without it, a broader search still prints anything that looks like another host
# firewall profile (a role named *firewall*, *host-fw*, *nodesec*; a defaults variable
# named *firewall*, *host_fw*, *node_fw*; a *firewall*.j2 template), so the BLOCKED reason
# names it; this unit drives none of those. The peering tunnel's own filter
# (goat-peering-wg's peering-wg.nft.j2, peering_wg_*) is not a host profile and matches none.
fw_profile_find() {
  local roles="$SRC/ops/ansible/roles" d f p
  if [ -f "$SRC/$FW_ROLE/defaults/main.yml" ] && grep -q '^goat_host_firewall_baseline:' "$SRC/$FW_ROLE/defaults/main.yml" \
     && [ -f "$SRC/$FW_ROLE/files/goat-host-firewall-render.py" ]; then
    echo "$FW_ROLE: role, switch goat_host_firewall_baseline (default $(sed -n 's/^goat_host_firewall_baseline: *//p' "$SRC/$FW_ROLE/defaults/main.yml"))"
    for p in peering-wg.yml peering-republisher.yml peering-edge.yml; do
      if grep -q 'name: goat-host-firewall' "$SRC/ops/ansible/$p" 2>/dev/null; then
        echo "ops/ansible/$p: includes goat-host-firewall ($(sed -n 's/^ *goat_host_firewall_profile: *\([a-z]*\).*/profile \1/p' "$SRC/ops/ansible/$p" | head -1))"
      else
        echo "ops/ansible/$p: does not include goat-host-firewall"
      fi
    done
    echo "$FW_ROLE/files/goat-host-firewall-render.py: renderer"
    return 0
  fi
  for d in "$roles"/*; do
    case "$(basename "$d")" in
      *firewall*|*host-fw*|*hostfw*|*nodesec*|*node-security*) echo "${d#"$SRC"/}: role name, not goat-host-firewall" ;;
    esac
  done
  for f in "$roles"/*/defaults/main.yml; do
    [ -f "$f" ] || continue
    if grep -qE '^[a-z0-9_]*(firewall|host_fw|hostfw|node_fw)[a-z0-9_]*:' "$f"; then
      echo "${f#"$SRC"/}: $(grep -oE '^[a-z0-9_]*(firewall|host_fw|hostfw|node_fw)[a-z0-9_]*' "$f" | tr '\n' ' ')"
    fi
  done
  for f in "$roles"/*/templates/*firewall*.j2 "$roles"/*/templates/*host-fw*.j2 "$roles"/*/templates/*hostfw*.j2; do
    [ -f "$f" ] && echo "${f#"$SRC"/}: template name"
  done
  return 1
}

# fw_profile_ports: the declared set from the rule set the ref's goat-host-firewall role
# loaded on one host, for one scanning source. $1 target host container, $2 scanning
# source address, $3 a file prefix to keep the evidence under. Prints lib/fwdeclared.py's
# object ({"tcp", "udp", "by_port", "changes", ...}) plus "iif" (the interface the scan
# arrives on), "applied" (the renderer's hash the role recorded) and "live" (whether the
# table loaded now is the one the role confirmed), or returns 1 when the host carries no
# loaded baseline table.
#
# What the role leaves on a host it applied (tasks/apply.yml): the rendered rule set in
# /etc/goat-host-firewall/ruleset.nft, the renderer's hash in applied.sha256, and the hash
# of `nft -s list table inet goat_host_firewall` it confirmed in applied-live.sha256. The
# interface the scan arrives on is the one the target routes the source through.
fw_profile_ports() {
  local tgt="$1" saddr="$2" pfx="$3" iif live want
  if ! docker exec "$tgt" nft -s list table inet goat_host_firewall > "$pfx.loaded.nft" 2>&1; then
    # No such table, or no command runs in the host at all (the daemon, or a full /tmp
    # for runc): only the first is the host's answer.
    docker exec "$tgt" nft list tables > "$pfx.tables.txt" 2>&1 || return 2
    return 1
  fi
  docker exec "$tgt" cat /etc/goat-host-firewall/ruleset.nft > "$pfx.ruleset.nft" 2>/dev/null || return 1
  live="$(sha256sum < "$pfx.loaded.nft" | cut -d' ' -f1)"
  want="$(docker exec "$tgt" cat /etc/goat-host-firewall/applied-live.sha256 2>/dev/null | tr -cd 'a-f0-9')"
  iif="$(docker exec "$tgt" ip -o route get "$saddr" 2>/dev/null | sed -n 's/.* dev \([^ ]*\).*/\1/p' | head -1)"
  python3 "$HERE/lib/fwdeclared.py" --ruleset "$pfx.ruleset.nft" --saddr "$saddr" --iif "${iif:-}" > "$pfx.declared.json" || return 1
  python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); d.update(iif=sys.argv[2], applied=sys.argv[3], live=(sys.argv[4] == sys.argv[5])); print(json.dumps(d))' \
    "$pfx.declared.json" "${iif:-}" "$(docker exec "$tgt" cat /etc/goat-host-firewall/applied.sha256 2>/dev/null | tr -cd 'a-f0-9')" "$want" "$live"
}

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

# portjudge's EXPECT object for one scan. $1 role, $2 side, $3 us, $4 partner, $5 target
# host container, $6 scanning source address, $7 evidence file prefix.
# NS_PROFILE_REQUIRED=1 (the baseline was switched on): the target's loaded rule set is
# the only declaration, and a target without one prints nothing and returns 1.
ns_expect() {
  local decl src notes='[]' prof rc=0
  prof="$(fw_profile_ports "$5" "$6" "$7")" || rc=$?
  [ "$rc" = 2 ] && return 2
  if [ "$rc" = 0 ] && [ -n "$prof" ]; then
    python3 - "$prof" "$1:$2" "$5" "$3" "$4" <<'PY'
import json, sys
p, side, tgt, us, them = json.loads(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5]
tcp, udp, notes = list(p["tcp"]), list(p["udp"]), []
for k in sorted(p["by_port"], key=lambda k: (k.split("/")[0], int(k.split("/")[1]))):
    notes.append(f"{k} admitted by rule {', '.join(p['by_port'][k])}")
for c in p.get("changes", []):
    notes.append(f"the deployment's override: {c}")
if not p.get("live"):
    notes.append("the table loaded now is not the one the role confirmed (applied-live.sha256)")
if side == "core:partner" and (tcp or udp):
    # The rule set admits these from any source; the lab gives the partner's edge no route
    # to our core (setup_hosts checks s<n>-<them>-edge-cannot-reach-<us>-core), so they
    # cannot answer and are not expected to.
    notes.append(f"admitted from any source but unreachable from the partner's edge (no route; "
                 f"s<n>-{them}-edge-cannot-reach-{us}-core): tcp {tcp} udp {udp}, not expected to answer")
    tcp, udp = [], []
print(json.dumps({"declared": {"tcp": tcp, "udp": udp}, "spire": {"tcp": [], "udp": []}, "drill": {"tcp": [], "udp": []},
                  "source": f"the ref's goat-host-firewall rule set as loaded on {tgt} (sha256 {p.get('applied') or p.get('sha256') or '?'}), from this source on {p.get('iif') or '?'}",
                  "notes": notes}))
PY
    return 0
  fi
  [ "${NS_PROFILE_REQUIRED:-0}" = 1 ] && return 1
  decl="$(ns_baseline "$1" "$2" "$3" "$4")"
  if [ -s "$RECORD/nodesec-profile.txt" ] && [ "${NS_FOUND:-0}" = 1 ]; then
    src="operator baseline: the ref's host firewall baseline ($FW_ROLE) is not switched on for this host (goat_host_firewall_baseline false)"
  elif [ -s "$RECORD/nodesec-profile.txt" ]; then
    src="operator baseline: the ref has no goat-host-firewall role; it has $(head -1 "$RECORD/nodesec-profile.txt" | cut -d: -f1), which this unit does not drive"
  else
    src="operator baseline: the ref declares no host firewall profile"
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
  local scan="$RECORD/$step.scan" rc=0 reason
  ns_expect "$role" "$side" "$us" "$them" "$tgt" "$saddr" "$RECORD/$step" > "$WORK/$step.expect.json" || rc=$?
  if [ "$rc" = 2 ]; then
    die "no command runs in $tgt to read its firewall ($(head -1 "$RECORD/$step.tables.txt" 2>/dev/null)); an environment fault, not the tree under test"
  fi
  if [ "$rc" != 0 ]; then
    # The switch is on in this sovereign's group_vars and the driver's phases ran, yet the
    # host carries no baseline table: the declaration the check compares against is absent.
    rec "$step" -- bash -c "echo '$tgt carries no table inet goat_host_firewall although goat_host_firewall_baseline is true in its group_vars and phases 11-12 ran (see the nsp/ns3 phase steps)'; exit 1"
    return 0
  fi
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

# THE SWITCH, AS A DEPLOYMENT TURNS IT ON. The role's README and the example
# group_vars/_examples/peering-host-firewall.example.yml: copy into each peering's edge and
# core group_vars (group_vars/<peering>_edge/host-firewall.yml and
# group_vars/<peering>_core/host-firewall.yml), set goat_host_firewall_baseline: true and
# the management sources. Every peering on one host gets the same file: the edge's rule
# set covers all its peerings, and a value that differed per peering would make each
# peering's run replace the other's. What this sovereign writes, and why each line:
#   management sources   edge: the scanner (the management source this unit scans from),
#                        the core host (a second management address, which the tightening
#                        override removes), and 127.0.0.1: the inventory reaches the edge
#                        over SSH to 127.0.0.1, and the renderer refuses (GB-213) a rule set
#                        that does not admit the managing session by source, the fixed
#                        loopback rule notwithstanding.
#                        core: the scanner and the edge host, which runs the core's plays
#                        over SSH (GB-213 without it).
#   closed listeners     core: udp/7446, the republisher's zenoh scouting socket on
#                        224.0.0.224, closed on purpose (GB-215 otherwise).
#   SPIRE                core: peering_spire_enabled and peering_spire_server_host as the
#                        edge's group_vars set them, so peering-republisher.yml admits the
#                        bundle endpoint (rule spire-bundle).
#   override             core spire-bundle: the rule admits the edge on the mesh interface
#                        only; this lab has no mesh, the edge reaches the core on the
#                        internal network, so the interface is loosened to eth0 with a
#                        reason and the sources narrowed to the edge's address.
#                        other-tcp on both: the drill's own surfaces (core REST 8000, the
#                        probe sessions' REST ports on the edge), from the docker host's
#                        address on that network only, loosened with a reason.
# $1 sovereign, $2 1 to add the tightening override (ssh-management on the edge from the
# scanner and the loopback session only), $3 file to write the edge's values to, $4 the
# core's.
fw_profile_vars() {
  local s="$1" tight="$2" o gw scan core edge probes="" t
  o="$(octet "$s")"; gw="$NB.$o.1"; scan="$NB.$o.30"; core="$(core_ip "$s")"; edge="$(edge_ip "$s")"
  for t in $(partners "$s"); do
    probes="$probes${probes:+, }$(probe_port "$s" "$t" 8101), $(probe_port "$s" "$t" 8102), $(probe_port "$s" "$t" 8103)"
  done
  {
    echo "# The host firewall baseline on $s's peering edge (roles/goat-host-firewall)."
    echo "goat_host_firewall_baseline: true"
    echo "goat_host_firewall_management_sources:"
    echo "  - \"$scan/32\"      # the management host"
    echo "  - \"$core/32\"      # the core host, a second management address"
    echo "  - \"127.0.0.1/32\"  # the play's own SSH session: the inventory reaches this edge at 127.0.0.1"
    echo "goat_host_firewall_override:"
    echo "  other-tcp:"
    echo "    ports: [$probes]"
    echo "    sources: [\"$gw/32\"]"
    echo "    reason: \"drill only: the federation test reads the probe sessions' REST ports from the docker host\""
    if [ "$tight" = 1 ]; then
      echo "  # Tightening: SSH from the management host only (and the play's loopback session)."
      echo "  ssh-management:"
      echo "    sources: [\"$scan/32\", \"127.0.0.1/32\"]"
    fi
  } > "$3"
  {
    echo "# The host firewall baseline on $s's core router host (roles/goat-host-firewall)."
    echo "goat_host_firewall_baseline: true"
    echo "goat_host_firewall_management_sources:"
    echo "  - \"$scan/32\"      # the management host"
    echo "  - \"$edge/32\"      # the edge host, which runs this host's plays over SSH"
    echo "goat_host_firewall_closed_listeners:"
    echo "  - \"udp/7446\"      # the republisher's zenoh scouting socket (224.0.0.224), closed on purpose"
    echo "goat_host_firewall_override:"
    echo "  spire-bundle:"
    echo "    interface: eth0"
    echo "    sources: [\"$edge/32\"]"
    echo "    reason: \"no mesh on this network: the edge's inbound SPIRE forward reaches the core on the internal network\""
    echo "  other-tcp:"
    echo "    ports: [8000]"
    echo "    sources: [\"$gw/32\"]"
    echo "    reason: \"drill only: the federation test reads the core router's REST port from the docker host\""
  } > "$4"
}

# Write the values into each peering's groups of the sovereign's clone, commit, push to its
# origin (its trunk: ansible-preflight.sh refuses to deploy commits the trunk lacks), and
# fast-forward the edge's checkout when it has one. $1 sovereign, $2 tight (0|1), $3 message.
fw_profile_declare() {
  local s="$1" c="$WORK/$1/clone" t
  fw_profile_vars "$s" "$2" "$WORK/$s/host-firewall-edge.yml" "$WORK/$s/host-firewall-core.yml"
  for t in $(partners "$s"); do
    mkdir -p "$c/ops/ansible/group_vars/${t}_edge" "$c/ops/ansible/group_vars/${t}_core"
    cp "$WORK/$s/host-firewall-edge.yml" "$c/ops/ansible/group_vars/${t}_edge/host-firewall.yml"
    { cat "$WORK/$s/host-firewall-core.yml"
      echo "# The SPIRE server runs on this core host (the edge's group_vars say the same)."
      echo "peering_spire_enabled: true"
      echo "peering_spire_server_host: \"$s-core-$t\""
    } > "$c/ops/ansible/group_vars/${t}_core/host-firewall.yml"
  done
  cp "$WORK/$s/host-firewall-edge.yml" "$RECORD/host-firewall-$s-edge$([ "$2" = 1 ] && echo -override).yml"
  cp "$WORK/$s/host-firewall-core.yml" "$RECORD/host-firewall-$s-core$([ "$2" = 1 ] && echo -override).yml"
  git -C "$c" add -A ops/ansible/group_vars
  git -C "$c" diff --cached --quiet || git -C "$c" commit -q -m "$3"
  git -C "$c" push -q origin HEAD:main
  if docker exec "$HP-$s-edge" test -d /opt/goat/.git 2>/dev/null; then
    host_exec "$HP-$s-edge" git -C /opt/goat pull -q --ff-only origin main
  fi
}

# The engine calls attach_boundary for each party after its clone and hosts exist and
# before any phase runs (setup_hosts). With NS_FW_ON=1 the party's group_vars carry the
# baseline from the first phase on: the three-party scenario runs with it on throughout.
attach_boundary() {
  docker network connect --ip "$(boundary_ip "$1")" "$NP-boundary" "$HP-$1-edge"
  if [ "${NS_FW_ON:-0}" = 1 ]; then
    fw_profile_declare "$1" 0 "$1: switch on the host firewall baseline on the peering edge and the core router"
  fi
}

# fw_profile_on: the switch on for both parties of the pair, then each side reruns phases
# 11 and 12, the phases whose plays include the role (peering-wg.yml, the edge;
# peering-republisher.yml, the core). Recorded as nsp-<us>-<them>-phases-11-to-12.
fw_profile_on() {
  local s t
  for s in "$A" "$B"; do
    fw_profile_declare "$s" 0 "$s: switch on the host firewall baseline on the peering edge and the core router"
  done
  for s in "$A" "$B"; do
    t="$(other "$s")"
    rec "nsp-$s-$t-phases-11-to-12" --expect-in "host-firewall | the boot unit is enabled and active" -- bash -c "drive '$s' '$t' '$AVENUE' 11 12"
  done
}

# fw_profile_override: one tightening override, ssh-management on each edge from the
# management host only, declared in group_vars as above; then phase 11 again on each side.
# Returns 1 when a rerun failed, so that the override checks say why.
fw_profile_override() {
  local s t rc=0
  for s in "$A" "$B"; do
    fw_profile_declare "$s" 1 "$s: tighten SSH on the peering edge to the management host"
  done
  for s in "$A" "$B"; do
    t="$(other "$s")"
    rec "nso-$s-$t-phase-11" --expect-in "tightened ssh-management" -- bash -c "drive '$s' '$t' '$AVENUE' 11 11; docker exec $HP-$s-edge cat /etc/goat-host-firewall/ruleset.nft"
    docker exec "$HP-$s-edge" grep -q "tightened ssh-management" /etc/goat-host-firewall/ruleset.nft 2>/dev/null || rc=1
  done
  return "$rc"
}

unit_main() {
  set_partners "$A" "$B"; set_partners "$B" "$A"; set_partners "$C" ""; export NODIR=""
  export_helpers
  NS_FOUND=0; NS_FW_ON=0; NS_PROFILE_REQUIRED=0
  fw_profile_find > "$RECORD/nodesec-profile.txt" 2>&1 && NS_FOUND=1
  setup_hosts 1
  peer_up 1 "$A" "$B" "$AVENUE"
  ns_tunnel_up "$A" "$B"

  say "node security: every port each edge and core answers on, from each side"
  ns_scan_party ns "$A"
  ns_scan_party ns "$B"

  if [ "$NS_FOUND" = 0 ]; then
    if [ -s "$RECORD/nodesec-profile.txt" ]; then
      ns_profile_blocked "the ref has no goat-host-firewall role ($FW_ROLE); it has $(head -1 "$RECORD/nodesec-profile.txt" | cut -d: -f1), which this unit does not drive"
    else
      ns_profile_blocked "the ref declares no host firewall profile: no $FW_ROLE (searched: fw_profile_find in units/nodesec.sh)"
    fi
    return 0
  fi
  if ! grep -q "^ops/ansible/peering-wg.yml: includes" "$RECORD/nodesec-profile.txt" \
     || ! grep -q "^ops/ansible/peering-republisher.yml: includes" "$RECORD/nodesec-profile.txt"; then
    ns_profile_blocked "the ref has $FW_ROLE, but phase 11's peering-wg.yml or phase 12's peering-republisher.yml does not include it, so the driver never applies it ($(tr '\n' ';' < "$RECORD/nodesec-profile.txt"))"
    return 0
  fi

  # The switch on in each sovereign's group_vars, applied by the driver's own phases 11-12.
  say "node security: the baseline switched on in group_vars, phases 11-12 again"
  fw_profile_on
  NS_PROFILE_REQUIRED=1
  ns_scan_party nsp "$A"
  ns_scan_party nsp "$B"
  say "node security: one tightening override"
  if fw_profile_override; then
    local s; for s in "$A" "$B"; do
      ns_scan "nso-$s-edge-from-$s-core" "$HP-$s-core" "$(core_ip "$s")" "$HP-$s-edge" "$(edge_ip "$s")" edge core "$s" "$(other "$s")"
      ns_scan "nso-$s-edge-from-$s-scanner" "scanner:$s" "$NB.$(octet "$s").30" "$HP-$s-edge" "$(edge_ip "$s")" edge mgmt "$s" "$(other "$s")"
    done
  else
    # A valid tightening the driver's phase 11 did not load is a failure of the ref, not a
    # check that could not run: the scans would only compare against the old rule set.
    local s w; for s in "$A" "$B"; do
      for w in core scanner; do
        rec "nso-$s-edge-from-$s-$w" -- bash -c "echo 'the tightening override (ssh-management on the edge from the management host) is not in the rule set on $HP-$s-edge after phase 11 ran with it in group_vars: see step nso-$s-$(other "$s")-phase-11'; exit 1"
      done
    done
  fi

  say "node security: three parties on one edge and one core, baseline on from the first phase"
  NS_FW_ON=1
  if [ "$AVENUE" = goatbuilder ]; then scenario3 goatbuilder script; else scenario3 script script; fi
  ns_scan_party ns3 "$A"
}
