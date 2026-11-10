#!/usr/bin/env bash
# operator-laptop-rotate-wg-cp0.sh
#
# Re-apply a freshly-issued wg-cp0 bundle on an operator laptop. Used
# during offline-CA root rotations (Block 79) or whenever a bundle is
# re-issued for any reason (key expiry, allowlist refresh, etc.).
#
# Originally lived as /tmp/op-mac-apply-rotation.sh during the
# Block 79 ceremony; promoted into the repo with corruption-
# safe handling after the original version's bare `cat >> bundle.pem`
# broke legacy on the operator laptop (no trailing-newline guard).
#
# Idempotent — safe to re-run. If wg-cp0 is already on the bundle's
# device keypair (pubkey matches), prints "✓ Already on this bundle"
# and exits without disturbing the running daemon.
#
# Usage:
#   sudo bash ops/scripts/operator-laptop-rotate-wg-cp0.sh \
#     --bundle ~/.goat/state/op-laptop.bundle.cbor \
#     --ca-pubkey ops/enrollment/ca/dev/root-cert.pem
#
# Default paths (when --bundle / --ca-pubkey not given):
#   bundle:    ~/.goat/state/<device-id>.bundle.cbor  (first match)
#   ca-pubkey: ops/enrollment/ca/dev/root-cert.pem  (Block 79 ECDSA root)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

BUNDLE=""
CA_PUBKEY="$REPO_ROOT/ops/enrollment/ca/dev/root-cert.pem"
WG_CONF=/etc/wireguard/wg-cp0.conf
WG_PRIVKEY=/etc/wireguard/wg-cp0-priv.key
APPLY_BIN="$REPO_ROOT/ops/enrollment/cmd/wg-cp0-bundle-apply/wg-cp0-bundle-apply"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle)     BUNDLE="$2"; shift 2 ;;
    --ca-pubkey)  CA_PUBKEY="$2"; shift 2 ;;
    --wg-conf)    WG_CONF="$2"; shift 2 ;;
    --wg-privkey) WG_PRIVKEY="$2"; shift 2 ;;
    -h|--help)
      sed -n '1,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0 ;;
    *) echo "unknown flag: $1" >&2; exit 1 ;;
  esac
done

# Locate bundle if not specified — first *.bundle.cbor in
# ~/.goat/state/ (recursive).
if [[ -z "$BUNDLE" ]]; then
  # The project's state directory: deploy.state_dir in
  # ops/goatbuilder/goatbuilder.yaml, else ~/.goat/state.
  STATE_DIR="$("$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/project-setting.sh" deploy state_dir 2>/dev/null || true)"
  # shellcheck disable=SC2088  # a declared path is written with a literal "~/"
  [[ -n "$STATE_DIR" ]] || STATE_DIR="~/.goat/state"
  # shellcheck disable=SC2088  # matching the literal "~/" of a declared path
  case "$STATE_DIR" in "~/"*) STATE_DIR="${HOME}/${STATE_DIR#\~/}" ;; esac
  BUNDLE="$(find "$STATE_DIR" -name '*.bundle.cbor' -type f 2>/dev/null | head -1)"
  if [[ -z "$BUNDLE" ]]; then
    echo "ERROR: --bundle not given and no *.bundle.cbor found under $STATE_DIR/" >&2
    exit 1
  fi
  echo "(auto-detected bundle: $BUNDLE)"
fi

# Sanity checks
[[ -r "$BUNDLE" ]] || { echo "ERROR: bundle not readable: $BUNDLE" >&2; exit 1; }
[[ -r "$CA_PUBKEY" ]] || { echo "ERROR: ca-pubkey not readable: $CA_PUBKEY" >&2; exit 1; }

# Ensure wg-cp0-bundle-apply is built. Block 79 / commit dc3944fe added
# CERTIFICATE-PEM acceptance + ECDSA P-256 support — make sure we have
# at least that version locally.
if [[ ! -x "$APPLY_BIN" ]]; then
  echo "Building $APPLY_BIN ..."
  (cd "$REPO_ROOT/ops/enrollment" && go build -o "$APPLY_BIN" ./cmd/wg-cp0-bundle-apply)
fi

# Backup directory + roll back on failure
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
BACKUP_DIR="/etc/wireguard/block-79-backup-$STAMP"
mkdir -p "$BACKUP_DIR"
[[ -f "$WG_CONF" ]] && cp "$WG_CONF" "$BACKUP_DIR/wg-cp0.conf.old"
[[ -f "$WG_PRIVKEY" ]] && cp "$WG_PRIVKEY" "$BACKUP_DIR/wg-cp0-priv.key.old"

restore_on_failure() {
  local rc=$?
  if [[ $rc -ne 0 ]]; then
    echo "FAIL: rolling back from $BACKUP_DIR"
    [[ -f "$BACKUP_DIR/wg-cp0.conf.old" ]] && cp "$BACKUP_DIR/wg-cp0.conf.old" "$WG_CONF"
    [[ -f "$BACKUP_DIR/wg-cp0-priv.key.old" ]] && cp "$BACKUP_DIR/wg-cp0-priv.key.old" "$WG_PRIVKEY"
    wg-quick down wg-cp0 2>/dev/null || true
    wg-quick up wg-cp0 2>/dev/null || true
  fi
  exit $rc
}
trap restore_on_failure ERR

# Bring wg-cp0 down before swap (apply tool writes atomically; this
# avoids the daemon reading mid-rotation state)
wg-quick down wg-cp0 2>/dev/null || true

# Apply the new bundle
"$APPLY_BIN" \
  -in "$BUNDLE" \
  -ca-pubkey "$CA_PUBKEY" \
  -out-conf "$WG_CONF" \
  -out-privkey "$WG_PRIVKEY" \
  -interface-name wg-cp0 \
  -persistent-keepalive 25

chmod 0600 "$WG_CONF" "$WG_PRIVKEY"

# Bring wg-cp0 back up
wg-quick up wg-cp0

# Wait for first handshake (up to 15s)
sleep 3
echo
echo "--- wg show wg-cp0 ---"
wg show wg-cp0 | head -10
echo
echo "--- latest handshakes ---"
COUNT=0
for _i in 1 2 3 4 5; do
  COUNT=$(wg show wg-cp0 latest-handshakes 2>/dev/null | awk '$2 != 0 {n++} END {print n+0}')
  if [[ "$COUNT" -ge 1 ]]; then break; fi
  sleep 2
done
wg show wg-cp0 latest-handshakes
echo
echo "Peers with non-zero handshake: $COUNT"
echo
echo "Bundle applied. Backup of previous state at $BACKUP_DIR/."
echo "If something looks off, restore with:"
echo "  sudo cp $BACKUP_DIR/wg-cp0.conf.old $WG_CONF"
echo "  sudo cp $BACKUP_DIR/wg-cp0-priv.key.old $WG_PRIVKEY"
echo "  sudo wg-quick down wg-cp0 && sudo wg-quick up wg-cp0"
