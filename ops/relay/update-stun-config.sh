#!/usr/bin/env bash
# update-stun-config.sh — Update Netbird config.yaml with external STUN + relay servers
#
# Replaces the local STUN server (unreachable behind NAT) with external
# STUN servers running on the VPS relays. Optionally registers VPS-hosted
# Netbird relay servers for lower-latency data relay (vs relaying through NETHOST).
#
# Usage (from admin machine, at the top of the tree; the relay tier's addresses
# come from ops/relay-env.sh, which a deployment's ops/relay-env.d/ supplies):
#   ssh -t goat@100.127.1.23 "STUN_IPS='$(. ops/relay-env.sh; echo "${RELAY_IPS[*]}")' bash -s" < ops/relay/update-stun-config.sh
# Run in place (bash ops/relay/update-stun-config.sh), it reads ops/relay-env.sh itself.
#
# Optional env vars:
#   STUN_IPS — space-separated VPS IPs (default: RELAY_IPS of ops/relay-env.sh)
#   RELAY_IPS — space-separated VPS IPs running Netbird relay on port 33073
#   RELAY_SECRET — auth secret for relay servers (reads from existing config if not set)
#
# Idempotent — safe to re-run.

set -euo pipefail

# External STUN servers — accepts STUN_IPS env var (space-separated IPs)
# or falls back to the relay tier's IPs in ops/relay-env.sh beside this script
# (read in a subshell: its RELAY_IPS array is not this script's RELAY_IPS).
_relay_env="$(dirname "${BASH_SOURCE[0]:-.}")/../relay-env.sh"
if [ -z "${STUN_IPS:-}" ] && [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "$_relay_env" ]; then
  # shellcheck disable=SC1090
  STUN_IPS="$(. "$_relay_env"; echo "${RELAY_IPS[*]}")"
fi
if [ -n "${STUN_IPS:-}" ]; then
  STUN_SERVERS=()
  for ip in $STUN_IPS; do
    STUN_SERVERS+=("stun:${ip}:3478")
  done
else
  STUN_SERVERS=(
    "stun:203.0.113.2:3478"      # fra (region 1)
    "stun:203.0.113.20:3478"     # isr (region 2)
    "stun:203.0.113.30:3478"     # mum (region 3)
  )
fi

# Find Netbird config
CONFIG_PATH=""
if [ -f /opt/netbird/config.yaml ]; then
  CONFIG_PATH=/opt/netbird/config.yaml
fi

# Try docker volume mount
if [ -z "$CONFIG_PATH" ]; then
  MOUNT_SRC=$(docker inspect netbird-server --format '{{range .Mounts}}{{if eq .Destination "/etc/netbird"}}{{.Source}}{{end}}{{end}}' 2>/dev/null || true)
  if [ -n "$MOUNT_SRC" ] && [ -f "$MOUNT_SRC/config.yaml" ]; then
    CONFIG_PATH="$MOUNT_SRC/config.yaml"
  fi
fi

if [ -z "$CONFIG_PATH" ] || [ ! -f "$CONFIG_PATH" ]; then
  echo "ERROR: Cannot find Netbird config.yaml"
  exit 1
fi

echo "==> Updating $CONFIG_PATH"

# Backup
sudo cp "$CONFIG_PATH" "${CONFIG_PATH}.bak"

# Build STUN YAML entries
STUN_YAML=""
for uri in "${STUN_SERVERS[@]}"; do
  STUN_YAML="${STUN_YAML}    - uri: \"${uri}\"\n"
done

# Update config using python3 (handles YAML properly)
sudo python3 << PYEOF
import yaml, sys

with open("$CONFIG_PATH") as f:
    config = yaml.safe_load(f)

config["server"]["stuns"] = [
$(for uri in "${STUN_SERVERS[@]}"; do echo "    {\"uri\": \"$uri\"},"; done)
]

# Remove stunPorts since we use external STUN servers
config["server"].pop("stunPorts", None)

# Register VPS-hosted relay servers (if RELAY_IPS is set)
# Netbird expects top-level 'relays' struct with 'addresses' (list of URL strings) and 'secret'
relay_ips = "${RELAY_IPS:-}"
relay_secret = "${RELAY_SECRET:-}"
if relay_ips.strip():
    # Use provided secret, or preserve existing secret from config
    if not relay_secret.strip():
        relay_secret = config.get("relays", {}).get("secret", "") if isinstance(config.get("relays"), dict) else ""
    if not relay_secret.strip():
        relay_secret = config.get("server", {}).get("authSecret", "")
    if not relay_secret.strip():
        print("  WARNING: No relay secret available — relay config skipped")
    else:
        config["relays"] = {
            "addresses": [f"rels://{ip.strip()}:33073" for ip in relay_ips.split()],
            "secret": relay_secret
        }
        print("  Relay servers configured:")
        for addr in config["relays"]["addresses"]:
            print(f"    {addr}")

with open("$CONFIG_PATH", "w") as f:
    yaml.dump(config, f, default_flow_style=False, sort_keys=False)

print("  STUN servers configured:")
for s in config["server"]["stuns"]:
    print(f"    {s['uri']}")
PYEOF

echo "==> Restarting Netbird server..."
cd /opt/netbird && sudo docker compose restart netbird-server

echo "==> Waiting for server to be ready..."
sleep 5

if sudo docker compose ps netbird-server 2>/dev/null | grep -q "running"; then
  echo "==> Netbird server running with external STUN servers"
else
  echo "WARNING: Netbird server may not be running. Check: docker compose logs netbird-server"
fi
