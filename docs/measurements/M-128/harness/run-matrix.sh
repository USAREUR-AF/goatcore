#!/usr/bin/env bash
# Run the comparison matrix on prepared machines (prepare.sh first).
#
#   run-matrix.sh <out dir>
#
# For each run index (RUNS), for each configuration (relay, TLS mode,
# transport, client build), in an order reversed on every second run:
#   1. the relay is started fresh (one control-plane account serves the
#      whole matrix),
#   2. both peers are restarted with NB_FORCE_RELAY=true and the transport
#      under test, and wait until each sees the other connected through the
#      relay,
#   3. the relay's idle memory is sampled with the two peers connected,
#   4. one iperf3 test per flow count (FLOWS) from peer-a to peer-b over the
#      NetBird interface, while the relay's CPU time and memory are sampled
#      once a second and the relay machine's TCP, UDP and kernel TLS counters
#      are read before and after.
#
# Knobs (environment): RUNS (3), FLOWS ("1 4 8 16"), DURATION (30 s),
# OMIT (3 s, excluded from iperf3's result), CLIENTS ("stock pr6774"),
# CONFIGS (relay:tls:transport list, see below), PAUSE (5 s between tests).
set -euo pipefail
. "$(dirname "$0")/lib.sh"
load_hosts
out=${1:?out dir}
mkdir -p "$out"

RUNS=${RUNS:-3}
FLOWS=${FLOWS:-1 4 8 16}
DURATION=${DURATION:-30}
OMIT=${OMIT:-3}
PAUSE=${PAUSE:-5}
CLIENTS=${CLIENTS:-stock pr6774}
# goat-relay: WebSocket with the kernel TLS module available (kTLS), with it
# unloaded (TLS in user space), and QUIC. The Go relay: WebSocket and QUIC;
# the module is left available, and the counters show it is not used.
CONFIGS=${CONFIGS:-goat:ktls:ws goat:user:ws goat:ktls:quic go:ktls:ws go:ktls:quic}
RELAY_BIND=${RELAY_BIND:-$RELAY_ADDR}
MGMT_URL="http://$CONTROL_ADDR:$MGMT_PORT"

{
  echo "started_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "runs=$RUNS flows=$FLOWS duration=$DURATION omit=$OMIT pause=$PAUSE"
  echo "clients=$CLIENTS"
  echo "configs=$CONFIGS"
  echo "netbird_version=$NETBIRD_VERSION goat_relay_ref=$GOAT_RELAY_REF"
} > "$out/matrix.txt"

log "control plane"
key=$(on_script control "$REMOTE_DIR/control-ctl.sh" start "$CONTROL_ADDR" "$MGMT_PORT" \
  "rels://$RELAY_ADDR:$RELAY_PORT" | tail -1)
[ -n "$key" ] || die "no setup key"

counters() { # role -> stdout
  on "$1" 'cat /proc/net/snmp /proc/net/netstat; echo "--- tls_stat"; cat /proc/net/tls_stat 2>/dev/null || echo absent; echo "--- tls_path"; bash /opt/rcmp/relay-ctl.sh tls-path'
}

wait_connected() { # dir
  local d=$1
  for _ in $(seq 1 90); do
    on peer-a "/opt/rcmp/bin/netbird status --json" > "$d/peer-a-status.json" 2>/dev/null || true
    on peer-b "/opt/rcmp/bin/netbird status --json" > "$d/peer-b-status.json" 2>/dev/null || true
    if python3 "$HARNESS_DIR/status_check.py" "$d/peer-a-status.json" "$d/peer-b-status.json" > "$d/peer-b-ip" 2>"$d/status-check.err"; then
      return 0
    fi
    sleep 2
  done
  cat "$d/status-check.err" >&2
  return 1
}

