#!/usr/bin/env bash
# Runs as root on a peer machine.
#
#   peer-ctl.sh up <stock|pr6774> <ws|quic> <management url> <setup key> <hostname>
#   peer-ctl.sh down
#   peer-ctl.sh status       netbird status --json
#
# The client daemon runs in the foreground mode of "netbird service run" under
# setsid, with NB_FORCE_RELAY=true so no direct connection is tried, and
# NB_RELAY_TRANSPORT set to the transport under test, so the client dials only
# that transport. NB_LAZY_CONN=off connects the peers at once instead of on first
# traffic.
set -euo pipefail
action=${1:?action}
D=/opt/rcmp
PIDF=$D/netbird.pid

down() {
  if [ -f "$PIDF" ]; then
    p=$(cat "$PIDF")
    if kill -0 "$p" 2>/dev/null; then
      kill "$p" || true
      for _ in $(seq 1 100); do kill -0 "$p" 2>/dev/null || break; sleep 0.1; done
      kill -9 "$p" 2>/dev/null || true
    fi
    rm -f "$PIDF"
  fi
  pkill -f "$D/bin/netbird-" 2>/dev/null || true
  ip link del wt0 2>/dev/null || true
  pkill -x iperf3 2>/dev/null || true
}

case "$action" in
  down) down ;;
  status) $D/bin/netbird status --json 2>/dev/null || true ;;
  up)
    client=${2:?client} transport=${3:?transport} mgmt=${4:?mgmt url} key=${5:?setup key} name=${6:?hostname}
    down
    ln -sf "$D/bin/netbird-$client" $D/bin/netbird
    mkdir -p $D/logs /var/lib/netbird
    log=$D/logs/netbird-$client-$transport-$(date -u +%Y%m%dT%H%M%SZ).log
    NB_FORCE_RELAY=true NB_RELAY_TRANSPORT=$transport NB_LAZY_CONN=off \
      setsid nohup "$D/bin/netbird-$client" service run --log-level info --log-file "$log" \
      > "$log.stdout" 2>&1 < /dev/null &
    echo $! > "$PIDF"
    for _ in $(seq 1 50); do
      if $D/bin/netbird status >/dev/null 2>&1; then break; fi
      sleep 0.2
    done
    $D/bin/netbird up --management-url "$mgmt" --setup-key "$key" --hostname "$name" > /dev/null
    echo "pid=$(cat $PIDF) log=$log"
    ;;
  *) echo "unknown action $action" >&2; exit 2 ;;
esac
