// `goat replay` — Phase G.6 audit-tail read.
//
// Reads past events from the Object-Lock audit-tail instead of subscribing
// live. The operator profile carries no MinIO credentials; instead the portal
// mints short-TTL presigned GET URLs (ADR 1038) for the audit objects
// covering a time window, and goat fetches the NDJSON directly from MinIO over
// those URLs. The same read path backs `goat sub --since` and standalone
// `goat narrate`, so it lives here as a shared `fetch_events`.
//
// Key filtering is client-side: the audit objects are NDJSON batches mixing
// many keys, so the portal lists by time and goat intersects each event's
// key-expression against the requested pattern.

use base64::Engine;
use clap::Args;
use panoscope_core::alias::AliasRegistry;
use panoscope_core::audit_event::AuditEvent;
use panoscope_core::audit_tail::{fetch_presigned_events, since_floor_unix_ms, SkippedObject};
use panoscope_core::portal::PortalError;
use zenoh::key_expr::KeyExpr;

#[derive(Args, Debug)]
pub struct ReplayArgs {
    /// Key expression to replay from the audit-tail (e.g. `acme/**`).
    pub key_pattern: String,
    /// How far back: a duration (`1h`, `90m`, `2h30m`) or an RFC3339
    /// timestamp. Default: last 1h.
    #[arg(long)]
    pub since: Option<String>,
    /// Show only the newest N events in the window.
    #[arg(long)]
    pub count: Option<usize>,
    /// Emit one JSON line per event.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(ctx: &super::Ctx, args: ReplayArgs) -> anyhow::Result<()> {
    let reg = crate::alias::load_aliases(&ctx.profile).await;
    let pattern = crate::alias::resolve_key_volatile_aware(&reg, args.key_pattern.trim())
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let events = fetch_events(ctx, args.since.as_deref(), &pattern).await?;
    let total = events.len();
    let slice: &[AuditEvent] = match args.count {
        Some(n) if n < total => &events[total - n..],
        _ => &events,
    };

    for ev in slice {
        if args.json {
            println!("{}", serde_json::to_string(ev)?);
        } else {
            println!("{}", render_event(&reg, ctx.uuid, ev));
        }
    }

    if total == 0 {
        eprintln!(
            "no audit events for `{}` in the requested window",
            crate::alias::display_key_volatile_aware(&reg, ctx.uuid, &pattern)
        );
    } else if slice.len() < total {
        eprintln!("{} event(s) (newest of {total})", slice.len());
    } else {
        eprintln!("{total} event(s)");
    }
    Ok(())
}

/// Shared G.6 read path: portal-presign → fetch NDJSON over the presigned
/// URLs → filter by key intersection + exact `since` floor → sort by
/// timestamp. Used by `goat replay`, `goat sub --since`, and standalone
/// `goat narrate`. `key_pattern` is the already-resolved (UUID-anchored) key
/// expression; `**` matches everything.
///
/// A window with any unread object is refused as a whole, even when the
/// caller only wants the newest N of it (`replay --count`, `sub --take`):
/// the fetcher cannot tell whether the unread object held events newer than
/// the ones it did read, so a "newest N" cut of a partial window is itself
/// unproven. Exit 0 keeps meaning "complete" (#1301).
pub(crate) async fn fetch_events(
    ctx: &super::Ctx,
    since: Option<&str>,
    key_pattern: &str,
) -> anyhow::Result<Vec<AuditEvent>> {
    let w = fetch_events_detailed(ctx, since, key_pattern).await?;
    // A skipped object holds events of this window the read never saw.
    // Printing the rest and exiting 0 presents a partial window as the whole
    // one, and downstream exports rely on a clean exit meaning complete (#1301).
    if !w.skipped.is_empty() {
        anyhow::bail!("{}", skipped_objects_message(&w.skipped, w.object_count));
    }
    if w.truncated {
        eprintln!(
            "note: window truncated to {} audit objects — narrow --since for older events",
            w.object_count
        );
    }
    Ok(w.events)
}

/// One audit-tail read, with the metadata `goat flow` needs as DATA rather
/// than as a stderr note.
pub(crate) struct AuditWindow {
    pub events: Vec<AuditEvent>,
    /// The portal capped the object listing. A truncated window can no
    /// longer prove a topic never published — it only saw part of it.
    pub truncated: bool,
    pub object_count: usize,
    /// The **effective** floor the portal actually applied, RFC3339.
    ///
    /// Load-bearing: the operator's `--since` is only a REQUEST. The portal
    /// silently clamps it (`auditPresignMaxWindow`, 7d) and echoes back what
    /// it really used. Reporting a horizon derived from the request would
    /// claim proof-of-absence over a stretch nobody examined — so callers
    /// must derive their horizon from THIS, not from the argument.
    pub effective_since: String,
    /// Objects listed for the window that could not be read. Non-empty means
    /// `events` is a partial view, like `truncated`.
    pub skipped: Vec<SkippedObject>,
}

