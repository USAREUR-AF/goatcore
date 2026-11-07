#!/usr/bin/env bash
set -euo pipefail

# goat — Restore NETHOST state from 1Password backup
#
# Downloads a backup from 1Password to LOCAL, then SCPs it to the NETHOST
# and restores Netbird management, Cloudflare tunnel creds, and configs.
#
# 1Password CLI (op) runs on LOCAL only — never installed on the NETHOST.
#
# Flow: 1Password → LOCAL disk → SCP → NETHOST → extract → restart
#
# Prerequisites:
#   - 1Password CLI (`op`) signed in on LOCAL
#   - SSH access to NETHOST (key or password via .envrc.local)
#   - .envrc sourced (NETHOST_HOST, NETHOST_USER)
#
# A deployment that exports other names for these keeps them through its
# ops/env-aliases.d/*.sh (ops/scripts/lib/env-aliases.sh), which may also set
# GOAT_DEPLOYMENT, the first word of the backup titles ops/backup.sh writes
# (default goat), and NETBIRD_DOMAIN, the management domain the closing check
# names (default netbird.legacy.example).
#
# Usage:
#   bash ops/restore.sh                              # restores latest backup
#   bash ops/restore.sh goat-backup-20260324  # restores specific backup

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
RESTORE_DIR="$SCRIPT_DIR/restore-tmp"

# Source credentials
if [ -f "$REPO_DIR/.envrc" ]; then
  set -a
  source "$REPO_DIR/.envrc" 2>/dev/null || true
  [ -f "$REPO_DIR/.envrc.local" ] && source "$REPO_DIR/.envrc.local" 2>/dev/null || true
  set +a
fi
# shellcheck source=scripts/lib/env-aliases.sh
. "$REPO_DIR/ops/scripts/lib/env-aliases.sh"
BACKUP_KIND="${GOAT_DEPLOYMENT:-goat}-backup"

: "${NETHOST_HOST:?Set NETHOST_HOST}"
: "${NETHOST_USER:?Set NETHOST_USER}"

# SSH setup — key auth preferred, sshpass fallback
SSH_TARGET="$NETHOST_USER@$NETHOST_HOST"
SCP_PREFIX="scp -o StrictHostKeyChecking=no"
SUDO_PREFIX="sudo"
if ! ssh -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o PasswordAuthentication=no "$SSH_TARGET" "echo ok" >/dev/null 2>&1; then
  : "${NETHOST_PASS:?SSH key auth failed and NETHOST_PASS not set}"
  SCP_PREFIX="sshpass -p '$NETHOST_PASS' scp -o StrictHostKeyChecking=no"
  SUDO_PREFIX="echo $NETHOST_PASS | sudo -S"
fi

run_remote() {
  if [ -n "${NETHOST_PASS:-}" ]; then
    eval "sshpass -p '$NETHOST_PASS' ssh -o StrictHostKeyChecking=no $SSH_TARGET '$1'"
  else
    ssh -o StrictHostKeyChecking=no "$SSH_TARGET" "$1"
  fi
}

# --------------------------------------------------
# Step 1: Find the backup in 1Password
# --------------------------------------------------
BACKUP_TITLE="${1:-}"

echo "============================================"
echo "  goat — Restore from 1Password"
echo "============================================"
echo ""

if ! command -v op >/dev/null 2>&1; then
  echo "ERROR: 1Password CLI (op) not installed."
  echo "  brew install 1password-cli"
  exit 1
fi

if ! op account get >/dev/null 2>&1; then
  echo "ERROR: Not signed in to 1Password."
  echo "  Run: eval \$(op signin)"
  exit 1
fi

if [ -z "$BACKUP_TITLE" ]; then
  echo "Available backups in 1Password (vault: Private):"
  echo ""
  op document list --vault Private --format json 2>/dev/null \
    | python3 -c "
import sys, json
docs = json.load(sys.stdin)
backups = [d for d in docs if '$BACKUP_KIND' in d.get('title','')]
if not backups:
    print('  No goat backups found.')
    sys.exit(1)
