#!/usr/bin/env bash
# Mint and install the acceptance assay's probe identity on one goatnet.
#
# The goat CLI rows of `goatbuilder assay <goatnet>` need a goat context that
# holds a fabric identity. A person's context is the wrong tool for an
# unattended probe: a person's fabric slot comes from three-stage onboarding,
# which a second admin approves, and it carries that person's writes and
# attribution. This is the non-person alternative, the same shape as every
# other infra peer (fabric-health, mesh-health, ...):
#
#   - a leaf from this goatnet's offline CA, CN goat-assay-probe-trial-peer
#     (row `assay-probe` in ops/enrollment/manifests/<goatnet>-certs.yaml);
#   - the router ACL subject infra-assay-probe, which may write ONLY
#     hello/assay-probe and volatile/goat-assay/** (roles/goat-zenoh); its read
#     is the universal read, which keeps the operator-audit deny;
#   - no portal credential and no NetBird credential: every row it runs reads
#     the portal's public endpoints or degrades without a token;
#   - installed as the goat context `assay-probe` on the goatnet's first
#     management machine, where the assay already runs its goat rows
#     (probe_via in ops/validation/values-<goatnet>.yaml). The key never lands
#     on a laptop's goat contexts, in the repo, or in a scratch file.
#
# OPERATOR-RUN, NEVER A PLAYBOOK SIDE EFFECT (live-state-guard-scheduling.md §4):
# minting key material is a decision with an audit trail. Idempotent: an
# existing leaf is reused; an installed context whose cert already matches is
# left alone.
#
# Usage:
#   ops/scripts/create-assay-probe-identity.sh <goatnet> [--dry-run]
#   ops/scripts/create-assay-probe-identity.sh <goatnet> --revoke
#
# --revoke removes the installed context from the management machine. The ACL
# half of a revocation is taking the CN out of zenoh_universal_read_cns (a
# partitioned goatnet) and the infra-assay-probe subject, then redeploying the
# router; rotation is --revoke, deleting the leaf pair under the manifest's
# out_dir, and running this again.
#
# Environment (all optional):
#   MGMT_HOST      ssh target; default probe_via from the goatnet's values file
#   MGMT_SSH_OPTS  extra ssh words, word-split on purpose (through an env door:
#                  MGMT_HOST=goat@127.0.0.1 MGMT_SSH_OPTS="-p 12202")
set -euo pipefail

die() { echo "create-assay-probe-identity: $*" >&2; exit 1; }

GOATNET="${1:-}"
[ -n "$GOATNET" ] || die "usage: create-assay-probe-identity.sh <goatnet> [--dry-run|--revoke]"
MODE="${2:-install}"
case "$MODE" in install|--dry-run|--revoke) ;; *) die "unknown argument: $MODE" ;; esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$SCRIPT_DIR/../.." && pwd)"
VALUES="$REPO/ops/validation/values-${GOATNET}.yaml"
MANIFEST="$REPO/ops/enrollment/manifests/${GOATNET}-certs.yaml"
CONTEXT="assay-probe"
CN="goat-assay-probe-trial-peer"

command -v yq >/dev/null || die "yq is required (same dependency as mint-site-certs.sh)"
[ -f "$VALUES" ] || die "no $VALUES — run ops/validation/generate-values.py $GOATNET"
[ "$(yq -r '.assay_probe_enabled // false' "$VALUES")" = "true" ] \
  || die "$GOATNET does not declare the probe: set assay_probe_enabled: true in its group_vars vars.yml and regenerate $VALUES"
[ -f "$MANIFEST" ] || die "no cert manifest $MANIFEST"
[ "$(yq -r '.leaves[] | select(.name == "assay-probe") | .cn' "$MANIFEST")" = "$CN" ] \
  || die "$MANIFEST has no leaf 'assay-probe' with cn $CN (copy the row from TEMPLATE-certs.yaml)"