/// The refusal `fetch_events` returns when objects were skipped: how many,
/// which ones (capped), and what to do.
pub(crate) fn skipped_objects_message(skipped: &[SkippedObject], object_count: usize) -> String {
    const SHOWN: usize = 20;
    let mut msg = format!(
        "{} of {object_count} audit object(s) in the window could not be read; \
         the events would be incomplete, so none are printed:",
        skipped.len()
    );
    for s in skipped.iter().take(SHOWN) {
        msg.push_str(&format!("\n  {} ({})", s.object, s.reason));
    }
    if skipped.len() > SHOWN {
        msg.push_str(&format!("\n  … and {} more", skipped.len() - SHOWN));
    }
    msg.push('\n');
    msg.push_str(skipped_objects_remedy(skipped));
    msg
}

/// The remedy line, keyed on the reasons actually seen. Advising "narrow
/// --since" for a 403 is right (the presigned URLs expired mid-run); the
/// same advice for a MinIO outage or a 404 would send the operator down a
/// path that cannot help.
fn skipped_objects_remedy(skipped: &[SkippedObject]) -> &'static str {
    let all = |pred: fn(&str) -> bool| skipped.iter().all(|s| pred(&s.reason));
    if all(|r| r.starts_with("HTTP 403")) {
        "HTTP 403 means the presigned URLs expired (5-minute TTL) before the \
         read reached them: narrow --since and re-run."
    } else if all(|r| r.starts_with("HTTP 404")) {
        "HTTP 404 means the object was listed but is gone from the store \
         (retired or removed between list and fetch): re-run; if it \
         persists, check the audit bucket's lifecycle rules."
    } else if all(|r| !r.starts_with("HTTP ")) {
        "The requests themselves failed (no HTTP status): the object store \
         did not answer. Check MinIO / the mesh path to it before re-running; \
         narrowing --since will not help."
    } else {
        "Mixed causes: HTTP 403 = presigned URL expired (narrow --since); \
         HTTP 404 = object gone from the store; no status = object store \
         unreachable."
    }
}

/// As `fetch_events`, but returns the window metadata instead of printing a
/// warning. `goat flow` needs the truncation flag to downgrade the audit
/// tier to `Partial` (forfeiting its power to prove absence) and the
/// effective floor to state an honest horizon. Callers that just want the
/// events and a human warning use `fetch_events`.
pub(crate) async fn fetch_events_detailed(
    ctx: &super::Ctx,
    since: Option<&str>,
    key_pattern: &str,
) -> anyhow::Result<AuditWindow> {
    let portal = ctx.profile.portal_client()?;
    let wire_since = since.map(normalize_since_for_portal);
    let presign = portal
        .audit_presign(wire_since.as_deref(), Some(key_pattern))
        .await
        .map_err(portal_audit_err)?;
    let truncated = presign.truncated;
    let object_count = presign.count;
    let effective_since = presign.since.clone();
    let urls: Vec<String> = presign.objects.iter().map(|o| o.url.clone()).collect();
    let fetched = fetch_presigned_events(&urls).await?;
    let skipped = fetched.skipped;
    let mut events = fetched.events;

    events.retain(|ev| key_matches(key_pattern, &ev.key_expr));
    if let Some(floor) = since_floor_unix_ms(since) {
        // Object listing is hour-granular; this refines to the exact floor.
        // Events with an unparseable ts are kept rather than silently dropped.
        events.retain(|ev| ev.ts_unix_ms().map(|t| t >= floor).unwrap_or(true));
    }
    events.sort_by_key(|ev| ev.ts_unix_ms().unwrap_or(0));
    Ok(AuditWindow {
        events,
        truncated,
        object_count,
        effective_since,
        skipped,
    })
}

