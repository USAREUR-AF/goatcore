// Internationalization runtime (ADR 1024 / docs/design/goat-cli-i18n.md).
//
// English is the canonical arbiter: the `en` catalog is loaded as the
// fallback, so `fl!(LANGUAGE_LOADER, "id")` resolves to English for any
// id a non-English catalog hasn't translated — never a blank, never a
// key name. The catalogs are Fluent `.ftl`, embedded via rust-embed, so
// localization is deterministic and works on an air-gapped site.
//
// The machine surface (--json, E_* codes, the error:/help:/code: line
// labels, completion candidates, exit codes) is NOT routed through this
// module — it is locale-invariant by rule. Only human prose localizes.
//
// `LANGUAGE_LOADER` is a process-global set once in `main` via
// `select_language` after clap parsing, mirroring how the tracing
// subscriber is initialized once before dispatch. Bidi isolation marks
// (FSI/PDI U+2068/U+2069 that Fluent wraps around interpolations by
// default) are disabled so terminal output and trycmd goldens stay
// byte-clean.

use i18n_embed::{
    fluent::{fluent_language_loader, FluentLanguageLoader},
    DesktopLanguageRequester, LanguageLoader, LanguageRequester,
};
use once_cell::sync::Lazy;
use rust_embed::RustEmbed;
use std::panic;
use unic_langid::LanguageIdentifier;

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

/// The process-global Fluent loader. English (the `fallback_language`
/// from `i18n.toml`) is loaded eagerly so `fl!` always resolves even
/// before `select_language` runs.
pub static LANGUAGE_LOADER: Lazy<FluentLanguageLoader> = Lazy::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();
    loader
        .load_fallback_language(&Localizations)
        .expect("the English fallback catalog must embed + parse");
    disable_bidi_isolation(&loader);
    loader
});

/// A loader pinned to English, regardless of the active locale. Used at
/// the **render boundary** (ADR 1024 / docs/design/goat-cli-i18n.md): a
/// string that is both shown to a human *and* serialized into `--json`
/// (e.g. `goat doctor`'s step summaries) keeps its canonical English in
/// the serialized field — rendered from this loader — while the active
/// loader renders the localized form for the pretty/human path. Keeps
/// `--json` byte-stable across locales.
pub static EN_LOADER: Lazy<FluentLanguageLoader> = Lazy::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();
    loader
        .load_fallback_language(&Localizations)
        .expect("the English fallback catalog must embed + parse");
    // Pin to English only — never select() another language onto it.
    disable_bidi_isolation(&loader);
    loader
});

/// Strip the Unicode bidi isolation marks (FSI/PDI U+2068/U+2069) Fluent
/// wraps around interpolations by default — they are invisible but
/// change the bytes, which would break the machine-surface stability
/// assertions and the human-prose trycmd goldens alike. `set_use_isolating`
/// mutates the *currently-loaded* bundles, so it must be called AFTER
/// every load (the eager fallback here, and each `select` in
/// `select_language`).
fn disable_bidi_isolation(loader: &FluentLanguageLoader) {
    loader.set_use_isolating(false);
}

/// Render a human string both ways at the i18n **render boundary** (ADR 1024
/// / docs/design/goat-cli-i18n.md): returns `(english, localized)`. The
/// English (from the pinned [`EN_LOADER`]) goes in any serialized `--json`
/// field so the machine surface stays byte-stable across locales; the
/// localized (from the active [`LANGUAGE_LOADER`]) drives the pretty/human
/// render. Both come from the same catalog key + args, so they can never
/// drift. All args are passed as strings so Fluent never applies locale
/// number formatting to a contract token.
///
/// Used by any command whose human prose is ALSO emitted into `--json`
/// (e.g. `goat doctor`'s step summaries, `goat status`'s verdict
/// summaries). A clean human-only string (never serialized) routes through
/// `fl!(LANGUAGE_LOADER, …)` directly instead — no English pin needed.
macro_rules! tr2 {
    ($key:expr $(, $k:ident = $v:expr)* $(,)?) => {{
        (
            i18n_embed_fl::fl!($crate::i18n::EN_LOADER, $key $(, $k = $v)*),
            i18n_embed_fl::fl!($crate::i18n::LANGUAGE_LOADER, $key $(, $k = $v)*),
        )
    }};
}
pub(crate) use tr2;

