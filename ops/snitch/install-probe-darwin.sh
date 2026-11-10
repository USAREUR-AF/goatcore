#!/usr/bin/env bash
# Install the snitch probe on a macOS host (Block 61H — multi-platform builds).
#
# Downloads the darwin/<arch> binary from the collector and installs a
# launchd LaunchDaemon that probes every 60s and reports to the collector.
#
# Usage:
#   curl -fsSL -H "Authorization: Bearer $OPERATOR_KEY" \
#     "$COLLECTOR/install?os=darwin" | sudo OPERATOR_KEY="$OPERATOR_KEY" bash
#   # or manually:
#   sudo OPERATOR_KEY=... bash install-probe-darwin.sh [--collector URL] [--interval SECONDS]
#
# Requires: root (for /Library/LaunchDaemons + /usr/local/bin), curl, ping.
# Idempotent: safe to re-run (updates binary and reloads service).
#
# launchd vs systemd: launchd uses a plist file at
# /Library/LaunchDaemons/net.goat.snitch-probe.plist plus
# `launchctl load` to register. Logs land in /var/log/snitch-probe.log
# (configured in the plist via StandardOutPath / StandardErrorPath).

set -euo pipefail

COLLECTOR="${SNITCH_COLLECTOR:-}"
PROBE_KEY="${SNITCH_PROBE_KEY:-}"
OPERATOR_KEY="${OPERATOR_KEY:-${SNITCH_OPERATOR_KEY:-}}"
INTERVAL=60
INSTALL_DIR=/usr/local/bin
SERVICE_LABEL="net.goat.snitch-probe"
PLIST_PATH="/Library/LaunchDaemons/${SERVICE_LABEL}.plist"
LOG_DIR=/var/log

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

log() { echo "[snitch-darwin] $(date +%H:%M:%S) $*"; }

if [ "$(id -u)" -ne 0 ]; then
  log "ERROR: Must run as root (sudo)"
  exit 1
fi

if [ "$(uname -s)" != "Darwin" ]; then
  log "ERROR: This installer is macOS-only. For Linux, use install-probe.sh; for Windows, install-probe-windows.ps1."
  exit 1
fi

if [ -z "$OPERATOR_KEY" ]; then
  log "ERROR: OPERATOR_KEY (or SNITCH_OPERATOR_KEY) is required to download the binary."
  exit 1
fi

# Detect architecture for the binary download.
case "$(uname -m)" in
  x86_64) ARCH="amd64" ;;
  arm64)  ARCH="arm64" ;;
  *) log "ERROR: Unsupported macOS architecture: $(uname -m)"; exit 1 ;;
esac

log "Downloading snitch binary for darwin/${ARCH}..."
TMP=$(mktemp /tmp/snitch.XXXXXX)
trap 'rm -f "$TMP"' EXIT
if ! curl -fsSL -H "Authorization: Bearer $OPERATOR_KEY" \
  "$COLLECTOR/bin/snitch?os=darwin&arch=${ARCH}" -o "$TMP"; then
  log "ERROR: Could not download binary. Verify OPERATOR_KEY and that collector has darwin/${ARCH} built."
  exit 1
fi
chmod +x "$TMP"
mv "$TMP" "$INSTALL_DIR/snitch"
trap - EXIT
log "Installed $INSTALL_DIR/snitch"

# Pre-flight self-test (Block 61H). Verifies the binary actually runs on
# this machine before we register the service. Non-fatal — operators can
# review the JSON and decide whether to proceed.
log "Running pre-flight self-test (10s)..."
"$INSTALL_DIR/snitch" self-test --timeout 10s --pretty=false || \
  log "WARNING: self-test reported failure — see output above. Service will start anyway."

# Render the launchd plist. RunAtLoad=true + KeepAlive=true means
# launchd auto-restarts the probe if it exits (matches systemd Restart=always).
cat > "$PLIST_PATH" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${SERVICE_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${INSTALL_DIR}/snitch</string>
    <string>probe</string>
    <string>--collector</string>
    <string>${COLLECTOR}</string>
    <string>--interval</string>
    <string>${INTERVAL}</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>SNITCH_COLLECTOR</key>
    <string>${COLLECTOR}</string>
    <key>SNITCH_PROBE_KEY</key>
    <string>${PROBE_KEY}</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ThrottleInterval</key>
  <integer>30</integer>
  <key>StandardOutPath</key>
  <string>${LOG_DIR}/snitch-probe.log</string>
  <key>StandardErrorPath</key>
  <string>${LOG_DIR}/snitch-probe.log</string>
</dict>
</plist>
EOF
chmod 0644 "$PLIST_PATH"
log "Wrote $PLIST_PATH"

# Reload service. launchctl bootout + bootstrap is the modern cycle;
# fall back to load -w / unload for older macOS.
if launchctl print "system/${SERVICE_LABEL}" >/dev/null 2>&1; then
  launchctl bootout "system/${SERVICE_LABEL}" 2>/dev/null || true
fi
if ! launchctl bootstrap system "$PLIST_PATH" 2>/dev/null; then
  # Fallback for older macOS that doesn't support bootstrap subcommand.
  launchctl unload "$PLIST_PATH" 2>/dev/null || true
  launchctl load -w "$PLIST_PATH"
fi

log "snitch-probe installed and running on macOS."
log "  Collector: $COLLECTOR"
log "  Interval:  ${INTERVAL}s"
log "  Auth:      $([ -n "$PROBE_KEY" ] && echo 'configured' || echo 'NONE')"
log "  Logs:      tail -f ${LOG_DIR}/snitch-probe.log"
log "  Stop:      sudo launchctl bootout system/${SERVICE_LABEL}"
log "  Verify:    snitch self-test"
