#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# netns-behavior.sh — load each rendered baseline into a throwaway network namespace
# and scan it from every side: the management source, an outsider, the partner's
# endpoint, the tunnel and the mesh. A side sees exactly the ports the profile
# declares for it, the tightening override closes what it names, a loosening admits
# what it names (through a published, DNAT-ed port too), and the managing session
# stays open. Prints each scan as evidence.
#
# Needs root (sudo -n), nft, ip and python3. Never touches the host's own firewall:
# everything happens in namespaces named hfw<pid>*, deleted on exit.
# usage: netns-behavior.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
RENDER="$HERE/../files/goat-host-firewall-render.py"
PROBE="$HERE/probe.py"
FIX="$HERE/fixtures"
H="hfw$$h"; P="hfw$$p"; C="hfw$$c"
WORK="$(mktemp -d)"
SERVER_PID=""
FAILS=0

log() { printf '[host-firewall/netns] %s\n' "$*"; }

SUDO="sudo -n"
[ "$(id -u)" = 0 ] && SUDO=""
if ! $SUDO true 2>/dev/null; then log "SKIP (needs root or passwordless sudo)"; exit 0; fi
for t in nft ip python3; do
  command -v "$t" >/dev/null 2>&1 || { log "SKIP ($t not on PATH)"; exit 0; }
done

cleanup() {
  [ -n "$SERVER_PID" ] && $SUDO kill "$SERVER_PID" 2>/dev/null || true
  for n in "$H" "$P" "$C"; do $SUDO ip netns del "$n" 2>/dev/null || true; done
  rm -rf "$WORK"
}
trap cleanup EXIT

inh() { $SUDO ip netns exec "$H" "$@"; }
inp() { $SUDO ip netns exec "$P" "$@"; }

for n in "$H" "$P" "$C"; do $SUDO ip netns add "$n"; done
inh ip link set lo up; inp ip link set lo up; $SUDO ip netns exec "$C" ip link set lo up

# veth <name in H> <name in P> <H address> <P address>...
veth() {
  local a="$1" b="$2" ha="$3"; shift 3
  $SUDO ip link add "hfwtmp$$" type veth peer name "hfwtmq$$"
  $SUDO ip link set "hfwtmp$$" netns "$H"; $SUDO ip link set "hfwtmq$$" netns "$P"
  inh ip link set "hfwtmp$$" name "$a"; inp ip link set "hfwtmq$$" name "$b"
  for x in $ha; do inh ip addr add "$x" dev "$a"; done
  for x in "$@"; do inp ip addr add "$x" dev "$b"; done
  inh ip link set "$a" up; inp ip link set "$b" up
}
veth eth0 out0 "198.51.100.1/24 192.0.2.1/24 203.0.113.1/24" \
  198.51.100.10/24 192.0.2.10/24 192.0.2.20/24 203.0.113.9/24
veth wg-fed0 tun0 169.254.10.1/31 169.254.10.0/31
veth ipsec-xab tun1 169.254.11.1/31 169.254.11.0/31
veth wt0 mesh0 100.127.0.1/24 100.127.0.9/24

# A container behind a published port: C holds 10.99.0.2:80, H DNATs tcp 9000 to it.
$SUDO ip link add "hfwtmp$$" type veth peer name "hfwtmq$$"
$SUDO ip link set "hfwtmp$$" netns "$H"; $SUDO ip link set "hfwtmq$$" netns "$C"
inh ip link set "hfwtmp$$" name docker0; $SUDO ip netns exec "$C" ip link set "hfwtmq$$" name eth0
inh ip addr add 10.99.0.1/24 dev docker0; inh ip link set docker0 up
$SUDO ip netns exec "$C" ip addr add 10.99.0.2/24 dev eth0
$SUDO ip netns exec "$C" ip link set eth0 up
$SUDO ip netns exec "$C" ip route add default via 10.99.0.1
inh sysctl -qw net.ipv4.ip_forward=1
inh nft -f - <<'NFT'
table ip hfw_docker_like {
  chain prerouting {
    type nat hook prerouting priority dstnat; policy accept;
    tcp dport 9000 dnat to 10.99.0.2:80
  }
}
NFT
$SUDO ip netns exec "$C" python3 "$PROBE" serve tcp:80 >/dev/null 2>&1 &

TCP="tcp:22,80,443,7447,7448,8000,8443,9000,9100"
UDP="udp:500,4500,5353,51820,51821"
inh python3 "$PROBE" serve tcp:22,80,443,7447,7448,8000,8443,9100 "$UDP" > "$WORK/serve.out" 2>&1 &
SERVER_PID=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do grep -q ready "$WORK/serve.out" 2>/dev/null && break; sleep 0.3; done