/// Rewrite a `--since` duration containing days into a unit the portal can
/// actually parse.
///
/// The portal parses `since` with Go's `time.ParseDuration`
/// (`audit_presign.go`), which supports ns/us/ms/s/m/h and **no `d`**. So
/// `--since 7d` fails duration parsing, then fails RFC3339 parsing, and
/// comes back 400 — the audit tier then reports itself *unreachable*, which
/// reads as an outage rather than a rejected argument. Our own grammar does
/// accept `d`, and `goat flow --since`'s help mentions a 7-day clamp, so
/// `7d` is the natural thing to type and the one thing that fails.
///
/// Normalizing here rather than in the caller fixes every consumer of the
/// audit tail at once — `goat replay`, `goat sub --since`, `goat narrate`
/// and `goat flow` — instead of only the newest one.
///
/// Conservative by construction: strings without a `d` are passed through
/// byte-for-byte, and anything `since_to_ms` cannot parse as a pure
/// duration (an RFC3339 timestamp, or garbage the portal should reject on
/// its own terms) is left alone.
pub(crate) fn normalize_since_for_portal(since: &str) -> String {
    let s = since.trim();
    if !s.contains('d') {
        return s.to_string();
    }
    let Some(ms) = crate::cmd::flow::since_to_ms(s) else {
        return s.to_string();
    };
    let secs = ms / 1000;
    // Emit the coarsest unit that divides exactly, so the wire value stays
    // legible in a portal log (`7d` → `168h`, not `604800s`).
    if secs % 3_600 == 0 {
        format!("{}h", secs / 3_600)
    } else if secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

/// Render one audit event as a line: timestamp, key (alias-rendered), byte
/// length, payload hash, and the inlined payload when present + printable.
pub(crate) fn render_event(reg: &AliasRegistry, uuid: bool, ev: &AuditEvent) -> String {
    let key = crate::alias::display_key_volatile_aware(reg, uuid, &ev.key_expr);
    let mut line = format!("{}  {}  {}B  {}", ev.ts, key, ev.payload_len, ev.payload_hash);
    if let Some(b64) = &ev.payload_bytes_b64 {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
            if let Ok(s) = std::str::from_utf8(&bytes) {
                if s.chars().all(|c| !c.is_control() || c == '\n' || c == '\t') {
                    line.push_str("  ");
                    line.push_str(s.trim_end());
                }
            }
        }
    }
    line
}

/// True when `key` falls under `pattern` (Zenoh key-expression intersection).
/// `**` matches everything. Falls back to exact-match if either side fails to
/// parse as a key expression.
fn key_matches(pattern: &str, key: &str) -> bool {
    match (
        KeyExpr::new(pattern.to_string()),
        KeyExpr::new(key.to_string()),
    ) {
        (Ok(p), Ok(k)) => p.intersects(&k),
        _ => pattern == key,
    }
}

