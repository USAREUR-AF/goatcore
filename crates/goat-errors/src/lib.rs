//! goat-errors — the one error catalog goat and goatbuilder share.
//!
//! Design of record: docs/design/error-interpretation.md (ADR 1199). The
//! catalog is ops/errors/catalog.yaml, compiled in at build time so a lookup
//! works offline. Two entry points:
//!
//! * [`explain`] — one code → its entry.
//! * [`interpret`] — any text → every entry it matches, through three layers
//!   in order: the exact codes the text carries, the catalog's `matches:`
//!   patterns over the whole text (so a tool's stderr carried inside a goat
//!   error is found too), and a lexical nearest-neighbour over the catalog
//!   for text no pattern knows. A fourth, semantic layer is a documented slot
//!   ([`Matcher`]), not built.
//!
//! Hits are ordered innermost first: a tool's own words before the goat verb
//! that carried them, so the first fix printed is the one closest to the
//! cause.

use regex::Regex;
pub mod phases;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

/// The catalog source, embedded at build time. Re-bake by rebuilding.
pub const CATALOG_YAML: &str = include_str!("../../../ops/errors/catalog.yaml");

/// Where `--report` writes, under the operator's home.
pub const REPORT_DIR: &str = ".goat/error-reports";

/// Lexical hits below this score are not reported (see [`Lexical`]).
pub const LEXICAL_THRESHOLD: f64 = 0.45;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub code: String,
    pub tool: String,
    pub title: String,
    pub what: String,
    #[serde(default)]
    pub causes: Vec<String>,
    #[serde(default)]
    pub fixes: Vec<String>,
    #[serde(default)]
    pub read: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matches: Vec<String>,
    /// `"<release tag>: <why>"` (`goatbuilder/vX.Y.Z` or `goat-cli-vX.Y.Z`) for a
    /// code no binary raises any more. The entry stays,
    /// so an old log's code still explains (ops/ci/check-error-codes-kept.py).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired: Option<String>,
}

/// The version of a catalog entry printed as a document (`explain <code> -o json`).
pub const ENTRY_SCHEMA: &str = "goat-errors.entry/v1";

/// One catalog entry as a document a program reads: `schema`, then the entry's
/// own fields unchanged. `explain <code> -o json|yaml` prints this, so the
/// document carries its version like every other the binaries print.
#[derive(Debug, Serialize)]
pub struct EntryDocument<'a> {
    pub schema: &'static str,
    #[serde(flatten)]
    pub entry: &'a Entry,
}

pub fn entry_document(entry: &Entry) -> EntryDocument<'_> {
    EntryDocument {
        schema: ENTRY_SCHEMA,
        entry,
    }
}

impl Entry {
    /// Our own binary's error, as opposed to a tool's words carried through.
    pub fn is_internal(&self) -> bool {
        matches!(self.tool.as_str(), "goat" | "goatbuilder")
    }
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    schema: String,
    entries: Vec<Entry>,
}

/// The loaded catalog with its compiled patterns.
pub struct Catalog {
    pub schema: String,
    pub entries: Vec<Entry>,
    patterns: Vec<(usize, Regex)>,
    idf: BTreeMap<String, f64>,
    docs: Vec<BTreeSet<String>>,
}

impl Catalog {
    /// Parse a catalog from YAML; every `matches:` pattern must compile.
    pub fn parse(yaml: &str) -> Result<Catalog, String> {
        let file: CatalogFile = serde_yaml::from_str(yaml).map_err(|e| format!("catalog: {e}"))?;
        if file.schema != "goat-errors/v1" {
            return Err(format!("catalog: schema {} is not goat-errors/v1", file.schema));
        }
        let mut patterns = Vec::new();
        let mut seen = BTreeSet::new();
        for (i, e) in file.entries.iter().enumerate() {
            if !seen.insert(e.code.clone()) {
                return Err(format!("catalog: code {} appears twice", e.code));
            }
            for m in &e.matches {
                let re = Regex::new(m).map_err(|err| format!("catalog: {}: pattern {m:?}: {err}", e.code))?;
                patterns.push((i, re));
            }
        }
        let docs: Vec<BTreeSet<String>> = file.entries.iter().map(entry_tokens).collect();
        let mut df: BTreeMap<String, usize> = BTreeMap::new();
        for d in &docs {
            for t in d {
                *df.entry(t.clone()).or_default() += 1;
            }
        }
        let n = docs.len().max(1) as f64;
        let idf = df.into_iter().map(|(t, c)| (t, (1.0 + n / (c as f64)).ln())).collect();
        Ok(Catalog { schema: file.schema, entries: file.entries, patterns, idf, docs })
    }

