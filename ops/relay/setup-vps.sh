#!/usr/bin/env bash
# setup-vps.sh — Bootstrap an AWS EC2 instance as a rathole + Coturn + Netbird relay
#
# Run this on a fresh Ubuntu 24.04 EC2 instance (t3.micro is sufficient).
# It installs up to three services:
#   - rathole:       forwards TCP 443 (Netbird management) to NETHOST
#   - coturn:        local STUN server on UDP 3478 for NAT traversal
#   - netbird relay: local data relay on TCP 33073 when P2P fails (requires NB_RELAY_AUTH_SECRET)
#
# Usage:
#   ssh ubuntu@VPS_IP 'bash -s' < ops/relay/setup-vps.sh
#
# Optional env vars:
#   TURN_SECRET          — shared Coturn static-auth-secret (all VPS must use the same one).
#                          If not set, generates a new random secret.
#   NB_RELAY_AUTH_SECRET — Netbird relay auth secret (same as management server's authSecret).
#                          If set, installs Docker and runs netbirdio/relay container.
#   RELAY_DNS_NAME       — DNS name for the relay TLS cert (required if NB_RELAY_AUTH_SECRET is set).
#                          e.g., relay.legacy.example (region 1) or relay-rlb.legacy.example.
#
# After running, it prints the values you need for:
#   - rathole client config (NETHOST side)
#   - Netbird management.json TURN server registration (NETHOST side)

set -euo pipefail

RATHOLE_VERSION="v0.5.0"
RATHOLE_URL="https://github.com/rapiz1/rathole/releases/download/${RATHOLE_VERSION}/rathole-x86_64-unknown-linux-gnu.zip"
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="/etc/rathole"

echo "==> Installing dependencies"
sudo apt-get update -qq
sudo apt-get install -y -qq unzip ufw

echo "==> Downloading rathole ${RATHOLE_VERSION}"
cd /tmp
curl -fsSL -o rathole.zip "$RATHOLE_URL"
unzip -o rathole.zip -d rathole-extract
sudo install -m 755 rathole-extract/rathole "$INSTALL_DIR/rathole"
rm -rf rathole.zip rathole-extract

echo "==> Verifying rathole binary"
rathole --version

echo "==> Generating Noise keypair"
KEY_OUTPUT=$(rathole --genkey 2>&1)
PRIVATE_KEY=$(echo "$KEY_OUTPUT" | grep -A1 "Private" | tail -1 | tr -d '[:space:]')
PUBLIC_KEY=$(echo "$KEY_OUTPUT" | grep -A1 "Public" | tail -1 | tr -d '[:space:]')

echo "==> Generating auth token"
AUTH_TOKEN=$(openssl rand -hex 32)

echo "==> Detecting public IP"
VPS_PUBLIC_IP=$(curl -s4 ifconfig.me || curl -s4 icanhazip.com)

echo "==> Creating config directory"
sudo mkdir -p "$CONFIG_DIR"

echo "==> Writing server config (TCP 443 only — Coturn handles UDP 3478)"
sudo tee "$CONFIG_DIR/server.toml" > /dev/null << EOF
[server]
bind_addr = "0.0.0.0:2333"
default_token = "${AUTH_TOKEN}"
heartbeat_interval = 30

[server.transport]
type = "noise"

[server.transport.noise]
local_private_key = "${PRIVATE_KEY}"

[server.services.netbird-mgmt]
type = "tcp"
bind_addr = "0.0.0.0:443"
EOF

sudo chmod 600 "$CONFIG_DIR/server.toml"

echo "==> Creating systemd service"
sudo tee /etc/systemd/system/rathole-server.service > /dev/null << 'EOF'
[Unit]
Description=Rathole Relay Server (goat)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/rathole -s /etc/rathole/server.toml
Restart=on-failure
RestartSec=5s
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable rathole-server
sudo systemctl start rathole-server

echo "==> Installing Coturn TURN/STUN server"
sudo apt-get install -y -qq coturn

echo "==> Setting TURN shared secret"
if [ -n "${TURN_SECRET:-}" ]; then
    echo "    Using provided TURN_SECRET from environment"
else
    TURN_SECRET=$(openssl rand -hex 32)
    echo "    Generated new TURN_SECRET (pass TURN_SECRET env var to use a shared secret)"
fi

