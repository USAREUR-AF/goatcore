//! compose_context — live substrate-context feeds for `goat compose`.
//!
//! This is the half of `goat compose` that no flat `--help` can offer: as
//! the operator types, the composer surfaces the portal's *current* state at
//! the cursor position — the prefixes they can publish under, the topic keys
//! that already exist, the formats already declared, and a warning when the
//! key they are typing would collide or land somewhere they can't write.
//!
//! ## Isolation (removable by construction, not provisional)
//! The feed set is **complete and committed** (ADR 1019 §14 — write-prefixes,
//! recent topics, declared schemas, conflict warnings). It stays *cleanly
//! removable* as an engineering property, not because it is on trial: the
//! whole feature is this one file plus the lines tagged `// CONTEXT-PANE:` in
//! `cmd/compose.rs` + `cmd/mod.rs`. It is read-only (portal GETs only),
//! session-snapshot (resolved once at launch — no background tasks, no cache,
//! no network in the event loop), and confined to the `compose` subcommand, so
//! a bug here cannot mutate any server-side layer or affect another command.
//! Operators can also disable it at runtime with `goat compose --no-context`.
//!
//! ## PORT (the pattern is CLI-agnostic)
//! `classify(position) -> ContextKind`, then `ContextSource::lookup(kind) ->
//! ContextState`, then `filter_items` for typeahead and `complete_form` for
//! how a picked item lands in the line; the caller renders however it likes.
//! To reuse it in another CLI: redefine `ContextKind` + `classify()` +
//! `complete_form()` for that CLI's grammar and implement `ContextSource`
//! against its backend; `filter_items` and the snapshot plumbing carry over
//! unchanged. The only goat-specific pieces are `classify()`'s value-name
//! table, `complete_form()`'s completion grammar, and the `PortalSnapshot`
//! adapter (with its catalog derivations) at the bottom of this file.
//!
//! ## Scope (all four feeds built)
//! - **WritePrefixes** — the operator's publishable prefixes, from the launch
//!   `GET /api/orgs` snapshot. Shown at a write namespace slot (`pub`,
//!   `topic create|delete`).
//! - **RecentTopics** — real topic keys declared on the goatnet, newest first,
//!   from the launch `GET /api/catalog` snapshot. Shown at a read key slot
//!   (`sub`, `flow`).
//! - **DeclaredSchemas** — the distinct formats already declared on the
//!   goatnet (`protobuf:<Msg>`, `json`, …), from the same catalog snapshot.
//!   Shown while entering an unconstrained `--format` / `--encoding` value.
//! - **ConflictWarning** — computed (not a list feed) from the snapshot
//!   against the key being typed at a write slot: the prefix isn't one you can
//!   publish under, or the exact key already exists. See `conflict_for`.

/// What live context, if any, is relevant at the current grammar position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextKind {
    /// Nothing live applies here (root, picking subcommands, plain flags).
    None,
    /// The operator's writable vendor/org prefixes (a write namespace slot).
    WritePrefixes,
    /// Topic keys already declared on the goatnet (a read key slot).
    RecentTopics,
    /// Wire formats already declared on the goatnet (a `--format`/`--encoding`
    /// value position).
    DeclaredSchemas,
}

/// The resolved display state of a context feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextState {
    /// Feature disabled (`--no-context`). The caller draws no context.
    Disabled,
    /// No context applies at this position.
    NotApplicable,
    /// Items to show, with a feed title.
    Loaded { title: String, items: Vec<String> },
    /// Lookup could not resolve (no profile, portal unreachable, timeout,
    /// or a portal that predates the feed). The builder still works; the
    /// pane just says why.
    Unavailable(String),
}

/// Portable seam between "what context is wanted" and "how to get it".
/// A different CLI implements this against its own backend.
pub trait ContextSource {
    fn lookup(&self, kind: &ContextKind) -> ContextState;
}