    pub fn get(&self, code: &str) -> Option<&Entry> {
        let want = normalize_code(code);
        self.entries.iter().find(|e| e.code == want)
    }

    pub fn codes(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.code.as_str()).collect()
    }
}

/// The embedded catalog, parsed once. A catalog that does not parse is a
/// build defect the CI gate refuses; here it is a panic with the reason.
pub fn catalog() -> &'static Catalog {
    static CAT: OnceLock<Catalog> = OnceLock::new();
    CAT.get_or_init(|| Catalog::parse(CATALOG_YAML).expect("the embedded error catalog must parse"))
}

/// `profile-missing` → `E_PROFILE_MISSING`; `gb-7` → `GB-007`; `x-ssh-1` → `X-SSH-001`.
pub fn normalize_code(input: &str) -> String {
    let s = input.trim().to_ascii_uppercase();
    if let Some(rest) = s.strip_prefix("GB-") {
        return format!("GB-{:0>3}", rest);
    }
    if s.starts_with("X-") {
        let mut parts: Vec<&str> = s.split('-').collect();
        if parts.len() == 3 {
            let n = parts.pop().unwrap_or("");
            return format!("{}-{}-{:0>3}", parts[0], parts[1], n);
        }
        return s;
    }
    let s = s.replace('-', "_");
    if s.starts_with("E_") {
        s
    } else {
        format!("E_{s}")
    }
}

/// One code looked up.
pub fn explain(code: &str) -> Option<&'static Entry> {
    catalog().get(code)
}

/// Which layer produced a hit.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Layer {
    /// The text carries the code itself.
    Code,
    /// A `matches:` pattern of the entry matched somewhere in the text.
    Pattern,
    /// No pattern matched; this entry's words are the nearest in the catalog.
    Lexical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    pub code: String,
    pub layer: Layer,
    /// The line of the text the hit came from (pattern/code), or the overlap score (lexical).
    pub evidence: String,
    /// Where in the text the evidence starts; orders hits of one layer.
    pub at: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    pub entry: Entry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interpretation {
    pub schema: &'static str,
    /// The first non-empty line of the text, for the headline.
    pub summary: String,
    /// Innermost first: a tool's words before the goat verb that carried them.
    pub hits: Vec<Hit>,
    /// True when no code and no pattern matched; any hits are lexical guesses.
    pub unrecognised: bool,
}

/// A matching layer. The three built ones are [`ExactCode`], [`Pattern`] and
/// [`Lexical`]; a semantic (embedding) layer would implement this same trait
/// and slot in after `Lexical` — deferred, see the ADR.
pub trait Matcher {
    fn name(&self) -> &'static str;
    fn find(&self, cat: &Catalog, text: &str) -> Vec<Hit>;
}

pub struct ExactCode;
pub struct Pattern;
pub struct Lexical;

fn code_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(GB-\d{3}|X-[A-Z]+-\d{3}|E_[A-Z][A-Z_]+)\b").expect("static regex"))
}

fn line_at(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = text[at..].find('\n').map(|i| at + i).unwrap_or(text.len());
    text[start..end].trim().chars().take(200).collect()
}

impl Matcher for ExactCode {
    fn name(&self) -> &'static str {
        "code"
    }
    fn find(&self, cat: &Catalog, text: &str) -> Vec<Hit> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        for m in code_regex().find_iter(text) {
            if let Some(e) = cat.get(m.as_str()) {
                if seen.insert(e.code.clone()) {
                    out.push(Hit { code: e.code.clone(), layer: Layer::Code, evidence: line_at(text, m.start()), at: m.start(), score: None, entry: e.clone() });
                }
            }
        }
        out
    }
}

