#!/usr/bin/env bash
# Block 68B build-image-orin.sh — pre-baked Orin image build.
#
# Builds a Tegra-flashable rapid-bringup image for the orin-mgmt OR
# orin-obs hardware class (single image; role selected at first boot
# via USB userdata.yml + serial-console prompt).
#
# Spec:    docs/design/rapid-site-bringup.md §4
# Spike:   docs/project/block-68a-config-input-spike.md
#
# Status: SKELETON. Hardware-dependent steps marked with TODO blocks.
# The bundle-mirror / repo-snapshot / docker-image-load / first-boot
# wire-up steps are runnable; the L4T flash-image assembly + Tegra-flash
# packaging steps need NVIDIA's BSP + a real Orin in recovery mode and
# are gated on the operator running this on a builder host with the
# JetPack 6.1 SDK installed and a target Orin connected.
#
# Builder-host requirements:
#   - x86_64 Ubuntu 22.04 (NVIDIA L4T flash tools target)
#   - JetPack 6.1 SDK Manager OR L4T_RELEASE_*.tbz2 + sample_root_filesystem
#   - sudo (the L4T flash sequence needs root for loop-mounting the rootfs)
#   - 80 GB free disk for the staging tree
#   - Operator's offline-CA YubiKey for the final image signature
#
# Usage:
#   ./build-image-orin.sh \
#     --bundle-tag bundle-vYYYYMMDD \
#     --output-dir /tmp/orin-mgmt-vYYYYMMDD \
#     --l4t-bsp-path /opt/nvidia/jetpack-6.1/Linux_for_Tegra
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# The image's install directories (the repo under $GOAT_HOST_OPT_DIR/repo, the
# state and log dirs first-boot.sh writes): the environment, else the tree's
# group_vars/all.yml, else /opt/goat and friends (lib-host-layout.sh).
# shellcheck source=ops/bootstrap/lib-host-layout.sh
. "$REPO_ROOT/ops/bootstrap/lib-host-layout.sh"
host_layout_load "$REPO_ROOT"
# The image namespace the cache pulls goat's own images from: GHCR_ORG, else
# the ghcr_org default group_vars/all.yml sets (the one every play renders a pin
# with), else usareur-af.
IMAGE_ORG="${GHCR_ORG:-$(sed -nE "s/^ghcr_org:.*[\"' ]([A-Za-z0-9][A-Za-z0-9._-]*)[\"']?[ }\"]*\$/\\1/p" "$REPO_ROOT/ops/ansible/group_vars/all.yml" 2>/dev/null | head -1 || true)}"
IMAGE_ORG="${IMAGE_ORG:-usareur-af}"

usage() {
  cat <<EOF
build-image-orin.sh — Block 68 pre-baked Orin image builder

Usage:
  $(basename "$0") --bundle-tag <tag> --output-dir <path> \\
                   --l4t-bsp-path <path>

Options:
  --bundle-tag <tag>      Bundle tag to build from (e.g. bundle-v20260506)
  --output-dir <path>     Where to assemble the build tree
  --l4t-bsp-path <path>   Path to L4T_RELEASE Linux_for_Tegra/ tree
  --skip-flash-image      Skip the L4T flash-image step (useful for CI smoke)
EOF
  exit 1
}

BUNDLE_TAG=""
OUTPUT_DIR=""
L4T_BSP=""
SKIP_FLASH=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle-tag) BUNDLE_TAG=$2; shift 2;;
    --output-dir) OUTPUT_DIR=$2; shift 2;;
    --l4t-bsp-path) L4T_BSP=$2; shift 2;;
    --skip-flash-image) SKIP_FLASH=1; shift;;
    -h|--help) usage;;
    *) echo "unknown arg: $1" >&2; usage;;
  esac
done

[[ -n "$BUNDLE_TAG" && -n "$OUTPUT_DIR" ]] || usage
if [[ $SKIP_FLASH -eq 0 ]]; then
  [[ -n "$L4T_BSP" && -d "$L4T_BSP" ]] || \
    { echo "--l4t-bsp-path required (or pass --skip-flash-image)" >&2; exit 1; }
fi

mkdir -p "$OUTPUT_DIR"
STAGE="$OUTPUT_DIR/stage"
mkdir -p "$STAGE"

step() { echo ""; echo "=== $* ==="; }

# -- Step 1: snapshot repo at signed bundle tag ---------------------------
step "Step 1: snapshot repo at $BUNDLE_TAG"
git -C "$REPO_ROOT" archive --format=tar "$BUNDLE_TAG" \
  | tar -x -C "$STAGE" -f -
# The unit and the script it starts name this build's layout.
host_layout_render "$STAGE/ops/bootstrap/first-boot/first-boot.service" \
  "$STAGE/ops/bootstrap/first-boot/first-boot.sh"
echo "  $(find "$STAGE" -type f | wc -l) files staged"

# -- Step 2: pre-load Docker image cache ---------------------------------
step "Step 2: assemble Docker image cache"
# Read service catalog from ops/serve/services.yaml + per-stack compose
# files; pull each image once and `docker save` to a tarball that the
# first-boot host loads with `docker load`.
mkdir -p "$STAGE/var-cache-docker-images"
IMAGES_FILE="$STAGE/var-cache-docker-images/manifest.txt"
{
  # Hard-coded core mgmt-stack list. Future improvement: derive from
  # ops/compose/mgmt-host/docker-compose.yml + observability roles.
  echo "ghcr.io/$IMAGE_ORG/postgres-spock:latest"
  echo "netbirdio/netbird-mgmt:patched-53b04e5"
  echo "netbirdio/signal:latest"
  echo "traefik:v3.0"
  echo "coredns/coredns:1.11.3"
  echo "eclipse/zenoh:1.9.0"
  echo "openobserve/openobserve:latest"
  echo "minio/minio:latest"
  echo "otel/opentelemetry-collector-contrib:latest"
  echo "prom/prometheus:latest"
  echo "prom/alertmanager:latest"
  echo "grafana/grafana:latest"
} > "$IMAGES_FILE"

