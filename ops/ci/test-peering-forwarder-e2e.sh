#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# test-peering-forwarder-e2e.sh — a peering through a forwarder, with our edge
# behind NAT, in four privileged containers (docs/design/peering-forwarder.md).
#
#   partner ──wg-fed0──▶ forwarder:51820 ──DNAT/SNAT──▶ wg-carry0 ◀── edge (behind nat)
#
# The files every host runs are the roles' own templates, rendered by ansible from
# roles/goat-peering-forwarder and roles/goat-peering-wg with
# peering_wg_via_forwarder: true, and the roles' own link scripts. What is not
# exercised here is the roles' task flow (systemd, apt, the refusals); that is the
# syntax check's and the live run's.
#
# Proves, from the hosts themselves:
#   1. the carrier comes up, dialled out from behind NAT
#   2. the partner's peering handshake completes with our edge through the forwarder
#   3. traffic crosses the peering both ways (ICMP across wg-fed0)
#   4. the forwarder sees the peering only as ciphertext on the carrier: its
#      wg-carry0 carries UDP to the edge's listen port and nothing it can open
#   5. the forwarder drops a UDP port it does not forward
#   6. our edge drops the peering port arriving anywhere but on the carrier
#
# Needs docker with --privileged and a kernel with the wireguard module, and
# ansible-playbook on the runner. Addresses are RFC 5737 and RFC 3927 ranges.
#
# usage: ops/ci/test-peering-forwarder-e2e.sh [--keep]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
KEEP=0
[ "${1:-}" = "--keep" ] && KEEP=1
IMAGE="${PF_E2E_IMAGE:-ubuntu:24.04}"
P="pf-e2e-$$"
WORK="$(mktemp -d)"
WAN=198.51.100
LAN=203.0.113

log() { printf '[peering-forwarder-e2e] %s\n' "$*"; }
fail() { printf '[peering-forwarder-e2e] FAIL: %s\n' "$*" >&2; exit 1; }

command -v docker >/dev/null || { log "SKIP (docker not on PATH)"; exit 0; }
command -v ansible-playbook >/dev/null || fail "ansible-playbook not on PATH"

cleanup() {
  if [ "$KEEP" = 0 ]; then
    docker rm -f "$P-forwarder" "$P-partner" "$P-nat" "$P-edge" >/dev/null 2>&1 || true
    docker network rm "$P-wan" "$P-lan" >/dev/null 2>&1 || true
    docker rmi "pf-e2e:$P" >/dev/null 2>&1 || true
    rm -rf "$WORK"
  else
    log "kept: containers $P-*, work $WORK"
  fi
}
trap cleanup EXIT

x() { local c="$1"; shift; docker exec "$P-$c" sh -c "$*"; }