impl Matcher for Pattern {
    fn name(&self) -> &'static str {
        "pattern"
    }
    fn find(&self, cat: &Catalog, text: &str) -> Vec<Hit> {
        let mut best: BTreeMap<usize, usize> = BTreeMap::new(); // entry index → earliest match
        for (i, re) in &cat.patterns {
            if let Some(m) = re.find(text) {
                let at = best.entry(*i).or_insert(m.start());
                if m.start() < *at {
                    *at = m.start();
                }
            }
        }
        best.into_iter()
            .map(|(i, at)| {
                let e = &cat.entries[i];
                Hit { code: e.code.clone(), layer: Layer::Pattern, evidence: line_at(text, at), at, score: None, entry: e.clone() }
            })
            .collect()
    }
}

const STOP: &[&str] = &[
    "the", "and", "for", "that", "this", "with", "from", "not", "was", "were", "are", "its", "has", "have", "had", "but", "you", "your", "one", "all", "any", "can", "into", "run", "when", "then", "than", "see", "error", "failed", "fail", "cannot", "could", "does", "did", "goat", "goatbuilder", "ops", "docs",
];

fn tokens(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            cur.push(ch.to_ascii_lowercase());
        } else {
            flush(&mut cur, &mut out);
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn flush(cur: &mut String, out: &mut BTreeSet<String>) {
    if cur.len() >= 3 && !cur.chars().all(|c| c.is_ascii_digit()) && !cur.chars().all(|c| c.is_ascii_hexdigit()) && !STOP.contains(&cur.as_str()) {
        out.insert(cur.clone());
    }
    cur.clear();
}

fn entry_tokens(e: &Entry) -> BTreeSet<String> {
    let mut s = String::new();
    s.push_str(&e.title);
    s.push(' ');
    s.push_str(&e.what);
    s.push(' ');
    s.push_str(&e.tool);
    for c in &e.causes {
        s.push(' ');
        s.push_str(c);
    }
    for m in &e.matches {
        s.push(' ');
        s.push_str(m);
    }
    tokens(&s)
}

impl Matcher for Lexical {
    fn name(&self) -> &'static str {
        "lexical"
    }
    /// IDF-weighted overlap: the weight of the query tokens the entry shares,
    /// over the weight of all query tokens. Deterministic; ties break on code.
    fn find(&self, cat: &Catalog, text: &str) -> Vec<Hit> {
        let q = tokens(text);
        let q_weight: f64 = q.iter().map(|t| cat.idf.get(t).copied().unwrap_or(0.0)).sum();
        if q.is_empty() || q_weight == 0.0 {
            return Vec::new();
        }
        let mut scored: Vec<(f64, usize, usize)> = cat
            .docs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let shared: Vec<&String> = q.intersection(d).collect();
                let w: f64 = shared.iter().map(|t| cat.idf.get(*t).copied().unwrap_or(0.0)).sum();
                (w / q_weight, shared.len(), i)
            })
            .filter(|(s, n, _)| *s >= LEXICAL_THRESHOLD && *n >= 2)
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(cat.entries[a.2].code.cmp(&cat.entries[b.2].code)));
        scored
            .into_iter()
            .take(3)
            .enumerate()
            .map(|(rank, (s, n, i))| {
                let e = &cat.entries[i];
                // `at` carries the rank so the final ordering keeps best-first.
                Hit { code: e.code.clone(), layer: Layer::Lexical, evidence: format!("{n} shared words, score {s:.2}"), at: rank, score: Some(s), entry: e.clone() }
            })
            .collect()
    }
}

/// Interpret any error text against the embedded catalog.
pub fn interpret(text: &str) -> Interpretation {
    interpret_with(catalog(), text)
}

pub fn interpret_with(cat: &Catalog, text: &str) -> Interpretation {
    let summary = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(200).collect();
    let mut hits = ExactCode.find(cat, text);
    for h in Pattern.find(cat, text) {
        if !hits.iter().any(|x| x.code == h.code) {
            hits.push(h);
        }
    }
    let unrecognised = hits.is_empty();
    if unrecognised {
        hits = Lexical.find(cat, text);
    }
    // Innermost first: external tools' words before our own verbs; within a
    // group, the order they appear in the text.
    hits.sort_by(|a, b| a.entry.is_internal().cmp(&b.entry.is_internal()).then(a.at.cmp(&b.at)).then(a.code.cmp(&b.code)));
    Interpretation { schema: "goat-errors.interpretation/v1", summary, hits, unrecognised }
}

