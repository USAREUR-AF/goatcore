# goat CLI — hosted docs (`docs.nova`), a mesh-internal docs service

Status: proposed (design). Sibling to the download surface [`goat-cli-mesh-download.md`](goat-cli-mesh-download.md); composes `goat-cli.md` ([ADR 1019](../adr/1019-goat-cli-unified-operator-binary.md)), [`goat-cli-onboarding-and-delight.md`](goat-cli-onboarding-and-delight.md), [`goat-cli-i18n.md`](goat-cli-i18n.md), [`goat-cli-install.md`](../operations/goat-cli-install.md), [`goat-cli-error-codes.md`](goat-cli-error-codes.md), [`goat-cli-tui.md`](goat-cli-tui.md).

## Why

On-mesh vendors get the `goat` binary from `get.nova` and its built-in `--help`, but rich docs — the onboarding narrative, the concept model, the full command reference, install/verify runbooks, the error-code catalog — live as repo markdown a vendor can't (and shouldn't) reach. The CLI deserves a **browsable, searchable docs site, served mesh-internally**, that always matches the binary a vendor just downloaded.

## Surface

A new **`docs.nova.netbird.<zone>`** — a sibling to `get.nova`, the same shape: a static site behind nginx + Traefik, **mesh-only** by the shared middleware (never public), LE cert minted on first request via the `le` resolver. The `get.nova` landing page links to it ("Read the docs →"), and the docs site links back to the download one-liner.

## Content — two sources, one book

1. **Auto-generated command reference (anti-drift).** A generator walks the clap command tree of the *same tagged binary* and emits a markdown reference (every command, flag, default, and the hidden-vs-visible split documented honestly). Because it is generated from the released binary, the hosted reference can never drift from `goat --help`. Mechanism: a `goat docs --markdown` emitter (mirrors `goat completion <shell>` — introspects `Cli::command()`), run once at build time.
2. **Curated concept + guide chapters.** mdBook chapters that **compose the canonical design docs** (no forking — `{{#include}}` / build-time copy so the docs stay single-source per the document-boundary rule): Overview (`goat-cli.md`), Getting started (`goat-cli-onboarding-and-delight.md` — init/hello/tutor + the sandbox), Install & verify (`goat-cli-install.md`), Languages (`goat-cli-i18n.md`), Error codes (`goat-cli-error-codes.md`), Terminal UI (`goat-cli-tui.md`).

## Render — mdBook

