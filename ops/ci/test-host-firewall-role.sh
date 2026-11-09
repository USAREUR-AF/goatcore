#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# test-host-firewall-role.sh — runs roles/goat-host-firewall for real, over SSH, from a
# controller container to a systemd target container, then scans the target.
#
#   1. core profile converges: the table is loaded, the boot unit is enabled and
#      active, the recorded hash equals the rendered one, the rollback is disarmed
#   2. a second run changes nothing
#   3. scans: the management side reaches 22 and the fabric port, an outsider reaches
#      the fabric port and nothing else that listens
#   4. refused before anything changes, the table's hash unchanged after each:
#      a management source that does not hold the controller (lockout, GB-213),
#      a loosening without a reason (GB-212), an unknown rule (GB-211), no management
#      sources (GB-210)
#   4b. check mode prints the hash and loads nothing
#   5. a tightening override applies and an outsider no longer reaches the fabric port
#   6. edge profile: a second peering's run keeps the first peering's tunnel rule
#   7. rollback: a ruleset that drops the controller, loaded with the rollback armed
#      exactly as tasks/apply.yml arms it, cuts SSH and is undone by the timer
#
# Every container has its own network namespace; the host's firewall is never touched.
# Needs docker with --privileged and cgroup v2.
# usage: ops/ci/test-host-firewall-role.sh [--keep]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
KEEP=0
[ "${1:-}" = "--keep" ] && KEEP=1
P="hfw-role-$$"
IMG="hfw-role:$$"
NET="$P-net"
LOGS="$(mktemp -d)"
TESTS=/src/ops/ansible/roles/goat-host-firewall/tests

log() { printf '[host-firewall-role] %s\n' "$*"; }
fail() { printf '[host-firewall-role] FAIL: %s\n' "$*" >&2; exit 1; }

command -v docker >/dev/null || { log "SKIP (docker not on PATH)"; exit 0; }

cleanup() {
  if [ "$KEEP" = 0 ]; then
    docker rm -f "$P-tgt" "$P-ctl" "$P-out" >/dev/null 2>&1 || true
    docker network rm "$NET" >/dev/null 2>&1 || true
    docker rmi "$IMG" >/dev/null 2>&1 || true
    rm -rf "$LOGS"
  else
    log "kept: containers $P-tgt $P-ctl $P-out, network $NET, logs $LOGS"
  fi
}
trap cleanup EXIT

log "image"
docker build -q -t "$IMG" - >/dev/null <<'DOCKERFILE' || fail "building the test image"
FROM ubuntu:24.04
RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
      systemd systemd-sysv sudo openssh-server openssh-client python3 python3-pip python3-yaml \
      nftables iproute2 iputils-ping >/dev/null \
 && pip3 install --break-system-packages -q 'ansible-core>=2.16,<2.21' \
 && useradd -m -s /bin/bash ops && echo 'ops ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/ops \
 && systemctl enable ssh
CMD ["/sbin/init"]
DOCKERFILE

docker network create "$NET" >/dev/null
for r in tgt ctl out; do
  docker run -d --privileged --cgroupns=host -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
    -v "$ROOT:/src:ro" --network "$NET" --name "$P-$r" --hostname "$r" "$IMG" >/dev/null
done
for r in tgt ctl out; do
  for _ in $(seq 1 30); do
    st=$(docker exec "$P-$r" systemctl is-system-running 2>/dev/null || true)
    case "$st" in running|degraded) break ;; esac
    sleep 1
  done
done

ip_of() { docker inspect -f "{{(index .NetworkSettings.Networks \"$NET\").IPAddress}}" "$P-$1"; }
TGT=$(ip_of tgt); CTL=$(ip_of ctl); OUT=$(ip_of out)
log "target $TGT, controller $CTL, outsider $OUT"

tgt() { docker exec "$P-tgt" sh -c "$*"; }
ctl() { docker exec "$P-ctl" sh -c "$*"; }

ctl 'mkdir -p /t && ssh-keygen -q -t ed25519 -N "" -f /t/id'
docker exec "$P-ctl" cat /t/id.pub | docker exec -i "$P-tgt" sh -c \
  'install -d -m 700 -o ops -g ops /home/ops/.ssh && cat > /home/ops/.ssh/authorized_keys && chown ops:ops /home/ops/.ssh/authorized_keys && chmod 600 /home/ops/.ssh/authorized_keys'
ctl "printf '%s\n' '[t]' 'tgt ansible_host=$TGT ansible_user=ops ansible_ssh_private_key_file=/t/id ansible_python_interpreter=/usr/bin/python3 ansible_ssh_common_args=\"-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5\"' > /t/hosts"
ctl 'ansible -i /t/hosts tgt -m ping' >/dev/null || fail "the controller cannot reach the target over SSH"

# Something listening on the ports the scans look at, beside sshd on 22.
docker exec -d "$P-tgt" python3 /src/ops/ansible/roles/goat-host-firewall/tests/probe.py serve tcp:7447,7448,8000,9100 udp:5353,51820,51821