// ---------------------------------------------------------------- languages
//
// The entries' prose (title, what, each cause) localizes; codes, fixes (the
// commands an operator types), read paths, exit codes, labels and every JSON
// field never do (docs/design/goat-cli-i18n.md §"The error catalog"). The
// English arbiter is generated from ops/errors/catalog.yaml into
// i18n/en/errors.ftl; the locales are filled by ops/scripts/translate-ftl.py.
// A non-English render is bilingual: the localized line, then `(en: …)` with
// the binding English, as goat renders its own errors (ADR 1024 decision 7).

type Bundle = fluent_bundle::concurrent::FluentBundle<fluent_bundle::FluentResource>;

#[derive(rust_embed::RustEmbed)]
#[folder = "i18n/"]
struct Locales;

static LANG: OnceLock<String> = OnceLock::new();
static BUNDLE: OnceLock<Option<Bundle>> = OnceLock::new();

/// Tell the catalog which language the binary negotiated (goat's or
/// goatbuilder's `--lang` / `GOAT_LANG` / OS locale). Set once, before the
/// first render; unset means English.
pub fn set_language(tag: &str) {
    let _ = LANG.set(tag.to_string());
}

/// The locales this build carries a translation of the catalog for.
pub fn locales() -> Vec<String> {
    let mut v: Vec<String> = Locales::iter()
        .filter_map(|p| p.strip_suffix("/errors.ftl").map(str::to_string))
        .collect();
    v.sort();
    v
}

/// Load one locale's catalog. `None` for English (the entries are the
/// English) and for a locale this build does not carry.
pub fn load_bundle(tag: &str) -> Option<Bundle> {
    let li: unic_langid::LanguageIdentifier = tag.parse().ok()?;
    let lang = li.language.as_str().to_string();
    if lang == "en" {
        return None;
    }
    let file = Locales::get(&format!("{tag}/errors.ftl")).or_else(|| Locales::get(&format!("{lang}/errors.ftl")))?;
    let src = String::from_utf8(file.data.into_owned()).ok()?;
    // A file with one bad entry still serves the rest; the bad one falls back.
    let res = fluent_bundle::FluentResource::try_new(src).unwrap_or_else(|(r, _)| r);
    let mut b = Bundle::new_concurrent(vec![li]);
    b.set_use_isolating(false);
    b.add_resource_overriding(res);
    Some(b)
}

fn active_bundle() -> Option<&'static Bundle> {
    BUNDLE.get_or_init(|| LANG.get().and_then(|t| load_bundle(t))).as_ref()
}

fn message(b: &Bundle, id: &str) -> Option<String> {
    let pattern = b.get_message(id)?.value()?;
    let mut errs = vec![];
    let out = b.format_pattern(pattern, None, &mut errs).into_owned();
    if errs.is_empty() && !out.trim().is_empty() { Some(out) } else { None }
}

fn ftl_id(code: &str) -> String {
    code.to_lowercase().replace('_', "-")
}

/// One prose line, bilingual when a translation exists and differs.
fn line(s: &mut String, indent: &str, label: &str, english: &str, localized: Option<String>) {
    match localized {
        Some(l) if l != english => {
            s.push_str(&format!("{indent}  {label} {l}\n"));
            s.push_str(&format!("{indent}  {}(en: {english})\n", " ".repeat(label.chars().count() + 1)));
        }
        _ => s.push_str(&format!("{indent}  {label} {english}\n")),
    }
}

/// Render one entry the way both binaries print it: title, what, why, fix, read.
pub fn render_entry(e: &Entry, indent: &str) -> String {
    render_entry_with(e, indent, active_bundle())
}

