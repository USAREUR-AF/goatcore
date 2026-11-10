// SPDX-License-Identifier: Apache-2.0
//
// Generic S3 NDJSON walker over the audit-subscriber's MinIO bucket
// layout. Returns parsed AuditEvent values; consumers attach their
// own apply-side semantics:
//
//   - panoscope: replay-into-topology (peer upsert + flow record_event)
//   - stethoscope: flow-rate count by vendor/key, decode-sample fetch,
//     audit-latency histogram
//
// Bucket layout (per ops/zenoh-trial-peers/audit-subscriber):
//   <bucket>/<key_prefix><YYYY>/<MM>/<DD>/<HH>/<batch>.ndjson
// where `key_prefix` typically scopes by site (e.g. "site-a/").
//
// Auth model: disabled-by-default. A disabled tail (the default) is a
// no-op: the walker returns `Ok(empty)` for every call, so consumers get
// a clean "no events" verdict rather than a fatal error, matching the
// portal-disabled UX shape. An *enabled* tail with an empty endpoint or
// credential is a configuration error, not a silent no-op; see
// `AuditTailConfig` for the refusal.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{Datelike, Timelike, Utc};
use futures::StreamExt;
use rusty_s3::{actions::S3Action, Bucket, Credentials, UrlStyle};
use serde::Deserialize;
use tracing::{debug, warn};
use url::Url;

use crate::audit_event::AuditEvent;

/// Default for [`AuditTailConfig::fetch_concurrency`]. Bounded rather than
/// unbounded: this reads a shared MinIO other surfaces depend on, and a burst
/// of parallel GETs would trade one surface's latency for everyone else's.
/// Each in-flight fetch can hold a whole object body, so this number times the
/// largest object is the walker's peak body memory.
pub const DEFAULT_AUDIT_FETCH_CONCURRENCY: usize = 4;

/// Concurrent fetches for [`fetch_presigned_events`]. Higher than the walker's
/// default because every URL in a presigned batch shares one short TTL; at most
/// this many bodies are held at once.
const PRESIGNED_FETCH_CONCURRENCY: usize = 16;

/// Operator-tunable config. `enabled` gates the real S3 calls; the
/// defaults make the walker a no-op so a missing MinIO substrate doesn't
/// crash startup. `enabled` with an empty endpoint or credential is a
/// configuration error and [`AuditTail::new`] refuses it, so no service
/// can report the tail as on while running without it.
#[derive(Debug, Clone)]
pub struct AuditTailConfig {
    pub enabled: bool,
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
    /// Objects fetched concurrently, in listing order. Each body is parsed
    /// and dropped when its turn in listing order comes, so at most this many
    /// object bodies are held at once: peak body memory is about
    /// `fetch_concurrency` x the largest object. `0` is treated as `1`.
    pub fetch_concurrency: usize,
}

impl Default for AuditTailConfig {
    fn default() -> Self {
        Self {
            enabled: false,
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

/// Presigned-URL lifetime — long enough for slow MinIO replies, short
/// enough that a leaked URL ages out fast.
const PRESIGN_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Default)]
pub struct AuditTailStats {
    pub objects_listed: usize,
    pub objects_fetched: usize,
    pub events_parsed: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListObjectsV2Response {
    #[serde(default, rename = "Contents")]
    contents: Vec<ListObjectsV2Content>,
    #[serde(default, rename = "NextContinuationToken")]
    next_continuation_token: Option<String>,
    #[serde(default, rename = "IsTruncated")]
    #[allow(dead_code)]
    is_truncated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListObjectsV2Content {
    #[serde(rename = "Key")]
    key: String,
}

pub struct AuditTail {
    cfg: AuditTailConfig,
    http: reqwest::Client,
    bucket: Option<Bucket>,
    creds: Option<Credentials>,
}

/// Names of the fields an enabled tail cannot run without, in the order
/// an operator sets them.
fn missing_required(cfg: &AuditTailConfig) -> Vec<&'static str> {
    [
        ("endpoint", cfg.endpoint.is_empty()),
        ("access_key", cfg.access_key.is_empty()),
        ("secret_key", cfg.secret_key.is_empty()),
    ]
    .into_iter()
    .filter_map(|(name, empty)| empty.then_some(name))
    .collect()
}

impl AuditTail {
    pub fn new(cfg: AuditTailConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(cfg.request_timeout)
            .build()
            .context("build reqwest client")?;
        let (bucket, creds) = if cfg.enabled {
            let missing = missing_required(&cfg);
            if !missing.is_empty() {
                bail!(
                    "audit tail enabled but {} empty: supply them or set enabled=false",
                    missing.join(", ")
                );
            }
            let endpoint = Url::parse(&cfg.endpoint)
                .with_context(|| format!("parse endpoint url {}", cfg.endpoint))?;
            let b = Bucket::new(endpoint, UrlStyle::Path, cfg.bucket.clone(), cfg.region.clone())
                .map_err(|e| anyhow!("Bucket::new: {e}"))?;
            (Some(b), Some(Credentials::new(&cfg.access_key, &cfg.secret_key)))
        } else {
            (None, None)
        };
        Ok(Self {
            cfg,
            http,
            bucket,
            creds,
        })
    }

