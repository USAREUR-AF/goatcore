#!/usr/bin/env bash
# Put everything on the four machines: packages, binaries, the relay's
# certificate (a throwaway CA made here, trusted by the two peers), the relay
# shared secret, and the remote scripts. Records each machine's facts into
# <out dir>/hosts/.
#
#   prepare.sh <out dir>
set -euo pipefail
. "$(dirname "$0")/lib.sh"
load_hosts
out=${1:?out dir}
mkdir -p "$out/hosts" "$STATE_DIR/tls"
[ -f "$BIN_DIR/versions.txt" ] || die "run fetch-binaries.sh first"
cp "$BIN_DIR/versions.txt" "$out/versions.txt"

tls=$STATE_DIR/tls
if [ ! -f "$tls/relay.crt" ] || [ "$(cat "$tls/relay.addr" 2>/dev/null)" != "$RELAY_ADDR" ]; then
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 30 \
    -subj "/CN=relay-compare test CA" -keyout "$tls/ca.key" -out "$tls/ca.crt" 2>/dev/null
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
    -subj "/CN=$RELAY_ADDR" -keyout "$tls/relay.key" -out "$tls/relay.csr" 2>/dev/null
  printf 'subjectAltName=IP:%s\nextendedKeyUsage=serverAuth\n' "$RELAY_ADDR" > "$tls/ext.cnf"
  openssl x509 -req -in "$tls/relay.csr" -CA "$tls/ca.crt" -CAkey "$tls/ca.key" -CAcreateserial \
    -days 30 -extfile "$tls/ext.cnf" -out "$tls/relay.crt" 2>/dev/null
  echo "$RELAY_ADDR" > "$tls/relay.addr"
fi
[ -f "$STATE_DIR/relay-secret" ] || head -c 32 /dev/urandom | base64 | tr -d '/+=' > "$STATE_DIR/relay-secret"

PKGS="iperf3 iproute2 iputils-ping python3 curl ca-certificates kmod procps openssl"
for role in control relay peer-a peer-b; do
  log "prepare $role"
  if [ "$HOST_MODE" = ssh ]; then
    on "$role" "export DEBIAN_FRONTEND=noninteractive; (apt-get update -q && apt-get install -y -q $PKGS) >/dev/null 2>&1 || (sleep 20; apt-get update -q && apt-get install -y -q $PKGS) >/dev/null"
  fi
  on "$role" "mkdir -p /opt/rcmp/bin /opt/rcmp/logs /opt/rcmp/tls; pkill -x iperf3 || true"
  for s in "$REMOTE_DIR"/*; do push "$role" "$s" "/opt/rcmp/$(basename "$s")"; done
  case "$role" in
    control)
      push control "$BIN_DIR/netbird-server" /opt/rcmp/bin/netbird-server
      push control "$STATE_DIR/relay-secret" /opt/rcmp/relay-secret
      ;;
    relay)
      push relay "$BIN_DIR/netbird-relay" /opt/rcmp/bin/netbird-relay
      push relay "$BIN_DIR/goat-relay" /opt/rcmp/bin/goat-relay
      push relay "$STATE_DIR/relay-secret" /opt/rcmp/relay-secret
      push relay "$tls/relay.crt" /opt/rcmp/tls/relay.crt
      push relay "$tls/relay.key" /opt/rcmp/tls/relay.key
      if [ "${RCMP_NO_MODPROBE:-0}" = 1 ]; then on relay "touch /opt/rcmp/no-modprobe"; fi
      ;;
    peer-*)
      push "$role" "$BIN_DIR/netbird-stock" /opt/rcmp/bin/netbird-stock
      push "$role" "$BIN_DIR/netbird-pr6774" /opt/rcmp/bin/netbird-pr6774
      push "$role" "$tls/ca.crt" /usr/local/share/ca-certificates/relay-compare-ca.crt
      on "$role" "update-ca-certificates >/dev/null 2>&1"
      ;;
  esac
  on "$role" "chmod 755 /opt/rcmp/bin/* /opt/rcmp/*.sh /opt/rcmp/*.py; chmod 600 /opt/rcmp/relay-secret 2>/dev/null || true"
  # shellcheck disable=SC2016  # expands on the remote machine
  on "$role" '
    echo "role_facts_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    . /etc/os-release; echo "os=$PRETTY_NAME"
    echo "kernel=$(uname -r)"
    echo "nproc=$(nproc)"
    echo "cpu=$(grep -m1 "model name" /proc/cpuinfo | cut -d: -f2- | sed "s/^ //")"
    echo "mem_kib=$(grep MemTotal /proc/meminfo | tr -s " " | cut -d" " -f2)"
    tok=$(curl -fsS -m 2 -X PUT http://169.254.169.254/latest/api/token -H "X-aws-ec2-metadata-token-ttl-seconds: 60" 2>/dev/null || true)
    if [ -n "$tok" ]; then
      for k in instance-type placement/availability-zone placement/group-name; do
        echo "ec2_$k=$(curl -fsS -m 2 -H "X-aws-ec2-metadata-token: $tok" http://169.254.169.254/latest/meta-data/$k 2>/dev/null || echo none)"
      done
    fi
    for k in net.core.rmem_max net.core.wmem_max net.core.rmem_default net.ipv4.tcp_congestion_control; do
      echo "$k=$(sysctl -n $k 2>/dev/null || echo n/a)"
    done
    echo "iperf3=$(iperf3 --version 2>/dev/null | head -1)"
    echo "wireguard_module=$( [ -d /sys/module/wireguard ] && echo loaded || (modprobe wireguard 2>/dev/null && echo loaded-now) || echo absent)"
    echo "tls_module=$( [ -d /sys/module/tls ] && echo loaded || echo not-loaded)"
  ' > "$out/hosts/$role.txt"
  sed 's/^/  /' "$out/hosts/$role.txt" >&2
done
python3 "$HARNESS_DIR/scrub.py" "${HOSTS_FILE:-$HARNESS_DIR/hosts.env}" "$out" >&2
log "prepared"