/// [`render_entry`] against an explicit locale catalog (`None` = English).
pub fn render_entry_with(e: &Entry, indent: &str, b: Option<&Bundle>) -> String {
    let id = ftl_id(&e.code);
    let tr = |key: String| b.and_then(|b| message(b, &key));
    let mut s = String::new();
    match tr(format!("{id}-title")) {
        Some(t) if t != e.title => {
            s.push_str(&format!("{indent}{} — {t}\n", e.code));
            s.push_str(&format!("{indent}  (en: {})\n", e.title));
        }
        _ => s.push_str(&format!("{indent}{} — {}\n", e.code, e.title)),
    }
    line(&mut s, indent, "what:", &e.what, tr(format!("{id}-what")));
    for (i, c) in e.causes.iter().enumerate() {
        line(&mut s, indent, if i == 0 { "why: " } else { "     " }, c, tr(format!("{id}-cause-{}", i + 1)));
    }
    for (i, f) in e.fixes.iter().enumerate() {
        s.push_str(&format!("{indent}  {} {}\n", if i == 0 { "fix: " } else { "     " }, f));
    }
    if !e.read.is_empty() {
        s.push_str(&format!("{indent}  read: {}\n", e.read));
    }
    if let Some(x) = e.exit {
        s.push_str(&format!("{indent}  exit: {x}\n"));
    }
    s
}

/// Render an interpretation for a person. `program` names the binary for the
/// report hint ("goat" or "goatbuilder").
pub fn render_human(i: &Interpretation, program: &str) -> String {
    let mut s = String::new();
    if i.unrecognised && i.hits.is_empty() {
        s.push_str("  not recognised: no catalog entry matches this text.\n");
        s.push_str(&format!("  share it: {program} explain --report -   (reads the text from stdin, writes ~/{REPORT_DIR}/<stamp>.txt)\n"));
        return s;
    }
    if i.unrecognised {
        s.push_str("  not recognised exactly; the nearest catalog entries:\n");
    }
    for h in &i.hits {
        let how = match h.layer {
            Layer::Code => "code".to_string(),
            Layer::Pattern => format!("matched: {}", h.evidence),
            Layer::Lexical => format!("nearest: {}", h.evidence),
        };
        s.push_str(&format!("  [{how}]\n"));
        s.push_str(&render_entry(&h.entry, "  "));
    }
    if i.unrecognised {
        s.push_str(&format!("  if none of these is it: {program} explain --report -   (writes ~/{REPORT_DIR}/<stamp>.txt to share)\n"));
    }
    s
}

/// Write the text and its interpretation to `~/.goat/error-reports/<stamp>.txt`
/// and return the path. The file carries no secret the text did not.
pub fn write_report(text: &str, i: &Interpretation, program: &str) -> std::io::Result<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = home.join(REPORT_DIR);
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = dir.join(format!("{program}-{stamp}.txt"));
    let mut body = String::new();
    body.push_str(&format!("# {program} error report\n# interpreted by goat-errors ({}); catalog {}\n\n", i.schema, catalog().schema));
    body.push_str("## text\n\n");
    body.push_str(text.trim_end());
    body.push_str("\n\n## interpretation\n\n");
    body.push_str(&render_human(i, program));
    std::fs::write(&path, body)?;
    Ok(path)
}

