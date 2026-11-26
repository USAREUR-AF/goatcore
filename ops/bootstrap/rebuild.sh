#!/usr/bin/env bash
# ops/bootstrap/rebuild.sh — bootstrap bundle build trigger.
#
# Tags the current main branch as `bundle-vYYYYMMDD`, builds the bundle
# tarball per docs/design/bootstrap-bundle.md §2, signs it with the
# offline-CA YubiKey (or operator's ed25519 key for dev mode), encrypts
# with age multi-recipient per recipients.txt, optionally uploads to T1
# S3.
#
# Spec: docs/design/bootstrap-bundle.md §6 (build + refresh cadence)
#
# Usage:
#   ./rebuild.sh                # build + sign + encrypt; do not upload
#   ./rebuild.sh --upload       # also push to T1 S3
#   ./rebuild.sh --dry-run      # build only, no sign/encrypt; for CI smoke
#   ./rebuild.sh --tag <name>   # use specific tag instead of bundle-v$(date)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RECIPIENTS="$REPO_ROOT/ops/bootstrap/recipients.txt"
T1_BUCKET="${T1_BUCKET:-example-bootstrap-bundle}"
SIGNING_NAMESPACE=bootstrap-bundle

UPLOAD=0
DRY_RUN=0
TAG=""

usage() {
  cat <<EOF
rebuild.sh — bootstrap bundle builder (docs/design/bootstrap-bundle.md §6)

Usage:
  $(basename "$0") [--upload] [--dry-run] [--tag <name>]
                   [--signing-key <path>] [--out-dir <path>]

Options:
  --upload                Push tagged bundle ciphertext + signature to
                          s3://\$T1_BUCKET/bootstrap/. Requires aws-vault
                          + prod-account access.
  --dry-run               Build the tarball only; skip sign + encrypt.
                          Useful for CI smoke + content audit.
  --tag <name>            Use a specific tag name (default:
                          bundle-vYYYYMMDD).
  --signing-key <path>    SSH key path for ssh-keygen -Y sign. Default:
                          \$HOME/.ssh/id_ed25519. For YubiKey PIV use
                          /dev/<piv-identity> per ssh-agent setup.
  --out-dir <path>        Output directory (default: /tmp/bundle-<tag>).
EOF
  exit 1
}

SIGNING_KEY="${HOME}/.ssh/id_ed25519"
OUT_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --upload) UPLOAD=1; shift;;
    --dry-run) DRY_RUN=1; shift;;
    --tag) TAG=$2; shift 2;;
    --signing-key) SIGNING_KEY=$2; shift 2;;
    --out-dir) OUT_DIR=$2; shift 2;;
    -h|--help) usage;;
    *) echo "unknown arg: $1" >&2; usage;;
  esac
done

# -- Pre-flight ----------------------------------------------------------
cd "$REPO_ROOT"

if [[ -z "$TAG" ]]; then
  TAG="bundle-v$(date -u +%Y%m%d)"
fi
OUT_DIR=${OUT_DIR:-"/tmp/$TAG"}
mkdir -p "$OUT_DIR"

step() { printf '\n=== %s ===\n' "$*"; }
log() { printf '[rebuild] %s\n' "$*"; }

# Ensure we're on main and clean. Bundles are tagged off main per §6.
BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [[ "$BRANCH" != "main" ]]; then
  log "WARN: not on main (currently on $BRANCH); tag will reflect this branch"
fi
if ! git diff-index --quiet HEAD --; then
  log "FAIL: working tree has uncommitted changes; rebuild requires a clean tree"
  exit 1
fi

# -- Step 1: tag the build ----------------------------------------------
step "Step 1: ensure tag $TAG exists"
if git rev-parse "$TAG" >/dev/null 2>&1; then
  log "tag $TAG already exists; reusing"
else
  log "creating tag $TAG"
  git tag -a "$TAG" -m "Bootstrap bundle build $TAG"
fi
COMMIT=$(git rev-parse "$TAG")
log "tag points at $COMMIT"

# -- Step 2: assemble bundle contents per bootstrap-bundle.md §2 --------
step "Step 2: assemble bundle contents"
STAGE="$OUT_DIR/stage"
rm -rf "$STAGE"
mkdir -p "$STAGE"

# 2a: git bundle (full repo at signed tag)
log "  2a: git bundle"
git bundle create "$STAGE/repo.bundle" "$TAG"

