#!/usr/bin/env bash
# GHSA-x44g: the metrics port binds loopback + the mesh address (+ the Docker
# bridge gateway), the play stops when the mesh interface has no IPv4
# address, and only logship_bind_all publishes on every interface. Runs
# tasks/bind.yml on localhost; the gateway address is given, so no docker.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fails=0
fail() { echo "  FAIL $1"; fails=$((fails + 1)); }
play() { ansible-playbook -i localhost, --connection local "$SCRIPT_DIR/bind.yml" "$@" 2>&1; }
ips() { printf '%s\n' "$1" | sed -n 's/.*"BIND_IPS=\(.*\)"$/\1/p' | head -1 | tr -d '\\'; }
noif=x44g-no-such-if0

# 1. No mesh interface and no bind address: fail closed.
if out="$(play -e logship_mesh_iface=$noif)"; then fail "no mesh address: play passed ($(ips "$out"))"
elif printf '%s\n' "$out" | grep -q "no IPv4 address on $noif"; then echo "  ok   no IPv4 on the mesh interface stops the play"
else fail "no mesh address: failed for another reason"; printf '%s\n' "$out" | tail -5; fi

gw=(-e logship_docker_gateway_address=172.17.0.1 -e logship_metrics_scraper_iface=br-0123456789ab)
# 2. Mesh address given, gateway on (the scraper's bridge known).
out="$(play -e logship_mesh_iface=$noif -e logship_bind_address=100.64.0.7 "${gw[@]}")" || true
[ "$(ips "$out")" = '["127.0.0.1", "100.64.0.7", "172.17.0.1"]' ] && echo "  ok   binds loopback, mesh, bridge gateway" || fail "mesh+gateway: $(ips "$out")"

# 3. Gateway binding off.
out="$(play -e logship_bind_address=100.64.0.7 -e '{"logship_metrics_bind_docker_gateway": false}')" || true
[ "$(ips "$out")" = '["127.0.0.1", "100.64.0.7"]' ] && echo "  ok   gateway off: loopback and mesh only" || fail "gateway off: $(ips "$out")"

# 4. Mesh address from the interface's facts (loopback stands in for wt0).
out="$(play -e logship_mesh_iface=lo "${gw[@]}")" || true
[ "$(ips "$out")" = '["127.0.0.1", "172.17.0.1"]' ] && echo "  ok   address read from the interface's facts" || fail "iface facts: $(ips "$out")"

# 4b. The scraper's network does not exist: no gateway binding at all (the
#     address every container can reach is not published unguarded).
out="$(play -e logship_bind_address=100.64.0.7 -e logship_docker_gateway_address=172.17.0.1 -e logship_metrics_scraper_network=x44g-no-such-net)" || true
[ "$(ips "$out")" = '["127.0.0.1", "100.64.0.7"]' ] && printf '%s\n' "$out" | grep -q "does not exist, so 9598 is not published on" \
  && echo "  ok   no scraper network: the gateway is not bound, and the play says so" || fail "no scraper net: $(ips "$out")"

# 4c. The scraper's bridge is read from a real Docker network (needs docker;
#     creates and removes one empty network).
if docker info >/dev/null 2>&1; then
  net="x44g-bindtest-$$"; docker network create "$net" >/dev/null
  want_if="br-$(docker network inspect -f '{{.Id}}' "$net" | cut -c1-12)"
  out="$(play -e logship_bind_address=100.64.0.7 -e logship_docker_gateway_address=172.17.0.1 -e logship_metrics_scraper_network="$net")" || true
  got_if="$(printf '%s\n' "$out" | sed -n 's/.*"SCRAPER_IF=\(.*\)"$/\1/p' | head -1)"
  docker network rm "$net" >/dev/null
  [ "$got_if" = "$want_if" ] \
    && echo "  ok   scraper bridge from the network id ($got_if)" || fail "scraper bridge: got '$got_if', want '$want_if'"
else echo "  SKIP scraper bridge from a real network (no docker)"; fi

# 5. bind_all: no mesh address needed, nothing resolved.
out="$(play -e logship_mesh_iface=$noif -e '{"logship_bind_all": true}')" || fail "bind_all: play failed"
[ "$(ips "$out")" = '"ALL"' ] && echo "  ok   logship_bind_all skips the mesh lookup" || fail "bind_all: $(ips "$out")"

[ "$fails" -eq 0 ] || { echo "goat-logship: bind FAILED ($fails)"; exit 1; }
echo "goat-logship: bind OK"