    /// True iff this tail will hit the network when called.
    pub fn is_enabled(&self) -> bool {
        self.bucket.is_some()
    }

    /// Walk the last `window_hours` of audit objects and return every
    /// parsed AuditEvent. Filters out events whose vendor doesn't
    /// match `vendor_filter` (`None` = all vendors). Returns the raw
    /// event list — caller decides what to aggregate (count, decode,
    /// histogram, etc.).
    ///
    /// MATERIALIZES THE WHOLE WINDOW. That is fine for the short, filtered
    /// windows most callers use (1 h, one vendor), and NOT fine for a wide
    /// one: panoscope's 168 h unfiltered replay parsed 32,876,051 events to
    /// derive 166 unique peers and peaked at 20 GiB doing it. If your
    /// window is wide, or you only fold events into an accumulator and never
    /// need them all at once, call [`Self::stream_recent_events`] instead —
    /// it is the same walk with the same ordering and stats, minus the Vec.
    pub async fn recent_events(
        &self,
        window_hours: u32,
        vendor_filter: Option<&str>,
    ) -> Result<(Vec<AuditEvent>, AuditTailStats)> {
        let mut events: Vec<AuditEvent> = Vec::new();
        let stats = self
            .stream_recent_events(window_hours, vendor_filter, |ev| events.push(ev))
            .await?;
        Ok((events, stats))
    }

