#!/usr/bin/env bash
# test.sh — the local-registry recipe, end to end on this machine.
#
#   ops/recipes/local-registry/test.sh            Docker registry when Docker runs, else `crane registry serve`
#   REQUIRE_DOCKER=1 ops/recipes/local-registry/test.sh   (CI) fail instead of falling back
#
# 1. a registry comes up (local-registry.sh up, or crane's in-process registry)
# 2. two goat images of the lite set are mirrored; each is served at its pinned digest
# 3. negative control: verify of an image that was NOT mirrored fails, naming it
# 4. the pins a goatlite with image_registry renders (standup-images.py --preset lite
#    --image-registry) name exactly what the registry now serves
# 5. with Docker: `docker pull` of that rendered pin succeeds from the local registry,
#    and the pulled image carries the pinned digest
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
LR="$HERE/local-registry.sh"
fail() { echo "FAIL  $*" >&2; exit 1; }
pass() { echo "ok    $*"; }

PORT="${LOCAL_REGISTRY_TEST_PORT:-5055}"
ADDR="127.0.0.1:$PORT"
TO="$ADDR/usareur-af"
mode=""; crane_pid=""
cleanup() {
  [ -n "$crane_pid" ] && kill "$crane_pid" 2>/dev/null || true
  [ "$mode" = docker ] && docker rm -f goat-local-registry >/dev/null 2>&1 || true
  [ "$mode" = docker ] && docker volume rm goat-local-registry-test >/dev/null 2>&1 || true
}
trap cleanup EXIT

if docker info >/dev/null 2>&1; then
  mode=docker
  docker rm -f goat-local-registry >/dev/null 2>&1 || true
  "$LR" up --addr "$ADDR" --data goat-local-registry-test
elif [ "${REQUIRE_DOCKER:-0}" = 1 ]; then
  fail "REQUIRE_DOCKER=1 and the Docker daemon is not reachable"
else
  command -v crane >/dev/null || fail "neither a Docker daemon nor crane is available to serve a registry"
  mode=crane
  crane registry serve --address "$ADDR" >/dev/null 2>&1 &
  crane_pid=$!
  for _ in $(seq 1 20); do curl -fsS "http://$ADDR/v2/" >/dev/null 2>&1 && break; sleep 0.5; done
fi
curl -fsS "http://$ADDR/v2/" >/dev/null || fail "no registry answers on $ADDR"
pass "a registry answers on http://$ADDR/v2/ ($mode)"

out="$("$LR" mirror --to "$TO" --only signal-server --only goat-admit-audit --insecure)" || { echo "$out"; fail "mirror failed"; }
echo "$out" | sed 's/^/      /'
[ "$(echo "$out" | grep -c '^ok ')" = 2 ] || fail "mirror did not report two images served at their digests"
pass "two goat images mirrored, each served at its pinned digest"

if out="$("$LR" verify --to "$TO" --only postgres-spock --insecure 2>&1)"; then
  fail "verify passed for an image that was never mirrored: $out"
fi
echo "$out" | grep -q "postgres-spock" || fail "the failed verify does not name the missing image: $out"
pass "negative control: verify of an image not mirrored fails and names it"

rendered="$(python3 "$REPO/ops/scripts/standup-images.py" --preset lite --image-registry "$TO")"
for name in signal-server goat-admit-audit; do
  pin="$(printf '%s\n' "$rendered" | grep "^${TO}/${name}[:@]" || true)"
  [ -n "$pin" ] || fail "the lite pins rendered against $TO carry no $name"
  src="$("$LR" list --to "$TO" --only "$name" | cut -d' ' -f2)"
  [ "$pin" = "$src" ] || fail "rendered pin $pin differs from the mirrored destination $src"
done
printf '%s\n' "$rendered" | grep -q '^ghcr.io/usareur-af/' && fail "a goat image still names ghcr.io under image_registry"
pass "the rendered goatlite pins name exactly what the registry serves; no goat image left on ghcr.io"

if [ "$mode" = docker ]; then
  pin="$(printf '%s\n' "$rendered" | grep "^$TO/signal-server[:@]")"
  docker pull "$pin" >/dev/null || fail "docker pull $pin from the local registry failed"
  got="$(docker image inspect --format '{{join .RepoDigests "\n"}}' "$pin" | grep "^$TO/signal-server@" | head -1)"
  [ "${got##*@}" = "${pin##*@}" ] || fail "the pulled image's digest ${got##*@} is not the pin's ${pin##*@}"
  docker rmi "$pin" >/dev/null 2>&1 || true
  pass "docker pulled the rendered pin from the local registry at its pinned digest"
else
  echo "skip  docker pull (no Docker daemon here; CI runs it with REQUIRE_DOCKER=1)"
fi
echo "local-registry recipe: all checks passed"
