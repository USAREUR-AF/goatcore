#!/usr/bin/env bash
# local-registry.sh — run goat's pinned images from a registry you hold
# (docs/design/pluggable-dependencies.md row 13; README.md beside this file).
#
#   local-registry.sh up     [--addr 127.0.0.1:5000] [--data <dir>]
#   local-registry.sh mirror --to <host:port/namespace> [--member <group> | --preset lite] [--only <name>]... [--insecure]
#   local-registry.sh verify --to <host:port/namespace> [--member <group> | --preset lite] [--only <name>]... [--insecure]
#   local-registry.sh list   --to <host:port/namespace> [--member <group> | --preset lite] [--only <name>]...
#
# up      starts the CNCF Distribution registry (registry:3, digest-pinned below) in
#         Docker, published on --addr, its data in --data (default a named volume).
# mirror  copies every goat image the goatnet pins (ops/scripts/standup-images.py)
#         from ghcr.io/<org>/<name> to <host:port/namespace>/<name>, manifest bytes
#         unchanged, so the digest the pin names is the digest the registry serves.
#         A copy whose digest differs fails the run by name.
# verify  checks the registry serves every listed image at its pinned digest.
# list    prints `<source> <destination>` for each image, and copies nothing.
#
# The default image set is --preset lite (what any goatlite runs). A goatlite then
# declares `image_registry: <host:port/namespace>` in goatlite.yaml and the renderer
# pins every goat image there (ops/scripts/goatlite-render.py). Images that are not
# goat's (ghcr.io/spiffe/..., docker.io/...) are not copied: their pins keep their
# own registries.
#
# Copies with crane if present, else skopeo. --insecure lets either speak plain
# HTTP to the destination (a LAN registry without TLS; 127.0.0.1 needs no flag for
# Docker, but crane and skopeo still want it).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
REGISTRY_IMAGE="docker.io/library/registry:3.0.0@sha256:6c5666b861f3505b116bb9aa9b25175e71210414bd010d92035ff64018f9457e"
CONTAINER=goat-local-registry
ORG="${GHCR_ORG:-usareur-af}"

die() { echo "local-registry: $*" >&2; exit 2; }

cmd="${1:-}"; [ -n "$cmd" ] || die "usage: local-registry.sh up|mirror|verify|list ... (see the header)"
shift
addr=127.0.0.1:5000; data=""; to=""; member=""; preset=lite; insecure=0; only=()
while [ $# -gt 0 ]; do
  case "$1" in
    --addr) addr="$2"; shift 2 ;;
    --data) data="$2"; shift 2 ;;
    --to) to="${2%/}"; shift 2 ;;
    --member) member="$2"; preset=""; shift 2 ;;
    --preset) preset="$2"; member=""; shift 2 ;;
    --only) only+=("$2"); shift 2 ;;
    --insecure) insecure=1; shift ;;
    *) die "unknown argument: $1" ;;
  esac
done

if [ "$cmd" = up ]; then
  command -v docker >/dev/null || die "docker is not installed"
  if docker inspect "$CONTAINER" >/dev/null 2>&1; then
    docker start "$CONTAINER" >/dev/null
    echo "local-registry: $CONTAINER already exists; started"
    exit 0
  fi
  vol="${data:-goat-local-registry}"
  docker run -d --name "$CONTAINER" --restart unless-stopped \
    -p "$addr:5000" -v "$vol:/var/lib/registry" "$REGISTRY_IMAGE" >/dev/null
  for _ in $(seq 1 30); do
    if curl -fsS "http://$addr/v2/" >/dev/null 2>&1; then
      echo "local-registry: serving on http://$addr (data: $vol)"
      exit 0
    fi
    sleep 1
  done
  die "the registry did not answer on http://$addr/v2/ within 30 s (docker logs $CONTAINER)"
fi

