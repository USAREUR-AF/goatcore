// SPDX-License-Identifier: Apache-2.0
//
// Authenticated metadata history through the portal projection. An explicitly
// disabled walker returns no events; enabled readers require HTTPS and an
// operator token. Raw S3 reads are gated. Consumers attach their own semantics
// to put/delete/unknown records and must preserve incomplete-read errors.

use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;

/// Default for [`AuditTailConfig::fetch_concurrency`].
pub const DEFAULT_AUDIT_FETCH_CONCURRENCY: usize = 4;

/// Concurrent fetches for [`fetch_presigned_events`]. Every URL in a presigned
/// batch shares one short TTL; at most this many bodies are held at once.
const PRESIGNED_FETCH_CONCURRENCY: usize = 16;
use chrono::Utc;
use tracing::warn;
use url::Url;

use crate::audit_event::AuditEvent;

/// Enabled history requires portal authority and an operator token.
/// Legacy S3 fields remain for config compatibility, but cannot enable reads.
#[derive(Debug, Clone)]
pub struct AuditTailConfig {
    pub enabled: bool,
    pub portal_url: String,
    pub portal_api_key: String,
    /// S3 endpoint URL (e.g. `http://198.18.0.X:9000` on-mesh).
    pub endpoint: String,
    /// AWS region label — MinIO ignores the value but the signer requires one.
    pub region: String,
    /// Bucket name (default `audit-integrity` — the Block 82S substrate).
    pub bucket: String,
    /// Key-prefix root, typically `<site>/` (e.g. `site-a/`).
    pub key_prefix: String,
    pub access_key: String,
    pub secret_key: String,
    /// Per-object request timeout.
    pub request_timeout: Duration,
    /// Hard ceiling on objects fetched per call. Defends against a
    /// runaway bucket growing the working set.
    pub max_objects: usize,
    /// Kept for config compatibility. The portal walk reads one object at a
    /// time, so this value does not change it.
    pub fetch_concurrency: usize,
}

impl Default for AuditTailConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            portal_url: String::new(),
            portal_api_key: String::new(),
            endpoint: String::new(),
            region: "us-east-1".into(),
            bucket: "audit-integrity".into(),
            key_prefix: String::new(),
            access_key: String::new(),
            secret_key: String::new(),
            request_timeout: Duration::from_secs(30),
            max_objects: 5_000,
            fetch_concurrency: DEFAULT_AUDIT_FETCH_CONCURRENCY,
        }
    }
}

#[derive(Debug, Default)]
pub struct AuditTailStats {
    pub objects_listed: usize,
    pub objects_fetched: usize,
    pub events_parsed: usize,
}

pub struct AuditTail {
    cfg: AuditTailConfig,
    portal: crate::PortalClient,
}

/// Names of the fields an enabled tail cannot run without, in the order
/// an operator sets them.
fn missing_required(cfg: &AuditTailConfig) -> Vec<&'static str> {
    [
        ("portal_url", cfg.portal_url.is_empty()),
        ("portal_api_key", cfg.portal_api_key.is_empty()),
    ]
    .into_iter()
    .filter_map(|(name, empty)| empty.then_some(name))
    .collect()
}

