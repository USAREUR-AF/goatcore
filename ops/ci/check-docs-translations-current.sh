#!/usr/bin/env bash
# check-docs-translations-current.sh — lock the goat docs translations to
# the current English source (docs/design/goat-cli-hosted-docs.md).
#
# Re-extract the POT from the current English chapters, msgmerge each
# po/<lang>.po against it, and REPORT every language with untranslated or
# fuzzy entries — i.e. an English string changed and a translation has not
# caught up yet. Missing translations do not fail anything (ADR 1024 addendum
# 2026-11-02, docs/design/goat-cli-i18n.md "The currency gate reports and does
# not fail"): a per-language build falls back to English for those entries.
# The run fails only when the check itself cannot run (no POT, no po files).
# `--strict` makes drift fail too, for someone refreshing translations who
# wants a hard answer.
#
# When it reports drift, msgmerge has already marked the exact entries —
# refresh them and re-run. See docs/goat-cli-book/po/README.md.
#
# Needs: mdbook, mdbook-i18n-helpers (mdbook-xgettext), gettext (msgmerge/msgfmt).
set -euo pipefail

STRICT=0
[ "${1:-}" = "--strict" ] && STRICT=1

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BOOK="$REPO_ROOT/docs/goat-cli-book"
cd "$BOOK"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Fresh POT from the current English chapters (NOT the committed one — this
# catches "chapter edited but messages.pot/.po not refreshed" too).
MDBOOK_OUTPUT='{"xgettext": {"pot-file": "messages.pot"}}' \
  mdbook build -d "$TMP/potbuild" >/dev/null 2>&1 || {
    echo "FAIL: could not extract the POT (is mdbook-i18n-helpers installed?)"; exit 1; }
POT="$TMP/potbuild/messages.pot"
[ -f "$POT" ] || { echo "FAIL: POT not produced at $POT"; exit 1; }

# release-notes.md is kept English-only by design (docs/design/goat-cli-hosted-docs.md):
# every release appends an English entry there; the full guide is retranslated only
# on larger releases. Drop its strings from the POT so an English-only release-note
# edit never trips this gate. In a per-language build the untranslated strings fall
# back to English automatically. Portable awk filter (not msggrep, whose --location
# flag semantics vary across gettext builds): drop any PO entry whose `#:` location
# comment references release-notes.md.
#
# reference.md is likewise NOT gettext-translated (po/README.md): the command
# reference is regenerated per language from the binary itself at release
# (`goat --lang <X> docs --markdown`), so its localization lives in the Fluent
# catalogs, not the po files. The committed English snapshot exists for repo
# browsing and would otherwise demand 29 dead translations on every regen.
awk 'BEGIN{RS="";FS="\n"} {drop=0; for(i=1;i<=NF;i++) if($i ~ /^#:.*(release-notes|reference)\.md/) drop=1; if(!drop) printf "%s\n\n", $0}' \
  "$POT" > "$POT.filtered" && mv "$POT.filtered" "$POT"

shopt -s nullglob
pos=(po/*.po)
[ ${#pos[@]} -gt 0 ] || { echo "FAIL: no po/*.po files found"; exit 1; }

drift=0
for po in "${pos[@]}"; do
  lang="$(basename "$po" .po)"
  merged="$TMP/$lang.po"
  # Fuzzy-match against the fresh POT: changed English -> fuzzy, new -> untranslated.
  msgmerge --quiet "$po" "$POT" -o "$merged" 2>/dev/null
  stats="$(msgfmt --statistics -o /dev/null "$merged" 2>&1 || true)"
  if echo "$stats" | grep -qE "[1-9][0-9]* (untranslated|fuzzy)"; then
    echo "DRIFT: $lang — ${stats}"
    [ -n "${GITHUB_ACTIONS:-}" ] && echo "::warning title=docs translation drift::$lang — ${stats}"
    drift=1
  fi
done

if [ "$drift" -eq 0 ]; then
  echo "OK: all ${#pos[@]} docs languages are current with the English source."
else
  echo
  echo "Docs translations lag the English source in the languages above. They"
  echo "render the English text for those entries until refreshed; nothing waits"
  echo "on them. To refresh, see docs/goat-cli-book/po/README.md."
fi
if [ "$STRICT" -eq 1 ]; then exit "$drift"; fi
exit 0
