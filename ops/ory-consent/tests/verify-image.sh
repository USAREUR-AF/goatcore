#!/bin/sh
# verify-image.sh — deploy-time check of a LOCAL consent image (GHSA-9v3v).
#
# Copies consent.js and scripts.hbs out of <image> with `docker create` +
# `docker cp` (the container is never started) and runs verify-patched.sh on
# them with VERIFY_PATCHED_ROOT, so a deploy can refuse an image whose consent
# handler still forwards self-editable identity.traits.groups / .aff into the
# JWT. It never pulls: the image must already be local (the ory role runs it
# after `docker compose pull`).
#
# Usage: verify-image.sh <image-ref> [<verify-patched.sh>]
# Exit:  0 the image carries the fixed readers
#        1 verify-patched.sh rejected the image
#        2 the check could not run (no docker, image not local, copy failed);
#          callers must treat this as a refusal too (fail closed)

set -eu

IMAGE="${1:?usage: verify-image.sh <image-ref> [<verify-patched.sh>]}"
VERIFY="${2:-$(dirname "$0")/verify-patched.sh}"

cannot() {
  echo "verify-image.sh: CANNOT CHECK $IMAGE — $*" >&2
  exit 2
}

[ -f "$VERIFY" ] || cannot "verify-patched.sh not found at $VERIFY"
command -v docker >/dev/null 2>&1 || cannot "docker not on PATH"
docker image inspect "$IMAGE" >/dev/null 2>&1 || cannot "image is not present locally (this check never pulls)"

ROOT=$(mktemp -d) || cannot "mktemp failed"
CID=""
cleanup() {
  [ -n "$CID" ] && docker rm -f "$CID" >/dev/null 2>&1 || true
  rm -rf "$ROOT"
}
trap cleanup EXIT INT TERM

CID=$(docker create --entrypoint=/bin/sh "$IMAGE" 2>/dev/null) || cannot "docker create failed"
mkdir -p "$ROOT/lib/routes" "$ROOT/views/partials"
docker cp "$CID:/usr/src/app/lib/routes/consent.js" "$ROOT/lib/routes/consent.js" >/dev/null 2>&1 \
  || cannot "no /usr/src/app/lib/routes/consent.js in the image"
docker cp "$CID:/usr/src/app/views/partials/scripts.hbs" "$ROOT/views/partials/scripts.hbs" >/dev/null 2>&1 \
  || cannot "no /usr/src/app/views/partials/scripts.hbs in the image"

if VERIFY_PATCHED_ROOT="$ROOT" sh "$VERIFY" "$IMAGE"; then
  exit 0
fi
exit 1