impl AuditTail {
    pub fn new(cfg: AuditTailConfig) -> Result<Self> {
        if cfg.enabled {
            let missing = missing_required(&cfg);
            anyhow::ensure!(
                missing.is_empty(),
                "audit metadata requires {}; direct S3 access is gated",
                missing.join(", ")
            );
            let url = Url::parse(&cfg.portal_url)
                .context("parse audit portal URL")?;
            anyhow::ensure!(
                url.scheme() == "https" || (cfg!(test) && url.host_str() == Some("127.0.0.1")),
                "audit portal requires HTTPS"
            );
        }
        let portal = crate::PortalClient::new(cfg.portal_url.clone(), cfg.portal_api_key.clone());
        Ok(Self { cfg, portal })
    }
    pub fn is_enabled(&self) -> bool {
        self.cfg.enabled
    }
    pub async fn recent_publications(
        &self,
        hours: u32,
        vendor: Option<&str>,
    ) -> Result<(Vec<AuditEvent>, AuditTailStats)> {
        let (events, stats) = self.recent_events(hours, vendor).await?;
        let events = publications_only(events)?;
        Ok((events, stats))
    }
    pub async fn recent_events(
        &self,
        hours: u32,
        vendor: Option<&str>,
    ) -> Result<(Vec<AuditEvent>, AuditTailStats)> {
        let mut events = Vec::new();
        let mut bytes = 0usize;
        let mut overflow = false;
        let stats = self
            .stream_recent_events(hours, vendor, |e| {
                if overflow {
                    return;
                }
                match serde_json::to_vec(&e) {
                    Ok(encoded)
                        if bytes + encoded.len() <= 64 * 1024 * 1024 && events.len() < 100_000 =>
                    {
                        bytes += encoded.len();
                        events.push(e);
                    }
                    _ => overflow = true,
                }
            })
            .await?;
        anyhow::ensure!(!overflow, "audit collection exceeds byte or row limit");
        Ok((events, stats))
    }
    pub async fn stream_recent_events<F>(
        &self,
        hours: u32,
        vendor: Option<&str>,
        mut emit: F,
    ) -> Result<AuditTailStats>
    where
        F: FnMut(AuditEvent),
    {
        let mut stats = AuditTailStats::default();
        if !self.is_enabled() {
            return Ok(stats);
        }
        anyhow::ensure!(
            hours > 0 && hours <= 168,
            "audit metadata window must be 1–168 hours"
        );
        let since = (Utc::now() - chrono::Duration::hours(i64::from(hours))).to_rfc3339();
        let mut after: Option<String> = None;
        let mut total_rows = 0usize;
        loop {
            let page = self
                .portal
                .audit_history_page(&since, after.as_deref())
                .await?;
            stats.objects_listed += page.objects.len();
            anyhow::ensure!(
                stats.objects_listed <= self.cfg.max_objects,
                "audit metadata object limit exceeded"
            );
            let next = page.objects.last().map(|o| o.key.clone());
            if page.truncated {
                anyhow::ensure!(
                    next.is_some() && next > after,
                    "audit pagination did not advance"
                );
            }
            for object in &page.objects {
                let fetched = self
                    .portal
                    .audit_metadata(std::slice::from_ref(object))
                    .await?;
                anyhow::ensure!(
                    fetched.skipped.is_empty(),
                    "audit metadata objects unavailable: {:?}",
                    fetched.skipped
                );
                stats.objects_fetched += 1;
                total_rows += fetched.events.len();
                anyhow::ensure!(
                    total_rows <= 100_000,
                    "audit metadata aggregate row limit exceeded"
                );
                for event in fetched.events {
                    if vendor.is_none_or(|v| event_vendor_matches(&event, v)) {
                        stats.events_parsed += 1;
                        emit(event);
                    }
                }
            }
            if !page.truncated {
                break;
            }
            after = next;
        }
        Ok(stats)
    }
}

fn event_vendor_matches(event: &AuditEvent, vendor: &str) -> bool {
    event.vendor == vendor || crate::alias::key_vendor_matches(&event.key_expr, vendor)
}

fn publications_only(events: Vec<AuditEvent>) -> Result<Vec<AuditEvent>> {
    anyhow::ensure!(
        events.iter().all(|event| matches!(event.sample_kind.as_str(), "put" | "delete")),
        "audit history contains unknown operation kinds; publication history is incomplete"
    );
    Ok(events.into_iter().filter(|event| event.sample_kind == "put").collect())
}