MGMT_HOST="${MGMT_HOST:-$(yq -r '.probe_via // ""' "$VALUES")}"
[ -n "$MGMT_HOST" ] && [ "$MGMT_HOST" != "localhost" ] || die "no management host: set MGMT_HOST or probe_via in $VALUES"
MGMT_SSH_OPTS="${MGMT_SSH_OPTS:-}"
# shellcheck disable=SC2086  # MGMT_SSH_OPTS is several ssh words on purpose
on_host() { ssh -o BatchMode=yes -o ConnectTimeout=15 $MGMT_SSH_OPTS "$MGMT_HOST" "$@"; }

if [ "$MODE" = "--revoke" ]; then
  echo "revoking: removing goat context $CONTEXT on $MGMT_HOST"
  on_host "rm -rf \"\$HOME/.goat/contexts/$CONTEXT\""
  echo "done. The CN stays admitted by the router until it leaves zenoh_universal_read_cns and the infra-assay-probe subject."
  exit 0
fi

SITE_ZONE="$(yq -r '.site_dns_zone // ""' "$VALUES")"
MESH_IP="$(yq -r '.mesh_ip_a // ""' "$VALUES")"
[ -n "$SITE_ZONE" ] && [ -n "$MESH_IP" ] || die "$VALUES lacks site_dns_zone or mesh_ip_a"
PORTAL="https://portal.${GOATNET}.${SITE_ZONE}"
ROUTER="tls/${MESH_IP}:7447"
ROOT_CERT="$REPO/$(yq -r '.ca.root_cert' "$MANIFEST")"
OUT_DIR="$(yq -r '.out_dir' "$MANIFEST")"; OUT_DIR="${OUT_DIR/#\~/$HOME}"
LEAF_CERT="$OUT_DIR/assay-probe-cert.pem"
LEAF_KEY="$OUT_DIR/assay-probe-key.pem"

echo "goatnet  $GOATNET"
echo "host     $MGMT_HOST (goat context $CONTEXT)"
echo "portal   $PORTAL"
echo "router   $ROUTER"
echo "leaf     CN=$CN from $ROOT_CERT"

if [ "$MODE" = "--dry-run" ]; then
  "$REPO/ops/scripts/mint-site-certs.sh" "$GOATNET" --only assay-probe --dry-run
  echo "dry run: would install $CONTEXT on $MGMT_HOST and run 'goat --context $CONTEXT status --json' there"
  exit 0
fi

# 1. The leaf. mint-site-certs.sh skips a leaf whose cert already exists.
"$REPO/ops/scripts/mint-site-certs.sh" "$GOATNET" --only assay-probe
[ -f "$LEAF_CERT" ] && [ -f "$LEAF_KEY" ] || die "the mint left no pair at $OUT_DIR"
subject="$(openssl x509 -in "$LEAF_CERT" -noout -subject)"
case "$subject" in *"CN=$CN"*|*"CN = $CN"*) ;; *) die "leaf subject is '$subject', not CN=$CN" ;; esac
openssl verify -CAfile "$ROOT_CERT" "$LEAF_CERT" >/dev/null || die "leaf does not chain to $ROOT_CERT"

