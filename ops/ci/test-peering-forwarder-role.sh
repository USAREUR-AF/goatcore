#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# test-peering-forwarder-role.sh — runs ops/ansible/peering-forwarder.yml for real
# on a systemd container, then proves the refusals refuse.
#
#   1. a well-formed declaration converges: the carrier unit is enabled and active,
#      the carrier holds exactly the edge peer, the table is loaded, IPv4 forwarding
#      is on, the state file says holds_peering_material false
#   2. a second run changes nothing
#   3. refused: an edge carrier key that is not a WireGuard key
#   4. refused: two peerings sharing a carrier port
#   5. refused: a file in the config directory the role did not write
#   6. refused: the host has a wt0 interface (a mesh peer)
#   7. refused: the host holds a peering edge's configuration
#
# Needs docker with --privileged and cgroup v2, and the wireguard kernel module.
# usage: ops/ci/test-peering-forwarder-role.sh [--keep]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
KEEP=0
[ "${1:-}" = "--keep" ] && KEEP=1
C="pf-role-$$"
IMG="pf-role:$$"
KEY_EDGE="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
KEY_EDGE2="BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA="

log() { printf '[peering-forwarder-role] %s\n' "$*"; }
fail() { printf '[peering-forwarder-role] FAIL: %s\n' "$*" >&2; exit 1; }

command -v docker >/dev/null || { log "SKIP (docker not on PATH)"; exit 0; }

cleanup() {
  if [ "$KEEP" = 0 ]; then
    docker rm -f "$C" >/dev/null 2>&1 || true
    docker rmi "$IMG" >/dev/null 2>&1 || true
  else
    log "kept: container $C"
  fi
}
trap cleanup EXIT

log "image"
docker build -q -t "$IMG" - >/dev/null <<'EOF' || fail "building the test image"
FROM ubuntu:24.04
RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
      systemd systemd-sysv sudo python3 python3-pip python3-apt wireguard-tools nftables iproute2 >/dev/null \
 && pip3 install --break-system-packages -q 'ansible-core>=2.16,<2.21'
CMD ["/sbin/init"]
EOF

docker run -d --privileged --cgroupns=host -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
  -v "$ROOT:/src:ro" --name "$C" --hostname forwarder "$IMG" >/dev/null
for _ in $(seq 1 30); do
  st=$(docker exec "$C" systemctl is-system-running 2>/dev/null || true)
  case "$st" in running|degraded) break ;; esac
  sleep 1
done

x() { docker exec "$C" sh -c "$*"; }
x 'mkdir -p /t && printf "[peering_forwarders]\nforwarder ansible_connection=local ansible_python_interpreter=/usr/bin/python3\n" > /t/hosts'

vars() { # file, then the peerings as JSON
  docker exec -i "$C" sh -c "cat > /t/$1.json" <<EOF
{"peering_forwarder_enabled": true, "peering_forwarder_public_interface": "eth0",
 "peering_forwarder_peerings": $2}
EOF
}
play() { # vars-file
  x "cd /src/ops/ansible && ANSIBLE_LOCALHOST_WARNING=0 ansible-playbook -i /t/hosts peering-forwarder.yml -e @/t/$1.json" >"/tmp/$C-$1.log" 2>&1
}
refused() { # vars-file, a phrase the refusal must carry
  if play "$1"; then
    cat "/tmp/$C-$1.log"; fail "$1 was accepted"
  fi
  grep -q "$2" "/tmp/$C-$1.log" || { cat "/tmp/$C-$1.log"; fail "$1 refused, but not with '$2'"; }
  log "ok: refused $1"
}

vars good "[{\"name\": \"xaa\", \"edge_carrier_public_key\": \"$KEY_EDGE\", \"partner_sources\": [\"192.0.2.20\"]}]"
play good || { cat "/tmp/$C-good.log"; fail "1: a well-formed declaration did not converge"; }
x 'systemctl is-enabled goat-peering-forwarder@wg-carry0.service && systemctl is-active goat-peering-forwarder@wg-carry0.service' >/dev/null \
  || fail "1: the carrier unit is not enabled and active"
[ "$(x 'wg show wg-carry0 peers')" = "$KEY_EDGE" ] || fail "1: the carrier does not hold exactly the edge"
x 'wg show wg-carry0 allowed-ips' | grep -q '169.254.30.1/32' || fail "1: the edge's AllowedIPs is not its carrier /32"
[ "$(x 'wg show wg-carry0 listen-port')" = 51821 ] || fail "1: the carrier does not listen on 51821"
table=$(x 'nft list table inet goat_peering_forwarder')
printf '%s\n' "$table" | grep -Eq 'ip saddr \{? ?192\.0\.2\.20 ?\}? udp dport 51820 dnat (ip )?to 169\.254\.30\.1:51820' \
  || { printf '%s\n' "$table"; fail "1: the table does not translate the peering port for the partner's source"; }
[ "$(x 'sysctl -n net.ipv4.ip_forward')" = 1 ] || fail "1: IPv4 forwarding is off"
x 'python3 -c "import json,sys; d=json.load(open(\"/var/log/goat-peering-forwarder-state.json\")); sys.exit(0 if d[\"holds_peering_material\"] is False and d[\"peerings\"][0][\"edge_carrier_public_key\"]==\"'"$KEY_EDGE"'\" and len(d[\"peerings\"][0][\"forwarder_carrier_public_key\"])==44 else 1)"' \
  || fail "1: the state file does not record the edge key, our key and holds_peering_material false"
grep -q 'peering_wg_forwarder_carrier_public_key: ' "/tmp/$C-good.log" || fail "1: the play did not print the forwarder's carrier key"
log "ok: 1 converged"

play good || fail "2: the second run failed"
# The state file is a timestamped record and is rewritten on every run; nothing else
# may change.
changed=$(awk '/^TASK \[/ {t=$0} /^changed: / {print t}' "/tmp/$C-good.log" | grep -v 'write the state file' || true)
[ -z "$changed" ] || { printf '%s\n' "$changed"; fail "2: the second run changed something besides the state file"; }
log "ok: 2 a second run changes nothing but the state file"

vars badkey '[{"name": "xaa", "edge_carrier_public_key": "not-a-key"}]'
refused badkey "is malformed"
vars sharedport "[{\"name\": \"xaa\", \"edge_carrier_public_key\": \"$KEY_EDGE\"}, {\"name\": \"xab\", \"interface\": \"wg-carry1\", \"peering_port\": 51830, \"carrier_local_ip\": \"169.254.30.2\", \"carrier_remote_ip\": \"169.254.30.3\", \"edge_carrier_public_key\": \"$KEY_EDGE2\"}]"
refused sharedport "Two peerings share"

x 'touch /etc/goat-peering-forwarder/partner-ca.txt'
refused good "holds /etc/goat-peering-forwarder/partner-ca.txt"
x 'rm /etc/goat-peering-forwarder/partner-ca.txt'

x 'ip link add wt0 type dummy'
refused good "member of a goatnet mesh"
x 'ip link del wt0'

x 'mkdir -p /etc/goat-peering-wg'
refused good "holds a peering"
x 'rmdir /etc/goat-peering-wg'

log "PASS"
