#!/usr/bin/env bash
set -euo pipefail

# goat — Backup NETHOST state to 1Password
#
# Backs up Netbird management DB, Docker Compose config, Cloudflare tunnel
# credentials, and system config from the NETHOST to a local snapshot, then
# uploads it to 1Password.
#
# Prerequisites:
#   - SSH key auth to NETHOST (passwordless)
#   - 1Password CLI (`op`) installed and signed in
#   - .envrc sourced (NETHOST_HOST, NETHOST_USER, NETBIRD_DOMAIN)
#
# A deployment that exports other names for these keeps them through its
# ops/env-aliases.d/*.sh (ops/scripts/lib/env-aliases.sh), which may also set
#   GOAT_DEPLOYMENT  first word of the backup's title, <name>-backup-<host>-<time>
#                    (default goat; ops/restore.sh lists the same titles)
#   NETHOST_NAME     the host name used when `hostname` cannot be read (default nethost)
#
# Usage: bash ops/backup.sh

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
SNAPSHOT_DIR="$SCRIPT_DIR/snapshots"

# Source credentials
if [ -f "$REPO_DIR/.envrc" ]; then
  set -a
  source "$REPO_DIR/.envrc" 2>/dev/null || true
  [ -f "$REPO_DIR/.envrc.local" ] && source "$REPO_DIR/.envrc.local" 2>/dev/null || true
  set +a
fi
# shellcheck source=scripts/lib/env-aliases.sh
. "$REPO_DIR/ops/scripts/lib/env-aliases.sh"

: "${NETHOST_HOST:?Set NETHOST_HOST}"
: "${NETHOST_USER:?Set NETHOST_USER}"
: "${NETBIRD_DOMAIN:?Set NETBIRD_DOMAIN}"

# SSH command — use key auth if available, fall back to sshpass
SSH_CMD="ssh -o StrictHostKeyChecking=no -o ConnectTimeout=10 $NETHOST_USER@$NETHOST_HOST"
SCP_CMD="scp -o StrictHostKeyChecking=no"
if ! ssh -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o PasswordAuthentication=no "$NETHOST_USER@$NETHOST_HOST" "echo ok" >/dev/null 2>&1; then
  : "${NETHOST_PASS:?SSH key auth failed and NETHOST_PASS not set}"
  SSH_CMD="sshpass -p '$NETHOST_PASS' ssh -o StrictHostKeyChecking=no -o ConnectTimeout=10 $NETHOST_USER@$NETHOST_HOST"
  SCP_CMD="sshpass -p '$NETHOST_PASS' scp -o StrictHostKeyChecking=no"
fi

run_remote() {
  eval "$SSH_CMD '$1'"
}

TIMESTAMP=$(date +%Y%m%d-%H%M%S)
HOSTNAME_REMOTE=$(run_remote "hostname" 2>/dev/null || echo "${NETHOST_NAME:-nethost}")
SNAPSHOT_NAME="${GOAT_DEPLOYMENT:-goat}-backup-${HOSTNAME_REMOTE}-${TIMESTAMP}"
SNAPSHOT_PATH="$SNAPSHOT_DIR/$SNAPSHOT_NAME"

echo "============================================"
echo "  goat — NETHOST Backup"
echo "============================================"
echo "  Host:     $NETHOST_USER@$NETHOST_HOST"
echo "  Snapshot: $SNAPSHOT_NAME"
echo ""

# --------------------------------------------------
# Step 1: Verify SSH connectivity
# --------------------------------------------------
echo "[1/4] Verifying SSH connectivity..."
if ! run_remote "echo ok" >/dev/null 2>&1; then
  echo "  ERROR: Cannot SSH to $NETHOST_USER@$NETHOST_HOST"
  echo "  Ensure SSH key auth is configured and NETHOST is reachable."
  exit 1
fi
echo "  Connected."

# --------------------------------------------------
# Step 2: Create snapshot on HOST
# --------------------------------------------------
echo "[2/4] Creating snapshot on HOST..."
REMOTE_TMP="/tmp/$SNAPSHOT_NAME"
run_remote "mkdir -p $REMOTE_TMP"

# Use sudo -S to accept password on stdin for non-NOPASSWD hosts
SUDO_PREFIX=""
if [ -n "${NETHOST_PASS:-}" ]; then
  SUDO_PREFIX="echo $NETHOST_PASS | sudo -S"
