#!/usr/bin/env bash
# units/forwarder.sh — network profile "edge behind NAT that cannot accept inbound": {A}'s
# edge sits behind a NAT router that masquerades outbound and forwards nothing inbound,
# and {A} listens through a peering forwarder (ADR 1218, docs/design/peering-forwarder.md)
# that {A} operates. {B} dials the forwarder's public address. Sourced by lib/unit.sh.
#
#   {B} edge ── boundary ──▶ {A} forwarder <pub>:51820 ── DNAT ──▶ wg-carry0 ◀── {A} edge
#                            {A} NAT <boundary_ip {A}>   ◀── masquerade ─── {A} edge (natlan)
#
# The forwarder is brought up the way the runbook says (peering-forwarder.md §7), with
# the tree's own plays run from {A}'s edge against {A}'s own overlay, committed to {A}'s
# repository: peering-prepare.yml (the edge's carrier key), peering-forwarder.yml (the
# forwarder's carrier key), peering-prepare.yml again (the carrier up). Then all fourteen
# phases through the avenue, {A} first and listening. Neither driver knows the forwarder;
# the harness plays the operator for those three runs only.
#
# Steps are named s<n>-* with n=fwd. Scenario forwarder-path reads them.

N=fwd
NATED="$A"; PARTNER="$B"
FWD_PUB="$NB.3.31"; FWD_MGMT="$NB.7.10"; NAT_LAN="$NB.6.2"; EDGE_LAN="$NB.6.20"
FWD_HOST="$HP-$A-fwd"; NAT_HOST="$HP-$A-nat"
HOSTS+=("$NAT_HOST" "$FWD_HOST"); NETS+=("$NP-natlan" "$NP-fwdmgmt")
export NATED PARTNER FWD_PUB EDGE_LAN NAT_LAN FWD_HOST NAT_HOST

# --- the network profile, through the engine's seams ---------------------------------
offer_endpoint() { # the NATed side offers the forwarder's public peering port
  if [ "$1" = "$NATED" ]; then echo "$FWD_PUB:51820"; else echo "$(boundary_ip "$1"):$(wgport "$1" "$2")"; fi
}
offer_mtu() { # one more WireGuard header inside the carrier: the design's 1300
  if [ "$1" = "$NATED" ]; then echo 1300; else echo 1380; fi
}
attach_boundary() {
  if [ "$1" != "$NATED" ]; then
    docker network connect --ip "$(boundary_ip "$1")" "$NP-boundary" "$HP-$1-edge"; return
  fi
  nat_up
}

ifname_of() { # $1 container, $2 address -> the interface holding it
  docker exec "$1" ip -o -4 addr show | awk -v a="$2" '{split($4, p, "/"); if (p[1] == a) print $2}'
}

# The NAT router holds {A}'s public boundary address. Its forward chain admits what the
# edge starts and the replies to it, and nothing initiated from the boundary.
nat_rules() { # $1 "closed" | "open" (open: also forwards new inbound, the probe's control)
  local pub lan
  pub="$(ifname_of "$NAT_HOST" "$(boundary_ip "$NATED")")"; lan="$(ifname_of "$NAT_HOST" "$NAT_LAN")"
  printf 'table ip fe2e_nat\ndelete table ip fe2e_nat\ntable ip fe2e_nat {\n'
  printf '  chain masq { type nat hook postrouting priority srcnat; policy accept;\n    ip saddr %s.6.0/24 oifname "%s" masquerade\n  }\n' "$NB" "$pub"
  printf '  chain relay { type filter hook forward priority filter; policy drop;\n'
  printf '    iifname "%s" oifname "%s" accept\n' "$lan" "$pub"
  printf '    iifname "%s" oifname "%s" ct state established,related accept\n' "$pub" "$lan"
  [ "$1" = open ] && printf '    iifname "%s" oifname "%s" accept\n' "$pub" "$lan"
  printf '    counter drop\n  }\n}\n'
}
nat_load() { nat_rules "$1" > "$WORK/nat-$1.nft"; docker exec -i "$NAT_HOST" nft -f - < "$WORK/nat-$1.nft"; }