while IFS= read -r img; do
  echo "  pulling $img"
  docker pull --platform linux/arm64 "$img" || \
    { echo "FAIL: docker pull $img" >&2; exit 1; }
done < "$IMAGES_FILE"

CACHE_TAR="$STAGE/var-cache-docker-images/images.tar"
echo "  saving combined image tarball to $CACHE_TAR"
# shellcheck disable=SC2046
docker save -o "$CACHE_TAR" $(< "$IMAGES_FILE")

# -- Step 3: stage first-boot artifacts ---------------------------------
step "Step 3: stage first-boot.service + script"
mkdir -p "$STAGE/etc/systemd/system" "$STAGE$GOAT_HOST_OPT_DIR"
cp "$STAGE/ops/bootstrap/first-boot/first-boot.service" \
   "$STAGE/etc/systemd/system/first-boot.service"
ln -sf /etc/systemd/system/first-boot.service \
   "$STAGE/etc/systemd/system/multi-user.target.wants/first-boot.service" 2>/dev/null \
   || true

# -- Step 4: L4T flash-image assembly -----------------------------------
if [[ $SKIP_FLASH -eq 1 ]]; then
  step "Step 4: L4T flash-image SKIPPED (--skip-flash-image)"
  echo "  staged tree at $STAGE — operator can rsync onto an Orin's"
  echo "  rootfs manually for smoke testing."
else
  step "Step 4: L4T flash-image assembly"
  # Precondition checks for the NVIDIA BSP layout.
  if [[ ! -d "$L4T_BSP/rootfs" ]]; then
    echo "FAIL: $L4T_BSP/rootfs missing — run NVIDIA's apply_binaries.sh first" >&2
    exit 1
  fi
  if [[ ! -x "$L4T_BSP/flash.sh" ]]; then
    echo "FAIL: $L4T_BSP/flash.sh not found or not executable" >&2
    exit 1
  fi

  echo "  4a: rsync staged tree onto L4T sample rootfs"
  if [[ $EUID -ne 0 ]]; then
    echo "  (rsync requires sudo — will prompt)"
  fi
  sudo rsync -aH --delete-excluded \
    --exclude '.git' \
    "$STAGE/" "$L4T_BSP/rootfs$GOAT_HOST_OPT_DIR/staged/"

  echo "  4b: ensure first-boot systemd units are enabled in target rootfs"
  sudo install -m 0644 \
    "$STAGE/ops/bootstrap/first-boot/first-boot.service" \
    "$L4T_BSP/rootfs/etc/systemd/system/first-boot.service"
  sudo install -m 0755 \
    "$STAGE/ops/bootstrap/first-boot/first-boot.sh" \
    "$L4T_BSP/rootfs$GOAT_HOST_OPT_DIR/repo/ops/bootstrap/first-boot/first-boot.sh"
  sudo ln -sf /etc/systemd/system/first-boot.service \
    "$L4T_BSP/rootfs/etc/systemd/system/multi-user.target.wants/first-boot.service"

  echo "  4c: invoke flash.sh in --no-flash mode to build the image"
  # BOARDID 3701 + BOARDSKU 0005 is the AGX Orin 64GB devkit. Operator
  # overrides via FLASH_BOARDID / FLASH_BOARDSKU env vars for other SKUs.
  FLASH_BOARDID=${FLASH_BOARDID:-3701}
  FLASH_BOARDSKU=${FLASH_BOARDSKU:-0005}
  FLASH_TARGET=${FLASH_TARGET:-jetson-agx-orin-devkit}
  (cd "$L4T_BSP" && sudo \
    BOARDID="$FLASH_BOARDID" \
    BOARDSKU="$FLASH_BOARDSKU" \
    ./flash.sh --no-flash --sign "$FLASH_TARGET" external)

  echo "  4d: copy signed image artifacts to OUTPUT_DIR"
  if [[ -d "$L4T_BSP/tools/kernel_flash/images" ]]; then
    sudo cp -a "$L4T_BSP/tools/kernel_flash/images" "$OUTPUT_DIR/flash-images"
    sudo chown -R "$USER:$USER" "$OUTPUT_DIR/flash-images"
    echo "  signed flash-images: $OUTPUT_DIR/flash-images/"
  else
    echo "  WARN: $L4T_BSP/tools/kernel_flash/images missing — flash.sh may have failed"
  fi

  echo ""
  echo "  Step 4 complete. To flash to a target Orin in recovery mode:"
  echo "    cd $L4T_BSP"
  echo "    sudo ./flash.sh --flash-only $FLASH_TARGET external"
fi

# -- Step 5: sign the resulting image ------------------------------------
step "Step 5: sign image with offline-CA YubiKey"
cat >&2 <<'EOF'
  TODO — once the L4T flash-image is assembled (Step 4), sign it with
  the offline-CA YubiKey:

    ssh-keygen -Y sign \
      -f /dev/<yubikey-piv-identity> \
      -n bootstrap-image \
      "$OUTPUT_DIR"/orin-mgmt-vYYYYMMDD.img \
    > "$OUTPUT_DIR"/orin-mgmt-vYYYYMMDD.img.sig

  Operator-fired step. Lives in docs/operations/image-builder-host.md.
EOF

step "build-image-orin.sh complete (skeleton-mode)"
echo "Staged tree: $STAGE"
echo "Next: operator runs L4T flash-image steps + signature ceremony"
echo "      per docs/operations/image-builder-host.md"
