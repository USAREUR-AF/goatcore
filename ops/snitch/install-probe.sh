#!/usr/bin/env bash
# Install the snitch probe on any mesh peer.
#
# Downloads the pre-built binary from the collector and installs a
# systemd service that probes every 60s and reports to the collector.
#
# Usage:
#   curl -fsSL <collector-url>/install | bash
#   # or manually:
#   bash install-probe.sh --collector URL [--interval SECONDS]   (or SNITCH_COLLECTOR=URL)
#
# Requires: root (for systemd), ping, arp, ip
# Idempotent: safe to re-run (updates binary and restarts service)

set -euo pipefail

COLLECTOR="${SNITCH_COLLECTOR:-}"
INTERVAL=60
INSTALL_DIR=/usr/local/bin
SERVICE_NAME=snitch-probe

while [[ $# -gt 0 ]]; do
  case "$1" in
    --collector) COLLECTOR="$2"; shift 2 ;;
    --interval)  INTERVAL="$2"; shift 2 ;;
    *)           echo "Unknown arg: $1" >&2; exit 1 ;;
  esac
done

if [ -z "$COLLECTOR" ]; then
  echo "snitch: no collector URL: pass --collector <url> or set SNITCH_COLLECTOR" >&2
  exit 1
fi

log() { echo "[snitch] $(date +%H:%M:%S) $*"; }

# Download binary from collector
log "Downloading snitch binary from $COLLECTOR..."
if curl -fsSL "$COLLECTOR/bin/snitch" -o /tmp/snitch; then
  chmod +x /tmp/snitch
  mv /tmp/snitch "$INSTALL_DIR/snitch"
  log "Installed $INSTALL_DIR/snitch"
else
  log "ERROR: Could not download binary from $COLLECTOR/bin/snitch"
  log "Build it locally: cd ops/snitch && go build -o snitch ./cmd/snitch"
  exit 1
fi

# Create systemd service
log "Installing systemd service..."
cat > /etc/systemd/system/${SERVICE_NAME}.service << EOF
[Unit]
Description=SNITCH mesh performance probe
After=network-online.target netbird.service
Wants=network-online.target

[Service]
Type=simple
ExecStart=$INSTALL_DIR/snitch probe --collector $COLLECTOR --interval $INTERVAL
Restart=always
RestartSec=30
Environment=SNITCH_COLLECTOR=$COLLECTOR

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now ${SERVICE_NAME}.service

log "SNITCH probe installed and running."
log "  Collector: $COLLECTOR"
log "  Interval:  ${INTERVAL}s"
log "  Logs:      journalctl -u ${SERVICE_NAME} -f"