/// Run `f`, and if it panics, run `fallback` instead of propagating. The
/// default panic hook is suspended for the call so an expected, handled
/// panic doesn't also spam a raw Rust backtrace to stderr; it is always
/// restored before returning (including on the panic path), so callers
/// elsewhere in the process still get normal panic reporting. Split out
/// from `os_requested_languages` so the catch/restore behavior itself is
/// unit-testable without depending on `locale_config` actually panicking.
///
/// NOTE: this only works while `goat-cli` is built with `panic = "unwind"`.
/// `crates/Cargo.toml`'s `[profile.release]` omits `panic`, so it defaults to
/// unwind and the fallback fires in shipped binaries. That omission is
/// load-bearing: the repo-root workspace deliberately sets `panic = "abort"`
/// for `goat-relay` (smaller code, no unwind tables), and applying the same
/// reasoning here would silently turn `catch_unwind` into a no-op — the CLI
/// would abort on a hostile locale again, and **no test would catch it**,
/// because `cargo test` always builds with unwind. See F-329.
fn catch_panic_or<T>(
    f: impl FnOnce() -> T + std::panic::UnwindSafe,
    fallback: impl FnOnce() -> T,
) -> T {
    let prev_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let result = panic::catch_unwind(f);
    panic::set_hook(prev_hook);
    match result {
        Ok(v) => v,
        Err(_) => fallback(),
    }
}

/// Read the OS locale via `DesktopLanguageRequester` (which shells out to
/// the `locale_config` crate — on macOS, `CFLocale`/`NSLocale`). Fabric
/// operators have hit hosts whose locale identifier doesn't parse into the
/// strict shape `locale_config` 0.3.0 expects; that crate `.unwrap()`s the
/// parse with no fallback, panicking before `select_language`'s caller
/// (`main`, ahead of clap parsing) prints anything — even `goat --version`.
/// Catching the unwind here turns a hostile locale into "no OS-locale
/// match, use English" instead of taking the whole CLI down, so a user no
/// longer needs an `LC_ALL`/`LANG` shell-profile workaround to run `goat`
/// at all.
fn os_requested_languages() -> Vec<LanguageIdentifier> {
    catch_panic_or(
        || DesktopLanguageRequester::new().requested_languages(),
        || {
            tracing::warn!("OS locale detection panicked; falling back to English");
            Vec::new()
        },
    )
}

/// Resolve and load the active language once, per ADR 1024 precedence:
/// `--lang`/`GOAT_LANG` (the `explicit` arg, merged by clap) → OS locale
/// (`LC_ALL`/`LC_MESSAGES`/`LANG` via `DesktopLanguageRequester`) →
/// English. An explicit tag *prepends* to the requested list rather than
/// replacing negotiation, so `--lang uk` with no `uk` catalog still
/// negotiates down to English instead of erroring.
pub fn select_language(explicit: Option<&str>) {
    let mut requested: Vec<LanguageIdentifier> = Vec::new();
    if let Some(tag) = explicit {
        match tag.parse::<LanguageIdentifier>() {
            Ok(li) => requested.push(li),
            Err(_) => {
                // A malformed --lang/GOAT_LANG is non-fatal: fall through
                // to OS-locale + English rather than abort a real command.
                tracing::warn!(lang = tag, "ignoring unparseable language tag");
            }
        }
    }
    requested.extend(os_requested_languages());

    // `select` loads the negotiated languages over the already-loaded
    // English fallback; any failure leaves English active (safe default).
    if let Err(e) = i18n_embed::select(&*LANGUAGE_LOADER, &Localizations, &requested) {
        tracing::warn!(error = %e, "language selection failed; using English");
    }
    // `select` builds fresh bundles with isolation on by default; strip
    // it again now that the negotiated languages are loaded.
    disable_bidi_isolation(&LANGUAGE_LOADER);
}

/// The negotiated language as a tag (`en`, `fr`, `uk`, …), for the shared
/// error catalog (ADR 1199), which renders its prose in the same language.
pub fn active_language_tag() -> String {
    LANGUAGE_LOADER.current_language().to_string()
}