case "$cmd" in mirror|verify|list) ;; *) die "unknown command: $cmd" ;; esac
[ -n "$to" ] || die "$cmd needs --to <host:port/namespace>, e.g. --to 127.0.0.1:5000/$ORG"
case "$to" in */*) ;; *) die "--to $to has no namespace: write <host:port>/<namespace>, e.g. $to/$ORG" ;; esac

if [ -n "$member" ]; then
  refs="$(python3 "$REPO/ops/scripts/standup-images.py" --member "$member" --ghcr-org "$ORG")"
else
  refs="$(python3 "$REPO/ops/scripts/standup-images.py" --preset "$preset" --ghcr-org "$ORG")"
fi

pairs=()
while IFS= read -r src; do
  [ -n "$src" ] || continue
  case "$src" in "ghcr.io/$ORG/"*) ;; *) continue ;; esac
  rest="${src#ghcr.io/"$ORG"/}"
  name="${rest%%[:@]*}"
  if [ "${#only[@]}" -gt 0 ]; then
    keep=0
    for o in "${only[@]}"; do [ "$o" = "$name" ] && keep=1; done
    [ "$keep" = 1 ] || continue
  fi
  pairs+=("$src $to/$rest")
done <<< "$refs"
[ "${#pairs[@]}" -gt 0 ] || die "no goat image selected (images under ghcr.io/$ORG/ in the pin list$( [ "${#only[@]}" -gt 0 ] && echo ", filtered by --only ${only[*]}"))"

if [ "$cmd" = list ]; then
  printf '%s\n' "${pairs[@]}"
  exit 0
fi

tool=""
if command -v crane >/dev/null; then tool=crane
elif command -v skopeo >/dev/null; then tool=skopeo
else die "neither crane nor skopeo is installed (either copies an image with its digest unchanged)"; fi

# Plain-HTTP flags for the destination, per tool (empty unless --insecure).
crane_tls=(); skopeo_src_tls=(); skopeo_dst_tls=()
if [ "$insecure" = 1 ]; then crane_tls=(--insecure); skopeo_src_tls=(--tls-verify=false); skopeo_dst_tls=(--dest-tls-verify=false); fi

# served_digest <ref-without-digest-or-with-tag> — the digest the destination serves
served_digest() {
  if [ "$tool" = crane ]; then
    crane digest ${crane_tls[@]+"${crane_tls[@]}"} "$1"
  else
    skopeo inspect --raw ${skopeo_src_tls[@]+"${skopeo_src_tls[@]}"} "docker://$1" | sha256sum | sed 's/^/sha256:/; s/ .*//'
  fi
}

fail=0
for p in "${pairs[@]}"; do
  src="${p%% *}"; dst="${p#* }"
  digest="${src##*@}"
  nametag="${dst%@*}"; nametag="${nametag#"$to"/}"   # <name>[:<tag>]
  case "$nametag" in
    *:*) repo="$to/${nametag%%:*}"; tag="${nametag#*:}" ;;
    *)   repo="$to/$nametag"; tag="pin-${digest#sha256:}" ;;   # a digest-only pin gets a tag naming its digest
  esac
  # The source by digest alone: skopeo refuses a reference carrying both a tag and a digest.
  srcname="${src%@*}"; last="${srcname##*/}"
  case "$last" in *:*) srcname="${srcname%:*}" ;; esac
  if [ "$cmd" = mirror ]; then
    if [ "$tool" = crane ]; then
      err="$(crane copy ${crane_tls[@]+"${crane_tls[@]}"} "$srcname@$digest" "$repo:$tag" 2>&1 >/dev/null)" \
        || { echo "could not copy $src -> $repo:$tag: $(printf '%s\n' "$err" | tail -n 1)"; fail=1; continue; }
    else
      err="$(skopeo copy --all --preserve-digests ${skopeo_dst_tls[@]+"${skopeo_dst_tls[@]}"} \
        "docker://$srcname@$digest" "docker://$repo:$tag" 2>&1 >/dev/null)" \
        || { echo "could not copy $src -> $repo:$tag: $(printf '%s\n' "$err" | tail -n 1)"; fail=1; continue; }
    fi
  fi
  got="$(served_digest "$repo:$tag" 2>/dev/null || true)"
  if [ "$got" = "$digest" ]; then
    echo "ok     $repo@$digest"
  else
    echo "could not verify $repo:$tag serves $digest (it serves: ${got:-nothing})"
    fail=1
  fi
done
[ "$fail" = 0 ] || { echo "local-registry: $cmd failed for the image(s) above" >&2; exit 1; }
echo "local-registry: $cmd ok, ${#pairs[@]} image(s) at $to with their pinned digests"