/// One presigned object `fetch_presigned_events` could not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedObject {
    /// The object URL with its query string removed: the query carries the
    /// presign signature, which must not reach a terminal or a log.
    pub object: String,
    /// The HTTP status or transport error that stopped the read.
    pub reason: String,
}

/// What a presigned read returned: the events it parsed and every object it
/// could not read. A caller must look at `skipped` — a non-empty list means
/// the events are a partial view of the window (#1301).
#[derive(Debug, Default)]
pub struct PresignedFetch {
    pub events: Vec<AuditEvent>,
    pub skipped: Vec<SkippedObject>,
}

/// Fetch + parse NDJSON audit objects from portal-minted presigned GET URLs
/// (goat-cli G.6 / ADR 1038 read path — no MinIO credentials on the client;
/// the portal already signed the URLs). Returns every parsed `AuditEvent`
/// across all URLs in object order. The bytes flow client→MinIO directly.
///
/// The portal signs every URL in one call with a 5-minute TTL. Fetched one
/// at a time, a large window outlived the TTL, every URL still waiting
/// answered 403 and was skipped, and the caller exited 0 (#1301). So the
/// fetch is concurrent (bounded, order kept by `buffered`), and an object
/// that fails comes back in `skipped` instead of only a log line. A line that
/// fails to parse is still dropped: it is not a missing object.
pub async fn fetch_presigned_events(urls: &[String]) -> Result<PresignedFetch> {
    anyhow::ensure!(
        cfg!(test),
        "raw history reads are gated; use portal metadata projection"
    );
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("build presigned-fetch client")?;
    // Parse each body as it comes off the stream rather than collecting the
    // stream first: at most PRESIGNED_FETCH_CONCURRENCY bodies are held at
    // once, not every object of the window.
    let mut fetches = futures::stream::iter(urls.iter().map(|u| {
        let http = &http;
        async move { (u, fetch_presigned_object(http, u).await) }
    }))
    .buffered(PRESIGNED_FETCH_CONCURRENCY);

    let mut out = PresignedFetch::default();
    while let Some((u, res)) = fetches.next().await {
        let body = match res {
            Ok(b) => b,
            Err(reason) => {
                let object = redact_presigned_url(u);
                warn!(object = %object, reason = %reason, "presigned audit fetch failed");
                out.skipped.push(SkippedObject { object, reason });
                continue;
            }
        };
        for line in body.split(|b| *b == b'\n') {
            if line.is_empty() {
                continue;
            }
            if let Ok(ev) = serde_json::from_slice::<AuditEvent>(line) {
                out.events.push(ev);
            }
        }
    }
    Ok(out)
}

pub(crate) async fn fetch_presigned_object(
    http: &reqwest::Client,
    url: &str,
) -> std::result::Result<Vec<u8>, String> {
    let resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| format!("request failed: {}", e.without_url()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("body read failed: {}", e.without_url()))
}

/// `scheme://host/path` of a presigned URL, without the signing query.
pub(crate) fn redact_presigned_url(u: &str) -> String {
    match url::Url::parse(u) {
        Ok(mut parsed) => {
            parsed.set_query(None);
            parsed.set_fragment(None);
            parsed.to_string()
        }
        Err(_) => "<unparseable presigned url>".to_string(),
    }
}

/// Convert a `since` argument into a Unix-millis floor for client-side
/// filtering: a relative duration (`1h`, `90m`, `2h30m`, `7d`), an RFC3339
/// timestamp, or `None`/empty → `None` (no floor). The presign endpoint
/// lists objects at hour granularity, so this exact floor refines the window
/// the operator asked for.
pub fn since_floor_unix_ms(since: Option<&str>) -> Option<i64> {
    let s = since?.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    let secs = parse_duration_secs(s)?;
    Some(Utc::now().timestamp_millis() - secs * 1000)
}