log "image"
IMG="pf-e2e:$P"
docker build --progress=plain -t "$IMG" - <<EOF || fail "building the test image"
FROM $IMAGE
# Retry transient mirror failures, but never build from a partial index refresh.
RUN printf '%s\n' 'Acquire::Retries "3";' 'Acquire::http::Timeout "20";' 'Acquire::https::Timeout "20";' > /etc/apt/apt.conf.d/80-ci-network
RUN apt-get update -o APT::Update::Error-Mode=any && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      wireguard-tools nftables iproute2 iputils-ping && rm -rf /var/lib/apt/lists/*
EOF

log "networks and containers"
docker network create --subnet "$WAN.0/24" "$P-wan" >/dev/null
docker network create --subnet "$LAN.0/24" "$P-lan" >/dev/null
run() { docker run -d --privileged --name "$P-$1" --hostname "$1" --network "$P-$2" --ip "$3" "$IMG" sleep infinity >/dev/null; }
run forwarder wan "$WAN.10"
run partner wan "$WAN.20"
run nat wan "$WAN.30"
docker network connect --ip "$LAN.2" "$P-lan" "$P-nat"
run edge lan "$LAN.10"

# Our edge has no public address: everything leaves through nat, which masquerades
# and forwards nothing inward.
x nat 'sysctl -qw net.ipv4.ip_forward=1'
docker exec -i "$P-nat" nft -f - <<'EOF'
table ip nat {
  chain post {
    type nat hook postrouting priority srcnat;
    oifname "eth0" masquerade
  }
}
table inet f {
  chain forward {
    type filter hook forward priority filter; policy drop;
    iifname "eth1" oifname "eth0" accept
    iifname "eth0" oifname "eth1" ct state established,related accept
  }
}
EOF
x edge "ip route replace default via $LAN.2"

log "keys, made on each host"
x forwarder 'mkdir -p -m 700 /etc/goat-peering-forwarder; umask 077; wg genkey > /etc/goat-peering-forwarder/wg-carry0.key'
x edge 'mkdir -p -m 700 /etc/goat-peering-wg; umask 077; wg genkey > /etc/goat-peering-wg/wg-fed0.key; wg genkey > /etc/goat-peering-wg/wg-carry0.key'
x partner 'mkdir -p -m 700 /etc/goat-peering-wg; umask 077; wg genkey > /etc/goat-peering-wg/wg-fed0.key'
FWD_CARRIER=$(x forwarder 'wg pubkey < /etc/goat-peering-forwarder/wg-carry0.key')
EDGE_CARRIER=$(x edge 'wg pubkey < /etc/goat-peering-wg/wg-carry0.key')
EDGE_FED=$(x edge 'wg pubkey < /etc/goat-peering-wg/wg-fed0.key')
PARTNER_FED=$(x partner 'wg pubkey < /etc/goat-peering-wg/wg-fed0.key')

log "render the roles' templates"
mkdir -p "$WORK/forwarder" "$WORK/edge" "$WORK/partner"
FR="$ROOT/ops/ansible/roles/goat-peering-forwarder"
WR="$ROOT/ops/ansible/roles/goat-peering-wg"
cat > "$WORK/forwarder.json" <<EOF
{"role_dir": "$FR", "out": "$WORK/forwarder",
 "files": [["forwarder.nft.j2", "forwarder.nft"], ["carrier.env.j2", "wg-carry0.env"], ["goat-peering-forwarder-link.sh.j2", "link"]],
 "peering_forwarder_enabled": true, "peering_forwarder_public_interface": "eth0",
 "_pf_peerings": [{"name": "xaa", "interface": "wg-carry0", "peering_port": 51820, "edge_peering_port": 51820,
   "carrier_port": 51821, "carrier_local_ip": "169.254.30.0", "carrier_remote_ip": "169.254.30.1",
   "carrier_prefix_len": 31, "carrier_mtu": 1420, "partner_sources": ["$WAN.20"],
   "edge_carrier_public_key": "$EDGE_CARRIER"}],
 "item": "{{ _pf_peerings[0] }}"}
EOF
cat > "$WORK/edge.json" <<EOF
{"role_dir": "$WR", "out": "$WORK/edge",
 "files": [["link.env.j2", "wg-fed0.env"], ["peering-wg.nft.j2", "wg-fed0.nft"], ["carrier.env.j2", "wg-carry0.env"], ["goat-peering-wg-link.sh.j2", "link"]],
 "peering_wg_via_forwarder": true, "peering_wg_forwarder_endpoint": "$WAN.10:51821",
 "peering_wg_forwarder_carrier_public_key": "$FWD_CARRIER",
 "peering_wg_local_link_ip": "169.254.10.1", "peering_wg_remote_link_ip": "169.254.10.0",
 "peering_wg_mtu": 1300, "peering_wg_we_listen": true,
 "_pw_peer_key": "$PARTNER_FED", "_pw_peer_endpoint": "", "_pw_peer_addr": ""}
EOF
cat > "$WORK/partner.json" <<EOF
{"role_dir": "$WR", "out": "$WORK/partner",
 "files": [["link.env.j2", "wg-fed0.env"], ["peering-wg.nft.j2", "wg-fed0.nft"], ["goat-peering-wg-link.sh.j2", "link"]],
 "peering_wg_local_link_ip": "169.254.10.0", "peering_wg_remote_link_ip": "169.254.10.1",
 "peering_wg_mtu": 1300, "peering_wg_we_listen": false,
 "_pw_peer_key": "$EDGE_FED", "_pw_peer_endpoint": "$WAN.10:51820", "_pw_peer_addr": ""}
EOF
for h in forwarder edge partner; do
  ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, -c local "$ROOT/ops/ci/fixtures/peering-forwarder-e2e/render.yml" \
    -e "@$WORK/$h.json" >"$WORK/render-$h.log" 2>&1 \
    || { cat "$WORK/render-$h.log"; fail "rendering the templates for $h"; }
done

grep -q 'iifname "wg-carry0" ip saddr 169.254.30.0 ip daddr 169.254.30.1 udp dport 51820 accept' "$WORK/edge/wg-fed0.nft" \
  || fail "the edge firewall does not bind the peering port to the carrier"

push() { # host dir dest
  for f in "$WORK/$2"/*; do docker cp "$f" "$P-$1:$3/$(basename "$f")" >/dev/null; done
}
push forwarder forwarder /etc/goat-peering-forwarder
x forwarder 'mv /etc/goat-peering-forwarder/link /usr/local/sbin/goat-peering-forwarder-link'
push edge edge /etc/goat-peering-wg
x edge 'mv /etc/goat-peering-wg/link /usr/local/sbin/goat-peering-wg-link'
push partner partner /etc/goat-peering-wg
x partner 'mv /etc/goat-peering-wg/link /usr/local/sbin/goat-peering-wg-link'

up() { # host env-file script
  x "$1" "set -a; . $2; set +a; $3 up"
}

log "forwarder up"
x forwarder 'sysctl -qw net.ipv4.ip_forward=1'
up forwarder /etc/goat-peering-forwarder/wg-carry0.env /usr/local/sbin/goat-peering-forwarder-link
log "edge: carrier, then the peering listener"
up edge /etc/goat-peering-wg/wg-carry0.env /usr/local/sbin/goat-peering-wg-link
up edge /etc/goat-peering-wg/wg-fed0.env /usr/local/sbin/goat-peering-wg-link

hs() { x "$1" "wg show $2 latest-handshakes" | awk '{print $2}'; }
wait_hs() {
  for _ in $(seq 1 30); do
    [ "$(hs "$1" "$2")" -gt 0 ] 2>/dev/null && return 0
    sleep 1
  done
  return 1
}
# The edge's first keepalive brings the carrier up from behind NAT.
x edge 'ping -c1 -W2 169.254.30.0 >/dev/null 2>&1 || true'
wait_hs edge wg-carry0 || fail "1: no carrier handshake from behind NAT"
log "1 ok: carrier up, dialled out from behind NAT"

log "partner dials the forwarder"
up partner /etc/goat-peering-wg/wg-fed0.env /usr/local/sbin/goat-peering-wg-link
x partner 'ping -c1 -W2 169.254.10.1 >/dev/null 2>&1 || true'
wait_hs partner wg-fed0 || fail "2: the partner's peering handshake did not complete through the forwarder"
wait_hs edge wg-fed0 || fail "2: our edge saw no peering handshake"
EP=$(x edge 'wg show wg-fed0 endpoints' | awk '{print $2}')
case "$EP" in 169.254.30.0:*) ;; *) fail "2: our edge sees the partner at $EP, not at the forwarder's carrier address" ;; esac
log "2 ok: peering handshake through the forwarder; our edge sees the partner at $EP"

x partner 'ping -c3 -W2 169.254.10.1' >/dev/null || fail "3: partner -> edge across the peering"
x edge 'ping -c3 -W2 169.254.10.0' >/dev/null || fail "3: edge -> partner across the peering"
log "3 ok: traffic crosses the peering both ways"

FWD_IFS=$(x forwarder 'ls /sys/class/net' | tr '\n' ' ')
case " $FWD_IFS " in *" wg-fed0 "*) fail "4: the forwarder has a peering interface" ;; esac
x forwarder 'ls /etc/goat-peering-forwarder' | sort | tr '\n' ' ' | grep -qx 'forwarder.nft wg-carry0.env wg-carry0.key ' \
  || fail "4: the forwarder holds files beyond its carrier key, env and table"
log "4 ok: the forwarder has no peering interface and holds only its carrier key, env and table"

before=$(x forwarder 'nft list chain inet goat_peering_forwarder input' | awk '/iifname "eth0" counter packets/ {print $5}')
x partner "bash -c 'printf x > /dev/udp/$WAN.10/51999'"
sleep 1
after=$(x forwarder 'nft list chain inet goat_peering_forwarder input' | awk '/iifname "eth0" counter packets/ {print $5}')
[ "${after:-0}" -gt "${before:-0}" ] || fail "5: the forwarder did not count-and-drop an unforwarded port ($before -> $after)"
log "5 ok: the forwarder drops a port it does not forward"

# A peering-port packet that arrives on the edge's LAN interface, not the carrier.
x nat "bash -c 'printf x > /dev/udp/$LAN.10/51820'"
sleep 1
x edge 'nft list chain inet goat_peering_wg_fed0 from_tunnel' | grep -E 'udp dport 51820 counter packets [1-9]' >/dev/null \
  || fail "6: our edge did not drop the peering port arriving off the carrier"
log "6 ok: our edge drops the peering port arriving anywhere but the carrier"

log "PASS"