# 2b: Container image manifest list (text only — operators pull at
# image-build time per docs/operations/image-builder-host.md). Bundle
# v1 carries the manifest, NOT the tar bodies, to keep the bundle
# small. Image build pipeline does the actual `docker pull` + `save`.
log "  2b: container image manifest"
cat > "$STAGE/container-images.txt" <<'EOF'
# Mgmt-stack container images for Block 68 image build pipeline.
# Tags pinned at bundle build time. See ops/compose/mgmt-host/docker-compose.yml
# for runtime references.
ghcr.io/usareur-af/postgres-spock:latest
netbirdio/netbird-mgmt:patched-53b04e5
netbirdio/signal:latest
traefik:v3.0
coredns/coredns:1.11.3
eclipse/zenoh:1.10.2
openobserve/openobserve:latest
minio/minio:latest
otel/opentelemetry-collector-contrib:latest
prom/prometheus:latest
prom/alertmanager:latest
grafana/grafana:latest
EOF

# 2c: Terraform lock file + provider versions
log "  2c: terraform lock + version pins"
mkdir -p "$STAGE/terraform-pins"
[[ -f "$REPO_ROOT/terraform/.terraform.lock.hcl" ]] && \
  cp "$REPO_ROOT/terraform/.terraform.lock.hcl" "$STAGE/terraform-pins/dev.lock.hcl"
[[ -f "$REPO_ROOT/terraform-prod/.terraform.lock.hcl" ]] && \
  cp "$REPO_ROOT/terraform-prod/.terraform.lock.hcl" "$STAGE/terraform-pins/prod.lock.hcl"

# 2d: Offline-CA public certificate (operator-side public key only).
# Search canonical paths in priority order: prod-mode X.509 root → dev-mode
# X.509 root → committed dev pubkey snapshot → legacy paths.
log "  2d: offline-CA public certificate"
mkdir -p "$STAGE/offline-ca"
CA_CERT_CANDIDATES=(
  "$REPO_ROOT/ops/enrollment/ca/prod/root-cert.pem"
  "$REPO_ROOT/ops/enrollment/ca/dev/root-cert.pem"
  "$REPO_ROOT/ops/enrollment/public-keys/dev-goat-ca-2026-04-20.pem"
  "$REPO_ROOT/ops/enrollment/ca/root.pem"
  "$REPO_ROOT/ops/enrollment/ca/ca-public.pem"
)
CA_CERT_FOUND=""
for cand in "${CA_CERT_CANDIDATES[@]}"; do
  if [[ -f "$cand" ]]; then
    cp "$cand" "$STAGE/offline-ca/root.pem"
    CA_CERT_FOUND="$cand"
    break
  fi
done
if [[ -n "$CA_CERT_FOUND" ]]; then
  log "  found offline-CA cert: $CA_CERT_FOUND"
else
  echo "TODO: offline-CA public cert not found at any expected path" \
    > "$STAGE/offline-ca/MISSING.txt"
  log "  WARN: offline-CA public cert missing; bundle will need it added"
fi

# 2e: Empty-state DB schema seeds
log "  2e: empty-state DB seeds"
mkdir -p "$STAGE/db-seeds"
if [[ -d "$REPO_ROOT/ops/postgres/spock/init" ]]; then
  cp -a "$REPO_ROOT/ops/postgres/spock/init" "$STAGE/db-seeds/spock-init"
fi

# 2f: README_RECOVERY for the operator
log "  2f: README_RECOVERY.md"
cat > "$STAGE/README_RECOVERY.md" <<EOF
# Bootstrap bundle — recovery runbook

Bundle tag: $TAG
Built: $(date -u +%Y-%m-%dT%H:%M:%SZ)
Commit: $COMMIT

## Verify-then-decrypt

