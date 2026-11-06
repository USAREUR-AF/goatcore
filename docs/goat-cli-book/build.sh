#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# build.sh — build the goat field guide into goat-docs.tar.gz, the release
# asset the goat-docs role serves at docs.<site>.<zone>.
#
#   docs/goat-cli-book/build.sh <path to a goat binary> <output .tar.gz>
#
# Per language: the command reference is generated from THIS binary in that
# locale (`goat --lang <X> docs --markdown`, so it matches `goat --help`), and
# the hand-written prose is translated by the gettext preprocessor from
# po/<lang>.po. English builds to the site root, each translated language to
# /<lang>/. theme/built-langs.js tells the language picker which languages
# exist, so it never links to a 404. Writes <output> and <output>.sha256.
#
# Needs mdbook and, for the translated builds, mdbook-gettext
# (mdbook-i18n-helpers) on PATH. Called by .github/workflows/goat-release.yml.
set -euo pipefail

GOAT="${1:?usage: build.sh <goat binary> <output .tar.gz>}"
OUT="${2:?usage: build.sh <goat binary> <output .tar.gz>}"
GOAT="$(cd "$(dirname "$GOAT")" && pwd)/$(basename "$GOAT")"
mkdir -p "$(dirname "$OUT")"
OUT="$(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")"
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

rm -rf book
langs="en"
for po in po/*.po; do [ -e "$po" ] || continue; langs="$langs $(basename "$po" .po)"; done
printf 'window.GOAT_DOCS_LANGS = [%s];\n' \
  "$(for l in $langs; do printf '"%s",' "$l"; done | sed 's/,$//')" > theme/built-langs.js
for lang in $langs; do
  if [ "$lang" = "en" ]; then
    "$GOAT" docs --markdown > src/reference.md
    mdbook build -d book
  else
    "$GOAT" --lang "$lang" docs --markdown > src/reference.md
    MDBOOK_BOOK__LANGUAGE="$lang" mdbook build -d "book/$lang"
  fi
done
test -f book/index.html || { echo "build.sh: book/index.html was not built" >&2; exit 1; }

name="$(basename "$OUT")"
tar czf "$OUT" -C book .
(cd "$(dirname "$OUT")" && sha256sum "$name" > "$name.sha256")
echo "built $name: languages $langs"
