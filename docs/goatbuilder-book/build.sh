#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# build.sh — build the goatbuilder book into goatbuilder-docs.tar.gz, the
# release asset the goat-docs role serves at docs.<site>.<zone>/goatbuilder/.
#
#   docs/goatbuilder-book/build.sh <path to a goatbuilder binary> <output .tar.gz>
#
# Composes the chapters from the canonical documents and the binary's --help
# (compose.sh), builds the book, and writes <output> and <output>.sha256.
# Needs mdbook on PATH. Called by .github/workflows/goatbuilder-release.yml.
set -euo pipefail

BIN="${1:?usage: build.sh <goatbuilder binary> <output .tar.gz>}"
OUT="${2:?usage: build.sh <goatbuilder binary> <output .tar.gz>}"
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"
mkdir -p "$(dirname "$OUT")"
OUT="$(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")"
HERE="$(cd "$(dirname "$0")" && pwd)"

"$HERE/compose.sh" "$BIN"
rm -rf "$HERE/book"
(cd "$HERE" && mdbook build -d book)
test -f "$HERE/book/index.html" || { echo "build.sh: book/index.html was not built" >&2; exit 1; }

name="$(basename "$OUT")"
tar czf "$OUT" -C "$HERE/book" .
(cd "$(dirname "$OUT")" && sha256sum "$name" > "$name.sha256")
echo "built $name"