/// An error with a catalog code, for the binaries' own refusal sites:
/// `Err(coded("GB-002", format!(...)))`. Displays as the message alone — the
/// words stay the script's words the parity gates compare — and the binary
/// reads the code back with `downcast_ref::<Coded>()` to interpret it
/// ([`interpret_coded`]). Every `GB-` entry also carries a `matches:` pattern
/// for its own message, so the text is recognised wherever it travels.
#[derive(Debug)]
pub struct Coded {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for Coded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Interpret a text whose code the binary already knows (a [`Coded`] error):
/// the code's entry leads, then whatever else the text matches.
pub fn interpret_coded(code: &str, text: &str) -> Interpretation {
    let mut i = interpret(text);
    if let Some(e) = explain(code) {
        i.hits.retain(|h| h.code != e.code);
        i.hits.push(Hit { code: e.code.clone(), layer: Layer::Code, evidence: i.summary.clone(), at: 0, score: None, entry: e.clone() });
        i.unrecognised = false;
        if i.hits.iter().any(|h| h.layer == Layer::Lexical) {
            i.hits.retain(|h| h.layer != Layer::Lexical);
        }
        i.hits.sort_by(|a, b| a.entry.is_internal().cmp(&b.entry.is_internal()).then(a.at.cmp(&b.at)).then(a.code.cmp(&b.code)));
    }
    i
}

/// Whether to print an interpretation under an error on stderr: `GOAT_EXPLAIN=1`
/// always, `GOAT_EXPLAIN=0` never, else when stderr is a terminal — so a
/// piped or captured run (a parity gate, a golden test, a log) keeps the
/// one-line error and a person at a terminal gets the whole of it.
pub fn explain_on_stderr() -> bool {
    match std::env::var("GOAT_EXPLAIN").as_deref() {
        Ok("1") | Ok("always") => true,
        Ok("0") | Ok("never") => false,
        _ => std::io::IsTerminal::is_terminal(&std::io::stderr()),
    }
}

impl std::error::Error for Coded {}

pub fn coded(code: &str, message: impl Into<String>) -> Coded {
    Coded { code: code.to_string(), message: message.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_parses_and_every_pattern_compiles() {
        let c = catalog();
        assert!(c.entries.len() > 50, "{} entries", c.entries.len());
        for e in &c.entries {
            assert!(!e.title.is_empty() && !e.what.is_empty() && !e.fixes.is_empty() && !e.causes.is_empty(), "{} incomplete", e.code);
        }
    }

    #[test]
    fn exact_code_in_any_form() {
        assert_eq!(explain("profile-missing").unwrap().code, "E_PROFILE_MISSING");
        assert_eq!(explain("gb-7").unwrap().code, "GB-007");
        assert_eq!(explain("x-ssh-1").unwrap().code, "X-SSH-001");
        assert!(explain(&format!("GB-{}", 999)).is_none());
    }

    #[test]
    fn pattern_matches_anywhere_in_the_text() {
        let i = interpret("goatbuilder: deploy FAILED — log /tmp/x.log\n  fatal: [thor]: FAILED! => {\"msg\": \"Missing sudo password\"}");
        let codes: Vec<&str> = i.hits.iter().map(|h| h.code.as_str()).collect();
        assert!(codes.contains(&"GB-029"), "{codes:?}");
        assert!(codes.contains(&"X-ANSIBLE-002"), "{codes:?}");
        assert!(codes.contains(&"X-ANSIBLE-004"), "{codes:?}");
        assert!(!i.unrecognised);
    }

    #[test]
    fn passthrough_reports_inner_tool_before_outer_verb() {
        let text = "goatbuilder: GB-029: deploy FAILED — log /var/folders/x/goat-deploy-netvis.log\n\
                    zagros_dev_hosts | UNREACHABLE! => {\"changed\": false, \"msg\": \"Failed to connect to the host via ssh: goat@100.127.0.155: Permission denied (publickey).\", \"unreachable\": true}";
        let i = interpret(text);
        let codes: Vec<&str> = i.hits.iter().map(|h| h.code.as_str()).collect();
        assert!(codes.contains(&"GB-029"), "{codes:?}");
        assert!(codes.contains(&"X-ANSIBLE-001"), "{codes:?}");
        assert!(codes.contains(&"X-SSH-001"), "{codes:?}");
        let first_internal = codes.iter().position(|c| c.starts_with("GB-")).unwrap();
        let last_external = codes.iter().rposition(|c| c.starts_with("X-")).unwrap();
        assert!(last_external < first_internal, "innermost first: {codes:?}");
        assert_eq!(i.hits[0].layer, Layer::Pattern);
        assert!(i.hits.iter().find(|h| h.code == "GB-029").unwrap().layer == Layer::Code);
    }

    #[test]
    fn op_run_malformed_reference_is_catalogued() {
        let i = interpret("[ERROR] 2026/10/28 08:34:15 invalid secret reference 'op://vault/incomplete-item': too few '/': secret references should have at least vault, item and field specified");
        assert_eq!(i.hits[0].code, "X-OP-002");
        assert!(i.hits[0].entry.fixes.iter().any(|f| f.contains("env -u")));
    }

    #[test]
    fn lexical_nearest_neighbour_hits_above_threshold() {
        let i = interpret("ssh: the host refused the key, permission denied for publickey when connecting");
        assert!(i.unrecognised);
        assert!(!i.hits.is_empty(), "expected a lexical neighbour");
        assert_eq!(i.hits[0].layer, Layer::Lexical);
        assert_eq!(i.hits[0].code, "X-SSH-001", "{:?}", i.hits.iter().map(|h| &h.code).collect::<Vec<_>>());
    }

    #[test]
    fn lexical_miss_below_threshold_is_unrecognised_with_no_hits() {
        let i = interpret("zzqx vrbl plonk 9931 kettle drum");
        assert!(i.unrecognised);
        assert!(i.hits.is_empty(), "{:?}", i.hits.iter().map(|h| &h.code).collect::<Vec<_>>());
        let s = render_human(&i, "goat");
        assert!(s.contains("not recognised"));
        assert!(s.contains("goat explain --report"));
    }

    #[test]
    fn coded_error_keeps_the_scripts_words_and_is_interpreted_by_its_code() {
        let e = coded("GB-002", "unknown goatnet \"zagros-dev\"; known: [\"zagros\", \"zagros_dev\"]");
        assert_eq!(e.to_string(), "unknown goatnet \"zagros-dev\"; known: [\"zagros\", \"zagros_dev\"]");
        let i = interpret_coded(&e.code, &e.to_string());
        assert_eq!(i.hits.len(), 1, "{:?}", i.hits.iter().map(|h| &h.code).collect::<Vec<_>>());
        assert_eq!(i.hits[0].code, "GB-002");
        assert_eq!(i.hits[0].layer, Layer::Code);
        // The same words with no code in hand are still recognised, by pattern.
        let j = interpret(&e.to_string());
        assert_eq!(j.hits[0].code, "GB-002");
        assert_eq!(j.hits[0].layer, Layer::Pattern);
    }

    #[test]
    fn every_locale_resolves_every_entry_without_a_raw_key() {
        let cat = catalog();
        let locs = locales();
        assert!(locs.contains(&"en".to_string()), "the English arbiter is embedded");
        for tag in locs.iter().filter(|t| t.as_str() != "en") {
            let b = load_bundle(tag).unwrap_or_else(|| panic!("{tag}: catalog does not load"));
            for e in cat.codes() {
                let e = cat.get(e).unwrap();
                let out = render_entry_with(e, "", Some(&b));
                let id = ftl_id(&e.code);
                assert!(!out.contains(&format!("{id}-")), "{tag}/{}: a raw message id leaked into the render", e.code);
                assert!(out.contains(&e.code) && out.contains("fix:"), "{tag}/{}: code and fix lines must survive", e.code);
                for f in &e.fixes {
                    assert!(out.contains(f.as_str()), "{tag}/{}: a fix command must render verbatim", e.code);
                }
            }
        }
    }

    #[test]
    fn a_translated_entry_renders_bilingual_and_english_renders_plain() {
        let e = catalog().get("GB-002").unwrap();
        assert!(!render_entry_with(e, "", None).contains("(en: "), "English is never doubled");
        if let Some(b) = load_bundle("fr") {
            let out = render_entry_with(e, "", Some(&b));
            assert!(out.contains(&format!("(en: {})", e.what)), "the binding English follows the localized line:\n{out}");
        }
    }

    #[test]
    fn render_carries_what_why_fix_read() {
        let s = render_entry(explain("GB-007").unwrap(), "");
        for k in ["what:", "why:", "fix:", "read:", "op run"] {
            assert!(s.contains(k), "missing {k} in {s}");
        }
        let j = serde_json::to_string(&interpret("GB-007: NOT SET: X")).unwrap();
        assert!(j.contains("\"schema\":\"goat-errors.interpretation/v1\""));
    }

    /// `explain <code> -o json` is a document a program reads, so it carries its
    /// version like every other (docs/design/goatbuilder-output.md): the entry's
    /// fields, unchanged, after `schema`.
    #[test]
    fn an_entry_document_carries_its_version_and_every_field() {
        let e = explain("GB-017").unwrap();
        let doc: serde_json::Value = serde_json::to_value(entry_document(e)).unwrap();
        assert_eq!(doc["schema"], "goat-errors.entry/v1");
        let bare: serde_json::Value = serde_json::to_value(e).unwrap();
        for (k, v) in bare.as_object().unwrap() {
            assert_eq!(&doc[k], v, "{k}");
        }
        assert_eq!(
            doc.as_object().unwrap().len(),
            bare.as_object().unwrap().len() + 1
        );
        let text = serde_json::to_string_pretty(&entry_document(e)).unwrap();
        assert!(
            text.starts_with("{\n  \"schema\": \"goat-errors.entry/v1\",\n  \"code\": \"GB-017\""),
            "{text}"
        );
    }
}