    /// The walk, streaming. Invokes `on_event` for each parsed AuditEvent as
    /// it is decoded and keeps no events. Objects are fetched
    /// `cfg.fetch_concurrency` at a time and each body is dropped once parsed,
    /// so peak memory is about `fetch_concurrency` object bodies, independent
    /// of the window or page size.
    ///
    /// Ordering, filtering, the max_objects ceiling and the returned stats are
    /// IDENTICAL to [`Self::recent_events`] — that method is now a thin wrapper
    /// around this one, so there is a single walk implementation and the two
    /// cannot drift.
    pub async fn stream_recent_events<F>(
        &self,
        window_hours: u32,
        vendor_filter: Option<&str>,
        mut on_event: F,
    ) -> Result<AuditTailStats>
    where
        F: FnMut(AuditEvent),
    {
        if !self.is_enabled() {
            return Ok(AuditTailStats::default());
        }
        let bucket = self.bucket.as_ref().unwrap();
        let creds = self.creds.as_ref().unwrap();

        let prefixes = build_hour_prefixes(&self.cfg.key_prefix, window_hours);
        let mut stats = AuditTailStats::default();

        'outer: for hour_prefix in prefixes {
            let mut continuation: Option<String> = None;
            loop {
                let resp = list_objects(
                    &self.http,
                    bucket,
                    creds,
                    &hour_prefix,
                    continuation.as_deref(),
                )
                .await
                .with_context(|| format!("list_objects_v2 prefix={hour_prefix}"))?;

                // Fetching objects ONE AT A TIME makes the wait the SUM of every round-trip,
                // and a 24 h window can hold thousands of objects, even against a MinIO that
                // answers a single request in milliseconds.
                //
                // Fetch a page's objects concurrently, bounded, and parse each
                // as it comes off the stream. `buffered` (not
                // `buffer_unordered`) keeps listing order, and it holds at most
                // `fetch_concurrency` futures — in flight or finished — so no
                // more than that many bodies are alive at once. Collecting the
                // stream first would hold every body of the page (up to 1000).
                let take = self
                    .cfg
                    .max_objects
                    .saturating_sub(stats.objects_fetched)
                    .min(resp.contents.len());
                if take < resp.contents.len() {
                    warn!(
                        max_objects = self.cfg.max_objects,
                        "audit_tail: hit max_objects ceiling — returning partial"
                    );
                }
                stats.objects_listed += resp.contents.len();
                let hit_ceiling = take < resp.contents.len();
                let mut fetches = futures::stream::iter(
                    resp.contents.into_iter().take(take).map(|o| {
                        let http = &self.http;
                        async move {
                            let r = fetch_object(http, bucket, creds, &o.key).await;
                            (o.key, r)
                        }
                    }),
                )
                .buffered(self.cfg.fetch_concurrency.max(1));

                while let Some((key, res)) = fetches.next().await {
                    match res {
                        Ok(body) => {
                            stats.objects_fetched += 1;
                            for line in body.split(|b| *b == b'\n') {
                                if line.is_empty() {
                                    continue;
                                }
                                let Ok(ev) = serde_json::from_slice::<AuditEvent>(line) else {
                                    continue;
                                };
                                stats.events_parsed += 1;
                                // Anchor-aware vendor match: `ev.vendor` is the
                                // raw first key segment (writer-side contract),
                                // which on slot-pool goatnets is the UUID anchor,
                                // not the vendor name — match the key's vendor
                                // segment too or `vendor=tarsier` reads 0 events
                                // while traffic flows.
                                if let Some(v) = vendor_filter {
                                    if ev.vendor != v
                                        && crate::alias::vendor_segment(&ev.key_expr) != v
                                    {
                                        continue;
                                    }
                                }
                                on_event(ev);
                            }
                        }
                        Err(e) => {
                            debug!(error = %e, key = %key, "audit_tail: skip object");
                        }
                    }
                }
                if hit_ceiling {
                    break 'outer;
                }

                match resp.next_continuation_token {
                    Some(t) => continuation = Some(t),
                    None => break,
                }
            }
        }