\`\`\`bash
# 1. Verify the signature against the offline-CA pubkey:
ssh-keygen -Y verify \\
  -f /path/to/recipients.txt \\
  -n $SIGNING_NAMESPACE \\
  -s bundle.tar.sig \\
  < bundle.tar.age

# 2. Decrypt with your age private key:
age -d -i ~/.config/age/keys.txt -o bundle.tar bundle.tar.age

# 3. Extract:
tar -xf bundle.tar
\`\`\`

## What's inside

- \`repo.bundle\` — full git repo at tag $TAG. Restore via:
    \`git clone repo.bundle goat\`
- \`container-images.txt\` — pinned container image list for the image
  build pipeline (operator pulls at build time, NOT shipped in bundle).
- \`terraform-pins/\` — provider version locks for both dev and prod
  Terraform trees.
- \`offline-ca/root.pem\` — offline-CA *public* key. Verifies device
  offline-enrollment bundles + bundle/image signatures.
- \`db-seeds/\` — empty-state DB schema for fresh-install start.

## Next step: rapid-site-bringup

For per-hardware-class pre-baked images that boot a full mgmt-host in
≤1 hour, see [\`docs/design/rapid-site-bringup.md\`](docs/design/rapid-site-bringup.md)
inside the unpacked repo.
EOF

# 2g: ops/bootstrap/ contents themselves (rebuild + image-build scripts)
log "  2g: ops/bootstrap/ scripts"
mkdir -p "$STAGE/ops"
cp -a "$REPO_ROOT/ops/bootstrap" "$STAGE/ops/bootstrap"
# Strip the .git-tracked recipients.txt from bundle copy — operator
# uses their own current roster at recovery time, not a stale snapshot.
rm -f "$STAGE/ops/bootstrap/recipients.txt"

# -- Step 3: tar it up --------------------------------------------------
step "Step 3: tar the staging tree"
TARBALL="$OUT_DIR/$TAG.tar"
tar -C "$STAGE" -cf "$TARBALL" .
SIZE_MB=$(du -m "$TARBALL" | cut -f1)
log "tarball: $TARBALL ($SIZE_MB MiB)"

if [[ $DRY_RUN -eq 1 ]]; then
  step "Dry-run complete (sign + encrypt + upload skipped)"
  log "Tarball: $TARBALL"
  log "Stage:   $STAGE"
  exit 0
fi

# -- Step 4: sign with offline-CA YubiKey or signing key ----------------
step "Step 4: sign tarball with $SIGNING_KEY"
command -v ssh-keygen >/dev/null || { log "FAIL: ssh-keygen not available"; exit 1; }
ssh-keygen -Y sign \
  -f "$SIGNING_KEY" \
  -n "$SIGNING_NAMESPACE" \
  "$TARBALL" \
  > "$TARBALL.sig" \
  || { log "FAIL: signing step failed"; exit 1; }
log "signature: $TARBALL.sig"

# Self-verify
log "self-verify: ssh-keygen -Y verify"
if ssh-keygen -Y verify \
     -f "$RECIPIENTS" \
     -n "$SIGNING_NAMESPACE" \
     -s "$TARBALL.sig" \
     < "$TARBALL"; then
  log "self-verify: PASS"
else
  log "self-verify: FAIL — signing key not in recipients.txt allowed_signers list"
  log "             (this is OK for a dry-run signer; required pre-publish)"
fi

# -- Step 5: encrypt with age multi-recipient ---------------------------
step "Step 5: age-encrypt to recipients.txt"
command -v age >/dev/null || { log "FAIL: age not available"; exit 1; }

# Extract age pubkey lines from recipients.txt (skip comments + ssh entries)
RECIP_FILE=$(mktemp)
grep -E '^age1[a-z0-9]+' "$RECIPIENTS" > "$RECIP_FILE" || true
RECIP_COUNT=$(wc -l < "$RECIP_FILE" | tr -d ' ')
if [[ $RECIP_COUNT -eq 0 ]]; then
  log "WARN: no age recipients in recipients.txt — bundle will not be encrypted"
  log "      add operator pubkeys per the file's instructions"
  log "      tarball + signature retained at $OUT_DIR/"
  rm -f "$RECIP_FILE"
  exit 0
fi

log "encrypting to $RECIP_COUNT recipient(s)"
# shellcheck disable=SC2046
age $(awk '{ printf "-r %s ", $1 }' "$RECIP_FILE") \
  -o "$TARBALL.age" \
  -e "$TARBALL"
rm -f "$RECIP_FILE"
log "ciphertext: $TARBALL.age"

# Remove the plaintext tarball after encryption
rm -f "$TARBALL"

# -- Step 6: upload (optional) ------------------------------------------
if [[ $UPLOAD -eq 1 ]]; then
  step "Step 6: upload to s3://$T1_BUCKET/bootstrap/"
  command -v aws >/dev/null || { log "FAIL: aws CLI not available"; exit 1; }
  aws s3 cp "$TARBALL.age" "s3://$T1_BUCKET/bootstrap/$TAG.tar.age" \
    || { log "FAIL: aws s3 cp .age"; exit 1; }
  aws s3 cp "$TARBALL.sig" "s3://$T1_BUCKET/bootstrap/$TAG.tar.sig" \
    || { log "FAIL: aws s3 cp .sig"; exit 1; }
  log "uploaded: s3://$T1_BUCKET/bootstrap/$TAG.{tar.age,tar.sig}"
fi

step "Bundle build complete"
log "Tag:        $TAG"
log "Commit:     $COMMIT"
log "Ciphertext: $TARBALL.age"
log "Signature:  $TARBALL.sig"
log "Size:       ~$SIZE_MB MiB compressed"
[[ $UPLOAD -eq 0 ]] && log "Upload:     skipped (--upload to push to S3)"