/// Pure: map the current grammar position — identified by the value-name of
/// the next unfilled positional argument, OR of the flag value being typed —
/// to a `ContextKind`.
///
/// goat's namespace-bearing commands take a topic-key / vendor positional
/// whose clap value-name or id contains one of these tokens. Matched
/// case-insensitively. Order matters: `KEY_PATTERN` (sub) and `KEY_EXPR`
/// (flow) contain both a read token (`PATTERN`/`EXPR`) and `KEY`, so the read
/// tokens are tested first and win — a subscribe/flow names an *existing* key
/// (recent-topics), while a `pub`/`create` key (`KEY`, no pattern) composes a
/// new one under your namespace (write-prefixes). `FORMAT`/`SCHEMA`/`ENCODING`
/// value-names (the `--format` / `--encoding` flags) mean a declared-schema
/// position.
pub fn classify(value_name: Option<&str>) -> ContextKind {
    const SCHEMA_TOKENS: [&str; 3] = ["FORMAT", "SCHEMA", "ENCODING"];
    const READ_KEY_TOKENS: [&str; 2] = ["PATTERN", "EXPR"];
    const WRITE_KEY_TOKENS: [&str; 4] = ["KEY", "TOPIC", "VENDOR", "PREFIX"];
    match value_name {
        Some(name) => {
            let upper = name.to_ascii_uppercase();
            let has = |toks: &[&str]| toks.iter().any(|t| upper.contains(t));
            if has(&SCHEMA_TOKENS) {
                ContextKind::DeclaredSchemas
            } else if has(&READ_KEY_TOKENS) {
                ContextKind::RecentTopics
            } else if has(&WRITE_KEY_TOKENS) {
                ContextKind::WritePrefixes
            } else {
                ContextKind::None
            }
        }
        None => ContextKind::None,
    }
}

/// Pure: how a picked context item lands in the command line, and the list
/// label it renders under. The completion grammar is a property of the feed,
/// not the CLI — so it lives with the feed definition and ports with it.
///   - a write prefix continues the topic-key path, so it completes with a
///     trailing `/` (`acme` → `acme/…`);
///   - a recent topic / declared schema is a finished token, so it completes
///     with a trailing space.
///
/// `None` has no completion (empty string) and no label.
pub fn complete_form(kind: &ContextKind, item: &str) -> (String, &'static str) {
    match kind {
        ContextKind::WritePrefixes => (format!("{item}/"), "prefix"),
        ContextKind::RecentTopics => (format!("{item} "), "topic"),
        ContextKind::DeclaredSchemas => (format!("{item} "), "schema"),
        ContextKind::None => (String::new(), ""),
    }
}

