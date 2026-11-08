#!/usr/bin/env bash
# Mint a goatlite founder's own fabric identity and install it as a goat
# context on this laptop (goatlite.md §8, F-610).
#
# A goatlite runs no identity console, so nothing issues its one person the
# welcome package a larger goatnet's console would. The founder already holds
# the goatnet's CA (the render minted it on this laptop), so this mints the
# person leaf from it, the same way the assay probe's leaf is minted:
#
#   - a leaf from this goatnet's CA, CN goat-founder-trial-peer (row `founder`
#     in ops/enrollment/manifests/<goatnet>-certs.yaml, written by the render);
#   - the router subject `founder` (roles/goat-zenoh, rendered where
#     zenoh_founder_cn is set): writes founder/** and `goat hello` under the
#     goatnet's name, reads like every peer;
#   - installed as the goat context <goatnet> under ~/.goat/contexts (or
#     $GOAT_PROFILE_DIR/contexts), mode 0700; the key never leaves this laptop.
#
# OPERATOR-RUN: minting key material is a deliberate act, never a playbook side
# effect. Idempotent: an existing leaf is reused; an installed context whose
# cert already matches is left alone.
#
# Usage:
#   ops/scripts/create-founder-identity.sh <goatnet> [--dry-run]
#   ops/scripts/create-founder-identity.sh <goatnet> --revoke
set -euo pipefail
die() { echo "create-founder-identity: $*" >&2; exit 1; }
GOATNET="${1:-}"
[ -n "$GOATNET" ] || die "usage: create-founder-identity.sh <goatnet> [--dry-run|--revoke]"
MODE="${2:-install}"
case "$MODE" in install|--dry-run|--revoke) ;; *) die "unknown argument: $MODE" ;; esac
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$SCRIPT_DIR/../.." && pwd)"
GV="$REPO/ops/ansible/group_vars/${GOATNET//-/_}_hosts/vars.yml"
MANIFEST="$REPO/ops/enrollment/manifests/${GOATNET}-certs.yaml"
CN="goat-founder-trial-peer"
CTX_ROOT="${GOAT_PROFILE_DIR:-$HOME/.goat}/contexts"
CTX="$CTX_ROOT/$GOATNET"

command -v yq >/dev/null || die "yq is required (the same dependency as mint-site-certs.sh)"
command -v openssl >/dev/null || die "openssl is required"
[ -f "$GV" ] || die "no $GV: is $GOATNET a goatnet in this checkout?"
grep -qE "^zenoh_founder_cn:[[:space:]]*$CN" "$GV" \
  || die "$GOATNET declares no founder (zenoh_founder_cn: $CN in $GV); a goatlite render writes it"
[ -f "$MANIFEST" ] || die "no cert manifest $MANIFEST"
[ "$(yq -r '.leaves[] | select(.name == "founder") | .cn' "$MANIFEST")" = "$CN" ] \
  || die "$MANIFEST has no leaf 'founder' with cn $CN; re-run the render (goatbuilder standup $GOATNET --to 1)"

if [ "$MODE" = "--revoke" ]; then
  echo "revoking: removing goat context $GOATNET at $CTX"
  rm -rf "$CTX"
  echo "done. The router admits the CN until zenoh_founder_cn leaves $GV and phase 9 re-runs."
  exit 0
fi

ZONE="$(grep -E '^site_dns_zone:' "$GV" | head -1 | sed -E 's/^site_dns_zone:[[:space:]]*//; s/["'\'']//g; s/[[:space:]]*#.*$//')"
HOST="$(grep -E '^goatlite_host:' "$GV" | head -1 | sed -E 's/^goatlite_host:[[:space:]]*//; s/["'\'']//g; s/[[:space:]]*#.*$//')"
[ -n "$ZONE" ] && [ -n "$HOST" ] || die "$GV lacks site_dns_zone or goatlite_host"
PORTAL="https://portal.${GOATNET}.${ZONE}"
ROUTER="tls/${HOST}:7447"
ROOT_CERT="$REPO/$(yq -r '.ca.root_cert' "$MANIFEST")"
OUT_DIR="$(yq -r '.out_dir' "$MANIFEST")"; OUT_DIR="${OUT_DIR/#\~/$HOME}"
LEAF_CERT="$OUT_DIR/founder-cert.pem"
LEAF_KEY="$OUT_DIR/founder-key.pem"

echo "goatnet  $GOATNET"
echo "context  $GOATNET at $CTX"
echo "router   $ROUTER"
echo "leaf     CN=$CN from $ROOT_CERT"
if [ "$MODE" = "--dry-run" ]; then
  "$REPO/ops/scripts/mint-site-certs.sh" "$GOATNET" --only founder --dry-run
  echo "dry run: would install the context and run 'goat --context $GOATNET status'"
  exit 0
fi

"$REPO/ops/scripts/mint-site-certs.sh" "$GOATNET" --only founder
[ -f "$LEAF_CERT" ] && [ -f "$LEAF_KEY" ] || die "the mint left no pair at $OUT_DIR"
subject="$(openssl x509 -in "$LEAF_CERT" -noout -subject)"
case "$subject" in *"CN=$CN"*|*"CN = $CN"*) ;; *) die "leaf subject is '$subject', not CN=$CN" ;; esac
openssl verify -CAfile "$ROOT_CERT" "$LEAF_CERT" >/dev/null || die "leaf does not chain to $ROOT_CERT"

want="$(openssl x509 -in "$LEAF_CERT" -noout -fingerprint -sha256)"
have=""; [ -f "$CTX/mtls.cert.pem" ] && have="$(openssl x509 -in "$CTX/mtls.cert.pem" -noout -fingerprint -sha256)"
if [ "$have" = "$want" ]; then
  echo "installed: the context already holds this leaf"
else
  echo "installing the context"
  ( umask 077
    mkdir -p "$CTX"; chmod 700 "$CTX"
    cp "$LEAF_CERT" "$CTX/mtls.cert.pem"
    cp "$LEAF_KEY" "$CTX/mtls.key.pem"
    cp "$ROOT_CERT" "$CTX/ca-roots.pem"
    [ -f "$CTX/portal.token" ] || : > "$CTX/portal.token"
    cat > "$CTX/profile.toml" <<TOML
portal_endpoint = "$PORTAL"
router_endpoint = "$ROUTER"
device_id = "founder-$GOATNET"
goatnet = "$GOATNET"
TOML
  )
fi

if command -v goat >/dev/null; then
  # goat status exits non-zero when any layer fails (a context with no portal
  # key fails the portal row), and under pipefail that would fail the pipe even
  # when the session row is ok; read the output, then the session row.
  status_json="$(goat --context "$GOATNET" status --json --timeout 20 2>/dev/null || true)"
  if printf '%s' "$status_json" | grep -q '"session":{"ok":true'; then
    echo "verified: goat --context $GOATNET status opens an mTLS session to the router"
  else
    echo "installed, not yet verified: 'goat --context $GOATNET status' opened no session." >&2
    echo "The router admits the founder after phase 9 runs with zenoh_founder_cn set; your laptop must reach $HOST on port 7447." >&2
    exit 3
  fi
else
  echo "installed; 'goat' is not on PATH, so the session was not checked."
fi