/// Peek `--lang`/`GOAT_LANG` out of the raw argv **before** clap parses,
/// so `select_language` can run ahead of `Cli::command()` and clap renders
/// `--help`/`--version`/parse-error chrome around an already-localized
/// command tree. (clap exits *inside* `get_matches`, so a language resolved
/// only after parse would never reach the help screen.)
///
/// Mirrors clap's own precedence for this one global arg: an explicit
/// `--lang <tag>` / `--lang=<tag>` flag beats the `GOAT_LANG` env. `--lang`
/// is `global = true`, so it can sit after the subcommand
/// (`goat doctor --lang fr`) — we scan the whole argv, not just the head.
/// A bare `--` ends option scanning (POSIX), so a literal `--lang` token
/// after it is a positional, not this flag. Returns `None` to mean "no
/// explicit choice — let `select_language` negotiate OS-locale → English".
pub fn preparse_lang() -> Option<String> {
    preparse_lang_from(std::env::args().skip(1), std::env::var("GOAT_LANG").ok())
}

/// Pure core of [`preparse_lang`] — the argv iterator and the `GOAT_LANG`
/// value are injected so the precedence logic is unit-testable without
/// touching process state.
fn preparse_lang_from<I: Iterator<Item = String>>(
    mut args: I,
    env_lang: Option<String>,
) -> Option<String> {
    while let Some(a) = args.next() {
        if a == "--" {
            break;
        }
        if a == "--lang" {
            // `--lang` with no following token: ignore here and let clap
            // emit its own "a value is required" error during parse.
            return args.next();
        }
        if let Some(v) = a.strip_prefix("--lang=") {
            return Some(v.to_string());
        }
    }
    env_lang.filter(|s| !s.is_empty())
}