# 1b. goat on the host. The assay runs its goat rows there, as this context;
#     a box no play installed goat on (every goatlite) gets the public goat-cli
#     release of the version this machine runs. The host downloads it itself
#     and checks it against the release's own .sha256 before installing.
GOAT_RELEASES="${GOAT_RELEASES:-https://github.com/usareur-af/goatcore/releases/download}"
if ! on_host "command -v goat >/dev/null"; then
  ver="$(goat --version 2>/dev/null | awk '{print $2}')"
  [ -n "$ver" ] || die "goat is on neither this machine nor $MGMT_HOST: build it first (docs/tutorials/00-goatlite.md section 1)"
  echo "installing goat $ver on $MGMT_HOST from $GOAT_RELEASES/goat-cli-v$ver"
  on_host "set -e; case \$(uname -m) in x86_64) a=amd64 ;; aarch64|arm64) a=arm64 ;; *) echo \"no goat-cli release for \$(uname -m)\" >&2; exit 1 ;; esac
    t=\$(mktemp -d); cd \"\$t\"
    curl -fsSL -o goat-linux-\$a '$GOAT_RELEASES/goat-cli-v$ver/goat-linux-'\$a
    curl -fsSL -o goat-linux-\$a.sha256 '$GOAT_RELEASES/goat-cli-v$ver/goat-linux-'\$a.sha256
    sha256sum -c goat-linux-\$a.sha256
    sudo install -m 0755 goat-linux-\$a /usr/local/bin/goat
    rm -f goat-linux-\$a goat-linux-\$a.sha256; cd /; rmdir \"\$t\"" \
    || die "could not install goat $ver on $MGMT_HOST (no release goat-cli-v$ver, no network, or no sudo): install goat there by hand, then re-run"
  on_host "goat --version" || die "goat installed on $MGMT_HOST but does not run"
fi

# 2. Install, unless the host already holds this exact cert. The key travels on
#    ssh's stdin into a 0600 file; it is never printed or written elsewhere.
want="$(openssl x509 -in "$LEAF_CERT" -noout -fingerprint -sha256)"
have="$(on_host "f=\"\$HOME/.goat/contexts/$CONTEXT/mtls.cert.pem\"; [ -f \"\$f\" ] && openssl x509 -in \"\$f\" -noout -fingerprint -sha256 || true")"
if [ "$have" = "$want" ]; then
  echo "installed: $CONTEXT already holds this leaf"
  on_host "umask 077; f=\"\$HOME/.goat/contexts/$CONTEXT/portal.token\"; [ -f \"\$f\" ] || : > \"\$f\""
else
  echo "installing $CONTEXT on $MGMT_HOST"
  on_host "umask 077; mkdir -p \"\$HOME/.goat/contexts/$CONTEXT\"; chmod 700 \"\$HOME/.goat/contexts/$CONTEXT\""
  on_host "umask 077; cat > \"\$HOME/.goat/contexts/$CONTEXT/mtls.cert.pem\"" < "$LEAF_CERT"
  on_host "umask 077; cat > \"\$HOME/.goat/contexts/$CONTEXT/mtls.key.pem\"" < "$LEAF_KEY"
  on_host "umask 077; cat > \"\$HOME/.goat/contexts/$CONTEXT/ca-roots.pem\"" < "$ROOT_CERT"
  # goat refuses to load a context without a portal.token file. An empty one
  # is this identity's portal credential: none (requests carry no key, and the
  # rows it runs read public endpoints or degrade without one).
  on_host "umask 077; : > \"\$HOME/.goat/contexts/$CONTEXT/portal.token\""
  on_host "umask 077; cat > \"\$HOME/.goat/contexts/$CONTEXT/profile.toml\"" <<EOF
# goat profile.toml — the acceptance assay's probe identity on $GOATNET.
# Written by ops/scripts/create-assay-probe-identity.sh; a non-person fabric
# leaf (CN=$CN). No portal or NetBird credential belongs in this context.
portal_endpoint = "$PORTAL"
router_endpoint = "$ROUTER"
device_id = "assay-probe-$GOATNET"
goatnet = "$GOATNET"
EOF
fi

# 3. Prove it: the router accepts the identity and the portal answers.
out="$(on_host "goat --context $CONTEXT status --json" 2>&1 || true)"
if printf '%s' "$out" | grep -q '"session":{"ok":true' && printf '%s' "$out" | grep -q '"cert":{"ok":true'; then
  echo "verified: goat --context $CONTEXT status reports the cert valid and an mTLS session open"
else
  echo "$out" | head -20 >&2
  die "the context is installed but goat status does not report an open session; is the router deployed with the infra-assay-probe subject and the CN in zenoh_universal_read_cns?"
fi