nat_up() {
  docker network create "${LABEL[@]}" --internal --subnet "$NB.6.0/24" "$NP-natlan" >/dev/null
  # A router only: no init, no container engine whose own rules would touch forwarding.
  docker run -d --name "$NAT_HOST" --hostname "$NAT_HOST" "${LABEL[@]}" --privileged \
    --network "$NP-boundary" --ip "$(boundary_ip "$NATED")" --entrypoint sleep "$HOST_IMAGE" infinity >/dev/null
  docker network connect --ip "$NAT_LAN" "$NP-natlan" "$NAT_HOST"
  docker exec "$NAT_HOST" sysctl -qw net.ipv4.ip_forward=1
  nat_load closed
  docker network connect --ip "$EDGE_LAN" "$NP-natlan" "$HP-$NATED-edge"
  docker exec "$HP-$NATED-edge" ip route replace "$NB.3.0/24" via "$NAT_LAN"
}

# The forwarder: the engine's host image on the boundary, with an egress-only management
# network for its packages. No key folder is mounted and no container engine runs: what
# it holds afterwards is what the role put there. {A}'s operator reaches it over SSH from
# {A}'s edge, through {A}'s NAT, as peering_forwarder_ssh_sources admits.
fwd_up() {
  local d="$WORK/fwd-mask"
  docker network create "${LABEL[@]}" --subnet "$NB.7.0/24" "$NP-fwdmgmt" >/dev/null
  docker create --name "$FWD_HOST" --hostname "$FWD_HOST" "${LABEL[@]}" --privileged --cgroupns=private \
    --network "$NP-fwdmgmt" --ip "$FWD_MGMT" --tmpfs /run --tmpfs /run/lock -v /lib/modules:/lib/modules:ro \
    -e container=docker "$HOST_IMAGE" >/dev/null
  rm -rf "$d"; mkdir -p "$d"
  ln -s /dev/null "$d/docker.service"; ln -s /dev/null "$d/docker.socket"
  tar -C "$d" -cf - docker.service docker.socket | docker cp - "$FWD_HOST:/etc/systemd/system/"
  docker start "$FWD_HOST" >/dev/null
  docker network connect --ip "$FWD_PUB" "$NP-boundary" "$FWD_HOST"
  for _ in $(seq 1 60); do docker exec "$FWD_HOST" systemctl is-active --quiet ssh 2>/dev/null && break; sleep 2; done
  docker exec "$FWD_HOST" systemctl is-active --quiet ssh || die "the forwarder host's sshd did not start"
  # The same as wait_ready on every other host: the machine's AppArmor profile for wg
  # lets /usr/bin/wg read keys only under /etc/wireguard; a copy on PATH runs unconfined.
  [ "${FE2E_WG_APPARMOR:-bypass}" = host ] || docker exec "$FWD_HOST" cp /usr/bin/wg /usr/local/sbin/wg
  docker exec -i "$FWD_HOST" sh -c 'install -d -m 0700 -o goat -g goat /home/goat/.ssh && cat > /home/goat/.ssh/authorized_keys && chown goat:goat /home/goat/.ssh/authorized_keys && chmod 0600 /home/goat/.ssh/authorized_keys' \
    < "$WORK/$NATED/keys/id_ed25519.pub"
}

# A change to {A}'s own overlay: committed, pushed to its origin, pulled on its edge, so
# ansible-preflight.sh finds the edge's checkout at its trunk.
overlay_commit() { # $1 message
  local c="$WORK/$NATED/clone"
  git -C "$c" add -A
  git -C "$c" commit -q -m "$1"
  git -C "$c" push -q origin main
  docker exec "$HP-$NATED-edge" git -C /opt/goat pull -q --ff-only origin main
}