[mdBook](https://rust-lang.github.io/mdBook/) (Rust-native, built-in search, themeable). `book.toml` + `src/SUMMARY.md` table of contents + the chapters above + the generated reference. `mdbook build` → static HTML in `book/`. Chosen over a bespoke renderer for the free search/nav and zero runtime (static files; nginx serves them).

## Build & distribution

Anti-drift hinges on building the docs from the **same commit/tag** as the binary. The `goat-cli-release.yml` pipeline gains a `docs` job (on the `goat-cli-v*` tag):

1. `goat docs --markdown > book/src/reference.md` (from the freshly-built binary).
2. `mdbook build`.
3. `tar czf goat-docs.tar.gz -C book .`; sha256 sidecar; attach to the GitHub Release alongside the binaries.

So every `goat-cli-v<semver>` release carries a matching `goat-docs.tar.gz` — docs and binary version together (the version is the release tag the asset hangs on).

## Deploy — `goat-docs` role (near-clone of `goat-get`)

A `goat-docs` ansible role + `goat-docs-deploy.yml`, structurally identical to `goat-get`: pull the `goat-docs.tar.gz` asset **on the host** (host egress; nothing multi-MB over the mesh), verify the sha256, unpack into the nginx webroot, render the Traefik dynamic config for `docs.nova.<zone>`, bring up via docker compose, easy-rollback ledger (ADR 0859). Defaults: `goat_docs_release_tag` (pinned, mirrors `goat_get_release_tag`), `goat_docs_hostname: "docs.{{ site_id }}.{{ site_dns_zone }}"`. **Bump path** identical to `get.nova`: cut the release → bump `goat_docs_release_tag` → re-run the deploy. Mesh DNS entry in `ops/dns/nova/mesh-hosts`.

## Phasing

1. **P1 — book + reference generator: built.** `goat docs --markdown` emits the full reference walked from the live clap tree (hidden eggs/`_complete` excluded, global flags documented once); `goat docs` points at the hosted site. The mdBook lives at `docs/goat-cli-book/` — `book.toml` (rust theme + a goat-amber `theme/goat.css`), `SUMMARY.md`, and purpose-written vendor-facing chapters (Welcome, Getting started, How the fabric works, Everyday tasks, Install & verify, Languages, Troubleshooting) plus the generated `reference.md` snapshot. The generator is unit-tested + golden-tested.
2. **P2 — release-pipeline docs job: built.** A `docs` job in `goat-cli-release.yml` builds goat, regenerates `reference.md` from that exact binary, runs `mdbook build`, and attaches `goat-docs.tar.gz` (+ `.sha256`) to the Release (wired into the `release` job's `needs` + asset staging). So every `goat-cli-v*` release carries a docs site matching its binary. (Re-dispatch the workflow on an existing tag to backfill the asset for a release cut before this job existed.)
3. **P3 — serve: built (deploy operator-fired).** The `goat-docs` role (`ops/ansible/roles/goat-docs/`) — a near-clone of `goat-get` that pulls `goat-docs.tar.gz` on the host, sha256-verifies it, unpacks the mdBook site into the nginx webroot, and routes `docs.nova.<zone>` via Traefik (mesh-only middleware, `le` resolver, easy-rollback ledger) — plus `goat-docs-deploy.yml` and the `docs.nova` `mesh-hosts` entry. Deploy: re-dispatch the release workflow on `goat-cli-v0.3.0` so the `docs` job attaches the asset, then `ansible-playbook ... -l <mgmt-host> goat-docs-deploy.yml`. (The `get.nova` index cross-link is a small follow-on.)

Each phase is independently shippable; the surface is dark until P3 deploys it.

## Localization (i18n)

The CLI is localized into 28 NATO-member languages + Ukrainian (+ Klingon); the docs site should match (ADR 1024). Two content kinds, very different effort:

- **The generated command reference** — its strings (every `--help` one-liner) are *already* translated in the CLI catalogs. So a per-language reference is nearly free.
- **The hand-written prose chapters** — not in any catalog; translating them is a real content campaign (~thousands of words × 29 languages), machine-translated → not-native-reviewed (the same honesty banner the CLI uses).

Phasing:
- **LP1 — localized reference: built.** `goat docs --markdown` now renders the *active-locale* command tree (`i18n::localize_command`), so `goat --lang <X> docs --markdown` emits the reference in language `X` (English with no `--lang`). The release `docs` job generates one `reference.md` per language for free.
- **LP2 — mdBook i18n scaffold + language picker: built.** `mdbook-i18n-helpers` (gettext: `po/messages.pot` → per-language `po/<lang>.po`); per-language books built in the release `docs` job (`docs.nova/<lang>/`); served by the `goat-docs` role. Plus an **elegant header language picker** on every page (a custom theme component): a dropdown of **flag + autonym** (`🇫🇷 Français`, `🇺🇦 Українська`, …; flag is a *visual aid*, the autonym is the identifier — so we never lean on the flag-≠-language anti-pattern), switching **keeps your place** (`/fr/<page>` ⇄ `/de/<page>`), persists the choice (`localStorage`), and respects `Accept-Language` on first visit. Judgment calls (config knobs): English → `🇬🇧` (vs `🇺🇸`); Klingon → `🖖 tlhIngan Hol` (no nation). `theme/built-langs.js` lists the built languages so the picker never links to a 404.
- **LP3 — translate the prose: built (all 29).** The hand-written chapters are translated into all 29 NATO-member + Ukrainian languages (`po/<lang>.po`, 509 prose strings each, `msgfmt`-valid, zero "fabric"→"factory" false friends); `built-langs.js` lists all 30 (en + 29).

### Currency report (translations versioned with the CLI)

Translations ship with the CLI version and their drift from English is reported on every book change. Missing or out-of-date translations do not fail a change or a release (ADR 1024 addendum 2026-11-02); an entry a language lacks renders in English:

- **Versioned together:** the docs tarball + every `po/<lang>.po` ship under the `goat-cli-v*` release tag, so a given binary's docs are exactly its translations.
- **Currency check** (`ops/ci/check-docs-translations-current.sh`, wired in `.github/workflows/docs-i18n-currency.yml`): on any PR touching `docs/goat-cli-book/**`, re-extract the POT from the *current* English chapters, `msgmerge` each `.po` against it, and report every language with fuzzy/untranslated entries as a warning. It fails only when the extraction itself breaks; `--strict` makes drift fail too, for someone refreshing translations.
- **Refresh workflow:** when the check reports drift, `msgmerge` has already marked the exact changed entries → re-translate just those (the fan-out scoped to fuzzy entries) → green. See `docs/goat-cli-book/po/README.md`.

The only remaining translation caveat is *quality* (machine-translated, not yet native-reviewed) — that note stays until native review, and is independent of version currency.

#### Gap the gate doesn't cover: binary-string drift (the v0.4.2→v0.4.3 lag)

The currency gate fires on changes to `docs/goat-cli-book/**` (the hand-written prose). But the **command reference** is regenerated at build time from the binary (`goat --lang <X> docs --markdown`), so when a *code* PR adds a new command or help string, no book file changes and the gate stays green — yet the regenerated reference in every non-English locale now carries untranslated strings. v0.4.3 hit this first: `--env` and `goat doctor --topic` shipped in the binary, their strings aren't translated, so a v0.4.3 docs build would render them in English inside otherwise-translated pages.

Handling (2026-06-10): the binary release tag is decoupled from the docs release tag for exactly this case. `goat_get_release_tag` advanced to `v0.4.3` (newest binary on get.nova/get.zagros); `goat_docs_release_tag` stays at `v0.4.2` (the fully-translated book remains the served base on docs.nova/docs.zagros). The 0.4.3 deltas are surfaced as **lightweight release notes** rather than a full retranslated rebuild — flag what changed, don't regenerate 30 books. The tags re-couple at the next release that lands the `.po` updates.

#### Release notes — the English-first, retranslate-on-big-changes flow (built)

The two-track answer, simplified: every release writes a short **English** entry in `src/release-notes.md`; the full guide is **retranslated only on larger releases**.

- **`src/release-notes.md`** is a normal book chapter (so it's searchable, themed, and in the nav), but it is **excluded from the currency gate**: `check-docs-translations-current.sh` strips any PO entry whose location references `release-notes.md` before the drift check (a portable awk filter — `msggrep --location` semantics vary across gettext builds). So an English-only release-note edit never demands 29 translations. In a per-language build the untranslated release-note strings fall back to English automatically, which is exactly what we want — the notes read in English on every locale, with a banner at the top of the chapter saying so.
- **Per release:** append an entry to `release-notes.md` (English). That's the whole obligation for a **PATCH** release (`x.y.Z`) — typically bug fixes and small additions; the translated guide keeps serving the prior tag and the notes communicate the delta.
- **On a larger (MINOR/MAJOR) release** (`x.Y.z` / `X.y.z`): do the full per-language retranslation pass, then re-couple `goat_docs_release_tag` to `goat_get_release_tag` so the translated guide catches up to the binary. The version number is load-bearing for this split, so a release that adds user-facing commands should bump the **minor**, not the patch (v0.4.3 added `--env` + `doctor --topic` as a patch — the lag it produced is why this flow exists).

#### Version-gated trigger + machine-translation catalog fill (built)

The trigger is automated in `.github/workflows/docs-translation-gate.yml`, on `goat-cli-v*` tag push (and `workflow_dispatch`), **separate from `goat-cli-release.yml`** so it never blocks the binary build:

- A `classify` job compares the tag to the previous `goat-cli-v*` tag. **PATCH** (`x.y.Z`) → no-op (release notes carry the delta in English). **MINOR/MAJOR** (`x.Y.z` / `X.y.z`) → run the `.ftl` coverage report.
- The report is `ops/scripts/translate-ftl.py --check`: it lists every message a locale catalog under `crates/goat-cli/i18n/` lacks against the English arbiter and exits 0 — binary-string drift, the gap the prose `.po` currency gate can't see (the reference is generated from the binary's Fluent catalogs, not the book). This is the "gate the regenerated reference" half.
- Filling the drift is `translate-ftl.py --apply` — machine translation through the Anthropic Messages API of the missing message-ids, validated so every contract token (backticked `tokens`, `{ $var }` placeables, select syntax) survives verbatim; a mismatch is refused, never written. It runs **locally** (operator's `ANTHROPIC_API_KEY`) or in CI via the workflow's `apply=true` dispatch, which opens a PR with the refreshed catalogs (needs the `ANTHROPIC_API_KEY` repo secret; without it the fill job skips with a notice). Because the catalogs are built into the binary at the tag, the fill is a **pre-tag** step (commit before tagging), not an auto-commit on tag push. Tool details: `docs/design/goat-cli-i18n.md` §"CI guard".

## Multi-site (not nova-a-only)

The docs site is **site-generic**: `docs.<site_id>.<site_dns_zone>` falls out of the role's `site_id`/`site_dns_zone`, and the deploy play is parameterized — `ansible-playbook ... -e target=<site>_hosts -e goat_docs_release_tag=goat-cli-v<semver> goat-docs-deploy.yml` deploys it to any goatnet. The sibling `goat-get` play already does this (`target | default('nova_hosts')`).

**Deployed instances:**
- **nova-a** — `docs.nova.netbird.goat.example` (+ `get.nova`).
- **ZAGROS** — `docs.zagros.netbird.zagros.example` (+ `get.zagros`), deployed 2026-06-09 (`-e target=zagros_hosts`). The roles needed **zero changes**.

The one non-role step per site is the mesh DNS record (`docs.<site>`/`get.<site>` → the mgmt mesh IP) in that site's CoreDNS zone — on ZAGROS this lives in the host `mesh-hosts`; fold it into the site's `ops/dns/<site>/` source as that site's DNS substrate lands in the repo.

## What this is NOT
- **Not public.** Mesh-only by the shared middleware, exactly like `get.nova`.
- **Not a second source of truth.** The book *composes* the canonical `docs/` markdown; it never forks it. The command reference is *generated*, never hand-maintained.
- **Not versionless.** The hosted docs tag tracks the CLI release tag, so a vendor reading `docs.nova` sees the docs for the binary `get.nova` is serving.