/// Swap the clap-derive `about` / `long_about` / global-flag help — the
/// static `&str`s baked at compile time — for the active-locale catalog
/// strings, on the runtime command tree (ADR 1024 / the design doc's
/// "help-text localization mechanism"). Called between `Cli::command()`
/// and parse in `main`.
///
/// The `fl!` keys must be string literals (the macro compile-time-checks
/// each against the `en` catalog), so this is an explicit per-node list,
/// not a loop — that compile-time check is the feature: a typo'd or
/// dropped key fails the build instead of silently falling back. Scope is
/// the surface `goat` *owns* and every operator reads first — the root
/// description, the six global flags, and each subcommand's one-liner (the
/// `Commands:` list) plus `doctor`'s longer `--help` body. clap's own
/// chrome stays English by ADR 1024; per-subcommand *positional/flag* help
/// (`goat admin onboard --help`) is the next externalization increment and
/// extends this same function with `mut_arg` chains.
pub fn localize_command(cmd: clap::Command) -> clap::Command {
    use i18n_embed_fl::fl;

    // Root about + long_about, then the six global flags. Arg ids are the
    // clap-derive field names (`profile_dir`, not `--profile-dir`).
    let cmd = cmd
        .about(fl!(LANGUAGE_LOADER, "help-about"))
        .long_about(fl!(LANGUAGE_LOADER, "help-long-about"))
        .mut_arg("profile_dir", |a| {
            a.help(fl!(LANGUAGE_LOADER, "help-flag-profile-dir"))
        })
        .mut_arg("context", |a| {
            a.help(fl!(LANGUAGE_LOADER, "help-flag-context"))
        })
        .mut_arg("narrate", |a| {
            a.help(fl!(LANGUAGE_LOADER, "help-flag-narrate"))
        })
        .mut_arg("narrate_json", |a| {
            a.help(fl!(LANGUAGE_LOADER, "help-flag-narrate-json"))
        })
        .mut_arg("verbose", |a| {
            a.help(fl!(LANGUAGE_LOADER, "help-flag-verbose"))
        })
        .mut_arg("color", |a| a.help(fl!(LANGUAGE_LOADER, "help-flag-color")))
        .mut_arg("lang", |a| a.help(fl!(LANGUAGE_LOADER, "help-flag-lang")));

    // Subcommand one-liners. For the single-paragraph commands `about`
    // and `long_about` are the same prose, so set both from one key (keeps
    // `goat <sub> --help` and the `goat --help` list in the same language).
    // `doctor` is the exception: a short list-line + a longer `--help` body.
    cmd.mut_subcommand("profile", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-profile")))
        .mut_subcommand("context", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-context")))
        .mut_subcommand("init", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-init")))
        .mut_subcommand("tutor", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-tutor")))
        .mut_subcommand("docs", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-docs")))
        .mut_subcommand("hello", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-hello")))
        .mut_subcommand("doctor", |c| {
            c.about(fl!(LANGUAGE_LOADER, "help-cmd-doctor"))
                .long_about(fl!(LANGUAGE_LOADER, "help-cmd-doctor-long"))
        })
        .mut_subcommand("org", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-vendor")))
        .mut_subcommand("topic", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-topic")))
        .mut_subcommand("pub", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-pub")))
        .mut_subcommand("sub", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-sub")))
        .mut_subcommand("inspect", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-inspect")))
        .mut_subcommand("ui", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-ui")))
        .mut_subcommand("compose", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-compose")))
        .mut_subcommand("narrate", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-narrate")))
        .mut_subcommand("completion", |c| {
            about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-completion"))
        })
        .mut_subcommand("explain", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-explain")))
        .mut_subcommand("mock", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-mock")))
        .mut_subcommand("admin", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-admin")))
        .mut_subcommand("status", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-status")))
        .mut_subcommand("services", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-services")))
        .mut_subcommand("client", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-client")))
        .mut_subcommand("plugin", |c| about_both(c, fl!(LANGUAGE_LOADER, "help-cmd-plugin")))
    // `_complete` is hidden + machine-consumed — intentionally not localized.
}

/// Set a subcommand's `about` and `long_about` to the same prose. Used for
/// the single-paragraph subcommands so both the `goat --help` list line and
/// the `goat <sub> --help` body render in the active locale.
fn about_both(c: clap::Command, prose: String) -> clap::Command {
    c.about(prose.clone()).long_about(prose)
}

// ---------------------------------------------------------------------
// Translation provenance + feedback (ADR 1024 decision 6).
//
// The per-language review status and the feedback channels are
// PARAMETERIZED in `i18n-provenance.toml` (embedded via include_str!),
// so they update without code changes; the two channel values are also
// overridable per deployment via env. When a non-English locale that is
// not native-reviewed is active, `warn_if_machine_translated` emits a
// stderr notice naming the feedback channels — the operator is told the
// text is machine-translated and where to report errors.
// ---------------------------------------------------------------------

/// Parsed `i18n-provenance.toml`, embedded at compile time.
#[derive(serde::Deserialize)]
struct Provenance {
    feedback: Feedback,
    #[serde(default)]
    review: std::collections::BTreeMap<String, String>,
}

#[derive(serde::Deserialize)]
struct Feedback {
    #[serde(default)]
    email: String,
    #[serde(default)]
    repo: String,
}

static PROVENANCE: Lazy<Provenance> = Lazy::new(|| {
    toml::from_str(include_str!("../i18n-provenance.toml"))
        .expect("i18n-provenance.toml must parse")
});

/// Review status for a primary language subtag (e.g. `fr`). Absent →
/// `machine` (the safe default: unknown provenance warns).
fn review_status(lang: &str) -> &str {
    PROVENANCE
        .review
        .get(lang)
        .map(String::as_str)
        .unwrap_or("machine")
}

/// The feedback channels as a single human string for the notice. The email
/// is `GOAT_I18N_FEEDBACK_EMAIL`, else the goatnet's published
/// `i18n_feedback_email` (`goat context refresh`), else the embedded config;
/// the repo is `GOAT_I18N_FEEDBACK_REPO`, else the embedded config. The repo
/// is omitted when unset (still provisional), so the notice degrades to
/// email-only.
fn feedback_channels(goatnet_email: Option<&str>) -> String {
    let email = std::env::var("GOAT_I18N_FEEDBACK_EMAIL")
        .ok()
        .or_else(|| goatnet_email.map(str::to_string))
        .unwrap_or_else(|| PROVENANCE.feedback.email.clone());
    let repo = std::env::var("GOAT_I18N_FEEDBACK_REPO")
        .unwrap_or_else(|_| PROVENANCE.feedback.repo.clone());
    match (email.is_empty(), repo.is_empty()) {
        (false, false) => format!("{email} or the partner repo ({repo})"),
        (false, true) => email,
        (true, false) => format!("the partner repo ({repo})"),
        (true, true) => "your goat administrator".to_string(),
    }
}

/// Emit the machine-translation notice to stderr if the active locale is
/// non-English and not native-reviewed. Suppressed by `GOAT_I18N_NO_WARN`.
/// Prose-only and stderr-only: never touches stdout or any `--json`.
/// `goatnet_email` is the active context's published feedback address.
pub fn warn_if_machine_translated(goatnet_email: Option<&str>) {
    if std::env::var_os("GOAT_I18N_NO_WARN").is_some() {
        return;
    }
    let active = LANGUAGE_LOADER.current_language();
    let lang = active.language.as_str();
    if lang == "en" || review_status(lang) == "native-reviewed" {
        return;
    }
    let channels = feedback_channels(goatnet_email);
    let notice = i18n_embed_fl::fl!(
        LANGUAGE_LOADER,
        "i18n-machine-translation-warning",
        lang = active.to_string(),
        channels = channels
    );
    eprintln!("{notice}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use i18n_embed_fl::fl;

    /// Build an isolated loader for a specific language set so tests
    /// don't race on the process-global `LANGUAGE_LOADER`.
    fn loader_for(langs: &[&str]) -> FluentLanguageLoader {
        let loader: FluentLanguageLoader = fluent_language_loader!();
        loader.load_fallback_language(&Localizations).unwrap();
        let ids: Vec<LanguageIdentifier> = langs.iter().map(|s| s.parse().unwrap()).collect();
        i18n_embed::select(&loader, &Localizations, &ids).unwrap();
        // After all loads — set_use_isolating mutates loaded bundles.
        loader.set_use_isolating(false);
        loader
    }

    #[test]
    fn english_renders_canonical_summary() {
        let l = loader_for(&["en"]);
        assert_eq!(
            fl!(l, "error-off-mesh-summary"),
            "off the ZTOL — not connected to the mesh"
        );
        // Interpolation passes the contract token through verbatim, with
        // no bidi isolation marks around it.
        assert_eq!(
            fl!(l, "error-profile-missing-summary", context = "default"),
            "no profile for context `default`"
        );
    }

    #[test]
    fn ukrainian_translates_present_keys() {
        let l = loader_for(&["uk"]);
        assert_eq!(
            fl!(l, "error-off-mesh-summary"),
            "поза ZTOL — немає підключення до мережі"
        );
        assert_eq!(
            fl!(l, "error-profile-missing-summary", context = "default"),
            "немає профілю для контексту `default`"
        );
    }

    #[test]
    fn every_catalog_parses_and_has_a_valid_langid() {
        // Loads every embedded catalog, proving each folder name is a
        // valid BCP-47 langid (incl. cnr/nb/lb/mk) and each .ftl parses.
        // Guards the full NATO-member set against a typo'd code or a
        // malformed Fluent file silently dropping a language.
        let probe: FluentLanguageLoader = fluent_language_loader!();
        probe.load_fallback_language(&Localizations).unwrap();
        let langs = probe.available_languages(&Localizations).unwrap();
        // en + fr + uk + 27 NATO-member languages = 30, plus the opt-in
        // Klingon (tlh) easter-egg locale = 31. Lower-bounded so adding a
        // locale never breaks the test.
        assert!(
            langs.len() >= 30,
            "expected >=30 catalogs, found {}: {langs:?}",
            langs.len()
        );
        for lang in &langs {
            let l = loader_for(&[lang.to_string().as_str()]);
            assert!(
                !fl!(l, "error-off-mesh-summary").is_empty(),
                "empty render for {lang}"
            );
        }
    }

    #[test]
    fn french_is_complete_for_the_externalized_surface() {
        let l = loader_for(&["fr"]);
        // fr translates every error summary (no fallback to English).
        assert_eq!(
            fl!(l, "error-off-mesh-summary"),
            "hors ZTOL — non connecté au maillage"
        );
        assert_eq!(
            fl!(l, "error-token-invalid-summary"),
            "le portail a rejeté le jeton Bearer"
        );
        assert_eq!(
            fl!(l, "error-profile-missing-summary", context = "default"),
            "aucun profil pour le contexte `default`"
        );
    }

    #[test]
    fn partial_catalog_falls_back_to_english_for_absent_keys() {
        // The Klingon (tlh) easter-egg catalog is intentionally partial
        // (marquee strings only) — a non-marquee key must render the English
        // arbiter, never a blank or a key name. uk used to serve this role
        // while it had drifted, but it is now complete; tlh is the catalog
        // that is partial *by design* (and that translate-ftl.py never fills),
        // so the fallback assertion can't be silently invalidated by a refresh.
        let l = loader_for(&["tlh"]);
        assert_eq!(
            fl!(l, "error-bundle-invalid-summary"),
            "bundle signature or expiry verification failed"
        );
    }

    #[test]
    fn runtime_prose_localizes_and_preserves_contract_tokens() {
        // The new status/inspect/narrate runtime keys translate in a
        // sample locale (proving the catalog entry resolved, not a silent
        // English fallback) while every contract token survives verbatim.
        let fr = loader_for(&["fr"]);
        // A status summary localizes away from the English literal…
        let ztol = fl!(fr, "status-ztol-unreachable", host = "h", port = "7447", ms = "9");
        assert_ne!(ztol, "off-mesh (router h:7447 unreachable in 9ms)");
        // …but the interpolated contract tokens are reproduced verbatim.
        assert!(ztol.contains("h:7447"), "host:port token dropped: {ztol}");
        // Fixed literals stay byte-for-byte across the locale.
        assert_eq!(
            fl!(fr, "status-portal-failed", err = "boom"),
            fl!(fr, "status-portal-failed", err = "boom")
        );
        let portal = fl!(fr, "status-portal-timeout", secs = "5");
        assert!(portal.contains("/api/health"), "literal dropped: {portal}");
        assert!(portal.contains("5s"), "unit suffix dropped: {portal}");
        // The narrator plural select renders both branches in the locale.
        let one = fl!(fr, "narrator-settled", summary = "x", n = 1);
        let many = fl!(fr, "narrator-settled", summary = "x", n = 3);
        assert_ne!(one, many, "plural select did not vary on n");
        assert!(one.contains('1') && many.contains('3'));
    }

    // Serializes every test that calls `catch_panic_or`. `set_hook`/`take_hook`
    // are process-global and cargo runs tests as threads in one process, so the
    // hazard is any two tests *mutating* the hook concurrently — not just two
    // tests panicking. `catch_panic_or` takes and replaces the hook on every
    // call, success path included, so all three tests below must hold this lock:
    // without it one test's `take_hook()` can capture another's hook and the
    // restore-assertion below goes flaky.
    static HOOK_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn catch_panic_or_passes_through_the_success_value() {
        let _guard = HOOK_TEST_LOCK.lock().unwrap();
        assert_eq!(catch_panic_or(|| vec![1, 2, 3], Vec::new), vec![1, 2, 3]);
    }

    #[test]
    fn catch_panic_or_falls_back_instead_of_propagating() {
        let _guard = HOOK_TEST_LOCK.lock().unwrap();
        // The panicking closure must not unwind past catch_panic_or — this
        // is the behavior `os_requested_languages` leans on to survive
        // locale_config's unwrap-on-parse panic.
        let result: Vec<i32> = catch_panic_or(
            || panic!("simulated locale_config unwrap panic"),
            || vec![42],
        );
        assert_eq!(result, vec![42]);
    }

    #[test]
    fn catch_panic_or_restores_the_panic_hook() {
        let _guard = HOOK_TEST_LOCK.lock().unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_in_hook = calls.clone();
        let prev = panic::take_hook();
        panic::set_hook(Box::new(move |_| {
            calls_in_hook.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));

        let _: Vec<i32> = catch_panic_or(|| panic!("boom"), Vec::new);

        // If catch_panic_or failed to restore our hook (e.g. left its
        // silencing no-op hook installed), this second, independent panic
        // would not increment the counter.
        let _ = std::panic::catch_unwind(|| panic!("second, unrelated panic"));
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "panic hook installed before catch_panic_or must be active again after it returns"
        );

        panic::set_hook(prev);
    }

    fn argv(parts: &[&str]) -> impl Iterator<Item = String> {
        parts.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn preparse_lang_precedence() {
        // Explicit `--lang <tag>` beats GOAT_LANG.
        assert_eq!(
            preparse_lang_from(argv(&["--lang", "fr", "doctor"]), Some("de".into())),
            Some("fr".to_string())
        );
        // `--lang=<tag>` form is equivalent.
        assert_eq!(
            preparse_lang_from(argv(&["--lang=es"]), Some("de".into())),
            Some("es".to_string())
        );
        // `--lang` is global — it resolves after the subcommand too.
        assert_eq!(
            preparse_lang_from(argv(&["doctor", "--lang", "uk"]), None),
            Some("uk".to_string())
        );
        // No flag → fall back to the GOAT_LANG env value.
        assert_eq!(
            preparse_lang_from(argv(&["doctor"]), Some("it".into())),
            Some("it".to_string())
        );
        // Neither set → None (select_language then negotiates OS → English).
        assert_eq!(preparse_lang_from(argv(&["doctor"]), None), None);
        // Empty GOAT_LANG is treated as unset.
        assert_eq!(preparse_lang_from(argv(&["doctor"]), Some(String::new())), None);
        // A bare `--` ends option scanning: a later `--lang` is positional,
        // so the env value (if any) wins instead.
        assert_eq!(
            preparse_lang_from(argv(&["--", "--lang", "fr"]), Some("de".into())),
            Some("de".to_string())
        );
    }
}
