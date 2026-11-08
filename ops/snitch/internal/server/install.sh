#!/usr/bin/env bash
# Install the snitch probe on any mesh peer.
#
# Usage (from any mesh peer):
#   curl -fsSL -H "Authorization: Bearer $OPERATOR_KEY" \
#     https://<collector>/install | sudo OPERATOR_KEY="$OPERATOR_KEY" bash
#
# OPERATOR_KEY is forwarded to the inner /bin/snitch download via the
# Authorization header (Block 50E.2 — query-string auth removed; bearer
# header only).
#
# Requires: root (for systemd), ping, arp/ip
# Idempotent: safe to re-run (updates binary and restarts service)

set -euo pipefail

# The collector fills in its own URL when it serves this script.
COLLECTOR="${SNITCH_COLLECTOR:-__SNITCH_PUBLIC_URL__}"
PROBE_KEY="${SNITCH_PROBE_KEY:-}"
# OPERATOR_KEY authenticates the binary download (operator-role).
# Distinct from PROBE_KEY which the installed daemon uses to POST reports.
OPERATOR_KEY="${OPERATOR_KEY:-${SNITCH_OPERATOR_KEY:-}}"
INTERVAL=60
INSTALL_DIR=/usr/local/bin
SERVICE_NAME=snitch-probe

while [[ $# -gt 0 ]]; do
  case "$1" in
    --collector)  COLLECTOR="$2"; shift 2 ;;
    --interval)   INTERVAL="$2"; shift 2 ;;
    --probe-key)  PROBE_KEY="$2"; shift 2 ;;
    *)            echo "Unknown arg: $1" >&2; exit 1 ;;
  esac
done

if [ -z "$COLLECTOR" ]; then
  echo "snitch: no collector URL: pass --collector <url> or set SNITCH_COLLECTOR" >&2
  exit 1
fi

log() { echo "[snitch] $(date +%H:%M:%S) $*"; }

if [ "$(id -u)" -ne 0 ]; then
  log "ERROR: Must run as root (sudo)"
  exit 1
fi

if [ -z "$PROBE_KEY" ]; then
  log "WARNING: No probe key provided. Set SNITCH_PROBE_KEY or use --probe-key."
  log "The probe will not be able to authenticate with the collector."
fi

# Download binary from collector. The /bin/snitch endpoint is operator-role
# protected (Block 50E.2 — bearer header only, no URL-query auth).
log "Downloading snitch binary from $COLLECTOR..."
if [ -z "$OPERATOR_KEY" ]; then
  log "ERROR: OPERATOR_KEY (or SNITCH_OPERATOR_KEY) is required to download the binary."
  log "Set it before piping to bash: OPERATOR_KEY=... sudo bash"
  exit 1
fi
if curl -fsSL -H "Authorization: Bearer $OPERATOR_KEY" "$COLLECTOR/bin/snitch" -o /tmp/snitch; then
  chmod +x /tmp/snitch
  mv /tmp/snitch "$INSTALL_DIR/snitch"
  log "Installed $INSTALL_DIR/snitch"
else
  log "ERROR: Could not download binary. Is the collector running and OPERATOR_KEY valid?"
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
Environment=SNITCH_PROBE_KEY=$PROBE_KEY

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now ${SERVICE_NAME}.service

log "SNITCH probe installed and running."
log "  Collector: $COLLECTOR"
log "  Interval:  ${INTERVAL}s"
log "  Auth:      $([ -n "$PROBE_KEY" ] && echo 'configured' || echo 'NONE')"
log "  Status:    systemctl status ${SERVICE_NAME}"
log "  Logs:      journalctl -u ${SERVICE_NAME} -f"