echo "==> Writing turnserver.conf"
sudo tee /etc/turnserver.conf > /dev/null << EOF
# Coturn TURN/STUN server — goat distributed relay
# Managed by setup-vps.sh

listening-port=3478
external-ip=${VPS_PUBLIC_IP}
fingerprint
use-auth-secret
static-auth-secret=${TURN_SECRET}
realm=legacy.example
no-software-attribute
no-cli
no-tls
no-dtls

# Relay port range
min-port=49152
max-port=65535

# Logging
log-file=/var/log/turnserver.log
simple-log
EOF

sudo chmod 600 /etc/turnserver.conf

echo "==> Enabling Coturn daemon"
sudo sed -i 's/^#\?TURNSERVER_ENABLED=.*/TURNSERVER_ENABLED=1/' /etc/default/coturn

sudo systemctl daemon-reload
sudo systemctl enable coturn
sudo systemctl start coturn

# ============================================================================
# Netbird Relay (optional — only installed if NB_RELAY_AUTH_SECRET is set)
# ============================================================================
if [ -n "${NB_RELAY_AUTH_SECRET:-}" ]; then
  echo "==> Installing Docker for Netbird relay"
  if ! command -v docker &>/dev/null; then
    sudo install -m 0755 -d /etc/apt/keyrings
    curl -fsSL https://download.docker.com/linux/ubuntu/gpg | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg 2>/dev/null
    sudo chmod a+r /etc/apt/keyrings/docker.gpg
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu $(lsb_release -cs) stable" | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
    sudo apt-get update -qq
    sudo apt-get install -y -qq docker-ce docker-ce-cli containerd.io > /dev/null
    sudo systemctl enable docker
  fi

  # Relay DNS name is required for TLS cert. Must be set by caller.
  # region 1 uses relay.legacy.example, others use relay-{name}.legacy.example.
  if [ -z "${RELAY_DNS_NAME:-}" ]; then
    echo "ERROR: RELAY_DNS_NAME must be set (e.g., relay.legacy.example)"
    exit 1
  fi

  # TLS: use certbot (HTTP-01 on port 80), NOT the relay binary's built-in
  # Let's Encrypt. The built-in client uses TLS-ALPN-01 on port 443 (rathole
  # occupies 443) and requires --letsencrypt-data-dir (silently does nothing
  # without it). Certbot is reliable and auto-renews via systemd timer.
  echo "==> Installing certbot for relay TLS"
  sudo apt-get install -y -qq certbot

  if [ ! -f "/etc/letsencrypt/live/${RELAY_DNS_NAME}/fullchain.pem" ]; then
    echo "==> Obtaining TLS cert via certbot HTTP-01"
    # Stop relay container if running (frees port 80)
    sudo docker stop netbird-relay 2>/dev/null || true
    sudo docker rm -f netbird-relay 2>/dev/null || true
    sudo certbot certonly --standalone --preferred-challenges http \
      -d "${RELAY_DNS_NAME}" \
      --email person-28@example.org \
      --agree-tos --non-interactive --force-renewal
  fi

  # Verify cert exists
  if [ ! -f "/etc/letsencrypt/live/${RELAY_DNS_NAME}/fullchain.pem" ]; then
    echo "ERROR: certbot failed — cert not found at /etc/letsencrypt/live/${RELAY_DNS_NAME}/fullchain.pem"
    exit 1
  fi

  echo "==> Starting Netbird relay container (with TLS cert from certbot)"
  sudo docker rm -f netbird-relay 2>/dev/null || true
  # Mount the full /etc/letsencrypt dir, not just live/, because cert files
  # under live/ are symlinks to ../../archive/. Mounting only live/ breaks
  # symlink resolution.
  sudo docker run -d \
    --name netbird-relay \
    --restart unless-stopped \
    -e NB_LOG_LEVEL=info \
    -e NB_LISTEN_ADDRESS=:33073 \
    -e "NB_EXPOSED_ADDRESS=rels://${RELAY_DNS_NAME}:33073" \
    -e "NB_AUTH_SECRET=${NB_RELAY_AUTH_SECRET}" \
    -p 33073:33073/tcp \
    -p 33073:33073/udp \
    -v /etc/letsencrypt:/etc/letsencrypt:ro \
    netbirdio/relay:0.82.0@sha256:8f6b9e4193653ede89e83dd4aa4e2cf2562579b3e2481dcf259a7ab28bcd5fa1 \
    --tls-cert-file "/etc/letsencrypt/live/${RELAY_DNS_NAME}/fullchain.pem" \
    --tls-key-file "/etc/letsencrypt/live/${RELAY_DNS_NAME}/privkey.pem"

  echo "==> Netbird relay running on port 33073 (TLS via certbot)"