/// Pure typeahead filter for a feed. An item matches when it starts with the
/// typed buffer (operator still typing it) OR the buffer starts with it
/// (operator finished a prefix and is typing the rest of the path). Empty
/// buffer => all items. Case-insensitive; order kept. Generic over the items,
/// so it carries to any CLI's feed.
pub fn filter_items(items: &[String], input: &str) -> Vec<String> {
    if input.is_empty() {
        return items.to_vec();
    }
    let needle = input.to_ascii_lowercase();
    items
        .iter()
        .filter(|it| {
            let hay = it.to_ascii_lowercase();
            hay.starts_with(&needle) || needle.starts_with(&hay)
        })
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// goat-specific adapter (the part another CLI would replace)
// ---------------------------------------------------------------------------

/// The slice of a catalog topic row the context feeds need. Keeping the
/// derivations over this plain struct (rather than `panoscope_core`'s wire
/// types) keeps them pure and unit-testable without a portal.
#[derive(Debug, Clone)]
pub struct TopicMeta {
    pub key: String,
    /// e.g. `protobuf:DenObservation`, `json`, `cbor`. Empty when a row
    /// declares no format.
    pub format: String,
    /// The registry's last-publish stamp, when the audit tailer has written
    /// it; `None` otherwise. Primary recency signal when present.
    pub last_seen_at: Option<i64>,
    /// The row's creation stamp — the recency fallback (the portal's
    /// `publish_count`/`last_seen_at` are often unpopulated today).
    pub created_at: i64,
}

/// How many recent topics to carry into the snapshot. A compose session needs
/// a useful typeahead pool, not the whole goatnet; the typeahead filter cuts
/// it to what matches as the operator types.
const RECENT_TOPICS_CAP: usize = 100;

/// Derive the recent-topics feed: distinct keys, newest first (last-seen then
/// created, both descending), capped.
pub fn recent_topics_state(topics: &[TopicMeta]) -> ContextState {
    let mut rows: Vec<&TopicMeta> = topics.iter().collect();
    rows.sort_by(|a, b| {
        b.last_seen_at
            .unwrap_or(0)
            .cmp(&a.last_seen_at.unwrap_or(0))
            .then(b.created_at.cmp(&a.created_at))
            .then(a.key.cmp(&b.key))
    });
    let mut seen = std::collections::HashSet::new();
    let items: Vec<String> = rows
        .into_iter()
        .map(|t| t.key.clone())
        .filter(|k| !k.is_empty() && seen.insert(k.clone()))
        .take(RECENT_TOPICS_CAP)
        .collect();
    ContextState::Loaded {
        title: "recent topics — keys declared on this goatnet (newest first)".to_string(),
        items,
    }
}

/// Derive the declared-schemas feed: the distinct non-empty formats declared
/// across the catalog, sorted.
pub fn declared_schemas_state(topics: &[TopicMeta]) -> ContextState {
    let mut items: Vec<String> = topics
        .iter()
        .map(|t| t.format.trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();
    items.sort();
    items.dedup();
    ContextState::Loaded {
        title: "declared schemas — formats registered on this goatnet".to_string(),
        items,
    }
}

/// Every declared topic key — the collision oracle for conflict warnings.
pub fn known_keys(topics: &[TopicMeta]) -> Vec<String> {
    topics
        .iter()
        .map(|t| t.key.clone())
        .filter(|k| !k.is_empty())
        .collect()
}

/// Read-only snapshot fetched once at TUI launch, holding every feed's
/// resolved state plus the raw data conflict detection needs. No feed is
/// fetched inside the event loop.
pub struct PortalSnapshot {
    write_prefixes: ContextState,
    recent_topics: ContextState,
    declared_schemas: ContextState,
    /// Declared keys (for collision detection). Empty when the catalog feed
    /// is unavailable.
    known_keys: Vec<String>,
    /// Prefixes the operator can publish under (for the can't-write warning).
    /// Empty when the write-prefixes feed is unavailable.
    writable: Vec<String>,
}

impl PortalSnapshot {
    /// Build from the two launch reads, each independently degradable:
    /// `prefixes` from `GET /api/orgs`, `topics` from `GET /api/catalog`.
    /// `Err(reason)` on either leaves that feed `Unavailable(reason)` and the
    /// composer fully usable on whatever did resolve.
    pub fn build(
        prefixes: Result<Vec<String>, String>,
        topics: Result<Vec<TopicMeta>, String>,
    ) -> Self {
        let (write_prefixes, writable) = match prefixes {
            Ok(mut p) => {
                p.sort();
                p.dedup();
                (
                    ContextState::Loaded {
                        title: "write prefixes — orgs you can publish under".to_string(),
                        items: p.clone(),
                    },
                    p,
                )
            }
            Err(reason) => (ContextState::Unavailable(reason), Vec::new()),
        };
        let (recent_topics, declared_schemas, known_keys) = match topics {
            Ok(t) => (
                recent_topics_state(&t),
                declared_schemas_state(&t),
                known_keys(&t),
            ),
            Err(reason) => (
                ContextState::Unavailable(reason.clone()),
                ContextState::Unavailable(reason),
                Vec::new(),
            ),
        };
        Self {
            write_prefixes,
            recent_topics,
            declared_schemas,
            known_keys,
            writable,
        }
    }

    /// A snapshot that reports every feed as unavailable (no profile, portal
    /// down before any read). The builder stays fully usable.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            write_prefixes: ContextState::Unavailable(reason.clone()),
            recent_topics: ContextState::Unavailable(reason.clone()),
            declared_schemas: ContextState::Unavailable(reason),
            known_keys: Vec::new(),
            writable: Vec::new(),
        }
    }

    /// A one-line conflict warning for the key being typed at a WRITE slot, or
    /// `None`. Pure, read-only against the launch snapshot. Two cases, checked
    /// in order — **can't write** (the leading prefix, once a `/` closes it,
    /// isn't one of your write-prefixes: the router may deny the publish,
    /// F-177 silent denial; only when the write-prefix feed resolved) then
    /// **would collide** (the exact key is already a declared topic, so a
    /// `topic create` would conflict; only when the catalog resolved).
    /// Applies only at a `WritePrefixes` position; read/schema positions and
    /// an empty buffer return `None`.
    pub fn conflict_for(&self, kind: &ContextKind, typed: &str) -> Option<String> {
        if *kind != ContextKind::WritePrefixes || typed.is_empty() {
            return None;
        }
        // The prefix segment is only "closed" once a '/' follows it; while the
        // operator is still typing the first segment it could still become a
        // valid prefix, so we don't warn yet.
        if let Some((prefix, _rest)) = typed.split_once('/') {
            if !self.writable.is_empty()
                && !prefix.is_empty()
                && !self.writable.iter().any(|w| w == prefix)
            {
                return Some(format!(
                    "`{prefix}` is not among your write-prefixes — the router may deny this write"
                ));
            }
        }
        if self.known_keys.iter().any(|k| k == typed) {
            return Some(format!(
                "`{typed}` already exists — a `topic create` would conflict"
            ));
        }
        None
    }
}