# One ansible run on {A}'s edge, the drivers' shape (peering-standup.sh play()).
edge_play() { # $1 inventory, $2 playbook, [$3 --limit group]
  local lim="" w="/var/lib/goat-federation/$PARTNER"
  [ -n "${3:-}" ] && lim="--limit $3 -e fed_edge_group=$3 -e fed_work=$w -e fed_counterparty=$w/theirs.canonical.json"
  docker exec -e HOME=/root -e GOAT_AWS_MODE=none -e GOAT_ANSIBLE_TRUNK_REMOTE=origin "$HP-$NATED-edge" bash -c \
    "cd /opt/goat/ops/ansible && ../scripts/with-aws none -- ../scripts/ansible-preflight.sh -i inventory/$1 $lim $2"
}

# The forwarder holds no peering key: the exact material of every WireGuard and TLS key
# either party's peering works with is searched for across its whole filesystem; no PEM
# private key sits anywhere outside /usr and the host's own SSH host keys; it has no
# peering interface; and its one private key is its carrier key, which is no edge's key.
fwd_holds_nothing() {
  local s f p="$WORK/fwd-patterns.txt" bad=0 hits pem ifs keys fk
  : > "$p"
  for s in "$NATED" "$PARTNER"; do
    docker exec "$HP-$s-edge" sh -c 'cat /etc/goat-peering-wg/*.key' >> "$p"
    docker exec "$HP-$s-edge" sh -c 'for f in /var/lib/goat-federation/*/pki/*key*.pem; do sed -n 2p "$f"; done' >> "$p"
    for f in ca-key.pem signing.pem core-key.pem; do sed -n 2p "$WORK/$s/keys/$f" >> "$p"; done
    cat "$WORK/$s/keys/signing.key" >> "$p"; echo >> "$p"
  done
  sed -i '/^[[:space:]]*$/d' "$p"
  echo "searching for $(wc -l < "$p" | tr -d ' ') key fragments of both parties"
  hits="$(docker exec -i "$FWD_HOST" sh -c 'cat > /dev/shm/fe2e-p; grep -rlF -f /dev/shm/fe2e-p / --exclude-dir=proc --exclude-dir=sys --exclude-dir=dev 2>/dev/null; rm -f /dev/shm/fe2e-p' < "$p")"
  [ -z "$hits" ] || { echo "peering key material found on the forwarder: $hits"; bad=1; }
  pem="$(docker exec "$FWD_HOST" sh -c "grep -rlE -- '-----BEGIN ([A-Z]+ )?PRIVATE KEY-----' /etc /root /home /var /tmp /opt /srv /run 2>/dev/null | grep -v '^/etc/ssh/ssh_host_'" || true)"
  [ -z "$pem" ] || { echo "PEM private keys on the forwarder: $pem"; bad=1; }
  ifs="$(docker exec "$FWD_HOST" ls /sys/class/net | tr '\n' ' ')"; echo "interfaces: $ifs"
  case " $ifs " in *" wg-fed"*) echo "the forwarder has a peering interface"; bad=1 ;; esac
  keys="$(docker exec "$FWD_HOST" sh -c 'ls /etc/goat-peering-forwarder' | tr '\n' ' ')"; echo "config: $keys"
  [ "$keys" = "forwarder.nft wg-carry0.env wg-carry0.key " ] || { echo "the forwarder's config holds more than its carrier key, env and table"; bad=1; }
  fk="$(docker exec "$FWD_HOST" sh -c 'wg pubkey < /etc/goat-peering-forwarder/wg-carry0.key')"
  for s in "$NATED" "$PARTNER"; do
    if [ "$fk" = "$(docker exec "$HP-$s-edge" wg show "$(iface "$s" "$(other "$s")")" public-key)" ]; then echo "the forwarder's key is $s's peering key"; bad=1; fi
  done
  [ "$bad" = 0 ] && echo "the forwarder holds no peering key and no TLS key"
  return "$bad"
}

