#!/usr/bin/env bash
# Runs as root on the control machine.
#
#   control-ctl.sh start <control addr> <mgmt port> <relay url>
#       Starts the upstream combined server (management, signal, STUN) with
#       an external relay, creates the owner account and a personal access
#       token through /api/setup, and creates a reusable setup key. Prints
#       the setup key on the last line.
#   control-ctl.sh stop
#
# The relay shared secret is read from /opt/rcmp/relay-secret. The server's
# data directory is wiped on start, so every start is a fresh account.
set -euo pipefail
action=${1:?action}
D=/opt/rcmp
PIDF=$D/server.pid

stop_server() {
  if [ -f "$PIDF" ]; then
    p=$(cat "$PIDF")
    kill "$p" 2>/dev/null || true
    for _ in $(seq 1 50); do kill -0 "$p" 2>/dev/null || break; sleep 0.1; done
    kill -9 "$p" 2>/dev/null || true
    rm -f "$PIDF"
  fi
  pkill -x netbird-server 2>/dev/null || true
}

case "$action" in
  stop) stop_server ;;
  start)
    addr=${2:?control addr} port=${3:?mgmt port} relay=${4:?relay url}
    stop_server
    rm -rf $D/nbdata
    mkdir -p $D/nbdata $D/logs
    secret=$(cat $D/relay-secret)
    enc=$(head -c 32 /dev/urandom | base64)
    cat > $D/server.yaml <<EOF
server:
  listenAddress: ":$port"
  exposedAddress: "http://$addr:$port"
  metricsPort: 9092
  healthcheckAddress: "127.0.0.1:9002"
  logLevel: "info"
  logFile: "console"
  authSecret: "$secret"
  dataDir: "$D/nbdata"
  relays:
    addresses:
      - "$relay"
    credentialsTTL: "24h"
    secret: "$secret"
  disableAnonymousMetrics: true
  disableGeoliteUpdate: true
  auth:
    issuer: "http://$addr:$port/oauth2"
    localAuthDisabled: false
    signKeyRefreshEnabled: false
    dashboardRedirectURIs:
      - "http://$addr:$port/nb-auth"
    cliRedirectURIs:
      - "http://localhost:53000/"
  store:
    engine: "sqlite"
    encryptionKey: "$enc"
EOF
    log=$D/logs/server-$(date -u +%Y%m%dT%H%M%SZ).log
    NB_SETUP_PAT_ENABLED=true setsid nohup $D/bin/netbird-server --config $D/server.yaml \
      > "$log" 2>&1 < /dev/null &
    echo $! > "$PIDF"
    base="http://127.0.0.1:$port"
    for _ in $(seq 1 120); do
      if curl -fsS "$base/api/instance" >/dev/null 2>&1; then break; fi
      if ! kill -0 "$(cat $PIDF)" 2>/dev/null; then cat "$log" >&2; exit 1; fi
      sleep 0.5
    done
    pw=$(head -c 18 /dev/urandom | base64 | tr -d '/+=')
    pat=$(curl -fsS -X POST "$base/api/setup" -H 'Content-Type: application/json' \
      -d "{\"email\":\"owner@example.org\",\"password\":\"Aa1!$pw\",\"name\":\"owner\",\"create_pat\":true,\"pat_expire_in\":7}" |
      python3 -c 'import json,sys; print(json.load(sys.stdin)["personal_access_token"])')
    echo "$pat" > $D/pat
    chmod 600 $D/pat
    key=$(curl -fsS -X POST "$base/api/setup-keys" -H "Authorization: Token $pat" \
      -H 'Content-Type: application/json' \
      -d '{"name":"bench","type":"reusable","expires_in":604800,"auto_groups":[],"usage_limit":0,"ephemeral":false}' |
      python3 -c 'import json,sys; print(json.load(sys.stdin)["key"])')
    echo "pid=$(cat $PIDF) log=$log"
    echo "$key"
    ;;
  *) echo "unknown action $action" >&2; exit 2 ;;
esac
