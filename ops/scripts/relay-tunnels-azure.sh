#!/usr/bin/env bash
# relay-tunnels-azure.sh — open, list or close the Azure Bastion tunnels that
# Ansible uses to reach an Azure relay tier.
#
#   ops/scripts/relay-tunnels-azure.sh <env> up|status|pin|down
#
# pin records each relay's SSH host key in $RELAY_KNOWN_HOSTS (written by the
# generator; the inventory checks keys against it). It reads the key only through
# a tunnel this script opened and is still running, so the far end is the VM
# Bastion reached inside Azure. It refuses to replace a pinned key that changed.
#
# Reads ops/relay-env-<env>.sh, written by
# ops/ansible/inventory/generate-relays-azure.sh. For relay i (in name order)
# it runs, in the background:
#
#   az network bastion tunnel --name <bastion> --resource-group <rg> \
#     --target-resource-id <vm id> --resource-port 22 --port <base port + i>
#
# The tunnels carry SSH only; authentication is still the relay's key, and
# Bastion logs each session against the operator's Entra ID identity. Needs
# Bastion Standard with tunnelling, and Reader on the VM, its NIC and the Bastion.
#
# State: pid and log per relay under ${XDG_STATE_HOME:-~/.local/state}/goat/tunnels/<env>/.
# Exit codes: 64 usage · 65 missing relay-env file · 66 a tunnel did not come up
#   (or, for pin, is not up) · 67 a relay's host key differs from the pinned one.
# Bash 3.2 portable.

set -euo pipefail

if [ $# -ne 2 ]; then
  echo "Usage: $0 <env> up|status|pin|down" >&2
  exit 64
fi
ENV_NAME="$1"
ACTION="$2"

REPO_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
RELAY_ENV_FILE="$REPO_DIR/ops/relay-env-${ENV_NAME}.sh"
if [ ! -f "$RELAY_ENV_FILE" ]; then
  echo "ERROR: $RELAY_ENV_FILE not found. Run ops/ansible/inventory/generate-relays-azure.sh $ENV_NAME <terraform_dir> first." >&2
  exit 65
fi
# shellcheck source=/dev/null
source "$RELAY_ENV_FILE"
if [ "${RELAY_CLOUD:-}" != "azure" ]; then
  echo "ERROR: $RELAY_ENV_FILE is not an Azure relay tier (RELAY_CLOUD=${RELAY_CLOUD:-unset})." >&2
  exit 65
fi

STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/goat/tunnels/${ENV_NAME}"
UP_TIMEOUT="${RELAY_TUNNEL_UP_TIMEOUT:-60}"

port_open() {
  python3 -c 'import socket,sys; s=socket.socket(); s.settimeout(1); sys.exit(0 if s.connect_ex(("127.0.0.1", int(sys.argv[1]))) == 0 else 1)' "$1"
}

pid_alive() {
  [ -f "$1" ] && kill -0 "$(cat "$1")" 2>/dev/null
}

# `az` is a shell wrapper around Python, so the recorded pid is the wrapper's;
# the process holding the port is its child. Stop the whole tree.
kill_tree() {
  local child
  for child in $(pgrep -P "$1" 2>/dev/null); do
    kill_tree "$child"
  done
  kill "$1" 2>/dev/null || true
}

i=0
case "$ACTION" in
  up)
    mkdir -p "$STATE_DIR"
    chmod 700 "$STATE_DIR"
    failed=0
    for name in "${RELAY_NAMES[@]}"; do
      port=$((RELAY_TUNNEL_BASE_PORT + i))
      vm_id="${RELAY_VM_IDS[$i]}"
      pid_file="$STATE_DIR/${name}.pid"
      log_file="$STATE_DIR/${name}.log"
      i=$((i + 1))
      if pid_alive "$pid_file" && port_open "$port"; then
        echo "relay-$name: already up on 127.0.0.1:$port"
        continue
      fi
      if port_open "$port"; then
        echo "ERROR: relay-$name: port $port is in use by something else" >&2
        failed=1
        continue
      fi
      az network bastion tunnel --name "$BASTION_NAME" --resource-group "$BASTION_RESOURCE_GROUP" \
        --target-resource-id "$vm_id" --resource-port 22 --port "$port" > "$log_file" 2>&1 &
      echo $! > "$pid_file"
      waited=0
      until port_open "$port"; do
        if ! pid_alive "$pid_file" || [ "$waited" -ge "$UP_TIMEOUT" ]; then
          echo "ERROR: relay-$name: tunnel did not come up; last lines of $log_file:" >&2
          tail -5 "$log_file" >&2 || true
          failed=1
          break
        fi
        sleep 1
        waited=$((waited + 1))
      done
      port_open "$port" && echo "relay-$name: up on 127.0.0.1:$port"
    done
    [ "$failed" -eq 0 ] || exit 66
    ;;
  status)
    for name in "${RELAY_NAMES[@]}"; do
      port=$((RELAY_TUNNEL_BASE_PORT + i))
      i=$((i + 1))
      if pid_alive "$STATE_DIR/${name}.pid" && port_open "$port"; then
        echo "relay-$name: up on 127.0.0.1:$port"
      else
        echo "relay-$name: down"
      fi
    done
    ;;
  pin)
    known_hosts="${RELAY_KNOWN_HOSTS:?relay-env file has no RELAY_KNOWN_HOSTS; re-run the generator}"
    touch "$known_hosts"
    failed=0
    for name in "${RELAY_NAMES[@]}"; do
      port=$((RELAY_TUNNEL_BASE_PORT + i))
      alias="relay-${ENV_NAME}-${name}"
      i=$((i + 1))
      if ! { pid_alive "$STATE_DIR/${name}.pid" && port_open "$port"; }; then
        echo "ERROR: relay-$name: no tunnel of ours on 127.0.0.1:$port; run '$0 $ENV_NAME up' first" >&2
        exit 66
      fi
      seen=$(ssh-keyscan -T 10 -t ed25519 -p "$port" 127.0.0.1 2>/dev/null | awk '!/^#/ && NF >= 3 {print $2" "$3; exit}')
      [ -n "$seen" ] || { echo "ERROR: relay-$name: no ed25519 host key read on :$port" >&2; exit 66; }
      pinned=$(awk -v a="$alias" '$1 == a {print $2" "$3; exit}' "$known_hosts")
      if [ -z "$pinned" ]; then
        echo "$alias $seen" >> "$known_hosts"
        echo "relay-$name: pinned $(echo "$seen" | cut -d' ' -f1) key"
      elif [ "$pinned" = "$seen" ]; then
        echo "relay-$name: key matches the pinned one"
      else
        echo "ERROR: relay-$name: host key differs from the one pinned in $known_hosts. Not replaced." >&2
        failed=1
      fi
    done
    [ "$failed" -eq 0 ] || exit 67
    ;;
  down)
    for name in "${RELAY_NAMES[@]}"; do
      pid_file="$STATE_DIR/${name}.pid"
      if pid_alive "$pid_file"; then
        kill_tree "$(cat "$pid_file")"
        echo "relay-$name: closed"
      fi
      rm -f "$pid_file"
    done
    ;;
  *)
    echo "Usage: $0 <env> up|status|pin|down" >&2
    exit 64
    ;;
esac
