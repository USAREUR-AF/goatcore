# goat-cli internationalization — catalog layout, prose-vs-contract boundary, provenance

| Field | Value |
|---|---|
| Status | **Built.** goat-cli externalized and translated into 29 languages + the `tlh` easter egg (2026-06, ADR 1024); the shared error catalog and goatbuilder's help added, every catalog filled to the English arbiter and gated on every pull request (2026-10-29, §"CLI catalog currency, the error catalog, and goatbuilder"). Every non-English catalog is `review: machine`. |
| Scope | How `crates/goat-cli` localizes its human-facing prose behind an English arbiter while holding the machine-readable surface locale-invariant: the embedded-catalog stack and on-disk layout, the **explicit prose-vs-contract boundary list** (enumerated against the real code, not by category), language selection + fallback, the bilingual render style, the provenance/trust model, and the CI guard that keeps the boundary honest. |
| Related | [ADR 1024](../adr/1024-goat-cli-internationalization.md) (the decision), `goat-cli.md` (the binary; §Conventions item 10 = the help corpus, §Error UX = the error layers), [`goat-cli-tui.md`](goat-cli-tui.md) (the TUI shares this catalog), [ADR 1019](../adr/1019-goat-cli-unified-operator-binary.md), `edge-attachment-growth-strategy.md` item D, [`minimalism-under-network-constraints.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/minimalism-under-network-constraints.md) (the doctrine the bilingual render style inherits). |

## What this is

The implementation contract for ADR 1024. The ADR decided *that* `goat` localizes its prose behind an English arbiter with an invariant machine surface; this doc decides *how* — which crates, where the catalogs live, the exact string-by-string boundary, and the rules a reviewer applies to keep a new string on the correct side of it.

Two properties carry the whole design and are worth stating before the mechanics:

1. **English-completeness is enforced by the compiler, not by discipline.** The message macro fails the build if a key is absent from the English catalog. "English is the arbiter and the fallback" is therefore a structural guarantee, not a convention someone has to remember.
2. **The boundary is by string *role*, not by surface.** A single line of output can be half prose and half contract — `POST /api/vendors/vendor-a/topics → 201 Created (portal-confirmed)` localizes the frame (`portal-confirmed`) while passing the method, path, status, and key through verbatim. The boundary is drawn at the level of *strings the program owns as prose* versus *tokens it must reproduce exactly*, and the two routinely coexist in one rendered line.

## The stack — standard Rust localization, not bespoke

Pinned (ADR 1024 decision 4 named Fluent as leading; this doc makes the crate choice final):

| Crate | Role |
|---|---|
| [`fluent`](https://crates.io/crates/fluent) / `fluent-bundle` | the [Fluent](https://projectfluent.org) runtime — locale-aware plurals, gender, interpolation, the `.ftl` message format |
| [`i18n-embed`](https://crates.io/crates/i18n-embed) | embeds the `.ftl` catalogs (via `rust-embed`) into the binary, negotiates the active language against a requested-language list, owns the `FluentLanguageLoader` |
| [`i18n-embed-fl`](https://crates.io/crates/i18n-embed) | the `fl!(LOADER, "message-id", arg = val)` macro — **compile-time checks the id exists in the fallback (English) catalog** |
| [`unic-langid`](https://crates.io/crates/unic-langid) | the `LanguageIdentifier` type used for selection + negotiation |
| [`rust-embed`](https://crates.io/crates/rust-embed) | compiles the `i18n/` tree into the binary — no runtime file reads, works air-gapped |
| [`cargo-i18n`](https://crates.io/crates/cargo-i18n) | dev/CI tool: validates catalogs, reports per-language coverage against English, drives the lint |

Nothing here is hand-rolled. OS-locale detection, embedding, plural rules, and coverage reporting all come from the library; we write only the selection-precedence glue (below), one panic guard around OS-locale detection (F-329 — `i18n-embed`’s desktop requester reaches `locale_config` 0.3.0, which `.unwrap()`s the locale parse and can abort the CLI before it prints anything), and the catalog content. This is the "standard stack, not a bespoke loader" the prompt and ADR both insist on.

**Why this stack and not the alternatives** — `gettext`/`.po` is the other mature option, rejected because Fluent's asymmetric plural/gender handling is what non-English member-nation and Slavic partner languages (Ukrainian's four-form plurals) actually need, and `i18n-embed-fl`'s compile-time fallback check is the mechanism that makes "English arbiter" a build invariant rather than a runtime hope. A bespoke `match locale { … }` loader was rejected outright per ADR 1024 — it reinvents negotiation and forfeits the coverage tooling.

## Catalog layout

The conventional `cargo-i18n` tree, rooted in the crate:

```
crates/goat-cli/
├── i18n.toml                       # cargo-i18n config (fallback_language = "en")
├── i18n-provenance.toml            # per-language review status + feedback channels (parameterized)
└── i18n/
    ├── en/
    │   └── goat_cli.ftl            # Tier 0 — canonical arbiter, always complete
    ├── fr/
    │   └── goat_cli.ftl            # Tier 1 — French (NATO official language)
    ├── uk/
    │   └── goat_cli.ftl            # Tier 1 — Ukrainian
    └── <de|es|it|pl|tr|…>/         # Tier 2 — full NATO-member set (machine-translated)
        └── goat_cli.ftl            #          each marked review=machine, warning-gated
```

`i18n.toml`:

```toml
fallback_language = "en"

[fluent]
assets_dir = "i18n"
```

**Message-id convention** — kebab-case, namespaced `<surface>-<thing>[-<role>]`, so an id is greppable back to its call site and a reviewer can see the surface at a glance:

```ftl
# i18n/en/goat_cli.ftl  (excerpt — the doctor proof-of-concept corpus)

doctor-step1-name = ZTOL membership
doctor-step1-remediation = { $err } — bring up netbird (`sudo netbird up --management-url <mgmt> --setup-key <key>`); check `netbird status`

doctor-step4-portal-token-rejected = HTTP { $status } — token expired or revoked; re-onboard via operator-onboard.yml

error-profile-missing-summary = no profile for context `{ $context }`
error-profile-missing-hint = run `goat profile init <bundle.cbor>`
```

Note the interpolations (`{ $err }`, `{ $status }`, `{ $context }`) and the backtick-quoted commands/paths inside the message: **those are contract tokens carried through the localized frame** — see the boundary rules below. The translator translates the prose around them and never the tokens themselves; an `.ftl` reviewer treats any backticked `goat …` / path / `E_*` / interpolation as do-not-translate.

**Id stability is itself a contract.** Message ids never change once shipped, the same way error codes don't — a renamed id silently drops every translation back to English fallback. Renames go through the same deprecation care as `--json` field renames.

## The prose-vs-contract boundary — enumerated

ADR 1024 decision 2 gives the boundary by category. This is the enumerated list a reviewer checks against, derived from the actual `crates/goat-cli` surfaces. **A string is localized only if it appears in the left column.**

### Localize (prose — routes through `fl!`)

| Surface | Concrete strings | Source today |
|---|---|---|
| **Command/flag help** | clap `about` / `long_about` / per-arg + per-subcommand doc summaries (the §Conventions item-10 corpus) | `main.rs` `#[command(about=…)]`, every `cmd/*.rs` doc-comment |
| **Error descriptions** | `GoatError::summary()` text + `hint()` text | `errors.rs:87` (`summary`), `errors.rs:106` (`hint`) |
| **Doctor prose** | every remediation hint + step summary **as rendered in pretty mode** (not the serialized field — see render-boundary below) | `cmd/doctor.rs` `print_pretty` (`format!` remediation strings, lines ~200–332) |
| **Status / inspect labels** | human verdict labels, glyph captions, roll-up summaries | `cmd/status.rs`, `cmd/inspect.rs`, `tui/diagnose.rs` |
| **Narrator frames** | the prose words in a cascade line — `portal-confirmed`, `cascade settled — …`, `Zenoh put accepted on …` | `narrator.rs` `Consequence` Display |
| **Confirmation prompts** | `--confirm` / destructive-delete / security prompts and their yes/no affordance | `cmd/topic.rs`, `cmd/pubsub.rs` (`--confirm`) |
| **Human runtime notes** | operator-facing `eprintln!` notes + `bail!`/`anyhow!` message text | swept per §Conventions item 10 across `cmd/*.rs` |

### Never localize (contract — locale-invariant by rule, enforced in CI)

| Class | Concrete tokens | Why |
|---|---|---|
| **`--json` payloads** | every field *name* and *value*, `schema_version` | scripts/`--jq`/`insta` snapshots pin to bytes |
| **Error codes** | the `E_*` strings (`E_PROFILE_MISSING`, …) | `errors.rs` `code()`; scripts grep them |
| **Cargo-style line labels** | the `error:` / `help:` / `code:` *prefixes* (not the text after) | `errors.rs` `DisplayHuman`; scripts `grep '^code:'` / `'^error:'` |
| **Command / subcommand / flag names** | `doctor`, `--json`, `--lang`, `pub`, … | clap identifiers; renaming them by locale breaks every invocation |
| **Machine step/field identifiers** | `StepOutcome.name` (`ztol-membership`, …), `--json` `verdict` values | shown in pretty *and* `--json` (`cmd/doctor.rs` `step_name`); a stable id, not prose |
| **Completion candidates** | `goat completion` + `_complete` stdout | shell-consumed, one per line |
| **Key expressions / prefixes** | Zenoh keys, topic keys, vendor prefixes | substrate identifiers |
| **Exit codes** | the numbers `0/2/3/4/5/6/124/130` | `exit_codes.rs`; CI branches on them |
| **Interpolated contract values** | `{ $context }`, `{ $err }`, `{ $status }`, `{ $endpoint }`, backticked `goat …` commands and paths inside a localized message | reproduced verbatim *inside* the localized frame |
| **clap's own chrome** | clap's `Usage:` / `Options:` / parser `error:` | not strings we own — English by ADR 1024 consequence; out of scope |

### The rules a reviewer applies

1. **Labels are anchors; text is prose.** In `errors.rs::DisplayHuman` the `error:` / `help:` / `code:` line prefixes stay literal English (a script does `grep '^code:'`); only the `summary()` / `hint()` *text* after the prefix localizes. This keeps both the grep anchors and the `E_*` code stable while the human sentence translates.
2. **A line is a localized frame with contract interpolations.** Narrator lines, doctor remediations, and most errors embed exact tokens (a key, a code, a count, an HTTP path). The frame is one `.ftl` message with `{ $var }` slots; the slot values are passed in verbatim and never appear in any catalog. A translator who sees `goat profile init <bundle.cbor>` in a message leaves it byte-identical.
3. **`--json` is a hard floor.** Any code path under `--json` (including `--narrate --json` NDJSON) emits zero localized bytes — not even the frame. Structured output is wholly contract. The bilingual mode (below) is likewise prose-only and never touches `--json`.
4. **When in doubt, it's contract.** A string a script might read is invariant. The cost of a wrongly-localized contract token is a silent locale-dependent automation break (ADR 1024 §Context point 1); the cost of a wrongly-invariant prose string is an operator reads English. The asymmetry says default to contract.

### The render-boundary rule (the one that bites)

Some human strings are also serialized into `--json` — so localizing *the string* would localize the machine surface. The worked example is `goat doctor`: `StepOutcome` derives `Serialize`, and `cmd/doctor.rs::run` emits the whole struct — `summary` and `hint` included — as the `--json` NDJSON line. The *same* `summary`/`hint` strings also feed the human `print_pretty` render. Routing them through `fl!` at construction would make `doctor --json` locale-dependent, breaking the `doctor_json_off_mesh_snapshot` insta test and the byte-stability CI guard.

The rule, therefore: **localize at the render boundary, not at value construction.** A field that is both serialized and displayed holds the **canonical English** in the struct (so `--json` stays byte-stable), and only the *pretty/human render path* (`print_pretty`, `DisplayHuman`, TUI widgets) substitutes the localized form. Mechanically, the displayed unit carries a stable **message-id + structured args**; the human renderer calls `fl!(id, args)` in the active locale, while the serialized path emits the canonical English (rendered from the `en` bundle, or the raw id — a schema choice). `errors.rs::DisplayHuman` is already on the right side of this line by construction: `GoatError` is rendered to stderr only and never serialized, so its `summary()`/`hint()` localize freely while `code()` and the line labels stay invariant. doctor is the cautionary case; the proof-of-concept (below) deliberately starts with the clean `errors.rs` surface and treats doctor as the second, render-boundary-aware conversion.

A corollary for `--json` schema evolution: if locale-stable *structured* consumption is ever wanted (a script that wants the message regardless of locale), the additive move is a new `msg_id` field on the `--json` object (behind a `schema_version` bump), never localizing the existing free-text field. The existing field stays canonical English forever.

## Language selection + fallback

One global flag, resolved once, mirroring the existing `--color` / `--context` shape operators already know.

**Precedence (highest first):** `--lang <code>` → `GOAT_LANG` env → OS locale (`LC_ALL` / `LC_MESSAGES` / `LANG`) → English.

- `--lang` is a `global = true` clap arg on `Cli` in `main.rs`, alongside `--color`, with `env = "GOAT_LANG"` so the flag and env collapse into one clap declaration (precedence: explicit flag beats env, clap's default behavior).
- OS-locale detection is `i18n-embed`'s `DesktopLanguageRequester` (reads the `LC_*`/`LANG` chain). We wrap it so an explicit `--lang`/`GOAT_LANG` *prepends* to the requested-language list rather than replacing the negotiation — a `--lang uk` with no `uk` catalog still negotiates down to `en`, never errors.
- Resolution happens **once in `main`**, and now **before** clap builds the command — because clap renders `--help`/`--version` and *exits inside `get_matches`*, so a language resolved only after parse would never reach the help screen. `i18n::preparse_lang()` peeks `--lang`/`GOAT_LANG` out of the raw argv (mirroring clap's flag-beats-env precedence for that one global arg; `--lang` is global so it is scanned across the whole argv, and a bare `--` ends option scanning), then `select_language` builds the requested-language vector per precedence and calls `i18n_embed::select(&LANGUAGE_LOADER, &available, &requested)`. The `LANGUAGE_LOADER` is the conventional process-global `static LANGUAGE_LOADER: Lazy<FluentLanguageLoader>`; every `fl!` call reads it. No locale is threaded through `Ctx` — it's ambient like the tracing subscriber, set before dispatch. (`preparse_lang` and `select_language` agree with what clap later parses into `cli.lang`, so the value is resolved exactly once.)
- **English fallback is automatic and total.** `i18n-embed` falls back per-message to the `fallback_language`, so a partial `uk` catalog renders translated where it can and English elsewhere — never a blank, never a key name (ADR 1024 decision 1). A language is therefore safe to ship partial.

**Help-text localization mechanism (the one non-obvious wiring) — landed.** clap-derive doc-comments are static `&str` baked at compile time, so they can't go through `fl!` directly. The pattern, now implemented as `i18n::localize_command`: after `Cli::command()` and before parse, walk the command tree and rewrite each `about` / `long_about` / global-flag help with `clap`'s `mut_arg` / `mut_subcommand`, each keyed to a `help-…` catalog id. The keys are string *literals* (the `fl!` macro compile-time-checks every one against the `en` catalog), so the rewrite is an explicit per-node list, not a loop — that compile-time check is the point: a dropped or typo'd key fails the build instead of silently falling back. The doc-comments on `Cli`/`Cmd` stay as the English source-of-truth and clap defaults, but `localize_command` runs unconditionally (English included), so the catalog — not the doc-comment — is what renders; a note on the `Cli` struct flags this so a future edit updates both.

- **Scope of the rewrite.** The surface every operator reads first: the root `about`/`long_about`, the six global flags, and each subcommand's one-line `about` (the `goat --help` `Commands:` list), plus `doctor`'s longer `--help` body. clap's *own* chrome (`Usage:`/`Options:`/`Commands:` headers, parser `error:` lines) stays English by ADR 1024. The hidden `_complete` (machine-consumed) is intentionally left English.
- **Deferred increment.** Per-subcommand *positional/flag* help on the deeper `goat <sub> --help` screens (e.g. `goat admin onboard --help`) is not yet externalized — it extends `localize_command` with the same `mut_subcommand(...).mut_arg(...)` mechanism and `help-cmd-<sub>-arg-<name>` keys. Bounded out for now as the largest, lowest-readership tail (advanced drill-down).

## Bilingual rendering (ADR 1024 decision 7)

For operationally-critical text the localized line aids comprehension but **English stays the binding contract** (ADR 1024 decision 6). The render style must therefore keep the English *plainly visible*, not de-emphasized.

**Chosen style: an explicit `(en: …)` line, not a dimmed gloss.**

```
error: профіль для контексту `default` відсутній
  (en: no profile for context `default`)
  help: запустіть `goat profile init <bundle.cbor>`
  (en: run `goat profile init <bundle.cbor>`)
  code: E_PROFILE_MISSING
```

Why the parenthetical line beats a dimmed-ANSI gloss:

- **It survives the pipe.** stderr from an infrastructure tool gets redirected, `grep`-ed, and logged. A dimmed gloss relies on ANSI color, which `NO_COLOR`, non-TTY, and minimal/air-gapped terminals strip — taking the *binding English* with it. The minimalism doctrine ("usable on first byte, no TTY-required affordance") forbids leaning on color for load-bearing content.
- **It keeps English authoritative, visually.** Dimming would render the *binding* text as the de-emphasized one — backwards for decision 6. Plain parenthetical English is co-equal and copy-pasteable.
- **The contract tokens line up.** `default`, `goat profile init <bundle.cbor>`, and `E_PROFILE_MISSING` are byte-identical on both lines — visually reinforcing that they're invariant.

**Scope of bilingual:**

- **Default-on subset** (ADR 1024 decision 7): errors, destructive/security confirmations, doctor remediation. These present `(en: …)` automatically when the active locale isn't English.
- **Global opt-in:** `--bilingual` flag / `GOAT_BILINGUAL` env turns it on for *all* prose (still prose-only).
- **Never:** everywhere-by-default; any `--json` path; the `code:` line (single, contract). When the active locale *is* English, bilingual is a no-op — no `(en:)` duplication of identical text.

## Provenance + trust model (ADR 1024 decision 6)

A stale or unreviewed translation in a security tool must be *visible*, not silently trusted. Three mechanisms, lightest first:

1. **English-completeness — compile-time.** `i18n-embed-fl`'s `fl!` macro fails the build on any id missing from English. Tier 0 cannot rot: the arbiter is complete by construction.
2. **Per-language coverage — CI.** `cargo i18n` reports each non-English catalog's coverage against English. Coverage < 100 % is *allowed* (partial languages are safe via fallback) but *reported*, so adoption decisions see the real state.
3. **Provenance header — per catalog.** Each non-English `goat_cli.ftl` carries a top-of-file comment block recording: source (translator / org / tooling), review status (`machine` / `human` / `native-reviewed`), and the **English-catalog git SHA the translation was last reconciled against**. A translation whose reconcile-SHA lags the current English SHA on keys it claims to cover is *stale* and flagged.

```ftl
### goat-cli — Ukrainian (uk)
### source: <translator/org>
### review: native-reviewed
### reconciled-against-en: <git-sha>
### coverage: see `cargo i18n` report
```

**For operationally-critical text, English is presented as authoritative regardless of provenance** — that is exactly what the bilingual default-on subset (errors, destructive confirmations, security instructions) delivers: the operator always sees the binding English alongside the aid. Provenance governs *trust display*; bilingual default-on governs *binding display*; they compose.

### Machine-translation warning + feedback (parameterized) — IMPLEMENTED

When a non-English locale that is **not** `native-reviewed` is active, the tool emits a one-line notice to stderr before output: the active language is machine-translated, the English is authoritative, and *here is where to report errors*. This is the operator-visible half of the provenance model — a stale or unreviewed translation is announced, not silently trusted.

Both the per-language review status and the feedback channels are **parameterized in `crates/goat-cli/i18n-provenance.toml`** (embedded via `include_str!`), so they update without code changes. The `[review]` table drives the warning (any language not `native-reviewed` warns); the `[feedback]` table holds the channels:

- `email` — the durable report channel. Empty in the embedded file: each goatnet publishes its own as `i18n_feedback_email` in `get.<site>/goatnet.toml` (group_var `goat_get_i18n_feedback_email`), which `goat context refresh` writes into the context's profile. Precedence: `GOAT_I18N_FEEDBACK_EMAIL`, then the active context's `i18n_feedback_email`, then this file; with none the notice names "your goat administrator".
- `repo` — the shared private partner repository (issues/PRs against the catalogs); omitted from the notice while unset.

Both channels are **also overridable at runtime per deployment** via `GOAT_I18N_FEEDBACK_EMAIL` / `GOAT_I18N_FEEDBACK_REPO`, so a partner standup points them at its own address/repo without a rebuild. The notice is suppressed by `GOAT_I18N_NO_WARN=1`, is stderr-only (never stdout or `--json`), and is skipped for the shell-completion paths. The notice text is itself a catalog message, so it *can* localize where translated and falls back to the authoritative English otherwise — but **by policy it stays English in every locale** (the trust anchor: "this output is machine-translated, the English is authoritative" is most trustworthy in the authoritative language). The auto-fill tool (`ops/scripts/translate-ftl.py`) treats `i18n-machine-translation-warning` as **arbiter-only** — never flagged as drift, never translated — and its golden (`tests/cmd/doctor_machine_translation_warning.toml`) asserts the English rendering under a non-English locale.

This is the mitigation that makes shipping a broad catalog set safe (see Tiering): every machine-translated language is announced as such with a route to fix it, rather than silently presented as trustworthy.

## CI guard

Keeps the boundary honest as new strings land (ADR 1024 §Implementation status, decision 2 "enforced in CI"). Three checks:

1. **No new hardcoded human-facing strings.** A lint (clippy-style custom lint or a scoped grep gate in `.github/workflows/`) rejects a new user-facing `eprintln!`/`println!`/`bail!`/clap-help literal that isn't routed through `fl!`. This is the same discipline §Conventions item 10 began, now blocking. Scoped to the human-output surfaces; the machine paths (`--json` writers, `code()`) are explicitly exempt.
2. **English-completeness + coverage.** The build already fails on a missing-from-English key (`fl!`); CI additionally runs `cargo i18n` and surfaces per-language coverage. The enforcement + fill layer is `ops/scripts/translate-ftl.py`: `--check` reports any locale catalog that has fallen behind the English arbiter (the *binary-string drift* that makes the generated docs command reference render English in translated locales — see `goat-cli-hosted-docs.md`); `--apply` fills the missing message-ids via the Anthropic Messages API, validating that every contract token (backticked `tokens`, `{ $var }` placeables, Fluent select syntax) survives translation verbatim and refusing any entry that drops one. It is **version-gated** in `.github/workflows/docs-translation-gate.yml`: a minor/major `goat-cli-v*` tag runs `--check` and fails on drift, so a sizeable release can't ship a half-translated reference; a patch release carries its delta in the English release notes instead. The fill runs locally (operator key) or in CI (`apply=true` dispatch + `ANTHROPIC_API_KEY` secret → PR). Not shipped in the binary — the air-gap invariant (no runtime translation) is unchanged.
3. **Machine-surface byte-stability under a non-English locale.** A `trycmd` golden run with `GOAT_LANG=uk` (and `LANG=uk_UA.UTF-8`) asserts that `--json` output, `E_*` codes, completion candidates, and exit codes are **byte-identical** to the English run. This is the test that would catch a contract token accidentally routed through the catalog — the one failure mode with teeth.

## Tiering

- **Tier 0 — English.** Canonical, compiler-complete, the fallback. Ships first as the externalized catalog.
- **Tier 1 — seed.** **French** (one of NATO's two official languages, the member-nation seed) + **Ukrainian**, proving the pipeline end-to-end with provenance recorded.
- **Tier 2 — full NATO-member set, machine-translated scaffolding.** All 28 distinct NATO-member languages ship now (Albanian, Bulgarian, Croatian, Czech, Danish, Dutch, Estonian, Finnish, French, German, Greek, Hungarian, Icelandic, Italian, Latvian, Lithuanian, Luxembourgish, Macedonian, Montenegrin, Norwegian, Polish, Portuguese, Romanian, Slovak, Slovenian, Spanish, Swedish, Turkish), each a complete catalog for the externalized surface, each marked `review: machine` and gated behind the machine-translation warning above. A language flips to `native-reviewed` in `i18n-provenance.toml` as a native speaker reviews it.

This **front-loads the catalog set** — a deliberate departure from a strictly demand-gated rollout (ADR 1024 decision 5's original posture). The departure is safe *because* the provenance marks + the runtime machine-translation warning (above) make every unreviewed catalog announce itself and carry a route to fix it: the failure mode the demand-gating guarded against — untrusted machine text silently presented as authoritative — is closed by the warning instead. The English arbiter remains the binding text throughout. New partner languages beyond NATO membership (e.g. additional partner-nation tongues) still arrive as additive catalogs per demand.

## Proof-of-concept: the error-render path, then `goat doctor`

The first end-to-end conversion (ADR 1024 §Implementation status "String externalization") targets the **error-render path** — `errors.rs::GoatError::summary()` / `hint()` rendered through `DisplayHuman` — deliberately, because it is the *clean* surface that proves the whole pipeline without the render-boundary entanglement:

- It is **human-only**: `GoatError` is rendered to stderr via `DisplayHuman` and **never serialized into any `--json`** (`main.rs::report_anyhow` → stderr; structured commands emit their own payloads, not the error). So localizing `summary()`/`hint()` cannot touch the machine surface — no `--json` to destabilize.
- It exercises the **highest-value boundary nuance** in one place: the `error:` / `help:` / `code:` line labels stay literal English (grep anchors), the `E_*` `code()` stays invariant, and only the sentence text localizes — the exact split ADR 1024 decision 2 turns on.
- It is the **default-on bilingual subset** (errors), so it also proves the `(en: …)` render in anger.

The proof wires the stack (deps + `i18n.toml` + `i18n/en/goat_cli.ftl`), adds `--lang`/`GOAT_LANG`/OS-locale resolution + the single resolved loader in `main`, routes `GoatError::summary()`/`hint()` through `fl!`, adds a partial `uk` catalog (proving negotiation + fallback), and leaves `code()` / exit codes / line labels invariant. A `trycmd`/test assertion confirms `E_*` codes and exit codes are byte-identical under `GOAT_LANG=uk`.

**`goat doctor` is the second conversion**, deliberately after errors because it forces the render-boundary discipline (above): its remediation hints — the "unreadable edge diagnostic" ADR 1024 centers on — are entangled with `--json` via `StepOutcome`. Converting doctor means localizing at `print_pretty` while `StepOutcome.summary`/`hint` keep canonical English for the byte-stable `--json` path, and the `doctor_json_off_mesh_snapshot` insta test stays green by construction. That is the worked proof that the render-boundary rule holds on a real entangled surface.

Neither conversion sweeps every string — full externalization (help text, all errors, status/inspect, narrator) follows once both patterns (clean human-only + render-boundary-aware) are proven.

**Status (2026-06-08):** the error-render path, `goat doctor`, the clap `--help` text, **and the `goat status` / `goat inspect` / `--narrate` runtime prose** are all externalized and translated into all 28 NATO-member languages + Ukrainian. doctor proved the render-boundary rule on a real entangled surface — its `StepOutcome.summary`/`hint` keep canonical English in `--json` (via the English-pinned `EN_LOADER`) while `print_pretty` renders the active locale, the `doctor_json_off_mesh_snapshot` staying byte-identical. The `--help` conversion added the pre-parse language resolution (`preparse_lang`, so the help screen itself localizes) and the runtime command-tree rewrite (`localize_command`): the root description, the six global flags, and every subcommand one-liner now render in the active locale, with clap's own chrome left English. The runtime-prose pass promoted the render-boundary macro (`tr2!`) from doctor.rs into `crate::i18n` and applied it to `status`'s verdict summaries (the `Verdict.summary` `--json` field stays English-pinned, `summary_loc` drives the glyph line); `inspect`'s finding/remedy/no-outcomes labels and the `--narrate` cascade lines use plain `fl!` (the `--json` paths there were already separate machine surfaces), with `narrator-settled` carrying per-language CLDR plural categories. Remaining surfaces to externalize: per-subcommand flag help on the deeper `goat <sub> --help` screens (the bounded-out tail), the confirmation prompts (`[y/N]` guards across admin/context/pubsub/topic/main — likely a shared `confirm_localized()` helper), and the larger mutating-command success/progress/error prose tail (admin/client/pubsub/topic/…).

**Easter-egg locale:** an opt-in Klingon (`tlh`) catalog rides the same machinery — a real ISO-639 code (passes the langid test) but never auto-selected from the OS locale, reachable only via `--lang tlh` / `GOAT_LANG=tlh`. It is a *partial* catalog (marquee strings only; the rest fall back to English) and is marked `easter-egg` in `i18n-provenance.toml` (phrasebook-grade, NOT native-reviewed, so the machine-translation notice still fires). Contract-token discipline is identical to every human locale; nothing reaches `--json`/exit codes. It is the canonical example that the i18n surface supports playful, opt-in locales at zero cost to the serious machine surface.

## CLI catalog currency, the error catalog, and goatbuilder

**Built 2026-10-29.** Three Fluent catalog sets now ship with the CLIs, all on this document's stack, one selection rule (`--lang` → `GOAT_LANG` → OS locale → English) and one provenance file (`crates/goat-cli/i18n-provenance.toml`):

| Catalog set | English arbiter | What it holds | Locales |
|---|---|---|---|
| `goat-cli` | `crates/goat-cli/i18n/en/goat_cli.ftl` | goat's own prose (help, doctor, status, errors, storm brake) | 29 + the `tlh` easter egg |
| `goat-errors` | `crates/goat-errors/i18n/en/errors.ftl`, **generated** from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write` | each catalog entry's title, what and causes (ADR 1199) | 29 |
| `goatbuilder` | `crates/goatbuilder/i18n/en/goatbuilder.ftl`, **generated** from the doc comments on `Cli`/`Cmd` by `GOATBUILDER_I18N_WRITE=1 cargo test -p goatbuilder i18n::` | help: the root, the global flags, every verb | 29 |

**What localizes, and what never does.** The rule is this document's boundary, applied to the two new sets:

| Localizes | Never localizes |
|---|---|
| an error entry's title, what and causes, rendered bilingual: the localized line, then `(en: …)` with the binding English | codes (`E_*`, `GB-*`, `X-*`), fixes (the commands an operator types), read paths, exit codes, the `what:`/`why:`/`fix:` labels, every JSON field of `explain -o json` |
| goatbuilder's help text, in a non-English locale only (English is clap's own rendering of the doc comments, byte for byte) | goatbuilder's verb output: report rows, verdict lines, `-o json|yaml` — parity gates and pipelines read them |
| the machine-translation notice (by policy English in every locale, as goat's) | anything inside backticks, every `{ $var }` |

goatbuilder's verb output stays English deliberately: its report rows are compared line for line with the scripts by the parity gates (ADR 1201) and parsed by pipelines, and their detail is mostly contract tokens (refs, paths, digests). The operator-facing *explanation* of a refusal or a doctor row reaches them in their language through the error catalog instead.

**The currency gate reports and does not fail.** `ops/scripts/translate-ftl.py` is the one tool for every set (`--catalog goat-cli|goatbuilder|goat-errors|all`). `--check` lists every English message missing from a shipped locale and exits 0, so a change that adds or edits English does not owe its translations, and a release does not wait for them. A locale that lacks a message renders it in English; one whose translation predates an English edit keeps showing the older text. `--apply` fills the gaps through the Anthropic Messages API when someone chooses to run it, 25 messages per request, and refuses any translation that loses a contract token.

## What this is NOT

- **Not polygloat.** polygloat translates *data formats* between vendor schemas and the canonical pivot (`edge-attachment-growth-strategy.md` §polygloat). This is human UI language. Zero shared machinery — a category distinction ADR 1024 §Context calls out explicitly.
- **Not runtime machine translation.** Catalogs are compiled in; no translation service, no network egress. Identical behavior on an air-gapped island and a connected laptop.
- **Not per-language binaries.** One binary per CLI; language resolved at runtime.
- **Not a localized machine surface.** `--json`, codes, names, completions, exit codes are invariant by rule and CI-enforced — the one move ADR 1024 refuses outright.
- **Not localizing clap's own chrome.** Scope is bounded to strings `goat` owns; clap's `Usage:`/`Options:`/parser-`error:` stay English.
- **Not a TUI-specific catalog.** The `goat-cli-tui` surfaces ([`goat-cli-tui.md`](goat-cli-tui.md)) localize their prose through *this same* catalog; TUI-displayed key expressions / IDs stay contract by the same boundary rules (ADR 1024 decision 2 — boundary by role, not surface).

## Living document

Updates land via PR. The next material revisions are the externalization + selection + CI-guard PR (which flips ADR 1024 Proposed → Accepted), and the Tier-1 seed-catalog PR (which records the first non-English provenance).
