# Translations (po/)

The goat docs site is localized with [mdbook-i18n-helpers] (gettext).

- `messages.pot` — the extracted source strings (regenerate with the
  command below after editing chapters).
- `<lang>.po` — per-language translations (one per built language). Empty
  `msgstr` entries fall back to English, so a partially-translated `.po`
  still builds.

The **command reference** chapter is NOT translated here — it is generated
per language from the binary itself (`goat --lang <X> docs --markdown`), so
it always matches `goat --help`. The gettext pass only touches the
hand-written prose chapters. The currency gate strips `reference.md` strings
from the POT (same as release-notes) so regenerating the committed English
snapshot never demands 29 dead translations; the stale reference entries in
the `.po` files were retired to obsolete (`#~`) when this landed (2026-07-07).

The **release notes** chapter (`src/release-notes.md`) is also NOT translated
— it is kept English-only on purpose (every release adds an English entry; the
full guide is retranslated only on larger releases). The currency gate strips
its strings from the POT before the drift check, so an English-only release-note
edit never demands 29 translations; in a per-language build they fall back to
English. See the "Release notes" flow in `docs/design/goat-cli-hosted-docs.md`.

## Regenerate the template
```bash
cd docs/goat-cli-book
MDBOOK_OUTPUT='{"xgettext": {"pot-file": "messages.pot"}}' mdbook build -d po
```

## Add / update a language
```bash
msgmerge --update po/<lang>.po po/messages.pot   # or msginit for a new one
# translate the msgstr entries, then:
MDBOOK_BOOK__LANGUAGE=<lang> mdbook build -d book/<lang>
```

[mdbook-i18n-helpers]: https://github.com/google/mdbook-i18n-helpers

## Currency report (CI)

`ops/ci/check-docs-translations-current.sh` (workflow `docs-i18n-currency.yml`)
reports, without failing, every language whose translation has not caught up
with an English change; those entries render in English until refreshed
(`--strict` makes drift fail). When it reports drift, `msgmerge` has marked the
exact fuzzy/untranslated entries; refresh just those and re-run.
