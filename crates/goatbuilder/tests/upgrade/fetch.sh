#!/usr/bin/env bash
# fetch.sh — put the released goatbuilder binaries the upgrade test runs into a
# directory, checked against the checksums pinned beside this script.
#
#   crates/goatbuilder/tests/upgrade/fetch.sh <dir>
#
# For each release in releases.sha256, downloads goatbuilder-<os>-<arch> for
# this machine into <dir>/<version>/ and checks it against the pinned SHA-256.
# The pin is the one the release published in its SHA256SUMS, copied into this
# repository when the release was added here, so a changed or substituted
# download fails here rather than at the release's own word. A file already
# in <dir> that matches its pin is not downloaded again, so a cached <dir>
# needs no network. Then run the test with
#
#   GOATBUILDER_UPGRADE_DIR=<dir> cargo test -p goatbuilder --test upgrade
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
dir="${1:?usage: fetch.sh <dir>}"
base="${GOATBUILDER_RELEASES_URL:-https://github.com/usareur-af/goatcore/releases/download}"
os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m | sed 's/x86_64/amd64/; s/aarch64/arm64/')"
asset="goatbuilder-$os-$arch"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

found=0
while read -r want path; do
  case "$path" in */"$asset") ;; *) continue ;; esac
  found=1
  version="${path%%/*}"
  out="$dir/$path"
  mkdir -p "$dir/$version"
  if [ -f "$out" ] && [ "$(sha256 "$out")" = "$want" ]; then
    echo "cached  $path"
  else
    curl -fsSL -o "$out.part" "$base/goatbuilder/$version/$asset"
    got="$(sha256 "$out.part")"
    if [ "$got" != "$want" ]; then
      rm -f "$out.part"
      echo "fetch.sh: $path has SHA-256 $got; the pin in releases.sha256 is $want" >&2
      exit 1
    fi
    mv "$out.part" "$out"
    echo "fetched $path"
  fi
  chmod +x "$out"
done < "$here/releases.sha256"
[ "$found" = 1 ] || { echo "fetch.sh: no release in releases.sha256 ships $asset" >&2; exit 1; }