backups.sort(key=lambda d: d.get('created_at',''), reverse=True)
for b in backups:
    print(f\"  {b['title']}  ({b.get('created_at','unknown date')})\")
print()
print(f\"Latest: {backups[0]['title']}\")
" 2>/dev/null || true

  echo ""
  read -rp "Enter backup title (or press ENTER for latest): " BACKUP_TITLE

  if [ -z "$BACKUP_TITLE" ]; then
    BACKUP_TITLE=$(op document list --vault Private --format json 2>/dev/null \
      | python3 -c "
import sys, json
docs = json.load(sys.stdin)
backups = [d for d in docs if '$BACKUP_KIND' in d.get('title','')]
backups.sort(key=lambda d: d.get('created_at',''), reverse=True)
print(backups[0]['title'])
" 2>/dev/null)
    echo "  Using latest: $BACKUP_TITLE"
  fi
fi

# --------------------------------------------------
# Step 2: Download from 1Password to LOCAL
# --------------------------------------------------
echo ""
echo "[1/4] Downloading backup from 1Password..."
mkdir -p "$RESTORE_DIR"
TARBALL="$RESTORE_DIR/${BACKUP_TITLE}.tar.gz"

op document get "$BACKUP_TITLE" --vault Private --out-file "$TARBALL" 2>/dev/null
if [ ! -f "$TARBALL" ]; then
  echo "  ERROR: Download failed. Check the title matches exactly."
  exit 1
fi
echo "  Downloaded: $TARBALL ($(du -h "$TARBALL" | cut -f1))"

# --------------------------------------------------
# Step 3: Extract locally
# --------------------------------------------------
echo "[2/4] Extracting..."
EXTRACT_DIR="$RESTORE_DIR/extracted"
rm -rf "$EXTRACT_DIR"
mkdir -p "$EXTRACT_DIR"
tar xzf "$TARBALL" -C "$EXTRACT_DIR"

# Find the snapshot directory (may be nested)
SNAP_DIR=$(find "$EXTRACT_DIR" -name "netbird.tar.gz" -exec dirname {} \; | head -1)
if [ -z "$SNAP_DIR" ]; then
  echo "  ERROR: Could not find netbird.tar.gz in the backup archive."
  echo "  Contents:"
  ls -R "$EXTRACT_DIR"
  exit 1
fi
echo "  Extracted to: $SNAP_DIR"
echo "  Contents:"
ls -lh "$SNAP_DIR"

# --------------------------------------------------
# Step 4: SCP to NETHOST
# --------------------------------------------------
echo ""
echo "[3/4] Copying to NETHOST ($NETHOST_HOST)..."

if ! run_remote "echo ok" >/dev/null 2>&1; then
  echo "  ERROR: Cannot SSH to $SSH_TARGET"
  exit 1
fi

REMOTE_TMP="/tmp/${GOAT_DEPLOYMENT:-goat}-restore-$$"
run_remote "mkdir -p $REMOTE_TMP"

for f in "$SNAP_DIR"/*.tar.gz; do
  fname=$(basename "$f")
  echo "  Uploading $fname..."
  eval "$SCP_PREFIX $f $SSH_TARGET:$REMOTE_TMP/$fname"
done

# Also copy non-tarball files (hosts, hostname, etc.)
for f in "$SNAP_DIR"/hosts "$SNAP_DIR"/hostname; do
  [ -f "$f" ] && eval "$SCP_PREFIX $f $SSH_TARGET:$REMOTE_TMP/$(basename $f)"
done

# --------------------------------------------------
# Step 5: Restore on NETHOST
# --------------------------------------------------
echo ""
echo "[4/4] Restoring on NETHOST..."
echo ""
echo "  WARNING: This will overwrite the current Netbird and Cloudflare configs."
read -rp "  Continue? (y/N): " CONFIRM
if [[ ! "$CONFIRM" =~ ^[yY]$ ]]; then
  echo "  Aborted. Files are on NETHOST at $REMOTE_TMP for manual restore."
  exit 0
fi

echo "  Stopping services..."
run_remote "$SUDO_PREFIX docker compose -f /opt/netbird/docker-compose.yml down 2>/dev/null || true"
run_remote "$SUDO_PREFIX systemctl stop cloudflared 2>/dev/null || true"

echo "  Restoring Netbird config..."
run_remote "$SUDO_PREFIX tar xzf $REMOTE_TMP/netbird.tar.gz -C / 2>/dev/null"

if [ -f "$SNAP_DIR/netbird-volumes.tar.gz" ]; then
  echo "  Restoring Netbird volumes (management DB + TLS certs)..."
  run_remote "$SUDO_PREFIX tar xzf $REMOTE_TMP/netbird-volumes.tar.gz -C /var/lib/docker/volumes/ 2>/dev/null"
fi

echo "  Restoring Cloudflare tunnel config..."
run_remote "$SUDO_PREFIX tar xzf $REMOTE_TMP/cloudflared.tar.gz -C / 2>/dev/null || true"
if [ -f "$SNAP_DIR/cloudflared-creds.tar.gz" ]; then
  echo "  Restoring tunnel credentials..."
  run_remote "$SUDO_PREFIX mkdir -p /root/.cloudflared"
  run_remote "$SUDO_PREFIX tar xzf $REMOTE_TMP/cloudflared-creds.tar.gz -C / 2>/dev/null || true"
fi

echo "  Restoring /etc/hosts..."
run_remote "$SUDO_PREFIX cp $REMOTE_TMP/hosts /etc/hosts 2>/dev/null || true"

echo "  Starting services..."
run_remote "$SUDO_PREFIX systemctl start cloudflared"
run_remote "$SUDO_PREFIX docker compose -f /opt/netbird/docker-compose.yml up -d"

echo "  Waiting 15s for services to initialize..."
sleep 15

echo "  Verifying..."
run_remote "$SUDO_PREFIX docker compose -f /opt/netbird/docker-compose.yml ps 2>/dev/null || true"
run_remote "$SUDO_PREFIX systemctl is-active cloudflared 2>/dev/null || true"
run_remote "$SUDO_PREFIX netbird status 2>/dev/null || true"

# Cleanup
run_remote "rm -rf $REMOTE_TMP 2>/dev/null || true"
rm -rf "$RESTORE_DIR"

echo ""
echo "============================================"
echo "  Restore complete"
echo "============================================"
echo ""
echo "  Verify from LOCAL:"
echo "    sudo netbird status"
echo "    curl -sk https://${NETBIRD_DOMAIN:-netbird.legacy.example}"
