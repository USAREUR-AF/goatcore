#!/usr/bin/env bash
# setup-client.sh — Install/update rathole client on the NETHOST infrastructure node
#
# Supports multiple relays. Each relay gets its own config file and systemd
# service instance via a template unit (rathole-client@<name>.service).
#
# Required env vars:
#   RELAY_NAME        — short name (e.g., "fra", "isr", "mum")
#   VPS_RELAY_IP      — VPS public IP address
#   VPS_RELAY_TOKEN   — shared auth token
#   VPS_RELAY_PUBKEY  — server's Noise public key
#
# Usage (from admin machine):
#   ssh -t goat@100.127.1.23 \
#     RELAY_NAME=isr \
#     VPS_RELAY_IP=203.0.113.20 \
#     VPS_RELAY_TOKEN=abc123 \
#     VPS_RELAY_PUBKEY=xyz789 \
#     'bash -s' < ops/relay/setup-client.sh
#
# Idempotent — safe to re-run. Overwrites config, restarts service.
# On first run for any relay, also installs rathole binary and template unit.
# Migrates from legacy single-relay config (client.toml / rathole-client.service)
# to the multi-relay model automatically.

set -euo pipefail

RATHOLE_VERSION="v0.5.0"
RATHOLE_URL="https://github.com/rapiz1/rathole/releases/download/${RATHOLE_VERSION}/rathole-x86_64-unknown-linux-gnu.zip"
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="/etc/rathole"

# Validate required env vars
for var in RELAY_NAME VPS_RELAY_IP VPS_RELAY_TOKEN VPS_RELAY_PUBKEY; do
    if [ -z "${!var:-}" ]; then
        echo "ERROR: $var is not set."
        echo "Usage: RELAY_NAME=isr VPS_RELAY_IP=x.x.x.x VPS_RELAY_TOKEN=abc VPS_RELAY_PUBKEY=xyz bash setup-client.sh"
        exit 1
    fi
done

NAME="$RELAY_NAME"

# -- Install rathole binary (idempotent) --
if ! command -v rathole &>/dev/null; then
    echo "==> Installing dependencies"
    sudo apt-get update -qq && sudo apt-get install -y -qq unzip > /dev/null

    echo "==> Installing rathole ${RATHOLE_VERSION}"
    cd /tmp
    curl -fsSL -o rathole.zip "$RATHOLE_URL"
    unzip -o rathole.zip -d rathole-extract
    sudo install -m 755 rathole-extract/rathole "$INSTALL_DIR/rathole"
    rm -rf rathole.zip rathole-extract
else
    echo "==> rathole already installed: $(rathole --version 2>&1 | head -1)"
fi

# -- Create config directory --
sudo mkdir -p "$CONFIG_DIR"

# -- Migrate legacy single-relay config --
if [ -f "$CONFIG_DIR/client.toml" ] && ! ls "$CONFIG_DIR"/client-*.toml &>/dev/null; then
    echo "==> Migrating legacy client.toml to client-legacy.toml"
    sudo mv "$CONFIG_DIR/client.toml" "$CONFIG_DIR/client-legacy.toml"
    # Stop and disable legacy service
    sudo systemctl stop rathole-client.service 2>/dev/null || true
    sudo systemctl disable rathole-client.service 2>/dev/null || true
fi

# -- Write per-relay client config --
echo "==> Writing client config: client-${NAME}.toml"
sudo tee "$CONFIG_DIR/client-${NAME}.toml" > /dev/null << EOF
[client]
remote_addr = "${VPS_RELAY_IP}:2333"
default_token = "${VPS_RELAY_TOKEN}"
heartbeat_timeout = 40
retry_interval = 1

[client.transport]
type = "noise"

[client.transport.noise]
remote_public_key = "${VPS_RELAY_PUBKEY}"

[client.services.netbird-mgmt]
type = "tcp"
local_addr = "127.0.0.1:443"
EOF

sudo chmod 600 "$CONFIG_DIR/client-${NAME}.toml"

# -- Create systemd template unit (idempotent) --
echo "==> Installing systemd template unit"
sudo tee /etc/systemd/system/rathole-client@.service > /dev/null << 'EOF'
[Unit]
Description=Rathole Relay Client — %i (goat)
After=network-online.target docker.service
Wants=network-online.target
Requires=docker.service

[Service]
Type=simple
ExecStart=/usr/local/bin/rathole -c /etc/rathole/client-%i.toml
Restart=on-failure
RestartSec=5s
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable "rathole-client@${NAME}.service"
sudo systemctl restart "rathole-client@${NAME}.service"

echo "==> Verifying service status"
sleep 2
sudo systemctl status "rathole-client@${NAME}.service" --no-pager

echo ""
echo "==> Testing connectivity to VPS (${NAME})"
if nc -z -w 5 "$VPS_RELAY_IP" 2333 2>/dev/null; then
    echo "  OK: VPS control channel reachable at ${VPS_RELAY_IP}:2333"
else
    echo "  FAIL: Cannot reach ${VPS_RELAY_IP}:2333 — check VPS UFW and AWS security group"
fi

echo ""
echo "========================================"
echo "  RELAY CLIENT ${NAME} CONFIGURED"
echo "========================================"
echo ""
echo "Active relay clients:"
sudo systemctl list-units 'rathole-client@*' --no-pager --no-legend 2>/dev/null || true
echo ""
echo "Test: curl -sk --resolve netbird.legacy.example:443:${VPS_RELAY_IP} https://netbird.legacy.example/api/accounts"
echo "Expected: {\"message\":\"no valid authentication provided\",\"code\":401}"
echo "========================================"
