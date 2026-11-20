#!/usr/bin/env bash
# Inner half of guard.sh: runs as root of a private user + network namespace
# (unshare -rn), so nothing here touches the host's network. Builds a stand-in
# host with veth links to stand-in peers, loads the rendered guard, and
# prints "<case> <code>" lines. Args: <guard.nft> <dnat|input>
set -euo pipefail
nft_file=$1 mode=$2
GW=172.17.0.1 MESH=100.64.0.7 PORT=9598
pids=()
trap 'kill "${pids[@]}" 2>/dev/null || true' EXIT
ip link set lo up
peer() { # peer <var> <host-if> <net> : veth <host-if> to a new netns, host .1, peer .2
  unshare -n sleep 120 >/dev/null 2>&1 &
  local pid=$!; pids+=("$pid"); sleep 0.2
  ip link add "$2" type veth peer name c0
  ip link set c0 netns "$pid"
  ip addr add "$3.1/24" dev "$2"; ip link set "$2" up
  nsenter -t "$pid" -n sh -c "ip link set lo up; ip addr add $3.2/24 dev c0; ip link set c0 up; ip route add default via $3.1"
  eval "$1=$pid"
}
peer P_MESH wt0 10.3.0
peer P_SCRAPE br-0123456789ab 10.1.0
peer P_OTHER br-other0 10.2.0
peer P_LAN lan0 192.168.77
ip addr add $MESH/32 dev wt0
ip link add docker0 type dummy; ip addr add $GW/16 dev docker0; ip link set docker0 up
serve() { python3 -c 'import http.server,sys; http.server.HTTPServer(("0.0.0.0", int(sys.argv[1])), http.server.SimpleHTTPRequestHandler).serve_forever()' $PORT >/dev/null 2>&1; }
if [ "$mode" = dnat ]; then
  # Docker's path: a published port is DNATed to the container and forwarded.
  peer P_CTR br-logship0 10.9.0
  nsenter -t "$P_CTR" -n bash -c "$(declare -f serve); PORT=$PORT; serve" & pids+=($!)
  echo 1 > /proc/sys/net/ipv4/ip_forward
  nft -f - <<NFT
table ip dockersim {
  chain pre { type nat hook prerouting priority dstnat; ip daddr { $GW, $MESH } tcp dport $PORT dnat to 10.9.0.2:$PORT; }
  chain out { type nat hook output priority -100; ip daddr { $GW, $MESH } tcp dport $PORT dnat to 10.9.0.2:$PORT; }
}
NFT
else
  # docker-proxy's path: the host itself accepts the connection.
  serve & pids+=($!)
fi
nft -f "$nft_file"
sleep 1
code() { curl -s -o /dev/null -w '%{http_code}' --max-time 2 "http://$1:$PORT/" || true; }
for dst in GW MESH; do
  a=${!dst}
  echo "$dst host $(code "$a")"
  for p in MESH SCRAPE OTHER LAN; do
    v=P_$p
    echo "$dst $p $(nsenter -t "${!v}" -n bash -c "$(declare -f code); PORT=$PORT; code $a")"
  done
done