impl ContextSource for PortalSnapshot {
    fn lookup(&self, kind: &ContextKind) -> ContextState {
        match kind {
            ContextKind::WritePrefixes => self.write_prefixes.clone(),
            ContextKind::RecentTopics => self.recent_topics.clone(),
            ContextKind::DeclaredSchemas => self.declared_schemas.clone(),
            ContextKind::None => ContextState::NotApplicable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_write_key_tokens_are_write_prefixes() {
        assert_eq!(classify(Some("KEY")), ContextKind::WritePrefixes); // pub
        assert_eq!(classify(Some("key")), ContextKind::WritePrefixes); // lowercase id
        assert_eq!(classify(Some("TOPIC")), ContextKind::WritePrefixes);
        assert_eq!(classify(Some("VENDOR")), ContextKind::WritePrefixes);
        assert_eq!(classify(Some("PREFIX")), ContextKind::WritePrefixes);
    }

    #[test]
    fn classify_read_key_tokens_are_recent_topics() {
        // sub's `key_pattern` and flow's `key_expr` name an EXISTING key —
        // the read-token test wins over the `KEY` substring they also carry.
        assert_eq!(classify(Some("KEY_PATTERN")), ContextKind::RecentTopics);
        assert_eq!(classify(Some("key_expr")), ContextKind::RecentTopics);
        assert_eq!(classify(Some("PATTERN")), ContextKind::RecentTopics);
    }

    #[test]
    fn classify_format_tokens_are_declared_schemas() {
        assert_eq!(classify(Some("FORMAT")), ContextKind::DeclaredSchemas);
        assert_eq!(classify(Some("ENCODING")), ContextKind::DeclaredSchemas);
        assert_eq!(classify(Some("schema_ref")), ContextKind::DeclaredSchemas);
    }

    #[test]
    fn classify_is_none_for_unrelated_or_absent_positionals() {
        assert_eq!(classify(Some("PATH")), ContextKind::None);
        assert_eq!(classify(Some("TEXT")), ContextKind::None);
        assert_eq!(classify(None), ContextKind::None);
    }

    #[test]
    fn complete_form_matches_the_feed_grammar() {
        assert_eq!(
            complete_form(&ContextKind::WritePrefixes, "org11x"),
            ("org11x/".to_string(), "prefix")
        );
        assert_eq!(
            complete_form(&ContextKind::RecentTopics, "org11x/obs/v1"),
            ("org11x/obs/v1 ".to_string(), "topic")
        );
        assert_eq!(
            complete_form(&ContextKind::DeclaredSchemas, "json"),
            ("json ".to_string(), "schema")
        );
        assert_eq!(
            complete_form(&ContextKind::None, "x"),
            (String::new(), "")
        );
    }

    fn sample_topics() -> Vec<TopicMeta> {
        vec![
            TopicMeta {
                key: "org11x/observations/v1".into(),
                format: "protobuf:Observation".into(),
                last_seen_at: Some(200),
                created_at: 10,
            },
            TopicMeta {
                key: "goat-maintainer/probe/v2".into(),
                format: "json".into(),
                last_seen_at: None,
                created_at: 50,
            },
            TopicMeta {
                key: "goat-maintainer/test/v1".into(),
                format: "json".into(),
                last_seen_at: None,
                created_at: 30,
            },
            TopicMeta {
                key: "airdefenders/tracks/v1".into(),
                format: "".into(), // no declared format
                last_seen_at: Some(100),
                created_at: 5,
            },
        ]
    }

    #[test]
    fn recent_topics_are_newest_first_and_deduped() {
        match recent_topics_state(&sample_topics()) {
            ContextState::Loaded { items, .. } => {
                // last_seen 200 > 100 > (none: created 50 > 30) orders them.
                assert_eq!(
                    items,
                    vec![
                        "org11x/observations/v1".to_string(),
                        "airdefenders/tracks/v1".to_string(),
                        "goat-maintainer/probe/v2".to_string(),
                        "goat-maintainer/test/v1".to_string(),
                    ]
                );
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[test]
    fn declared_schemas_are_distinct_nonempty_sorted() {
        match declared_schemas_state(&sample_topics()) {
            ContextState::Loaded { items, .. } => {
                assert_eq!(items, vec!["json".to_string(), "protobuf:Observation".to_string()]);
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[test]
    fn lookup_maps_every_kind() {
        let snap = PortalSnapshot::build(
            Ok(vec!["goat-maintainer".into()]),
            Ok(sample_topics()),
        );
        assert!(matches!(
            snap.lookup(&ContextKind::WritePrefixes),
            ContextState::Loaded { .. }
        ));
        assert!(matches!(
            snap.lookup(&ContextKind::RecentTopics),
            ContextState::Loaded { .. }
        ));
        assert!(matches!(
            snap.lookup(&ContextKind::DeclaredSchemas),
            ContextState::Loaded { .. }
        ));
        assert_eq!(snap.lookup(&ContextKind::None), ContextState::NotApplicable);
    }

    #[test]
    fn a_failed_feed_is_unavailable_without_breaking_the_others() {
        // Catalog down, orgs up: write-prefixes still load; topic feeds say why.
        let snap = PortalSnapshot::build(
            Ok(vec!["goat-maintainer".into()]),
            Err("portal did not respond within 2s".into()),
        );
        assert!(matches!(
            snap.lookup(&ContextKind::WritePrefixes),
            ContextState::Loaded { .. }
        ));
        assert_eq!(
            snap.lookup(&ContextKind::RecentTopics),
            ContextState::Unavailable("portal did not respond within 2s".into())
        );
        assert_eq!(
            snap.lookup(&ContextKind::DeclaredSchemas),
            ContextState::Unavailable("portal did not respond within 2s".into())
        );
    }

    #[test]
    fn unavailable_snapshot_reports_every_feed() {
        let snap = PortalSnapshot::unavailable("no profile for context `zagros`");
        for kind in [
            ContextKind::WritePrefixes,
            ContextKind::RecentTopics,
            ContextKind::DeclaredSchemas,
        ] {
            assert_eq!(
                snap.lookup(&kind),
                ContextState::Unavailable("no profile for context `zagros`".into())
            );
        }
    }

    #[test]
    fn conflict_warns_when_prefix_is_not_writable() {
        let snap = PortalSnapshot::build(Ok(vec!["goat-maintainer".into()]), Ok(sample_topics()));
        // Typing under a prefix you don't control (segment closed by '/').
        let w = snap.conflict_for(&ContextKind::WritePrefixes, "org11x/new/v1");
        assert!(w.unwrap().contains("not among your write-prefixes"));
        // Your own prefix, novel key → no warning.
        assert_eq!(
            snap.conflict_for(&ContextKind::WritePrefixes, "goat-maintainer/new/v9"),
            None
        );
    }

    #[test]
    fn conflict_warns_when_the_key_already_exists() {
        let snap = PortalSnapshot::build(Ok(vec!["goat-maintainer".into()]), Ok(sample_topics()));
        let w = snap.conflict_for(&ContextKind::WritePrefixes, "goat-maintainer/test/v1");
        assert!(w.unwrap().contains("already exists"));
    }

    #[test]
    fn conflict_is_silent_while_still_typing_the_prefix_and_off_write_slots() {
        let snap = PortalSnapshot::build(Ok(vec!["goat-maintainer".into()]), Ok(sample_topics()));
        // Still typing the first segment (no '/') and not an existing key.
        assert_eq!(snap.conflict_for(&ContextKind::WritePrefixes, "andu"), None);
        // Empty buffer, and non-write positions, never warn.
        assert_eq!(snap.conflict_for(&ContextKind::WritePrefixes, ""), None);
        assert_eq!(
            snap.conflict_for(&ContextKind::RecentTopics, "goat-maintainer/test/v1"),
            None
        );
    }

    #[test]
    fn conflict_cant_warn_about_write_when_prefixes_unavailable() {
        // orgs feed down → no writable set → no false "can't write"; the
        // collision oracle (catalog) still fires.
        let snap = PortalSnapshot::build(Err("no profile".into()), Ok(sample_topics()));
        assert_eq!(
            snap.conflict_for(&ContextKind::WritePrefixes, "org11x/new/v1"),
            None
        );
        assert!(snap
            .conflict_for(&ContextKind::WritePrefixes, "goat-maintainer/test/v1")
            .unwrap()
            .contains("already exists"));
    }

    fn items() -> Vec<String> {
        vec![
            "andes-pat".into(),
            "airfield".into(),
            "demo-zed".into(),
        ]
    }

    #[test]
    fn filter_empty_returns_all() {
        assert_eq!(filter_items(&items(), ""), items());
    }

    #[test]
    fn filter_matches_prefix_being_typed() {
        assert_eq!(
            filter_items(&items(), "and"),
            vec!["andes-pat".to_string()]
        );
        assert_eq!(
            filter_items(&items(), "AIR"),
            vec!["airfield".to_string()]
        );
    }

    #[test]
    fn filter_keeps_prefix_when_typing_the_rest_of_the_path() {
        assert_eq!(
            filter_items(&items(), "andes-pat/observations/v1"),
            vec!["andes-pat".to_string()]
        );
    }

    #[test]
    fn filter_no_match_is_empty() {
        assert!(filter_items(&items(), "zzz").is_empty());
    }
}