# scan <label> <src> <dst> <expected open ports, space separated>
scan() {
  local label="$1" src="$2" dst="$3" want="$4" got
  got="$(inp python3 "$PROBE" scan "$src" "$dst" "$TCP" "$UDP" | tr '\n' ' ' | sed 's/ $//')"
  if [ "$got" = "$want" ]; then
    log "PASS  $label: open [$got]"
  else
    log "FAIL  $label: open [$got], want [$want]"
    FAILS=$((FAILS + 1))
  fi
}

# load <fixture> [override JSON]
load() {
  local fx="$1" ov="${2:-}"
  if [ -n "$ov" ]; then
    printf '%s' "$ov" > "$WORK/override.json"
    python3 "$RENDER" --input "$FIX/$fx.json" --override "$WORK/override.json" > "$WORK/r.nft"
  else
    python3 "$RENDER" --input "$FIX/$fx.json" > "$WORK/r.nft"
  fi
  inh nft -f "$WORK/r.nft"
  log "loaded $fx${ov:+ with override} ($(tail -n 1 "$WORK/r.nft"))"
  [ "$(inh nft list tables | grep -c 'goat_host_firewall')" = 1 ] || {
    log "FAIL  more than one goat_host_firewall table"; FAILS=$((FAILS + 1)); }
}

log "== edge, wireguard: baseline"
load edge-wireguard
scan "management source"  198.51.100.10 198.51.100.1 "tcp/22"
scan "outsider"           203.0.113.9   203.0.113.1  ""
scan "partner endpoint"   192.0.2.10    192.0.2.1    "udp/51821"
scan "tunnel (wg-fed0)"   169.254.10.0  169.254.10.1 "tcp/7448 tcp/8443"
scan "mesh (wt0)"         100.127.0.9   100.127.0.1  ""
scan "published port, outsider" 203.0.113.9 203.0.113.1 ""
got_lo="$(inh python3 "$PROBE" scan 127.0.0.1 127.0.0.1 tcp:22,7448 | tr '\n' ' ')"
[ "$got_lo" = "tcp/22 tcp/7448 " ] && log "PASS  loopback: open [$got_lo]" || {
  log "FAIL  loopback: open [$got_lo]"; FAILS=$((FAILS + 1)); }

log "== edge, wireguard: tightening override"
load edge-wireguard '{"icmp-echo": {"drop": true}, "wg-fed0/tunnel-extra": {"drop": true}, "ssh-management": {"interface": "eth0"}}'
scan "management source"  198.51.100.10 198.51.100.1 "tcp/22"
scan "tunnel (wg-fed0)"   169.254.10.0  169.254.10.1 "tcp/7448"
scan "mesh (wt0)"         100.127.0.9   100.127.0.1  ""
if inp ping -c 1 -W 1 -I 198.51.100.10 198.51.100.1 >/dev/null 2>&1; then
  log "FAIL  ping answered after icmp-echo was dropped"; FAILS=$((FAILS + 1))
else
  log "PASS  ping unanswered after icmp-echo was dropped"
fi

log "== edge, wireguard: a loosening with a reason (published port 9000)"
load edge-wireguard '{"other-tcp": {"ports": [9000], "reason": "a published service the site runs"}}'
scan "outsider (published 9000 now admitted)" 203.0.113.9 203.0.113.1 "tcp/9000"

log "== edge, ipsec-fips: baseline"
load edge-ipsec
scan "partner endpoint"   192.0.2.20    192.0.2.1    "udp/500 udp/4500"
scan "not the partner"    192.0.2.10    192.0.2.1    ""
scan "tunnel (ipsec-xab)" 169.254.11.0  169.254.11.1 "tcp/7448"
scan "management source"  198.51.100.10 198.51.100.1 "tcp/22"

log "== core: baseline"
load core
scan "management source"  198.51.100.10 198.51.100.1 "tcp/22 tcp/7447"
scan "outsider"           203.0.113.9   203.0.113.1  "tcp/7447"
scan "mesh (wt0)"         100.127.0.9   100.127.0.1  "tcp/7447 udp/5353"

log "== core: tightening override"
load core '{"fabric-tls": {"sources": ["100.127.0.0/16"]}}'
scan "outsider"           203.0.113.9   203.0.113.1  ""
scan "mesh (wt0)"         100.127.0.9   100.127.0.1  "tcp/7447 udp/5353"
scan "management source"  198.51.100.10 198.51.100.1 "tcp/22"

if [ "$FAILS" -gt 0 ]; then log "FAIL ($FAILS)"; exit 1; fi
log "PASS"
