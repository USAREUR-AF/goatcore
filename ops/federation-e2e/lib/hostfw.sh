#!/usr/bin/env bash
# lib/hostfw.sh — the goat-host-firewall baseline's values for a sovereign's peering hosts
# (sourced by lib/engine.sh).
#
# Every scenario switches the baseline on for each edge from the start (edge_fw_declare,
# called by gen_sovereign), as the edge peering plays require (GB-279). A unit that measures
# the default (the baseline off) writes the edge file again with it off.
# units/nodesec.sh also switches it on for the core router and adds a tightening override.
#
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
  local s="$1" tight="$2" o gw scan core edge probes="" reach="" t
  o="$(octet "$s")"; gw="$NB.$o.1"; scan="$NB.$o.30"; core="$(core_ip "$s")"; edge="$(edge_ip "$s")"
  for t in $(partners "$s"); do
    probes="$probes${probes:+, }$(probe_port "$s" "$t" 8101), $(probe_port "$s" "$t" 8102), $(probe_port "$s" "$t" 8103)"
    reach="$reach $(drill_reach_ip "$s" "$t")"
  done
  {
    echo "# The host firewall baseline on $s's peering edge (roles/goat-host-firewall)."
    echo "goat_host_firewall_baseline: true"
    echo "goat_host_firewall_management_sources:"
    echo "  - \"$scan/32\"      # the management host"
    echo "  - \"$core/32\"      # the core host, a second management address"
    echo "  - \"127.0.0.1/32\"  # the play's own SSH session: the inventory reaches this edge at 127.0.0.1"
    for t in $reach; do
      echo "  - \"$t/32\"  # drill only: a partner's phase 14 starts its probe sessions from this address over SSH (drill_reach_ip)"
    done
    echo "goat_host_firewall_override:"
    echo "  other-tcp:"
    echo "    ports: [$probes]"
    echo "    sources: [\"$gw/32\"$(for t in $reach; do printf ', "%s/32"' "$t"; done)]"
    echo "    reason: \"drill only: the federation test reads the probe sessions' REST ports from the docker host and from a partner's drill reach\""
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

# drill_reach_ip: the address a partner's driver host takes on this sovereign's network when a
# unit gives it the drill's reach for phase 14 (units/onecmd.sh, units/nohand-loop.sh). It is
# fixed, one per partner, so this sovereign's edge firewall admits it by source: the partner's
# probe launcher reaches the edge over SSH, and its acceptance reads the probe sessions' REST
# ports. A drill's own surface; a production operator has no reach into the partner's network.
# $1 this sovereign, $2 the partner.
drill_reach_ip() { echo "$NB.$(octet "$1").$((40 + $(party_pos "$2")))"; }

# The baseline on for every edge, in a sovereign's clone before its first commit, when the tree
# carries the role (an older tree's plays never read these values). One file in the parent
# group peering_edges, whose children are every peering's edge group: the values are the same
# for each of this sovereign's peerings (see above), and a tree with no peering yet (scenario
# nothing-hand-written) gets no peering's group directory. `goatbuilder federate allocate`
# reads a hand-written file in a peering's edge group as that peering already running (GB-230).
# A unit that changes one edge (units/nodesec.sh) writes that peering's own group, which
# overrides this one. $1 sovereign, $2 its clone.
edge_fw_declare() {
  local s="$1" c="$2"
  grep -q '^goat_host_firewall_baseline:' "$c/ops/ansible/roles/goat-host-firewall/defaults/main.yml" 2>/dev/null || return 0
  fw_profile_vars "$s" 0 "$WORK/$s/host-firewall-edge.yml" "$WORK/$s/host-firewall-core.yml"
  mkdir -p "$c/ops/ansible/group_vars/peering_edges"
  cp "$WORK/$s/host-firewall-edge.yml" "$c/ops/ansible/group_vars/peering_edges/host-firewall.yml"
}