one_config() { # run relay mode transport client
  local r=$1 relay=$2 mode=$3 transport=$4 client=$5
  local id="$relay-$mode-$transport-$client"
  local d="$out/$id/r$r"
  mkdir -p "$d"
  log "run $r: $id"
  on peer-a "bash /opt/rcmp/peer-ctl.sh down" || true
  on peer-b "bash /opt/rcmp/peer-ctl.sh down" || true
  on_script relay "$REMOTE_DIR/relay-ctl.sh" start "$relay" "$mode" "$RELAY_BIND" "$RELAY_PORT" \
    "$RELAY_ADDR:$RELAY_PORT" > "$d/relay-start.txt"
  local pid; pid=$(sed -n 's/^pid=\([0-9]*\).*/\1/p' "$d/relay-start.txt")
  on relay "sleep 3; grep -E 'VmRSS|VmHWM' /proc/$pid/status" > "$d/relay-mem-no-peers.txt"
  on_script peer-a "$REMOTE_DIR/peer-ctl.sh" up "$client" "$transport" "$MGMT_URL" "$key" peer-a > "$d/peer-a-up.txt"
  on_script peer-b "$REMOTE_DIR/peer-ctl.sh" up "$client" "$transport" "$MGMT_URL" "$key" peer-b > "$d/peer-b-up.txt"
  if ! wait_connected "$d"; then
    log "FAIL: peers not connected through the relay for $id run $r"
    echo "not-connected" > "$d/FAILED"
    return 0
  fi
  local bip; bip=$(cat "$d/peer-b-ip")
  on peer-a "ping -c 3 -W 2 $bip" > "$d/ping.txt" 2>&1 || true
  on peer-a "ss -tunp state established 2>/dev/null | grep -F '$RELAY_ADDR:$RELAY_PORT' || true; echo '--- udp'; ss -uanp 2>/dev/null | grep -F '$RELAY_ADDR:$RELAY_PORT' || true" > "$d/peer-a-sockets.txt"
  on peer-a "grep -hiE 'relay|quic|websocket' /opt/rcmp/logs/netbird-$client-$transport-*.log 2>/dev/null | tail -n 40" > "$d/peer-a-relay-log.txt" || true
  sleep 10
  on relay "grep -E 'VmRSS|VmHWM' /proc/$pid/status" > "$d/relay-mem-idle-2-peers.txt"
  on peer-b "pkill -x iperf3 || true; iperf3 -s -D -B $bip -p 5201 --logfile /opt/rcmp/logs/iperf3-server.log; sleep 1"
  local f
  for f in $FLOWS; do
    local fd="$d/f$f"
    mkdir -p "$fd"
    counters relay > "$fd/relay-counters-before.txt"
    on relay "setsid nohup python3 /opt/rcmp/sampler.py $pid /opt/rcmp/samples.csv 1 >/dev/null 2>&1 < /dev/null & echo \$! > /opt/rcmp/sampler.pid"
    sleep 1
    on peer-a "iperf3 -c $bip -p 5201 -P $f -t $DURATION -O $OMIT -J" > "$fd/iperf.json" 2> "$fd/iperf.err" || true
    on relay "kill \$(cat /opt/rcmp/sampler.pid) 2>/dev/null; sleep 1; cat /opt/rcmp/samples.csv" > "$fd/relay-samples.csv"
    counters relay > "$fd/relay-counters-after.txt"
    sleep "$PAUSE"
  done
  on peer-b "pkill -x iperf3 || true"
  on relay "grep -E 'VmRSS|VmHWM' /proc/$pid/status" > "$d/relay-mem-end.txt"
  local rlog; rlog=$(sed -n 's/^pid=[0-9]* log=//p' "$d/relay-start.txt")
  on relay "cat $rlog" > "$d/relay.log" 2>/dev/null || true
}

order=$CONFIGS
reversed=""
for c in $order; do reversed="$c $reversed"; done

r=1
while [ "$r" -le "$RUNS" ]; do
  list=$order
  if [ $((r % 2)) -eq 0 ]; then list=$reversed; fi
  for c in $list; do
    relay=${c%%:*}; rest=${c#*:}; mode=${rest%%:*}; transport=${rest#*:}
    for client in $CLIENTS; do
      one_config "$r" "$relay" "$mode" "$transport" "$client"
    done
  done
  r=$((r + 1))
done

on peer-a "bash /opt/rcmp/peer-ctl.sh down" || true
on peer-b "bash /opt/rcmp/peer-ctl.sh down" || true
on relay "bash /opt/rcmp/relay-ctl.sh stop" || true
python3 "$HARNESS_DIR/scrub.py" "${HOSTS_FILE:-$HARNESS_DIR/hosts.env}" "$out" >&2
echo "finished_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$out/matrix.txt"
log "matrix done: $out"
