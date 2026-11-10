#!/usr/bin/env bash
# push-from-host.sh — bridge a locally-built image (no real T0
# upstream) into Orin-C Forgejo (T2) as oci.goat/<service>:<tag>.
#
# Usage:
#   push-from-host.sh <source-host> <local-image-tag> <service-name> <site>
#
# Example:
#   push-from-host.sh \
#     operator@source-host.example \
#     ghcr.io/usareur-af/admit-audit:latest \
#     admit-audit \
#     <site>
#
# What it does:
#   1. SSH to <source-host> (via ProxyJump if configured in ssh config)
#   2. docker save <local-image-tag> → tarball, streamed via SSH
#      to Orin-C
#   3. docker load on Orin-C
#   4. tag for Forgejo: orin-c.<site>.<mesh-zone>:3030/goat/<service>:<digest-short>
#   5. docker push to Forgejo (requires GOAT_PUBLISHER_USER + GOAT_PUBLISHER_TOKEN env)
#   6. emit the resulting digest for canonical-set.yaml entry
#
# Composes with [Block 75E first-consumer migration]; replaces the
# multi-step manual recipe in ../README.md.
#
# Pre-requisites:
#   - SSH access to <source-host> with sudo NOPASSWD for docker
#   - SSH access to Orin-C of <site> with sudo NOPASSWD for docker
#   - Forgejo `goat` org exists; GOAT_PUBLISHER_USER + GOAT_PUBLISHER_TOKEN
#     have package:write on the org
#   - Orin-C Forgejo allows insecure-registry push from <source-host>
#     OR has HTTPS terminator (today: HTTP only on :3030)

set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "Usage: $0 <source-host> <local-image-tag> <service-name> <site>"
  echo "Example: $0 goat@192.168.1.132 ghcr.io/usareur-af/admit-audit:latest admit-audit vega"
  exit 1
fi

SOURCE_HOST=$1
LOCAL_TAG=$2
SERVICE=$3
SITE=$4

# Orin-C of the named site is the per-site obs/utility Orin per ADR 0826.
# Resolution: read from inventory or use mesh-DNS.
case "$SITE" in
  vega)
    ORIN_C_HOST="goat@192.168.1.253"
    FORGEJO_HOSTPORT="192.168.1.253:3030"   # LAN-direct; mesh-internal
    ;;
  *)
    echo "ERROR: site '$SITE' not recognized; only vega has an Orin-C today"
    exit 1
    ;;
esac

GOAT_PUBLISHER_USER=${GOAT_PUBLISHER_USER:-}
GOAT_PUBLISHER_TOKEN=${GOAT_PUBLISHER_TOKEN:-}
if [[ -z "$GOAT_PUBLISHER_USER" || -z "$GOAT_PUBLISHER_TOKEN" ]]; then
  echo "ERROR: GOAT_PUBLISHER_USER + GOAT_PUBLISHER_TOKEN must be set in env"
  echo "       (Forgejo OCI registry credentials for the goat org)"
  exit 1
fi

PROXY_JUMP_FLAG=""
if [[ "$SOURCE_HOST" == *"@192.168."* || "$ORIN_C_HOST" == *"@192.168."* ]]; then
  PROXY_JUMP_FLAG="-o ProxyJump=goat@100.127.1.23"
fi

echo "[1/5] Streaming docker save | docker load: $LOCAL_TAG → $ORIN_C_HOST"
ssh $PROXY_JUMP_FLAG "$SOURCE_HOST" "sudo docker save $LOCAL_TAG" | \
  ssh $PROXY_JUMP_FLAG "$ORIN_C_HOST" "sudo docker load"

echo "[2/5] Tagging for Forgejo on Orin-C"
TARGET_TAG="${FORGEJO_HOSTPORT}/goat/${SERVICE}:latest"
ssh $PROXY_JUMP_FLAG "$ORIN_C_HOST" "sudo docker tag $LOCAL_TAG $TARGET_TAG"

echo "[3/5] Logging in to Forgejo registry on Orin-C as $GOAT_PUBLISHER_USER"
echo "$GOAT_PUBLISHER_TOKEN" | ssh $PROXY_JUMP_FLAG "$ORIN_C_HOST" "sudo docker login $FORGEJO_HOSTPORT -u $GOAT_PUBLISHER_USER --password-stdin"

echo "[4/5] Pushing $TARGET_TAG"
ssh $PROXY_JUMP_FLAG "$ORIN_C_HOST" "sudo docker push $TARGET_TAG"

echo "[5/5] Reading back published digest"
DIGEST=$(ssh $PROXY_JUMP_FLAG "$ORIN_C_HOST" "sudo docker inspect $TARGET_TAG --format '{{index .RepoDigests 0}}'" | head -1)
echo
echo "Published: $DIGEST"
echo
echo "Update canonical-set.yaml with:"
echo "  - upstream_name: $LOCAL_TAG"
echo "    canonical_name: oci.goat/$SERVICE"
echo "    digest: $(echo "$DIGEST" | sed 's/.*@//')"
echo "    sign_anchor: goat-first-party"
echo "    consumers: [mgmt-host]"
echo
echo "And per-site .env override (e.g. group_vars/${SITE//-/_}_hosts/vars.yml):"
echo "  ADMIT_AUDIT_IMAGE: \"${FORGEJO_HOSTPORT}/goat/${SERVICE}@$(echo "$DIGEST" | sed 's/.*@//')\""