/// Parse a compact duration like `1h`, `90m`, `2h30m`, `7d`, `45s` into
/// seconds. Returns `None` on any unrecognized unit or empty input.
fn parse_duration_secs(s: &str) -> Option<i64> {
    let mut total: i64 = 0;
    let mut num = String::new();
    let mut saw_unit = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: i64 = num.parse().ok()?;
        num.clear();
        let mult = match c {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86_400,
            _ => return None,
        };
        total += n * mult;
        saw_unit = true;
    }
    // A trailing bare number (no unit) is ambiguous — reject rather than
    // guess seconds-vs-minutes.
    if !num.is_empty() || !saw_unit {
        return None;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_history_excludes_deletes_and_refuses_unknown_kinds() {
        let put = AuditEvent { sample_kind: "put".into(), ..Default::default() };
        let delete = AuditEvent { sample_kind: "delete".into(), ..Default::default() };
        assert_eq!(publications_only(vec![put.clone(), delete]).unwrap(), vec![put.clone()]);
        assert!(publications_only(vec![put, AuditEvent::default()]).is_err());
        assert!(publications_only(vec![AuditEvent::default()]).is_err());
        assert!(publications_only(vec![]).unwrap().is_empty());
    }

    #[test]
    fn vendor_filter_keeps_slot_pool_keys() {
        let anchor = "0123456789abcdef0123456789abcdef";
        let event = AuditEvent { vendor: anchor.into(), key_expr: format!("{anchor}/vendor/topic"), ..Default::default() };
        assert!(event_vendor_matches(&event, "vendor"));
        assert!(event_vendor_matches(&event, anchor));
        assert!(!event_vendor_matches(&event, "foreign"));
    }

    /// Serve `/ok` with one NDJSON event and anything else with 403, the
    /// answer MinIO gives an expired presigned URL. Returns the base URL.
    #[allow(clippy::disallowed_methods)] // test-only server; dies with the test runtime
    async fn presign_stub() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    // Read until the end of the request head. A single
                    // read is enough today, but a client that sends its
                    // request line and headers in two writes must not be
                    // mistaken for one asking a different path.
                    let mut raw = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        match sock.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                raw.extend_from_slice(&buf[..n]);
                                if raw.windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                        }
                    }
                    let req = String::from_utf8_lossy(&raw);
                    let resp = if req.starts_with("GET /ok") {
                        let body = concat!(
                            r#"{"vendor":"v","key_expr":"v/a","ts":"2026-09-21T12:00:00.000Z","#,
                            r#""payload_hash":"sha256:00","payload_len":0}"#,
                            "\n"
                        );
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    } else {
                        "HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                            .to_string()
                    };
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn presigned_fetch_returns_an_expired_object_as_skipped() {
        let base = presign_stub().await;
        let urls = vec![
            format!("{base}/ok?X-Amz-Signature=aaa"),
            format!("{base}/expired.ndjson?X-Amz-Signature=secret"),
        ];
        let got = fetch_presigned_events(&urls).await.unwrap();
        assert_eq!(got.events.len(), 1);
        assert_eq!(got.skipped.len(), 1);
        assert_eq!(got.skipped[0].object, format!("{base}/expired.ndjson"));
        assert!(
            got.skipped[0].reason.contains("403"),
            "{}",
            got.skipped[0].reason
        );
        assert!(!got.skipped[0].object.contains("secret"));
    }

    #[tokio::test]
    async fn presigned_fetch_reads_every_object_of_a_large_window() {
        let base = presign_stub().await;
        let urls: Vec<String> = (0..40).map(|i| format!("{base}/ok?n={i}")).collect();
        let got = fetch_presigned_events(&urls).await.unwrap();
        assert_eq!(got.events.len(), 40);
        assert!(got.skipped.is_empty());
    }

    #[test]
    fn redact_presigned_url_drops_the_signature() {
        assert_eq!(
            redact_presigned_url(
                "http://minio:9000/audit/2026/09/21/12/b.ndjson?X-Amz-Signature=s#f"
            ),
            "http://minio:9000/audit/2026/09/21/12/b.ndjson"
        );
        assert_eq!(
            redact_presigned_url("not a url"),
            "<unparseable presigned url>"
        );
    }

    #[test]
    fn disabled_walker_returns_empty() {
        let cfg = AuditTailConfig::default();
        let tail = AuditTail::new(cfg).unwrap();
        assert!(!tail.is_enabled());
    }

    fn live_cfg() -> AuditTailConfig {
        AuditTailConfig {
            enabled: true,
            portal_url: "https://portal.invalid".into(),
            portal_api_key: "fixture-operator".into(),
            ..AuditTailConfig::default()
        }
    }

    #[test]
    fn enabled_with_credentials_builds_a_live_tail() {
        assert!(AuditTail::new(live_cfg()).unwrap().is_enabled());
    }

    #[test]
    fn enabled_without_a_secret_is_refused_and_names_the_gap() {
        let cfg = AuditTailConfig {
            portal_api_key: String::new(),
            ..live_cfg()
        };
        let msg = AuditTail::new(cfg)
            .err()
            .expect("an enabled tail with no secret must not build")
            .to_string();
        assert!(msg.contains("portal_api_key"), "{msg}");
        assert!(!msg.contains("portal_url"), "{msg}");
    }

    #[test]
    fn enabled_with_nothing_set_names_every_gap() {
        let cfg = AuditTailConfig {
            enabled: true,
            ..AuditTailConfig::default()
        };
        let msg = AuditTail::new(cfg).err().unwrap().to_string();
        assert!(msg.contains("portal_url, portal_api_key"), "{msg}");
    }

    #[test]
    fn default_fetch_concurrency_is_small() {
        assert_eq!(
            AuditTailConfig::default().fetch_concurrency,
            DEFAULT_AUDIT_FETCH_CONCURRENCY
        );
        assert_eq!(DEFAULT_AUDIT_FETCH_CONCURRENCY, 4);
    }

    #[test]
    fn parse_duration_secs_handles_units_and_combos() {
        assert_eq!(parse_duration_secs("45s"), Some(45));
        assert_eq!(parse_duration_secs("90m"), Some(5400));
        assert_eq!(parse_duration_secs("1h"), Some(3600));
        assert_eq!(parse_duration_secs("2h30m"), Some(9000));
        assert_eq!(parse_duration_secs("7d"), Some(604_800));
    }

    #[test]
    fn parse_duration_secs_rejects_bare_or_bad_unit() {
        assert_eq!(parse_duration_secs("30"), None); // no unit — ambiguous
        assert_eq!(parse_duration_secs("5y"), None); // unknown unit
        assert_eq!(parse_duration_secs(""), None);
        assert_eq!(parse_duration_secs("1h30"), None); // trailing bare number
    }

    #[test]
    fn since_floor_parses_duration_and_rfc3339_and_none() {
        assert!(since_floor_unix_ms(None).is_none());
        assert!(since_floor_unix_ms(Some("")).is_none());
        // RFC3339 absolute is honored exactly.
        let ts = "2026-06-11T13:00:00Z";
        let dt = chrono::DateTime::parse_from_rfc3339(ts).unwrap();
        assert_eq!(since_floor_unix_ms(Some(ts)), Some(dt.timestamp_millis()));
        // A duration produces a floor in the recent past.
        let now = Utc::now().timestamp_millis();
        let floor = since_floor_unix_ms(Some("1h")).unwrap();
        assert!(floor <= now && floor >= now - 3_600_000 - 5_000);
    }

    #[test]
    fn configured_walker_refuses_raw_storage_credentials() {
        let cfg = AuditTailConfig {
            enabled: true,
            endpoint: "https://storage.invalid".into(),
            access_key: "fixture".into(),
            secret_key: "fixture".into(),
            ..Default::default()
        };
        assert!(AuditTail::new(cfg).is_err());
    }
}