# The probe that would reach the edge without the NAT: {B}'s edge routes {A}'s LAN via
# {A}'s public address and tries ICMP and TCP 22 (the edge's sshd). Exit 0 when either
# reaches the edge.
probe_nated_edge() {
  local e="$HP-$PARTNER-edge" rc=1
  docker exec "$e" ip route replace "$NB.6.0/24" via "$(boundary_ip "$NATED")"
  if docker exec "$e" ping -c1 -W2 "$EDGE_LAN" >/dev/null 2>&1; then echo "ICMP reached $EDGE_LAN"; rc=0; else echo "ICMP did not reach $EDGE_LAN"; fi
  if docker exec "$e" timeout 4 bash -c "exec 3<>/dev/tcp/$EDGE_LAN/22" 2>/dev/null; then echo "TCP 22 reached $EDGE_LAN"; rc=0; else echo "TCP 22 did not reach $EDGE_LAN"; fi
  docker exec "$e" ip route del "$NB.6.0/24" >/dev/null 2>&1 || true
  return "$rc"
}
export -f edge_play fwd_holds_nothing probe_nated_edge nat_load nat_rules ifname_of iface other pidx partners
export NB HOST_IMAGE

unit_main() {
  local i s w key fkey
  set_partners "$A" "$B"; set_partners "$B" "$A"; set_partners "$C" ""; NODIR=""
  export_helpers
  say "forwarder: $NATED behind NAT listens through its forwarder; $PARTNER dials; avenue $AVENUE"
  setup_hosts "$N"
  fwd_up
  prepare_edge "$NATED" "$PARTNER" true
  w="$WORK/fwd-runs"; mkdir -p "$w"

  say "forwarder: bring-up in three runs (peering-forwarder.md §7)"
  mkdir -p "$WORK/$NATED/clone/ops/ansible/group_vars/${PARTNER}_edge" "$WORK/$NATED/clone/ops/ansible/group_vars/peering_forwarders"
  printf '# %s listens through its peering forwarder (ADR 1218).\npeering_wg_via_forwarder: true\n' "$NATED" \
    > "$WORK/$NATED/clone/ops/ansible/group_vars/${PARTNER}_edge/forwarder.yml"
  printf '# The peering forwarder %s operates: in no goatnet group.\n[peering_forwarders]\n%s-forwarder ansible_host=%s ansible_user=goat\n' \
    "$NATED" "$NATED" "$FWD_PUB" > "$WORK/$NATED/clone/ops/ansible/inventory/peering-forwarders.ini"
  overlay_commit "$NATED: the edge listens through a peering forwarder"
  rec "s$N-$NATED-prepare-makes-the-carrier-key" --expect-in "carrier wg-carry0 public key:" -- bash -c \
    "set -o pipefail; edge_play peering-edges.ini peering-prepare.yml ${PARTNER}_edge 2>&1 | tee '$w/prepare-1.out'"
  key="$(grep -o 'carrier wg-carry0 public key: [A-Za-z0-9+/]\{43\}=' "$w/prepare-1.out" | head -1 | awk '{print $NF}')"
  [ -n "$key" ] || die "peering-prepare.yml with peering_wg_via_forwarder printed no carrier key for the forwarder"

  local pub; pub="$(ifname_of "$FWD_HOST" "$FWD_PUB")"
  printf 'peering_forwarder_enabled: true\npeering_forwarder_public_interface: %s\npeering_forwarder_ssh_sources: ["%s"]\npeering_forwarder_peerings:\n  - name: %s\n    edge_carrier_public_key: "%s"\n    partner_sources: ["%s"]\n' \
    "$pub" "$(boundary_ip "$NATED")" "$PARTNER" "$key" "$(boundary_ip "$PARTNER")" \
    > "$WORK/$NATED/clone/ops/ansible/group_vars/peering_forwarders/forwarder.yml"
  overlay_commit "$NATED: the forwarder carries the peering with $PARTNER"
  rec "s$N-forwarder-role-applied" --expect-in "peering_wg_forwarder_carrier_public_key:" --expect-in "failed=0" -- bash -c \
    "set -o pipefail; edge_play peering-forwarders.ini peering-forwarder.yml 2>&1 | tee '$w/forwarder.out'"
  fkey="$(grep -o 'peering_wg_forwarder_carrier_public_key: [A-Za-z0-9+/]\{43\}=' "$w/forwarder.out" | head -1 | awk '{print $NF}')"
  [ -n "$fkey" ] || die "peering-forwarder.yml printed no forwarder carrier key for the edge"
  rec "s$N-forwarder-carrier-and-table-in-place" --expect-in '"holds_peering_material": false' --expect-in "udp dport 51820 dnat ip to 169.254.30.1:51820" -- \
    docker exec "$FWD_HOST" sh -c 'systemctl is-active goat-peering-forwarder@wg-carry0 && nft list table inet goat_peering_forwarder && cat /var/log/goat-peering-forwarder-state.json'

  printf '# %s listens through its peering forwarder (ADR 1218).\npeering_wg_via_forwarder: true\npeering_wg_forwarder_endpoint: "%s:51821"\npeering_wg_forwarder_carrier_public_key: "%s"\n' \
    "$NATED" "$FWD_PUB" "$fkey" > "$WORK/$NATED/clone/ops/ansible/group_vars/${PARTNER}_edge/forwarder.yml"
  overlay_commit "$NATED: the edge's carrier to its forwarder"
  rec "s$N-$NATED-prepare-brings-the-carrier-up" --expect-in "forwarder $FWD_PUB:51821 holds $fkey" -- bash -c \
    "set -o pipefail; edge_play peering-edges.ini peering-prepare.yml ${PARTNER}_edge 2>&1 | tee '$w/prepare-2.out'"
  rec "s$N-$NATED-carrier-handshake" -- bash -c \
    "for _ in \$(seq 1 30); do t=\$(docker exec $HP-$NATED-edge wg show wg-carry0 latest-handshakes 2>/dev/null | cut -f2); [ \"\${t:-0}\" != 0 ] && break; sleep 2; done; docker exec $HP-$NATED-edge wg show wg-carry0; [ \"\${t:-0}\" != 0 ]"

  say "forwarder: all fourteen phases, $NATED first and listening through the forwarder"
  peer_up "$N" "$NATED" "$PARTNER" "$AVENUE"

  say "forwarder: what the path is, and what the forwarder holds"
  i="$(iface "$PARTNER" "$NATED")"
  rec "s$N-$PARTNER-endpoint-is-the-forwarder" --expect-in "$FWD_PUB:51820" --expect-not-in "$(boundary_ip "$NATED"):" -- \
    docker exec "$HP-$PARTNER-edge" wg show "$i" endpoints
  rec "s$N-$NATED-sees-$PARTNER-at-the-carrier" --expect-in "169.254.30.0:" -- \
    docker exec "$HP-$NATED-edge" wg show "$(iface "$NATED" "$PARTNER")" endpoints
  for s in "$NATED" "$PARTNER"; do
    rec "s$N-$s-tunnel-carried-it" -- bash -c \
      "docker exec $HP-$s-edge wg show $(iface "$s" "$(other "$s")") transfer | awk '{print; if (\$2 < 1 || \$3 < 1) exit 1}'"
  done
  rec "s$N-$PARTNER-cannot-reach-$NATED-edge-through-its-nat" --expect-exit 1 -- bash -c probe_nated_edge
  nat_load open
  rec "s$N-the-same-probe-reaches-$NATED-edge-with-the-nat-open" -- bash -c probe_nated_edge
  nat_load closed
  rec "s$N-round-trip-$NATED-to-$PARTNER" --expect-in "PASS" -- bash -c "round_trip_eventually '$NATED' '$PARTNER'"
  rec "s$N-round-trip-$PARTNER-to-$NATED" --expect-in "PASS" -- bash -c "round_trip_eventually '$PARTNER' '$NATED'"
  rec "s$N-forwarder-holds-no-peering-key" -- bash -c fwd_holds_nothing
  docker exec "$FWD_HOST" sh -c 'wg show; nft list ruleset' > "$RECORD/s$N-forwarder-state.txt" 2>&1 || true
  docker exec "$HP-$NATED-edge" sh -c 'wg show; ip route; nft list ruleset' > "$RECORD/s$N-$NATED-edge-state.txt" 2>&1 || true
  collect "$N"
}