        Ok(stats)
    }
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

async fn fetch_presigned_object(
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
fn redact_presigned_url(u: &str) -> String {
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

async fn list_objects(
    http: &reqwest::Client,
    bucket: &Bucket,
    creds: &Credentials,
    prefix: &str,
    continuation: Option<&str>,
) -> Result<ListObjectsV2Response> {
    let mut action = bucket.list_objects_v2(Some(creds));
    let q = action.query_mut();
    q.insert("prefix", prefix.to_string());
    q.insert("max-keys", "1000".to_string());
    if let Some(token) = continuation {
        q.insert("continuation-token", token.to_string());
    }
    let url = action.sign(PRESIGN_TTL);
    let resp = http
        .get(url)
        .send()
        .await
        .context("list_objects_v2 GET")?;
    let status = resp.status();
    let body = resp.bytes().await.context("list_objects_v2 body read")?;
    if !status.is_success() {
        return Err(anyhow!(
            "list_objects_v2 returned {} body={}",
            status,
            String::from_utf8_lossy(&body).chars().take(200).collect::<String>()
        ));
    }
    quick_xml::de::from_reader(body.as_ref())
        .context("parse ListObjectsV2 XML")
}

async fn fetch_object(
    http: &reqwest::Client,
    bucket: &Bucket,
    creds: &Credentials,
    key: &str,
) -> Result<Vec<u8>> {
    let action = bucket.get_object(Some(creds), key);
    let url = action.sign(PRESIGN_TTL);
    let resp = http
        .get(url)
        .send()
        .await
        .with_context(|| format!("get_object {key}"))?;
    if !resp.status().is_success() {
        return Err(anyhow!("get_object {key} returned {}", resp.status()));
    }
    Ok(resp.bytes().await.context("get_object body read")?.to_vec())
}

/// Build the hour-level prefix list for a window, newest first.
/// audit-subscriber path layout: `<key_prefix><YYYY>/<MM>/<DD>/<HH>/...`.
fn build_hour_prefixes(root_prefix: &str, window_hours: u32) -> Vec<String> {
    let now = Utc::now();
    let mut out = Vec::with_capacity(window_hours as usize);
    for h in 0..window_hours {
        let t = now - chrono::Duration::hours(h as i64);
        out.push(format!(
            "{root_prefix}{:04}/{:02}/{:02}/{:02}/",
            t.year_ce().1,
            t.month(),
            t.day(),
            t.hour()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgraded_xml_parser_preserves_paginated_s3_listing() {
        let xml = br#"<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
          <IsTruncated>true</IsTruncated><NextContinuationToken>a&amp;b</NextContinuationToken>
          <Contents><Key>audit/one&amp;two.jsonl</Key></Contents>
          <Contents><Key>audit/second.jsonl</Key></Contents>
        </ListBucketResult>"#;
        let listing: ListObjectsV2Response = quick_xml::de::from_reader(&xml[..]).unwrap();
        assert!(listing.is_truncated);
        assert_eq!(listing.next_continuation_token.as_deref(), Some("a&b"));
        assert_eq!(listing.contents.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
                   vec!["audit/one&two.jsonl", "audit/second.jsonl"]);
        let empty: ListObjectsV2Response = quick_xml::de::from_reader(&b"<ListBucketResult/>"[..]).unwrap();
        assert!(empty.contents.is_empty());
        assert!(empty.next_continuation_token.is_none());
        assert!(quick_xml::de::from_reader::<_, ListObjectsV2Response>(&b"<ListBucketResult><Contents></ListBucketResult>"[..]).is_err());
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
        assert!(got.skipped[0].reason.contains("403"), "{}", got.skipped[0].reason);
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
        assert_eq!(redact_presigned_url("not a url"), "<unparseable presigned url>");
    }

    #[test]
    fn disabled_walker_returns_empty() {
        let cfg = AuditTailConfig::default();
        let tail = AuditTail::new(cfg).unwrap();
        assert!(!tail.is_enabled());
        // Smoke: hour-prefix builder produces window_hours entries.
        assert_eq!(build_hour_prefixes("nova-a/", 3).len(), 3);
    }

    fn live_cfg() -> AuditTailConfig {
        AuditTailConfig {
            enabled: true,
            endpoint: "http://127.0.0.1:9000".into(),
            access_key: "reader".into(),
            secret_key: "reader-secret".into(),
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
            secret_key: String::new(),
            ..live_cfg()
        };
        let msg = AuditTail::new(cfg)
            .err()
            .expect("an enabled tail with no secret must not build")
            .to_string();
        assert!(msg.contains("secret_key"), "{msg}");
        assert!(!msg.contains("access_key"), "{msg}");
        assert!(!msg.contains("endpoint"), "{msg}");
    }

    #[test]
    fn enabled_with_nothing_set_names_every_gap() {
        let cfg = AuditTailConfig {
            enabled: true,
            ..AuditTailConfig::default()
        };
        let msg = AuditTail::new(cfg).err().unwrap().to_string();
        assert!(msg.contains("endpoint, access_key, secret_key"), "{msg}");
    }

    /// A minimal path-style S3 stand-in for the walker: one bucket, `objects`
    /// keys `obj-000.ndjson`, `obj-001.ndjson`, ... listed `page_size` per
    /// ListObjectsV2 page with a numeric continuation token. Object `i` holds
    /// one event whose `key_expr` is `v/obj/<i>`. `object_gets` counts object
    /// GETs the server has received, so a test can see how far ahead of the
    /// parser the fetcher has run.
    struct S3Stub {
        endpoint: String,
        object_gets: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    fn stub_event(i: usize) -> String {
        format!(
            concat!(
                r#"{{"vendor":"v","key_expr":"v/obj/{}","ts":"2026-09-21T12:00:00.000Z","#,
                r#""payload_hash":"sha256:00","payload_len":0}}"#,
                "\n"
            ),
            i
        )
    }

    #[allow(clippy::disallowed_methods)] // test-only server; dies with the test runtime
    async fn s3_stub(objects: usize, page_size: usize) -> S3Stub {
        use std::sync::atomic::Ordering;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let object_gets = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let gets = object_gets.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let gets = gets.clone();
                tokio::spawn(async move {
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
                    let target = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let url = Url::parse(&format!("http://stub{target}")).unwrap();
                    let (status, body) = if url.path() == "/audit-integrity"
                        || url.path() == "/audit-integrity/"
                    {
                        let start: usize = url
                            .query_pairs()
                            .find(|(k, _)| k == "continuation-token")
                            .and_then(|(_, v)| v.parse().ok())
                            .unwrap_or(0);
                        let end = (start + page_size).min(objects);
                        let mut xml = String::from("<ListBucketResult>");
                        for i in start..end {
                            xml.push_str(&format!(
                                "<Contents><Key>obj-{i:03}.ndjson</Key></Contents>"
                            ));
                        }
                        if end < objects {
                            xml.push_str(&format!(
                                "<IsTruncated>true</IsTruncated>\
                                 <NextContinuationToken>{end}</NextContinuationToken>"
                            ));
                        }
                        xml.push_str("</ListBucketResult>");
                        ("200 OK", xml)
                    } else if let Some(i) = url
                        .path()
                        .strip_prefix("/audit-integrity/obj-")
                        .and_then(|r| r.strip_suffix(".ndjson"))
                        .and_then(|n| n.parse::<usize>().ok())
                    {
                        gets.fetch_add(1, Ordering::SeqCst);
                        ("200 OK", stub_event(i))
                    } else {
                        ("404 Not Found", String::new())
                    };
                    let resp = format!(
                        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        S3Stub {
            endpoint: format!("http://{addr}"),
            object_gets,
        }
    }

    fn stub_cfg(stub: &S3Stub) -> AuditTailConfig {
        AuditTailConfig {
            endpoint: stub.endpoint.clone(),
            ..live_cfg()
        }
    }

    fn object_index(ev: &AuditEvent) -> usize {
        ev.key_expr.rsplit('/').next().unwrap().parse().unwrap()
    }

    #[test]
    fn default_fetch_concurrency_is_small() {
        assert_eq!(
            AuditTailConfig::default().fetch_concurrency,
            DEFAULT_AUDIT_FETCH_CONCURRENCY
        );
        assert_eq!(DEFAULT_AUDIT_FETCH_CONCURRENCY, 4);
    }

    /// The walker must parse each object as it arrives, not fetch the whole
    /// page first: when the event from object `i` is handed to `on_event`,
    /// the server has seen at most `i + 1 + fetch_concurrency` object GETs.
    /// Collecting the page before parsing has fetched all 40 by the time the
    /// first event is seen.
    #[tokio::test]
    async fn stream_parses_each_object_before_fetching_past_the_concurrency_window() {
        use std::sync::atomic::Ordering;
        let stub = s3_stub(40, 1000).await;
        let concurrency = 3;
        let tail = AuditTail::new(AuditTailConfig {
            fetch_concurrency: concurrency,
            ..stub_cfg(&stub)
        })
        .unwrap();
        let mut seen: Vec<(usize, usize)> = Vec::new();
        let gets = stub.object_gets.clone();
        let stats = tail
            .stream_recent_events(1, None, |ev| {
                seen.push((object_index(&ev), gets.load(Ordering::SeqCst)));
            })
            .await
            .unwrap();
        assert_eq!(stats.objects_fetched, 40);
        assert_eq!(seen.len(), 40);
        assert!(
            seen[0].1 <= concurrency,
            "first event parsed only after {} of 40 objects were requested",
            seen[0].1
        );
        for (i, gets_then) in &seen {
            assert!(
                *gets_then <= i + 1 + concurrency,
                "event from object {i} parsed after {gets_then} GETs (concurrency {concurrency})"
            );
        }
    }

    /// A `fetch_concurrency` of 0 must not stall the walk.
    #[tokio::test]
    async fn zero_fetch_concurrency_still_reads_every_object() {
        let stub = s3_stub(5, 1000).await;
        let tail = AuditTail::new(AuditTailConfig {
            fetch_concurrency: 0,
            ..stub_cfg(&stub)
        })
        .unwrap();
        let (events, stats) = tail.recent_events(1, None).await.unwrap();
        assert_eq!(events.len(), 5);
        assert_eq!(stats.objects_fetched, 5);
    }

    /// Listing order is kept across pages, whatever the concurrency.
    #[tokio::test]
    async fn stream_keeps_listing_order_across_pages() {
        let stub = s3_stub(23, 7).await;
        let tail = AuditTail::new(AuditTailConfig {
            fetch_concurrency: 5,
            ..stub_cfg(&stub)
        })
        .unwrap();
        let mut order = Vec::new();
        let stats = tail
            .stream_recent_events(1, None, |ev| order.push(object_index(&ev)))
            .await
            .unwrap();
        assert_eq!(order, (0..23).collect::<Vec<_>>());
        assert_eq!(stats.objects_listed, 23);
        assert_eq!(stats.objects_fetched, 23);
        assert_eq!(stats.events_parsed, 23);
    }

    /// The max_objects ceiling stops mid-page and the stats say so: page one
    /// (4 objects) is read whole, page two is listed (4 more) but only one
    /// object of it is fetched.
    #[tokio::test]
    async fn stream_honours_the_max_objects_ceiling() {
        let stub = s3_stub(12, 4).await;
        let tail = AuditTail::new(AuditTailConfig {
            max_objects: 5,
            fetch_concurrency: 2,
            ..stub_cfg(&stub)
        })
        .unwrap();
        let mut order = Vec::new();
        let stats = tail
            .stream_recent_events(1, None, |ev| order.push(object_index(&ev)))
            .await
            .unwrap();
        assert_eq!(order, vec![0, 1, 2, 3, 4]);
        assert_eq!(stats.objects_listed, 8);
        assert_eq!(stats.objects_fetched, 5);
        assert_eq!(stats.events_parsed, 5);
    }

    /// `recent_events` and `stream_recent_events` return the same events in
    /// the same order with the same stats, filtered or not.
    #[tokio::test]
    async fn recent_and_stream_agree_on_events_and_stats() {
        let stub = s3_stub(17, 6).await;
        let tail = AuditTail::new(AuditTailConfig {
            max_objects: 15,
            ..stub_cfg(&stub)
        })
        .unwrap();
        for filter in [None, Some("v"), Some("other")] {
            let (events, a) = tail.recent_events(1, filter).await.unwrap();
            let mut streamed = Vec::new();
            let b = tail
                .stream_recent_events(1, filter, |ev| streamed.push(ev.key_expr))
                .await
                .unwrap();
            let listed: Vec<String> = events.into_iter().map(|e| e.key_expr).collect();
            assert_eq!(listed, streamed, "filter {filter:?}");
            assert_eq!(
                (a.objects_listed, a.objects_fetched, a.events_parsed),
                (b.objects_listed, b.objects_fetched, b.events_parsed),
                "filter {filter:?}"
            );
            assert_eq!(a.objects_fetched, 15);
            assert_eq!(listed.len(), if filter == Some("other") { 0 } else { 15 });
        }
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
    fn hour_prefixes_are_newest_first() {
        let prefixes = build_hour_prefixes("nova-a/", 3);
        assert_eq!(prefixes.len(), 3);
        // Each prefix should start with the configured root.
        for p in &prefixes {
            assert!(p.starts_with("nova-a/"));
            // and end with `/` after the hour segment.
            assert!(p.ends_with('/'));
        }
    }
}