vars() { # name, JSON
  printf '%s' "$2" | docker exec -i "$P-ctl" sh -c "cat > /t/$1.json"
}
play() { # vars-file
  ctl "cd $TESTS && ANSIBLE_HOST_KEY_CHECKING=0 ansible-playbook -i /t/hosts converge.yml -e @/t/$1.json" >"$LOGS/$1.log" 2>&1
}
refused() { # vars-file, the code the refusal must carry
  local before after
  before=$(tgt 'cat /etc/goat-host-firewall/applied.sha256; nft list table inet goat_host_firewall | sha256sum')
  if play "$1"; then cat "$LOGS/$1.log"; fail "$1 was accepted"; fi
  grep -q "$2" "$LOGS/$1.log" || { cat "$LOGS/$1.log"; fail "$1 refused, but not with $2"; }
  after=$(tgt 'cat /etc/goat-host-firewall/applied.sha256; nft list table inet goat_host_firewall | sha256sum')
  [ "$before" = "$after" ] || fail "$1 was refused but the loaded table changed"
  log "ok: refused $1 ($2), nothing changed"
}
scan() { # container, label, expected open ports
  local got
  got=$(docker exec "$P-$1" python3 /src/ops/ansible/roles/goat-host-firewall/tests/probe.py scan \
        0.0.0.0 "$TGT" tcp:22,7447,7448,8000,9100 udp:5353,51820,51821 | tr '\n' ' ' | sed 's/ $//')
  [ "$got" = "$3" ] || fail "scan from $2: open [$got], want [$3]"
  log "ok: scan from $2: open [$got]"
}

CORE_BASE='"goat_host_firewall_baseline": true, "goat_host_firewall_profile": "core", "goat_host_firewall_mesh_dns_port": ""'

log "1: core profile converges"
scan out "outsider, before" "tcp/22 tcp/7447 tcp/7448 tcp/8000 tcp/9100 udp/5353 udp/51820 udp/51821"
vars core "{$CORE_BASE, \"goat_host_firewall_management_sources\": [\"$CTL/32\"]}"
play core || { cat "$LOGS/core.log"; fail "1: the core profile did not converge"; }
tgt 'nft list table inet goat_host_firewall' | grep 'policy drop' >/dev/null || fail "1: the table is not loaded"
tgt 'systemctl is-enabled goat-host-firewall.service && systemctl is-active goat-host-firewall.service' >/dev/null \
  || fail "1: the boot unit is not enabled and active"
tgt 'systemctl is-active goat-host-firewall-rollback.timer' >/dev/null 2>&1 && fail "1: the rollback is still armed"
want=$(tgt 'sha256sum /etc/goat-host-firewall/ruleset.nft' | cut -d' ' -f1)
[ "$want" = "$(tgt 'cat /etc/goat-host-firewall/applied.sha256')" ] || fail "1: the recorded hash is not the loaded file's"
grep -q "sha256 $want" "$LOGS/core.log" || fail "1: the play did not print the hash it loaded"
log "ok: 1 converged, sha256 $want"

log "2: a second run changes nothing"
play core || fail "2: the second run failed"
grep -Eq 'changed=0 ' "$LOGS/core.log" || { grep -B1 '^changed:' "$LOGS/core.log"; fail "2: the second run changed something"; }
log "ok: 2 changed=0"

log "3: scans"
scan ctl "the management source" "tcp/22 tcp/7447 udp/51820"
scan out "an outsider" "tcp/7447 udp/51820"

log "4: refusals"
vars lockout "{$CORE_BASE, \"goat_host_firewall_management_sources\": [\"198.51.100.10/32\"]}"
refused lockout GB-213
vars noreason "{$CORE_BASE, \"goat_host_firewall_management_sources\": [\"$CTL/32\"], \"goat_host_firewall_override\": {\"other-tcp\": {\"ports\": [9100]}}}"
refused noreason GB-212
vars unknown "{$CORE_BASE, \"goat_host_firewall_management_sources\": [\"$CTL/32\"], \"goat_host_firewall_override\": {\"node-exporter\": {\"ports\": [9100], \"reason\": \"x\"}}}"
refused unknown GB-211
vars nomgmt "{$CORE_BASE, \"goat_host_firewall_management_sources\": []}"
refused nomgmt GB-210

log "4b: check mode renders and loads nothing"
vars checkmode "{$CORE_BASE, \"goat_host_firewall_management_sources\": [\"$CTL/32\"], \"goat_host_firewall_override\": {\"icmp-echo\": {\"drop\": true}}}"
before=$(tgt 'cat /etc/goat-host-firewall/applied.sha256; nft list table inet goat_host_firewall | sha256sum')
ctl "cd $TESTS && ansible-playbook -i /t/hosts converge.yml --check -e @/t/checkmode.json" >"$LOGS/checkmode.log" 2>&1 \
  || { cat "$LOGS/checkmode.log"; fail "4b: the check-mode run failed"; }
grep -q 'check mode: would load ruleset sha256 ' "$LOGS/checkmode.log" || fail "4b: check mode did not report the hash"
[ "$before" = "$(tgt 'cat /etc/goat-host-firewall/applied.sha256; nft list table inet goat_host_firewall | sha256sum')" ] \
  || fail "4b: check mode changed the loaded table"
