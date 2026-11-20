#!/usr/bin/env bash
# Produce every binary the measurement runs, into $BIN_DIR, and record
# where each came from in $BIN_DIR/versions.txt.
#
#   netbird-relay   upstream Go relay, copied out of netbirdio/relay:<version>
#   netbird-server  upstream combined management + signal server,
#                   copied out of netbirdio/netbird-server:<version>
#   goat-relay      copied out of ghcr.io/usareur-af/goat-relay:<tag>
#   netbird-stock   the NetBird client built from tag v<version>
#   netbird-pr6774  the same tag with the change of netbirdio/netbird#6774
#
# Both clients are built here with the same toolchain image and flags, so the
# only difference between them is the change under test.
#
# Needs docker and git. Runs on the machine that drives the measurement.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

GOAT_RELAY_IMAGE=${GOAT_RELAY_IMAGE:-ghcr.io/usareur-af/goat-relay:v0.1.0}
PR6774_COMMIT=${PR6774_COMMIT:-c5730ed8dfae15b9a46b06f8a8e950ed5b5147b7}
GO_IMAGE=${GO_IMAGE:-golang:1.26.10-bookworm}

mkdir -p "$BIN_DIR" "$STATE_DIR/src"
versions="$BIN_DIR/versions.txt"
: > "$versions"

copy_out() { # image path dest
  docker pull -q "$1" >/dev/null
  local c; c=$(docker create "$1")
  docker cp "$c:$2" "$BIN_DIR/$3" >/dev/null
  docker rm "$c" >/dev/null
  printf '%s %s %s\n' "$3" "$(docker image inspect "$1" --format '{{index .RepoDigests 0}}')" \
    "$(sha256sum "$BIN_DIR/$3" | cut -d' ' -f1)" >> "$versions"
}

copy_out "netbirdio/relay:$NETBIRD_VERSION" /go/bin/netbird-relay netbird-relay
copy_out "netbirdio/netbird-server:$NETBIRD_VERSION" /go/bin/netbird-server netbird-server
copy_out "$GOAT_RELAY_IMAGE" /usr/local/bin/goat-relay goat-relay

src="$STATE_DIR/src/netbird"
if [ ! -d "$src/.git" ]; then
  git clone -q https://github.com/netbirdio/netbird.git "$src"
fi
git -C "$src" fetch -q --tags origin
git -C "$src" fetch -q origin "pull/6774/head"
git -C "$src" cat-file -e "$PR6774_COMMIT^{commit}" || die "PR 6774 commit $PR6774_COMMIT not found"

# The pull request branch has main merged into it; take only its own change
# (the diff from its merge base with main) and apply that to the release tag.
base=$(git -C "$src" merge-base "$PR6774_COMMIT" "$(git -C "$src" rev-parse origin/main)")
patch="$STATE_DIR/pr6774.patch"
git -C "$src" diff "$base" "$PR6774_COMMIT" > "$patch"

build() { # name worktree
  local name=$1 tree=$2 commit
  commit=$(git -C "$tree" rev-parse HEAD)
  docker run --rm -v "$tree:/src" -v "$BIN_DIR:/out" -v "$STATE_DIR/gocache:/root/.cache" \
    -w /src/client -e CGO_ENABLED=0 -e GOOS=linux -e GOARCH=amd64 -e GOFLAGS=-buildvcs=false "$GO_IMAGE" \
    go build -trimpath -tags load_wgnt_from_rsrc \
    -ldflags "-s -w -X github.com/netbirdio/netbird/version.version=$NETBIRD_VERSION -X main.commit=$commit" \
    -o "/out/netbird-$name" .
  printf 'netbird-%s source v%s %s %s\n' "$name" "$NETBIRD_VERSION" "$commit" \
    "$(sha256sum "$BIN_DIR/netbird-$name" | cut -d' ' -f1)" >> "$versions"
}

for name in stock pr6774; do
  tree="$STATE_DIR/src/netbird-$name"
  if [ ! -d "$tree" ]; then
    git -C "$src" worktree add -q --detach "$tree" "v$NETBIRD_VERSION"
  fi
  git -C "$tree" checkout -q --detach "v$NETBIRD_VERSION"
  git -C "$tree" reset -q --hard
  if [ "$name" = pr6774 ]; then
    git -C "$tree" apply "$patch"
  fi
  build "$name" "$tree"
done
printf 'pr6774.patch from %s (merge base %s) %s\n' "$PR6774_COMMIT" "$base" \
  "$(sha256sum "$patch" | cut -d' ' -f1)" >> "$versions"

cat "$versions"