else
  SUDO_PREFIX="sudo"
fi

run_remote "$SUDO_PREFIX tar czf $REMOTE_TMP/netbird.tar.gz -C / opt/netbird 2>/dev/null || echo WARN:netbird-tar-failed"
run_remote "$SUDO_PREFIX tar czf $REMOTE_TMP/cloudflared.tar.gz -C / etc/cloudflared 2>/dev/null || echo WARN:cloudflared-tar-failed"
run_remote "$SUDO_PREFIX tar czf $REMOTE_TMP/cloudflared-creds.tar.gz -C / root/.cloudflared 2>/dev/null || echo WARN:creds-tar-failed"
run_remote "$SUDO_PREFIX cp /etc/hosts $REMOTE_TMP/hosts 2>/dev/null || true"
run_remote "cp /etc/hostname $REMOTE_TMP/hostname 2>/dev/null || true"
run_remote "ip addr show > $REMOTE_TMP/ip-addrs.txt 2>/dev/null || true"
run_remote "$SUDO_PREFIX netbird status > $REMOTE_TMP/netbird-status.txt 2>/dev/null || true"
run_remote "$SUDO_PREFIX docker compose -f /opt/netbird/docker-compose.yml ps > $REMOTE_TMP/containers.txt 2>/dev/null || true"
echo "  Snapshot created on HOST."

# --------------------------------------------------
# Step 3: Copy to LOCAL
# --------------------------------------------------
echo "[3/4] Copying snapshot to LOCAL..."
mkdir -p "$SNAPSHOT_DIR"
eval "$SCP_CMD -r $NETHOST_USER@$NETHOST_HOST:$REMOTE_TMP $SNAPSHOT_PATH"

# Create a single tarball for 1Password upload
TARBALL="$SNAPSHOT_DIR/${SNAPSHOT_NAME}.tar.gz"
tar czf "$TARBALL" -C "$SNAPSHOT_DIR" "$SNAPSHOT_NAME"

# Cleanup remote
run_remote "rm -rf $REMOTE_TMP"

echo "  Snapshot saved: $SNAPSHOT_PATH/"
echo "  Tarball: $TARBALL"

# --------------------------------------------------
# Step 4: Upload to 1Password
# --------------------------------------------------
echo "[4/4] Uploading to 1Password..."

if ! command -v op >/dev/null 2>&1; then
  echo "  WARNING: 1Password CLI (op) not installed. Skipping upload."
  echo "  Install: brew install 1password-cli"
  echo "  Manual upload: op document create '$TARBALL' --title '$SNAPSHOT_NAME' --vault Private"
  exit 0
fi

# Check if signed in
if ! op account get >/dev/null 2>&1; then
  echo "  1Password not signed in. Attempting sign-in..."
  if [ -n "${OP_SERVICE_ACCOUNT_TOKEN:-}" ]; then
    echo "  Using service account token."
  else
    echo "  Run: eval \$(op signin)"
    echo "  Then re-run this script."
    echo ""
    echo "  Or upload manually:"
    echo "    op document create '$TARBALL' --title '$SNAPSHOT_NAME' --vault Private"
    exit 0
  fi
fi

# Upload as a document
ITEM_TITLE="$SNAPSHOT_NAME"
if op document create "$TARBALL" --title "$ITEM_TITLE" --vault Private 2>/dev/null; then
  echo "  Uploaded to 1Password: $ITEM_TITLE (vault: Private)"
else
  echo "  Upload failed. Try manually:"
  echo "    op document create '$TARBALL' --title '$SNAPSHOT_NAME' --vault Private"
fi

echo ""
echo "============================================"
echo "  Backup complete"
echo "============================================"
echo ""
echo "  Local snapshot: $SNAPSHOT_PATH/"
echo "  Tarball:        $TARBALL"
echo ""
echo "  To restore:"
echo "    1. op document get '$ITEM_TITLE' --out-file restore.tar.gz"
echo "    2. tar xzf restore.tar.gz"
echo "    3. scp netbird.tar.gz to HOST, extract to /opt/netbird"
echo "    4. scp cloudflared.tar.gz to HOST, extract to /etc/cloudflared"
echo "    5. scp cloudflared-creds.tar.gz to HOST, extract to /root/.cloudflared"
echo "    6. Restart services: sudo systemctl restart cloudflared && cd /opt/netbird && sudo docker compose up -d"