log "ok: 4b check mode changed nothing"

log "5: a tightening override, then a loosening with a reason"
vars tight "{$CORE_BASE, \"goat_host_firewall_management_sources\": [\"$CTL/32\"], \"goat_host_firewall_override\": {\"fabric-tls\": {\"sources\": [\"$CTL/32\"]}, \"icmp-echo\": {\"drop\": true}}}"
play tight || { cat "$LOGS/tight.log"; fail "5: the tightening override did not apply"; }
tgt 'grep -q "^# tightened fabric-tls" /etc/goat-host-firewall/ruleset.nft' || fail "5: the ruleset does not record the tightening"
scan out "an outsider, tightened" "udp/51820"
scan ctl "the management source, tightened" "tcp/22 tcp/7447 udp/51820"
vars loose "{$CORE_BASE, \"goat_host_firewall_management_sources\": [\"$CTL/32\"], \"goat_host_firewall_override\": {\"other-tcp\": {\"ports\": [9100], \"reason\": \"a site exporter\"}}}"
play loose || { cat "$LOGS/loose.log"; fail "5: the loosening with a reason did not apply"; }
tgt 'grep -q "^# loosened other-tcp: ports \[\] -> \[9100\]; reason: a site exporter" /etc/goat-host-firewall/ruleset.nft' \
  || fail "5: the ruleset does not record the loosening and its reason"
scan out "an outsider, loosened" "tcp/7447 tcp/9100 udp/51820"
log "ok: 5"

log "6: edge profile, two peerings on one host"
tgt 'ip link add wg-fed0 type dummy && ip link add wg-fed1 type dummy'
EDGE_BASE="\"goat_host_firewall_baseline\": true, \"goat_host_firewall_profile\": \"edge\", \"goat_host_firewall_management_sources\": [\"$CTL/32\"]"
vars edge0 "{$EDGE_BASE, \"goat_host_firewall_peering\": {\"name\": \"wg-fed0\", \"transport\": \"wireguard\", \"listen_port\": 51821, \"peer_addr\": \"$OUT\", \"tunnel_interface\": \"wg-fed0\", \"remote_link_ip\": \"169.254.10.0\", \"edge_port\": 7448, \"extra_tcp_ports\": [], \"via_forwarder\": false}}"
vars edge1 "{$EDGE_BASE, \"goat_host_firewall_peering\": {\"name\": \"wg-fed1\", \"transport\": \"wireguard\", \"listen_port\": 51823, \"peer_addr\": \"192.0.2.30\", \"tunnel_interface\": \"wg-fed1\", \"remote_link_ip\": \"169.254.12.0\", \"edge_port\": 7449, \"extra_tcp_ports\": [], \"via_forwarder\": false}}"
play edge0 || { cat "$LOGS/edge0.log"; fail "6: the first peering did not converge"; }
scan out "the partner endpoint of wg-fed0" "udp/51820 udp/51821"
play edge1 || { cat "$LOGS/edge1.log"; fail "6: the second peering did not converge"; }
tgt 'nft list table inet goat_host_firewall' | grep '"wg-fed0/tunnel"' >/dev/null || fail "6: the second peering's run cut the first's tunnel rule"
tgt 'nft list table inet goat_host_firewall' | grep '"wg-fed1/tunnel"' >/dev/null || fail "6: the second peering's tunnel rule is missing"
scan out "the partner endpoint of wg-fed0, after wg-fed1" "udp/51820 udp/51821"
log "ok: 6"

log "7: rollback"
# A new SSH connection, never the multiplexed one ansible keeps open.
fresh_ssh() { ctl "ssh -o ControlPath=none -o ConnectTimeout=3 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -i /t/id ops@$TGT true" >/dev/null 2>&1; }
before=$(tgt 'nft list table inet goat_host_firewall | sha256sum')
# A ruleset that admits SSH from nobody the controller is, loaded with the rollback
# armed by the same commands tasks/apply.yml runs.
tgt "python3 /src/ops/ansible/roles/goat-host-firewall/files/goat-host-firewall-render.py --input /src/ops/ansible/roles/goat-host-firewall/tests/fixtures/core.json > /tmp/cut.nft"
tgt '{ printf "table inet goat_host_firewall\ndelete table inet goat_host_firewall\n"; nft list table inet goat_host_firewall; } > /etc/goat-host-firewall/rollback.nft'
tgt 'systemd-run --unit=goat-host-firewall-rollback --on-active=8 /usr/sbin/nft -f /etc/goat-host-firewall/rollback.nft' >/dev/null 2>&1
tgt 'nft -f /tmp/cut.nft'
if fresh_ssh; then fail "7: the cutting ruleset did not cut SSH"; fi
log "ok: SSH is cut"
for _ in $(seq 1 20); do
  fresh_ssh && break
  sleep 1
done
fresh_ssh || fail "7: the rollback did not restore SSH"
[ "$(tgt 'nft list table inet goat_host_firewall | sha256sum')" = "$before" ] || fail "7: the restored table is not the previous one"
log "ok: 7 the timer restored the previous table and SSH"

log "PASS"