/// Map a portal error from the presign call into an operator-facing message.
/// The audit-read endpoint is operator-gated and depends on server-side MinIO
/// config, so 403 / 503 get specific hints.
pub(crate) fn portal_audit_err(e: PortalError) -> anyhow::Error {
    match e {
        PortalError::Disabled => anyhow::anyhow!(
            "no portal configured in this profile — run `goat profile init <bundle.cbor>` (or `goat doctor`)"
        ),
        PortalError::Http { status, .. } if status.as_u16() == 503 => anyhow::anyhow!(
            "this portal has no audit-tail read configured (no MinIO endpoint/credentials server-side); \
             the audit read path is unavailable here"
        ),
        PortalError::Http { status, .. } if status.as_u16() == 403 => anyhow::anyhow!(
            "audit-tail read is operator-gated — this token is not an operator key"
        ),
        PortalError::Http { status, .. } if status.as_u16() == 401 => anyhow::anyhow!(
            "portal rejected the token (E_TOKEN_INVALID) — check `goat profile show`"
        ),
        other => anyhow::anyhow!("audit presign failed: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skipped(n: usize) -> Vec<SkippedObject> {
        skipped_with(n, "HTTP 403 Forbidden")
    }

    fn skipped_with(n: usize, reason: &str) -> Vec<SkippedObject> {
        (0..n)
            .map(|i| SkippedObject {
                object: format!("http://minio:9000/audit/b{i}.ndjson"),
                reason: reason.to_string(),
            })
            .collect()
    }

    #[test]
    fn remedy_follows_the_reasons_seen_not_a_fixed_ttl_story() {
        // Every object refused: the TTL story is right, and only then.
        assert!(skipped_objects_message(&skipped(3), 9).contains("narrow --since"));
        // Object store down: no HTTP status at all. "narrow --since" would
        // send the operator down a path that cannot help.
        let down =
            skipped_objects_message(&skipped_with(4, "request failed: error sending request"), 9);
        assert!(down.contains("did not answer"), "{down}");
        assert!(!down.contains("narrow --since and re-run"), "{down}");
        // Object retired between list and fetch.
        let gone = skipped_objects_message(&skipped_with(1, "HTTP 404 Not Found"), 9);
        assert!(gone.contains("gone from the store"), "{gone}");
        assert!(!gone.contains("narrow --since and re-run"), "{gone}");
        // A mix names each cause rather than picking one.
        let mut mixed = skipped_with(1, "HTTP 403 Forbidden");
        mixed.extend(skipped_with(1, "request failed: timed out"));
        let msg = skipped_objects_message(&mixed, 9);
        assert!(msg.contains("Mixed causes"), "{msg}");
    }

    #[test]
    fn skipped_objects_message_names_each_object_and_the_ttl_remedy() {
        let msg = skipped_objects_message(&skipped(3), 500);
        assert!(msg.starts_with("3 of 500 audit object(s)"), "{msg}");
        assert!(msg.contains("http://minio:9000/audit/b2.ndjson (HTTP 403 Forbidden)"));
        assert!(msg.contains("narrow --since"));
    }

    #[test]
    fn skipped_objects_message_caps_the_list() {
        let msg = skipped_objects_message(&skipped(25), 500);
        assert!(msg.contains("b19.ndjson"));
        assert!(!msg.contains("b20.ndjson"));
        assert!(msg.contains("… and 5 more"));
    }

    #[test]
    fn days_are_normalized_because_go_parseduration_has_no_d_unit() {
        // The portal parses `since` with Go's time.ParseDuration, which
        // supports ns/us/ms/s/m/h only. `7d` was rejected outright (400) and
        // surfaced as an *unreachable* audit tier — an outage-shaped message
        // for a rejected argument.
        assert_eq!(normalize_since_for_portal("7d"), "168h");
        assert_eq!(normalize_since_for_portal("1d"), "24h");
        assert_eq!(normalize_since_for_portal("1d12h"), "36h");
        // Not a whole number of hours — drop to the next exact unit rather
        // than round.
        assert_eq!(normalize_since_for_portal("1d30m"), "1470m");
    }

    #[test]
    fn normalization_leaves_alone_everything_the_portal_already_accepts() {
        for s in ["1h", "90m", "2h30m", "45s", ""] {
            assert_eq!(normalize_since_for_portal(s), s, "rewrote a working value: {s}");
        }
        // An RFC3339 timestamp must pass through byte-for-byte.
        let ts = "2026-07-01T00:00:00Z";
        assert_eq!(normalize_since_for_portal(ts), ts);
        // Garbage is the portal's to reject, on its own terms — not ours to
        // silently reshape into something that parses.
        assert_eq!(normalize_since_for_portal("7dogs"), "7dogs");
    }

    fn ev(key: &str, ts: &str, len: usize, b64: Option<&str>) -> AuditEvent {
        AuditEvent {
            vendor: key.split('/').next().unwrap_or("").to_string(),
            key_expr: key.to_string(),
            ts: ts.to_string(),
            src_zid: String::new(),
            payload_hash: "sha256:deadbeef".to_string(),
            payload_len: len,
            payload_bytes_b64: b64.map(|s| s.to_string()),
            audit_subscriber_id: String::new(),
        }
    }

    #[test]
    fn key_matches_handles_globs_and_exact() {
        assert!(key_matches("org11x/**", "org11x/obs/v1"));
        assert!(key_matches("**", "anything/at/all"));
        assert!(key_matches("acme/obs/v1", "acme/obs/v1"));
        assert!(!key_matches("org11x/**", "acme/obs/v1"));
    }

    #[test]
    fn key_matches_volatile_prefix() {
        assert!(key_matches("volatile/acme/**", "volatile/acme/blob/x/chunk/00000000"));
        assert!(!key_matches("volatile/acme/**", "volatile/other/stream/y"));
    }

    #[test]
    fn render_event_inlines_printable_payload() {
        let b64 = base64::engine::general_purpose::STANDARD.encode("hello world");
        let reg = AliasRegistry::empty();
        let line = render_event(&reg, true, &ev("acme/hello/v1", "2026-06-11T13:00:00Z", 11, Some(&b64)));
        assert!(line.contains("acme/hello/v1"));
        assert!(line.contains("11B"));
        assert!(line.ends_with("hello world"));
    }

    #[test]
    fn render_event_omits_binary_payload() {
        // A control-char payload is not appended (stays hash-only).
        let b64 = base64::engine::general_purpose::STANDARD.encode([0x00u8, 0x01, 0x02]);
        let reg = AliasRegistry::empty();
        let line = render_event(&reg, true, &ev("acme/raw/v1", "2026-06-11T13:00:00Z", 3, Some(&b64)));
        assert!(line.contains("acme/raw/v1"));
        assert!(!line.contains('\u{0}'));
    }
}