else
  echo "==> Skipping Netbird relay (NB_RELAY_AUTH_SECRET not set)"
fi

echo "==> Configuring UFW (reset for idempotency)"
sudo ufw --force reset
sudo ufw allow 22/tcp      # SSH
sudo ufw allow 80/tcp      # ACME HTTP-01 (certbot for relay TLS)
sudo ufw allow 443/tcp     # Netbird management (rathole)
sudo ufw allow 3478/udp    # Coturn STUN/TURN
sudo ufw allow 2333/tcp    # rathole control channel
sudo ufw allow 33073/tcp   # Netbird relay (data relay)
sudo ufw allow 49152:65535/udp  # Coturn relay ports
sudo ufw --force enable

echo "==> Configuring iptables rate limiting (DDoS mitigation)"
# Limit new TCP connections on 443 to 20/sec per source IP (burst 40).
# Enrollment is infrequent — this stops SYN floods without affecting legitimate peers.
# Runs before UFW rules via a separate chain.
sudo iptables -N RATE_LIMIT 2>/dev/null || sudo iptables -F RATE_LIMIT
sudo iptables -A RATE_LIMIT -p tcp --dport 443 -m conntrack --ctstate NEW -m hashlimit \
  --hashlimit-above 20/sec --hashlimit-burst 40 --hashlimit-mode srcip \
  --hashlimit-name tcp443 -j DROP
sudo iptables -A RATE_LIMIT -p udp --dport 3478 -m hashlimit \
  --hashlimit-above 50/sec --hashlimit-burst 100 --hashlimit-mode srcip \
  --hashlimit-name udp3478 -j DROP
# Insert at top of INPUT chain (idempotent — remove first if exists)
sudo iptables -D INPUT -j RATE_LIMIT 2>/dev/null || true
sudo iptables -I INPUT 1 -j RATE_LIMIT

# Persist iptables rules across reboots
sudo apt-get install -y -qq iptables-persistent > /dev/null 2>&1 || true
sudo netfilter-persistent save 2>/dev/null || true

echo "==> Verifying service status"
sudo systemctl status rathole-server --no-pager
echo ""
sudo systemctl status coturn --no-pager

echo ""
echo "========================================"
echo "  VPS RELAY SETUP COMPLETE"
echo "========================================"
echo ""
echo "Rathole (management forwarding):"
echo "  remote_addr        = \"${VPS_PUBLIC_IP}:2333\""
echo "  default_token      = \"${AUTH_TOKEN}\""
echo "  remote_public_key  = \"${PUBLIC_KEY}\""
echo ""
echo "Coturn (STUN):"
echo "  stun_url           = stun:${VPS_PUBLIC_IP}:3478"
echo "  static_auth_secret = ${TURN_SECRET}"
echo "  realm              = legacy.example"
echo ""
echo "Netbird Relay:"
if [ -n "${NB_RELAY_AUTH_SECRET:-}" ]; then
  echo "  relay_url          = rels://${RELAY_DNS_NAME}:33073"
  echo "  tls                = certbot (HTTP-01, auto-renews via systemd timer)"
  echo "  status             = running"
else
  echo "  status             = not installed (pass NB_RELAY_AUTH_SECRET + RELAY_DNS_NAME to enable)"
fi
echo ""
echo "Store these in .envrc.local on your admin machine:"
echo ""
echo "  export VPS_RELAY_IP=${VPS_PUBLIC_IP}"
echo "  export VPS_RELAY_TOKEN=${AUTH_TOKEN}"
echo "  export VPS_RELAY_PUBKEY=${PUBLIC_KEY}"
echo "  export VPS_TURN_SECRET=${TURN_SECRET}"
echo ""
echo "TURN server registration (when NETHOST is available):"
echo "  Add to Netbird management.json TURNConfig.Turns:"
echo "    { \"Proto\": \"udp\", \"URI\": \"turn:${VPS_PUBLIC_IP}:3478\", \"Username\": \"\", \"Password\": \"\" }"
echo "  Set TURNConfig.Secret to: ${TURN_SECRET}"
echo ""
echo "Next step: run ops/relay/setup-client.sh on the NETHOST."
echo "========================================"
