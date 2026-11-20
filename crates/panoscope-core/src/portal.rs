// SPDX-License-Identifier: Apache-2.0
//
// Portal HTTP client for goat-zenoh-portal's vendor + topic + schema
// surface. Used by:
//   - panoscope's descriptor cache (fetch_fds)
//   - stethoscope's registration / schema-body checks (list_topics,
//     get_topic, list_schemas, fetch_fds)
//
// Auth: every endpoint is Bearer-token gated. The client is constructed
// with an operator API key (env-driven on each consumer's side); empty
// key returns a Disabled client whose every call yields
// `FetchOutcome::Disabled` / `Err(PortalError::Disabled)`. Graceful for
// partial deploys where the key isn't wired yet.
//
// Endpoints (per ops/goat-zenoh-portal/internal/api/server.go):
//   GET /api/orgs/{anchor}/topics            (legacy alias: /api/vendors/...)
//   GET /api/orgs/{anchor}/topics/{id}
//   GET /api/orgs/{anchor}/topics/{id}/schemas
//   GET /api/schemas/{key...}     (catch-all by key_pattern)
//
// ADR 1052 Phase 2: this client prefers the `/api/orgs*` surface and falls
// back to the deprecated `/api/vendors*` on a 404 from an un-migrated portal
// (the segment is probed once per client by `seg()`). The listing response is
// dual-read (`orgs` preferred, `vendors` fallback). See
// docs/design/vendor-to-org-machine-rename.md.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

#[derive(Debug, Error)]
pub enum PortalError {
    #[error("portal client disabled (empty base url or api key)")]
    Disabled,
    /// Non-2xx with no usable body. `path` is the request's URL path (never
    /// the query, which can carry a token) and `retry_after` the raw
    /// `Retry-After` header when the portal sent one (a 429 or a 503 names
    /// when to come back). Both are empty for an error built without a
    /// response; construct through [`PortalError::http`].
    #[error("portal returned {status} for {path}")]
    Http {
        status: StatusCode,
        path: String,
        retry_after: Option<String>,
    },
    /// HTTP 404 on a lookup whose 404 means "the vendor or topic is not
    /// registered". `path` as for `Http`.
    #[error("portal vendor or topic not found ({path})")]
    NotFound { path: String },
    #[error("network or parse error: {0}")]
    Transient(String),
    /// The portal answered (2xx) but the body did not deserialize into the
    /// shape this build expects — a response-contract mismatch, not an
    /// unreachable portal. Kept distinct from `Transient` so the CLI can
    /// surface `E_PORTAL_BAD_RESPONSE` ("upgrade goat") instead of the
    /// misleading `E_PORTAL_UNREACHABLE`. The `goat schema show` decode
    /// break (proto_source map vs String) was exactly this class.
    #[error("portal returned an undecodable response body: {0}")]
    Decode(String),

    /// Non-2xx where the portal's error body was captured. The server's
    /// `error.message` names the actual objection (e.g. which workload has
    /// no mesh-group derivation), where a bare `Http` forces the CLI into a
    /// status-code guess-list that ends "the server's message names which
    /// one" — without showing it. `Http` remains for call sites that have
    /// not migrated to body capture.
    ///
    /// `code` is the onboard-API envelope's `error.code`
    /// (`ops/goat-id-console` `onboardError`) when the body carried one. It
    /// tells apart refusals that share a status — `operator_only`,
    /// `self_only`, `not_sponsor` are all 403 — where the status alone
    /// cannot.
    #[error("portal returned {status} for {path}: {message}")]
    HttpWithBody {
        status: StatusCode,
        code: Option<String>,
        message: String,
        path: String,
        retry_after: Option<String>,
    },
}

/// The longest `Retry-After` value carried. The header is a number of seconds
/// or an HTTP date; anything longer is not one of those and is dropped.
const RETRY_AFTER_CAP: usize = 64;

fn retry_after_of(resp: &reqwest::Response) -> Option<String> {
    let v = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim();
    (!v.is_empty() && v.len() <= RETRY_AFTER_CAP).then(|| v.to_string())
}

impl PortalError {
    /// A non-2xx answer with no body worth salvaging, carrying the request
    /// path and any `Retry-After` so the CLI can say which call was refused
    /// and when to retry.
    pub fn http(status: StatusCode, resp: &reqwest::Response) -> Self {
        PortalError::Http {
            status,
            path: resp.url().path().to_string(),
            retry_after: retry_after_of(resp),
        }
    }

    /// A 404 whose meaning is "not registered", carrying the request path.
    pub fn not_found(resp: &reqwest::Response) -> Self {
        PortalError::NotFound {
            path: resp.url().path().to_string(),
        }
    }

    /// A bare status with no request behind it, for callers and tests that
    /// synthesize a refusal.
    pub fn status(status: StatusCode) -> Self {
        PortalError::Http {
            status,
            path: String::new(),
            retry_after: None,
        }
    }

    /// The request path behind an HTTP-shaped error; empty when unknown.
    pub fn path(&self) -> &str {
        match self {
            PortalError::Http { path, .. }
            | PortalError::HttpWithBody { path, .. }
            | PortalError::NotFound { path } => path,
            _ => "",
        }
    }

    /// The portal's `Retry-After`, when it sent one.
    pub fn retry_after(&self) -> Option<&str> {
        match self {
            PortalError::Http { retry_after, .. } | PortalError::HttpWithBody { retry_after, .. } => {
                retry_after.as_deref()
            }
            _ => None,
        }
    }

    /// The HTTP status behind either refusal shape. Mappers key their
    /// status-specific guidance off this so the bare and body-carrying
    /// variants cannot drift apart — an arm written against `http_status`
    /// covers both by construction.
    pub fn http_status(&self) -> Option<StatusCode> {
        match self {
            PortalError::Http { status, .. } | PortalError::HttpWithBody { status, .. } => {
                Some(*status)
            }
            _ => None,
        }
    }

    /// The server's own message, when a non-2xx body carried a usable one.
    /// Mappers append this UNDER their curated guidance rather than
    /// replacing it: the guidance explains the refusal class, the server
    /// names the instance.
    pub fn server_message(&self) -> Option<&str> {
        match self {
            PortalError::HttpWithBody { message, .. } => Some(message.as_str()),
            _ => None,
        }
    }

    /// The onboard-API refusal code (`error.code`), when the body carried one.
    pub fn error_code(&self) -> Option<&str> {
        match self {
            PortalError::HttpWithBody { code, .. } => code.as_deref(),
            _ => None,
        }
    }
}

/// Consume a non-2xx response, salvaging the portal's error message when the
/// body carries a usable one. Falls back to the bare `Http` variant on an
/// empty, opaque, or unreadable body, so status-keyed handling still applies.
async fn non_2xx(resp: reqwest::Response) -> PortalError {
    let status = resp.status();
    let path = resp.url().path().to_string();
    let retry_after = retry_after_of(&resp);
    let body = read_capped(resp, ERROR_BODY_CAP).await;
    let text = body.as_deref().map(String::from_utf8_lossy);
    match text.as_deref().and_then(salvage_message) {
        Some(message) => PortalError::HttpWithBody {
            status,
            code: text.as_deref().and_then(salvage_code),
            message,
            path,
            retry_after,
        },
        None => PortalError::Http {
            status,
            path,
            retry_after,
        },
    }
}

/// The onboard-API envelope's `error.code`, when it is a plain identifier.
/// A mapper matches on it, so anything else (empty, long, or carrying
/// characters outside `[a-z0-9_]`) is dropped rather than shown or matched.
fn salvage_code(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    // The legacy envelope's `error.code`, else the RFC 9457 body's top-level
    // `code` (the portal sends both today).
    let code = v
        .pointer("/error/code")
        .or_else(|| v.get("code"))?
        .as_str()?;
    let plain = !code.is_empty()
        && code.len() <= 64
        && code
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    plain.then(|| code.to_string())
}

/// How much of a non-2xx body is pulled off the wire. Salvage keeps at most
/// 400 characters, so nothing past this can reach the operator; a refusal
/// answered with a huge body is not buffered whole for it.
const ERROR_BODY_CAP: usize = 64 * 1024;

/// The first `cap` bytes of the body, or `None` when it could not be read.
/// The read stops at the cap rather than draining the rest; the cut may land
/// inside a multi-byte character, which the lossy decode above renders as
/// U+FFFD at the very end of a text the 400-character cap discards anyway.
async fn read_capped(mut resp: reqwest::Response, cap: usize) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        let room = cap - buf.len();
        buf.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if buf.len() >= cap {
            break;
        }
    }
    Some(buf)
}

/// Extract the portal's own error message from a non-2xx body, if usable.
/// Pure — decoupled from the HTTP client so every shape is unit-testable.
/// Recognized: `{"error":{"message":…}}`, `{"error":"…"}`, `{"message":"…"}`,
/// a bare JSON string (`"quota exceeded"`), or plain text (HTML is refused;
/// JSON without a message field salvages nothing rather than dumping raw
/// JSON). The result is sanitized for terminal display: control and format
/// characters collapse to spaces (a body cannot smuggle ANSI, reorder the
/// text with a bidi override, or hide text behind zero-width and tag
/// characters), anything shaped like an echoed bearer credential or an
/// `op_`/`vp_` API key is redacted, and the text is capped at 400
/// characters plus an ellipsis marking the cut.
fn salvage_message(body: &str) -> Option<String> {
    let raw = match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v) => v
            .pointer("/error/message")
            .and_then(serde_json::Value::as_str)
            .or_else(|| v.get("error").and_then(serde_json::Value::as_str))
            .or_else(|| v.get("message").and_then(serde_json::Value::as_str))
            // The portal answers with RFC 7807 problem JSON; its human line is `detail`.
            .or_else(|| v.get("detail").and_then(serde_json::Value::as_str))
            .or_else(|| v.as_str())
            .map(str::to_string)?,
        Err(_) => {
            let t = body.trim();
            if t.is_empty() || t.starts_with('<') {
                return None;
            }
            t.to_string()
        }
    };
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || is_format(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let redacted = redact_api_keys(&redact_bearer(&cleaned));
    let trimmed = redacted.trim();
    let capped: String = if trimmed.chars().count() > 400 {
        trimmed.chars().take(400).chain("…".chars()).collect()
    } else {
        trimmed.to_string()
    };
    (!capped.is_empty()).then_some(capped)
}

/// Unicode general category Cf (format): the 170 code points of Unicode
/// 15.0 in the ranges below. `char::is_control` covers Cc only, and the
/// standard library exposes no category predicate; the set is small and
/// changes rarely enough to carry here rather than pull a crate in for one
/// call. Check it against `unicodedata` when the table's Unicode version
/// moves.
fn is_format(c: char) -> bool {
    const RANGES: &[(u32, u32)] = &[
        (0x00AD, 0x00AD),
        (0x0600, 0x0605),
        (0x061C, 0x061C),
        (0x06DD, 0x06DD),
        (0x070F, 0x070F),
        (0x0890, 0x0891),
        (0x08E2, 0x08E2),
        (0x180E, 0x180E),
        (0x200B, 0x200F),
        (0x202A, 0x202E),
        (0x2060, 0x2064),
        (0x2066, 0x206F),
        (0xFEFF, 0xFEFF),
        (0xFFF9, 0xFFFB),
        (0x110BD, 0x110BD),
        (0x110CD, 0x110CD),
        (0x13430, 0x1343F),
        (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A),
        (0xE0001, 0xE0001),
        (0xE0020, 0xE007F),
    ];
    let cp = c as u32;
    RANGES.iter().any(|&(lo, hi)| (lo..=hi).contains(&cp))
}

/// Replace the token after any case-insensitive `Bearer` with `[redacted]`.
/// A well-behaved portal never echoes credentials, but a misconfigured
/// intercepting proxy or debug endpoint can reflect the request headers —
/// and this text goes straight to the operator's terminal (and scrollback).
///
/// A separator is any Unicode whitespace plus `=` and `:`, which covers the
/// shapes an echoed `Authorization` header actually arrives in: `Bearer tok`,
/// `Bearer=tok`, `Bearer:tok`, and the non-breaking space a proxy's
/// pretty-printer can substitute. Splitting on the ASCII space alone let
/// those last three through unredacted, which defeats the point of a function
/// that exists to keep credentials out of terminal scrollback. `Bearertoken`
/// is not a header and is left alone. Everything outside a replaced token is
/// preserved verbatim.
fn redact_bearer(s: &str) -> String {
    fn is_sep(c: char) -> bool {
        c.is_whitespace() || c == '=' || c == ':'
    }
    let width = |t: &str, keep: bool| -> usize {
        t.chars()
            .take_while(|c| is_sep(*c) == keep)
            .map(char::len_utf8)
            .sum()
    };

    // Byte ranges of the credentials to replace, found left to right.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut at = 0usize;
    while at < s.len() {
        let Some(kw) = s[at..].char_indices().map(|(i, _)| at + i).find(|&i| {
            s.get(i..i + 6)
                .is_some_and(|w| w.eq_ignore_ascii_case("bearer"))
                // Token-initial only: `nonbearer` is not the keyword.
                && s[..i].chars().next_back().map(is_sep).unwrap_or(true)
        }) else {
            break;
        };
        let after = kw + 6;
        let seps = width(&s[after..], true);
        if seps == 0 {
            // `Bearer` ends the text or runs into the next word — no
            // credential follows it.
            at = after;
            continue;
        }
        let start = after + seps;
        let token = width(&s[start..], false);
        if token > 0 {
            spans.push((start, start + token));
        }
        at = start + token;
    }

    let mut out = String::with_capacity(s.len());
    let mut prev = 0;
    for (start, end) in spans {
        out.push_str(&s[prev..start]);
        out.push_str("[redacted]");
        prev = end;
    }
    out.push_str(&s[prev..]);
    out
}

/// Replace any token shaped like a portal API key — `op_…` (operator) or
/// `vp_…` (vendor) — with `[redacted]`, whether or not a `Bearer` keyword
/// precedes it. A validation error that quotes the offending value, or a
/// reflected query parameter, would otherwise print the key itself. The
/// match is `\b(op|vp)_[A-Za-z0-9_-]+`: the prefix must start a word
/// (`stop_x` is not a key) and the key runs to the first character outside
/// the key alphabet, so surrounding punctuation is preserved.
fn redact_api_keys(s: &str) -> String {
    fn is_word(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_'
    }
    fn is_key_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_' || c == '-'
    }
    let mut out = String::with_capacity(s.len());
    let mut prev: Option<char> = None;
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        let at_word_start = match prev {
            None => true,
            Some(p) => !is_word(p),
        };
        if at_word_start && (rest.starts_with("op_") || rest.starts_with("vp_")) {
            let key_len: usize = rest
                .chars()
                .take_while(|c| is_key_char(*c))
                .map(char::len_utf8)
                .sum();
            // `op_` alone, with nothing after the underscore, is not a key.
            if key_len > 3 {
                out.push_str("[redacted]");
                prev = rest[..key_len].chars().next_back();
                rest = &rest[key_len..];
                continue;
            }
        }
        out.push(c);
        prev = Some(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Per-fetch outcome for the legacy fetch_fds path. Distinguishes the
/// three operator-visible cases panoscope already shapes its UX around.
#[derive(Debug)]
pub enum FetchOutcome {
    Ready(Vec<u8>),
    NoSchema,
    Disabled,
    Transient(String),
}

/// Vendor row as returned by `GET /api/vendors`. The portal carries
/// more fields than this; we deserialize only the prefix + display
/// name (everything else is ignored via #[serde(default)]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vendor {
    pub prefix: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    /// Notional/synthetic org flag (the shipped precursor of the ADR 1075
    /// `fidelity: notional` label). Default false for portals that predate
    /// the field.
    #[serde(default)]
    pub synthetic: bool,
    #[serde(default)]
    pub sponsor_authorized: bool,
    #[serde(default)]
    pub sponsor_max_tier: String,
    /// Live (non-retired) declared topic rows for this org. `None` when the
    /// portal predates the field — unknown, which a catalog must render as
    /// "shown", never as "nothing declared".
    #[serde(default)]
    pub declared_topics: Option<i64>,
}

/// `GET /api/catalog` — every org with its topics inline, one request.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub orgs: Vec<CatalogOrg>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CatalogOrg {
    pub prefix: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub synthetic: bool,
    #[serde(default)]
    pub topics: Vec<CatalogTopic>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CatalogTopic {
    /// The topic row's id; 0 from a portal that predates the field.
    #[serde(default)]
    pub id: i64,
    pub key_pattern: String,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub schema_ref: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub publish_count: i64,
    #[serde(default)]
    pub last_seen_at: Option<i64>,
    #[serde(default)]
    pub created_at: i64,
}

/// Listing envelope from `GET /api/orgs` (or legacy `/api/vendors`). The
/// migrated portal dual-emits BOTH `orgs` and `vendors` (identical rows), so
/// these are distinct optional fields — a `#[serde(alias)]` would trip a
/// duplicate-field error when both are present (ADR 1052 Phase 2). Prefer
/// `orgs`; fall back to `vendors` for an un-migrated portal.
#[derive(Deserialize)]
struct VendorsEnvelope {
    #[serde(default)]
    orgs: Option<Vec<Vendor>>,
    #[serde(default)]
    vendors: Option<Vec<Vendor>>,
}

impl VendorsEnvelope {
    fn into_rows(self) -> Vec<Vendor> {
        self.orgs.or(self.vendors).unwrap_or_default()
    }
}

/// One slot's UUID↔alias mapping from `GET /api/aliases` (ADR 1039).
/// `slot_id` is the canonical machine key; `alias` is the human name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasRow {
    pub slot_id: String,
    pub alias: String,
}

#[derive(Deserialize)]
struct AliasesEnvelope {
    aliases: Vec<AliasRow>,
}

/// The `GET /api/access/data-roles` resolver response (ADR 1074 B1). Only
/// the `roles` array is consumed here (the derived `data.view.*` grammar);
/// `principal`/`producers`/`derived_at`/`freshness`/`notes` are ignored —
/// panoscope folds the roles into its `ViewerScope`, nothing else.
#[derive(Deserialize)]
struct DataRolesEnvelope {
    #[serde(default)]
    roles: Vec<String>,
}

/// One actor's declared throughput envelope from `GET /api/envelopes`
/// (ADR 1045). The eight `{min,mean,high,max}` × `{msgs/s,bytes/s}` numbers a
/// live footprint is judged against, keyed by the write `anchor`. `source` is
/// "default" (rough class-baseline placeholder) or "edited" (party-set). Extra
/// portal fields (created_at/updated_at/class_key) are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvelopeRow {
    pub anchor: String,
    pub min_msgs_per_s: f64,
    pub mean_msgs_per_s: f64,
    pub high_msgs_per_s: f64,
    pub max_msgs_per_s: f64,
    pub min_bytes_per_s: f64,
    pub mean_bytes_per_s: f64,
    pub high_bytes_per_s: f64,
    pub max_bytes_per_s: f64,
    #[serde(default)]
    pub source: String,
}

#[derive(Deserialize)]
struct EnvelopesEnvelope {
    envelopes: Vec<EnvelopeRow>,
}

/// Topic row as returned by `GET /api/vendors/{vendor}/topics`. The
/// portal serializes more fields than this; we deserialize only what
/// stethoscope's checks consume (serde ignores the rest).
#[derive(Debug, Clone, Serialize)]
pub struct Topic {
    pub id: i64,
    /// The owning anchor prefix. Deserialized from `org_prefix` (primary)
    /// OR the deprecated `vendor_prefix` — see the manual `Deserialize` impl
    /// below. The Rust field name + serialized output stay `vendor_prefix`
    /// so the 20 consumers + `--json` shape are untouched (that rename is the
    /// deferred internal-identifier cosmetic).
    pub vendor_prefix: String,
    pub key_pattern: String,
    /// e.g. `protobuf:DenObservation`, `json`, `cbor`. Empty for
    /// rows that don't declare one (portal allows missing format).
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub schema_ref: String,
    #[serde(default)]
    pub description: String,
    /// The portal's `topics.publish_count` column. NOTE: no `UPDATE`
    /// anywhere in the portal writes it — it is created `DEFAULT 0` and
    /// stays there ("audit-logger integration is a follow-on", schema.sql).
    /// Kept on the wire shape for contract stability; `goat topic show`
    /// stopped rendering it because a hardcoded 0 reads as "nothing
    /// published" rather than "nobody counts". Use `goat flow` instead.
    #[serde(default)]
    pub publish_count: i64,
    /// The registry's own monotonic last-publish stamp. The portal already
    /// emits it (`omitempty`) and both web surfaces render it; this client
    /// was the only consumer dropping it. `None` until the audit tailer
    /// stamps the column, which is why `skip_serializing_if` matters —
    /// today the field is always absent, so `topic ls --json` output stays
    /// byte-identical and no golden moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<i64>,
    #[serde(default)]
    pub created_at: i64,
}

// ADR 1052 Phase 3 client-read flip. The portal dual-emits BOTH `org_prefix`
// and `vendor_prefix` (identical value) on each topic row, so a plain
// `#[serde(alias)]` would dup-error. Resolve via a shadow struct that captures
// both as optionals and prefers `org_prefix` — so when a Phase-4 portal drops
// `vendor_prefix`, the anchor still populates. Serialize stays derived (emits
// `vendor_prefix`), so there is no `--json`/consumer churn.
impl<'de> Deserialize<'de> for Topic {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            id: i64,
            #[serde(default)]
            vendor_prefix: Option<String>,
            #[serde(default)]
            org_prefix: Option<String>,
            key_pattern: String,
            #[serde(default)]
            format: String,
            #[serde(default)]
            schema_ref: String,
            #[serde(default)]
            description: String,
            #[serde(default)]
            publish_count: i64,
            #[serde(default)]
            last_seen_at: Option<i64>,
            #[serde(default)]
            created_at: i64,
        }
        let r = Raw::deserialize(deserializer)?;
        Ok(Topic {
            id: r.id,
            vendor_prefix: r.org_prefix.or(r.vendor_prefix).unwrap_or_default(),
            key_pattern: r.key_pattern,
            format: r.format,
            schema_ref: r.schema_ref,
            description: r.description,
            publish_count: r.publish_count,
            last_seen_at: r.last_seen_at,
            created_at: r.created_at,
        })
    }
}

/// Schema metadata row from `GET /api/vendors/{v}/topics/{id}/schemas`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaMeta {
    pub id: i64,
    pub topic_id: i64,
    pub version: String,
    #[serde(default)]
    pub compatibility: String,
    /// e.g. `protobuf-fds`, `protobuf-source`, `json-schema`.
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub size_bytes: i64,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub uploaded_at: i64,
    #[serde(default)]
    pub uploaded_by: String,
}

#[derive(Deserialize)]
struct TopicsEnvelope {
    topics: Vec<Topic>,
}

/// One row of the import diff returned by
/// `POST /api/vendors/{v}/topics/import`. `action` is one of
/// `create` / `update` / `unchanged` / `retire` (see the portal's
/// `importDiffEntry` in internal/api/bulk.go).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportDiffEntry {
    pub key_pattern: String,
    pub action: String,
}

/// Response body of the bulk topic import. Returned for both the
/// `dry_run=true` preview and the applied run (the portal sends the
/// same diff either way; `dry_run` distinguishes them). Mirrors the
/// portal's `importDiff` struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportDiff {
    pub vendor: String,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub entries: Vec<ImportDiffEntry>,
    #[serde(default)]
    pub summary: std::collections::HashMap<String, i64>,
}

/// Deserialize a value that may legitimately arrive as JSON `null` into
/// its `Default`. The portal serializes Go nil slices/maps as `null`
/// (e.g. `"enums": null` when a schema has no enums), and plain
/// `#[serde(default)]` only fills *absent* keys — a present `null` still
/// hits the field's own deserializer, which rejects it for `Vec`/`Map`/
/// `String`. Using this on every portal-populated collection makes the
/// decode tolerant of null, absent, and present-with-value alike.
fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// Response envelope from `GET /api/schemas/render/{key}`. Mirrors the
/// portal's `renderSchemaByKey` writeJSON map (schema_render.go). We
/// deserialize only the fields `goat schema show`/`diff` consume; serde
/// ignores `examples` + the schema-meta fields we don't render.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RenderResponse {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub format: String,
    /// The protobuf message the topic's `format` points the viewer at
    /// (`protobuf:<Name>` → `<Name>`); empty for non-protobuf topics.
    #[serde(default)]
    pub primary_message: String,
    #[serde(default)]
    pub schema: RenderSchemaMeta,
    #[serde(default, deserialize_with = "null_default")]
    pub rendered: RenderedSchema,
    /// Reconstructed `.proto` source, keyed by file name. The portal
    /// emits `decoder::WriteProtoText`'s per-file `map[string]string`
    /// (a nil map serializes as `null`); `show --proto` concatenates the
    /// files in name order.
    #[serde(default, deserialize_with = "null_default")]
    pub proto_source: std::collections::BTreeMap<String, String>,
}

/// The `schema` sub-object of a render response — version-scoped
/// metadata plus the sibling-version list the picker offers.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RenderSchemaMeta {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub compatibility: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: i64,
    #[serde(default, deserialize_with = "null_default")]
    pub all_versions: Vec<String>,
    #[serde(default)]
    pub format_storage: String,
}

/// The flattened message + enum tree from `decoder.RenderDescriptorSet`.
/// Nested types are emitted at the top level and referenced by full
/// name, matching the portal's render walker.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RenderedSchema {
    #[serde(default, deserialize_with = "null_default")]
    pub files: Vec<RenderedFile>,
    #[serde(default, deserialize_with = "null_default")]
    pub messages: Vec<RenderedMessage>,
    #[serde(default, deserialize_with = "null_default")]
    pub enums: Vec<RenderedEnum>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RenderedFile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub package: String,
    #[serde(default)]
    pub syntax: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RenderedMessage {
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "null_default")]
    pub fields: Vec<RenderedField>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RenderedField {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub number: i32,
    /// `singular` | `optional` | `repeated` | `map`.
    #[serde(default)]
    pub cardinality: String,
    /// Scalar kind (`string`, `uint64`, …) or `message` / `enum` / `map`.
    #[serde(default, rename = "type")]
    pub ty: String,
    /// Full name of the referenced message/enum when `ty` is composite.
    #[serde(default)]
    pub type_ref: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RenderedEnum {
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "null_default")]
    pub values: Vec<RenderedEnumValue>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RenderedEnumValue {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub number: i32,
}

/// Request body for `POST /api/vendors`. Mirrors the Go reference's
/// `createVendorReq`. The portal accepts `auto_approve: true` only
/// when the operator token has admin scope.
#[derive(Debug, Clone, Default, Serialize)]
pub struct NewVendor {
    pub prefix: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    #[serde(default)]
    pub synthetic: bool,
    #[serde(default)]
    pub auto_approve: bool,
    /// Sponsorship annotation (ADR 1071), copied from the manifest's
    /// `sponsorship:` block at onboard so the identity can mint
    /// candidate-onboarding invites / vouch at stage 2 from day one.
    #[serde(default)]
    pub sponsor_authorized: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sponsor_max_tier: String,
    /// Optional Ory-login console binding for the sponsorship (ADR 1072).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sponsor_email: String,
    /// Machine-enrollment grant seeded at onboard (ADR 1096): how many machines
    /// this custodian may enroll without a separate operator grant. Paired with
    /// `affiliation` (the unit a machine inherits) — a quota with no affiliation
    /// is skipped server-side. Omitted (0) leaves the grant unset.
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub machine_quota: i64,
    /// The custodian's unit, stamped on every machine they enroll (from the
    /// manifest `affiliation.id`). Only meaningful alongside `machine_quota`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub affiliation: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub site: String,
    /// Onboard the slot WITHOUT minting its fabric credential (mint-on-collect,
    /// docs/design/fabric-credential-collection.md). The slot, the person
    /// linkage and the org row are created; the api_key and cert are not. Their
    /// owner mints them by collecting from the console, which is the only way
    /// the plaintext reaches the person rather than passing through an operator.
    ///
    /// PEOPLE ONLY: a machine cannot complete an Ory login and so can never
    /// collect, so deferring a machine's credential would strand it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub defer_credential: bool,
    /// Allocate this org's slot from the PRE-CUT PARTNER BLOCK bound to this
    /// lane-set (ADR 1138), e.g. `["REL-ALL","REL-PARTNER"]`, instead of the lowest
    /// free slot. Order-insensitive.
    ///
    /// This is what makes a partner mint land on a slot whose
    /// `partner-<anchor>` subject the router has ALREADY rendered — no ACL
    /// edit, no fabric restart. Omitted = an ordinary slot, which for a partner
    /// means they are NOT partitioned and read the whole fabric until someone
    /// hand-carves them.
    ///
    /// EXPLICIT, never inferred from affiliation: which lanes a partner may
    /// read is a release decision with an owner. The portal REFUSES the mint if
    /// no block declares this exact lane-set rather than falling back to an
    /// ordinary slot.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub partner_lanes: Vec<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Response from `POST /api/vendors/{vendor}/rotate` — the in-place
/// cert re-issue. Carries the fresh one-time bundle URL + portal api_key
/// for the SAME anchor; the operator hands the tarball to the peer and
/// restarts it. Only the leaf cert + api_key are new (the anchor / alias
/// / write namespace / ACL are unchanged). The server emits the full
/// createVendorResponse shape; we keep only the rotation-relevant fields.
#[derive(Debug, Clone, Deserialize)]
pub struct RotatedCert {
    pub prefix: String,
    pub api_key: String,
    pub bundle_url: String,
    pub bundle_token: String,
    pub uri_san: String,
    #[serde(default)]
    pub system_handle: String,
}

/// Response from `POST /api/orgs/{vendor}/rekey` — the key-only reissue
/// (goat#1193). Carries the fresh portal api_key and its anchor, nothing else:
/// rekey mints no cert, so there is no bundle URL or token to return.
#[derive(Debug, Clone, Deserialize)]
pub struct RekeyedKey {
    pub prefix: String,
    pub api_key: String,
}

/// Response from `POST /api/orgs` (slot-pool onboard, ADR 1025) — the full
/// `createVendorResponse` the server emits. The native onboard path (P2.1 of
/// docs/design/onboard-rotate-revoke-streamlining.md) keeps the WHOLE body
/// (api_key, bundle_url, uri_san, slot_id) — unlike [`PortalClient::create_vendor`],
/// which discards it. The shape is identical to [`RotatedCert`] plus the
/// slot/approval fields the onboard adds. `bundle_url` is a portal-relative PATH
/// (`/api/vendor-bundle/<anchor>/<token>.tar.gz`); prepend the portal base to fetch.
#[derive(Debug, Clone, Deserialize)]
pub struct OnboardedOrg {
    pub prefix: String,
    pub api_key: String,
    pub bundle_url: String,
    #[serde(default)]
    pub bundle_token: String,
    #[serde(default)]
    pub uri_san: String,
    #[serde(default)]
    pub approved: bool,
    /// Per-uuid slot-pool mode (ADR 1025): the assigned anchor (= cert CN =
    /// write namespace). Empty in per-vendor mode (prefix is then the anchor).
    #[serde(default)]
    pub slot_id: String,
    /// The human handle the operator onboarded (the slot alias, ADR 1039).
    #[serde(default)]
    pub system_handle: String,
    /// Set non-empty by the portal on a partial success (HTTP 200): the row was
    /// created but a follow-on step — ACL render / storage provision — failed, so
    /// the credential won't authorize writes yet. The caller fails loud on it.
    #[serde(default)]
    pub note_to_share: String,
    /// True when a deferred onboard found the handle already holding a live slot
    /// and returned it rather than allocating one (#1576). The slot pre-dates the
    /// call, so the caller must never roll it back.
    #[serde(default)]
    pub reused: bool,
    /// The person linkage recorded on the slot (Kratos identity UUID), echoed by
    /// the portal; empty when none is recorded.
    #[serde(default)]
    pub identity_id: String,
}

/// Request body for `POST /api/vendors/{vendor}/topics`. Mirrors the
/// Go reference's `createTopicReq`. `schema_ref` + `description` +
/// `publish_to_storage` are optional from the operator's view but the
/// JSON field is always serialized (matching the Go side).
#[derive(Debug, Clone, Serialize)]
pub struct NewTopic {
    pub key_pattern: String,
    pub format: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub schema_ref: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub publish_to_storage: bool,
}

/// Request body for `POST /api/onboard/invite` (ADR 1062, three-stage candidate
/// onboarding). Every field is optional (minimal-burden, Amendment 1): mint a
/// bare invite and let the enrollee fill the rest. Escalated admin tiers
/// (`iam-admin`/`platform-admin`) may be set here — sponsor-offered — but never
/// by the enrollee.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewInvite {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub env: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tier: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub affiliation: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub site: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub machine_quota: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub ttl_hours: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// When set, the console emails the one-time nomination link to this
    /// candidate right after minting (the machine-path equivalent of the web
    /// invite page's "email the link" action). Empty = mint only.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    /// Collaboration-environment decision, pre-bound onto the invite exactly as
    /// the web invite page's checkbox does (Outline / filedrop / Zulip, separate
    /// from the mesh).
    ///
    /// `None` = say nothing and inherit the server default (which matches the
    /// web form's checked-by-default). This field did not exist until F-363:
    /// the web page defaulted it ON and the machine path could not express it at
    /// all, so every CLI-minted invite silently landed on the opposite of what
    /// the same request through the page would have produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collab: Option<bool>,
}

fn is_zero_i64(n: &i64) -> bool {
    *n == 0
}

/// Response from `POST /api/onboard/invite`. The plaintext `token` is shown
/// once (never persisted server-side); `nomination_url` is the link a sponsor
/// hands the candidate.
#[derive(Debug, Clone, Deserialize)]
pub struct InviteMinted {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub sponsor: String,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub nomination_url: String,
    #[serde(default)]
    pub nomination_path: String,
    /// The PUBLIC front-desk link (external candidates), when the portal has a
    /// public intake base configured; empty otherwise.
    #[serde(default)]
    pub intake_url: String,
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default)]
    pub escalated: bool,
    /// Set to the candidate address when the mint was asked to email the link
    /// and the send succeeded; empty otherwise.
    #[serde(default)]
    pub emailed_to: String,
    /// Set when a requested email send failed (the link is still returned).
    #[serde(default)]
    pub email_error: String,
}

/// One row from `GET /api/onboard/ssh-keys` — a submitted host-shell key and
/// where it is in the gate. `status` is the whole story: `submitted` grants
/// nothing, `approved` means an admin decided but the key is NOT on any host
/// yet, `installed` means it actually is. Never collapse the last two when
/// reporting to a human (docs/design/ssh-key-self-service.md).
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct SshKey {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub key_type: String,
    /// The base64 key body. The console serves it to operator keys so an
    /// installer can write the line and a revoke can remove it by body.
    #[serde(default)]
    pub key_body: String,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub decided_by: String,
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default)]
    pub note: String,
}

/// One row from `GET /api/onboard/invites` (the operator invite queue). The
/// token hash is never serialized by the server. `prebound` carries the
/// pre-bound tier/affiliation/site as raw JSON (a nested object).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Invite {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub sponsor: String,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub escalated: bool,
    /// Server sends `prebound_json` as a JSON STRING; parse lazily via `tier()`.
    #[serde(default, rename = "prebound_json")]
    pub prebound_json: String,
}

impl Invite {
    /// The pre-bound tier, parsed from the prebound_json blob ("" if none).
    pub fn tier(&self) -> String {
        serde_json::from_str::<serde_json::Value>(&self.prebound_json)
            .ok()
            .and_then(|v| v.get("tier").and_then(|t| t.as_str()).map(String::from))
            .unwrap_or_default()
    }
}

/// One org in the first-class org registry (ADR 1112, console
/// `/api/org-registry/*`). Distinct from the portal's producer/vendor org —
/// this is the affiliation registry (nation → service → unit relations).
#[derive(Debug, Clone, Deserialize)]
pub struct OrgRegistryOrg {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub kind: String,
    /// ADR 1171. national | multinational; "" = not yet declared, which every
    /// org registered before that axis existed carries.
    #[serde(default)]
    pub composition: String,
    #[serde(default)]
    pub nation: String,
    #[serde(default)]
    pub provenance: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub parent: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// One ranked fuzzy-match candidate (ADR 1112 §7). `band` is
/// exact | high | low; `demoted` marks a pairing a human already judged
/// distinct — never auto-collapse, a human confirms every attach.
#[derive(Debug, Clone, Deserialize)]
pub struct OrgRegistryCandidate {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub nation: String,
    #[serde(default)]
    pub parent: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub band: String,
    #[serde(default)]
    pub via: String,
    #[serde(default)]
    pub demoted: bool,
}

/// An onboarding proposal (the admin/sponsor review queue). Mirrors the portal
/// `onboard_proposals` row surfaced by `GET /api/onboard/proposals`.
#[derive(Debug, Clone, Deserialize)]
pub struct Proposal {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub sponsor: String,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub escalated: bool,
    #[serde(default)]
    pub complete: bool,
    /// JSON array of `{field, sponsor_value, requested_value}` petitions; "[]" if none.
    #[serde(default)]
    pub petitions: String,
    #[serde(default)]
    pub note: String,
}

/// One repo grant/override target: `owner/name` split, plus a permission for an
/// add (blank for a remove). Mirrors the console `repoaccess.Grant` /
/// `repoaccess.RepoRef` wire shapes (docs/design/onboarding-github-repo-grants.md).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RepoGrantRef {
    #[serde(default)]
    pub repo: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub permission: String,
    /// Optional explicit owner for a remove (an add carries it in `repo` as
    /// `owner/name`). Omitted on the wire when blank.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
}

/// The stored per-onboardee override set: repos to add on top of the policy, and
/// repos to remove from it. Mirrors the console `repoaccess.Overrides`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RepoOverrides {
    #[serde(default)]
    pub add: Vec<RepoGrantRef>,
    #[serde(default)]
    pub remove: Vec<RepoGrantRef>,
}

/// One row of the effective (policy ⊕ overrides) repo-access set, tagged with the
/// source that put it there (`policy` | `audible`).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct EffectiveRepoGrant {
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub permission: String,
    #[serde(default)]
    pub source: String,
}

/// Response of `GET`/`POST /api/onboard/proposals/{id}/repo-access`: the computed
/// effective set, the stored overrides, the captured github username, and whether
/// the onboarding token is configured (grants are skipped without it).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RepoAccessView {
    #[serde(default)]
    pub effective: Vec<EffectiveRepoGrant>,
    #[serde(default)]
    pub overrides: RepoOverrides,
    #[serde(default)]
    pub github: String,
    #[serde(default)]
    pub has_token: bool,
}

/// Result of `POST /api/onboard/proposals/{id}/approve` — the manifest + the
/// operator command (the manual mint fallback when no runner is deployed).
#[derive(Debug, Clone, Deserialize)]
pub struct ApproveResult {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub manifest_yaml: String,
    #[serde(default)]
    pub manifest_path: String,
    #[serde(default)]
    pub command: String,
}

#[derive(Deserialize)]
struct SchemasEnvelope {
    schemas: Vec<SchemaMeta>,
}

/// Multipart form fields for `POST /api/vendors/{v}/topics/{id}/schemas`.
/// `bytes` are the raw file body (a `.proto` source when
/// `format = "protobuf-source"`, or a serialized FileDescriptorSet when
/// `format = "protobuf-fds"`); `file_name` is the multipart part's
/// filename (cosmetic — the portal reads the bytes, not the name).
#[derive(Debug, Clone)]
pub struct NewSchema {
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub version: String,
    pub compatibility: String,
    pub format: String,
}

/// One presigned audit object: its bucket key and a short-TTL GET URL the
/// client fetches NDJSON from directly (goat-cli G.6 / ADR 1038).
#[derive(Debug, Clone, Deserialize)]
pub struct PresignedObject {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub schema_version: u8,
    #[serde(default)]
    pub audit_url: String,
    pub key: String,
    pub url: String,
}

/// Response of `GET /api/audit/presign`: the presigned objects covering the
/// requested window, plus the TTL and a truncation flag.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuditPresign {
    #[serde(default)]
    pub bucket: String,
    #[serde(default)]
    pub since: String,
    #[serde(default)]
    pub ttl_seconds: u64,
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub objects: Vec<PresignedObject>,
}

/// Response from `POST /api/users/groups` and `POST /api/users/state` — the
/// machine siblings of the console user-admin routes. `groups` is set by the
/// group endpoint, `state` by the state endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct UserMutated {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub state: String,
}

/// One row from `/api/onboard/operator-keys`.
///
/// The key VALUE is deliberately absent: the console serialises it as `-`, so
/// no listing can carry live key material however wide the caller sets its
/// filter. The fingerprint is what identifies a key in a listing or an audit
/// record.
#[derive(Debug, Clone, Deserialize)]
pub struct OperatorKeyRow {
    pub id: String,
    pub handle: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub reason: String,
    pub status: String,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub decided_by: String,
}

/// Response from `/api/onboard/operator-keys/{id}/collect` — the one and only
/// time the key value crosses the wire.
#[derive(Debug, Clone, Deserialize)]
pub struct OperatorKeyDelivery {
    pub handle: String,
    pub key: String,
}

/// Response from `POST /api/onboard/operator-keys/reconcile`.
///
/// Named for what each side MEANS rather than for the direction of the diff:
/// a grant with no record is somebody with access nobody decided to give them,
/// and a record with no grant is somebody holding a key the portal refuses.
#[derive(Debug, Clone, Deserialize)]
pub struct OperatorKeyReconcile {
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub applied: bool,
    /// Revoked in the record, still granted by the portal. The only case
    /// `--apply` acts on.
    #[serde(default)]
    pub revoked_but_still_granted: Vec<String>,
    /// A grant with no request behind it — a service's own deploy, or a key
    /// issued before this registry existed. Reported, never removed.
    #[serde(default)]
    pub granted_but_not_ours: Vec<String>,
    /// An active record with no grant: somebody holds a key the portal refuses.
    #[serde(default)]
    pub recorded_but_not_granted: Vec<String>,
    /// Revoked in the record with no revocation tombstone on disk: a key for
    /// the handle pinned in the portal's env is still accepted (GHSA-2w7v).
    /// `--apply` writes the tombstone.
    #[serde(default)]
    pub revoked_without_tombstone: Vec<String>,
}

#[derive(Clone)]
pub struct PortalClient {
    inner: Option<PortalInner>,
}

#[derive(Clone)]
struct PortalInner {
    http: Client,
    repair_resolve: Option<(String, std::net::SocketAddr)>,
    repair_actor: Option<reqwest::header::HeaderValue>,
    base_url: String,
    // The base URL for the onboarding surface (`/api/onboard/*`). The onboarding
    // engine moved from the data-fabric portal to goat-id-console
    // (docs/design/onboarding-extraction-plan.md), so `goat admin invite/queue`
    // must target the console while the cert-mint (`/api/orgs`) + whoami + access
    // calls stay on the portal `base_url`. Defaults to `base_url` (back-compat: a
    // profile without an `onboard_endpoint` keeps the pre-cutover behaviour);
    // overridden by [`PortalClient::with_onboard_endpoint`].
    onboard_base_url: String,
    // True once a distinct onboard endpoint was configured via
    // `with_onboard_endpoint`. Defaults to false so a client built
    // without an override reads as "fallback in play".
    onboard_endpoint_configured: bool,
    api_key: String,
    // Resolved-once API segment for the org/vendor surface (ADR 1052 Phase 2):
    // `true` once we've confirmed the portal serves `/api/orgs*`, `false` if it
    // only serves the legacy `/api/vendors*`. Arc<OnceLock> so the cloneable
    // Inner shares one cell. See `seg()`.
    orgs_seg: Arc<OnceLock<bool>>,
}

/// Default portal request timeout (seconds). Sized for a degraded tactical
/// mesh where a round-trip to the portal can take ~10 s; override with
/// `GOAT_PORTAL_TIMEOUT_SECS`.
pub const DEFAULT_PORTAL_TIMEOUT_SECS: u64 = 20;

/// Resolve the portal request timeout: `GOAT_PORTAL_TIMEOUT_SECS` if set to a
/// positive integer, else [`DEFAULT_PORTAL_TIMEOUT_SECS`].
fn portal_timeout_secs() -> u64 {
    std::env::var("GOAT_PORTAL_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_PORTAL_TIMEOUT_SECS)
}

/// The resolved portal request budget as a [`Duration`] — the single source of
/// truth for "how long are we willing to wait on the portal". [`PortalClient`]
/// applies this internally; any caller that wraps a portal call in its own
/// outer timeout (e.g. `goat doctor`'s step-4 probe) MUST use this same value,
/// or the two disagree and a slow-but-healthy portal gets reported as down.
pub fn portal_timeout() -> Duration {
    Duration::from_secs(portal_timeout_secs())
}

/// Enroll one machine (system principal) against a custodian's enrollment_grant
/// (ADR 1096 stage 2). Mirrors the portal `POST /api/machines/enroll` body.
/// `custodian` is omitted for self-service (the server uses the caller's own
/// identity) and required when an operator enrolls on someone's behalf or for an
/// infrastructure node-class.
#[derive(Debug, Clone, Serialize)]
pub struct MachineEnrollSpec {
    pub machine_handle: String,
    pub node_class: String,
    pub autonomy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custodian: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// A PLATFORM's declared sub-workload composition (function classes) — ADR 1098.
    /// Omitted for a simple system.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workloads: Option<Vec<String>>,
    /// Scope the minted MESH KEY to an exercise's synthetic-site groups
    /// (`EX-<PREFIX>-<class>`) instead of the goatnet site scope
    /// (machine-mesh-alignment). The name must be in the portal's exercise
    /// map; identity is untouched — the exercise is a key scope, not a cert
    /// attribute. Omitted = site scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exercise: Option<String>,
    /// ADR 1251: the same keyless-source and carriage fields as a bulk entry
    /// ([`MachineBulkSpec`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyless: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapters: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receive: Option<MachineReceiveSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pods: Option<Vec<String>>,
}

/// The machine's minted bundle as the portal returns it (`POST /api/machines/enroll`
/// 201). `slot_id` is the machine's own anchor (cert CN + write namespace);
/// `bundle_url` is a portal-relative PATH. `quota_remaining` is the custodian's
/// remaining enrollment quota after this mint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineEnrollResult {
    pub slot_id: String,
    pub system_handle: String,
    pub custodian: String,
    pub node_class: String,
    pub autonomy: String,
    pub affiliation: String,
    #[serde(default)]
    pub workloads: Vec<String>,
    /// Empty for a keyless source, which is minted no key (ADR 1251).
    #[serde(default)]
    pub api_key: String,
    /// Empty for a keyless source, which has no bundle (ADR 1251).
    #[serde(default)]
    pub bundle_url: String,
    #[serde(default)]
    pub bundle_token: String,
    #[serde(default)]
    pub uri_san: String,
    #[serde(default)]
    pub quota_remaining: i64,
    #[serde(default)]
    pub note_to_share: String,
    /// Full NetBird group names DERIVED from the node-class (ADR 1116 §9) —
    /// the answer to "which groups does this machine belong in", which nobody
    /// types any more. Always populated, whether or not the portal minted the
    /// key itself.
    #[serde(default)]
    pub mesh_groups: Vec<String>,
    /// One-off setup key carrying exactly those groups. Empty when the portal
    /// is not configured to mint (`mesh-key-mint-enabled` off), in which case
    /// `mesh_note` says so rather than letting an absent key read as "no mesh
    /// membership needed".
    #[serde(default)]
    pub mesh_setup_key: String,
    #[serde(default)]
    pub mesh_note: String,
    /// Names the exercise the mesh key was scoped to; empty for the goatnet
    /// site scope.
    #[serde(default)]
    pub mesh_scope: String,
    /// A keyless source (ADR 1251): a slot and custody, no key.
    #[serde(default)]
    pub keyless: bool,
    /// The named carriage rules the entry's `pods` asked for, one per pod:
    /// `{pod_slot, rule_id, state, error_code, error}` (ADR 1251).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carriage: Vec<serde_json::Value>,
}

/// The per-machine half of an enrollment act — what varies machine-to-machine
/// inside a bulk act (mirrors the portal's `machineSpec`). `Deserialize` so
/// the CLI can read a YAML/JSON list of these straight from a spec file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineBulkSpec {
    pub machine_handle: String,
    pub node_class: String,
    pub autonomy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// A PLATFORM's declared sub-workload composition (function classes) —
    /// ADR 1098. Omitted for a simple system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workloads: Option<Vec<String>>,
    /// A keyless source (ADR 1251, pod-carriage-and-provenance.md §7.3): the
    /// portal allocates a slot and records custody but mints no key or
    /// certificate. Omitted for an ordinary machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyless: Option<bool>,
    /// The ingress adapters (their slots) that may publish for a keyless
    /// source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapters: Option<Vec<String>>,
    /// How a keyless source is received by its adapter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receive: Option<MachineReceiveSpec>,
    /// Pods (their slots) asked to carry this machine: each becomes a named
    /// carriage rule request at enrollment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pods: Option<Vec<String>>,
}

/// How an ingress adapter receives a keyless source (§7.3): the interface
/// (`udp`), the port it listens on and the one sender address it accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineReceiveSpec {
    pub interface: String,
    pub port: u16,
    pub sender: String,
}

/// `POST /api/machines/enroll-bulk` body (machine-mesh-alignment D7): N
/// machines under ONE custodian in one operator act — N independent mints,
/// one authorization, one summary audit act. `custodian`/`exercise` follow
/// the single-door contract; the server caps an act at 100 entries.
#[derive(Debug, Clone, Serialize)]
pub struct MachineBulkEnrollSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custodian: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exercise: Option<String>,
    pub machines: Vec<MachineBulkSpec>,
}

/// One entry of the bulk-enroll response: `status` is `"minted"` (with the
/// full single-enroll bundle in `result`) or `"failed"` (with the same
/// error-code vocabulary the single door refuses with).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineBulkEnrollEntry {
    pub machine_handle: String,
    pub status: String,
    #[serde(default)]
    pub error_code: String,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub result: Option<MachineEnrollResult>,
}

/// The `POST /api/machines/enroll-bulk` 200 body. The server returns 200 even
/// with partial failures — the per-entry statuses ARE the result of the act;
/// a caller that wants all-or-nothing checks `failed == 0` and revokes.
/// `quota_remaining` is `-1` when the post-act quota read failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineBulkEnrollResult {
    pub custodian: String,
    #[serde(default)]
    pub minted: i64,
    #[serde(default)]
    pub failed: i64,
    #[serde(default)]
    pub quota_remaining: i64,
    #[serde(default)]
    pub entries: Vec<MachineBulkEnrollEntry>,
}

/// The `POST /api/machines/{anchor}/mesh-key` 200 body (machine-mesh-alignment
/// D1): the quota-neutral mesh-key re-mint for an already-enrolled machine —
/// groups re-derived from the BORN-AS node-class, a fresh one-off key. Same
/// mesh-field semantics as [`MachineEnrollResult`]: an empty `mesh_setup_key`
/// means minting is off and `mesh_note` says so.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineMeshKeyResult {
    pub slot_id: String,
    pub system_handle: String,
    pub node_class: String,
    #[serde(default)]
    pub mesh_groups: Vec<String>,
    #[serde(default)]
    pub mesh_setup_key: String,
    #[serde(default)]
    pub mesh_note: String,
    #[serde(default)]
    pub mesh_scope: String,
}

/// The `DELETE /api/machines/{anchor}` 200 body — the machine revoke leg
/// (mesh-first, ADR 1031). Every list is the inventory of what was actually
/// removed rather than a count, because "cut" is only checkable if it names
/// the peer and key ids. `mesh_cut: false` is a real answer, not a failure: a
/// portal with no mgmt credential never minted the key either, and `mesh_note`
/// names the manual step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineRevokeResult {
    pub slot_id: String,
    pub system_handle: String,
    #[serde(default)]
    pub node_class: String,
    #[serde(default)]
    pub custodian: String,
    #[serde(default)]
    pub mesh_cut: bool,
    #[serde(default)]
    pub peers_deleted: Vec<String>,
    #[serde(default)]
    pub keys_deleted: Vec<String>,
    #[serde(default)]
    pub keys_already_spent: Vec<String>,
    #[serde(default)]
    pub mesh_note: String,
    #[serde(default)]
    pub slot_freed: bool,
    #[serde(default)]
    pub quota_remaining: i64,
    #[serde(default)]
    pub note: String,
}

/// One enrolled machine as the custody surface returns it (`GET /api/machines`,
/// ADR 1096 stage 5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineListItem {
    pub slot_id: String,
    pub system_handle: String,
    pub custodian: String,
    pub node_class: String,
    pub autonomy: String,
    pub affiliation: String,
    #[serde(default)]
    pub workloads: Vec<String>,
    #[serde(default)]
    pub enrolled_at: i64,
}

/// A custodian's derived quota view, as carried on a single-custodian machine
/// list (`GET /api/machines?custodian=…` or a vendor-scoped call).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineQuotaView {
    pub custodian: String,
    pub quota: i64,
    pub used: i64,
    pub remaining: i64,
}

/// Set a custodian's machine enrollment quota (`POST /api/machines/grant`,
/// operator-only). Include `affiliation` (and optional `site`) to seed or
/// replace the full grant — required the first time, since a machine inherits
/// its custodian's unit. Omit `affiliation` to adjust only the quota of a grant
/// that already exists.
#[derive(Debug, Clone, Serialize)]
pub struct MachineGrantSpec {
    pub custodian: String,
    pub quota: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affiliation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// ADR 1113 delegated network-admin entitlement. `None` means "say nothing"
    /// — the server leaves the existing entitlement untouched, so a routine
    /// quota bump can never silently revoke it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_admin: Option<bool>,
}
/// Preview by default; applying requires the exact server preview revision.
#[derive(Debug, Clone, Serialize)]
pub struct AffiliationRepairSpec {
    pub from: Vec<String>,
    pub to: String,
    pub reason: String,
    pub apply: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffiliationRepairRow {
    pub kind: String,
    pub id: String,
    pub affiliation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffiliationRepairResult {
    pub from: Vec<String>,
    pub to: String,
    pub revision: String,
    pub rows: Vec<AffiliationRepairRow>,
    pub grants: usize,
    pub slots: usize,
    pub applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_id: Option<i64>,
}

/// The `GET /api/machines` body: the fleet (operator) or a custodian's machines,
/// with the quota present only on a single-custodian view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineListResult {
    #[serde(default)]
    pub machines: Vec<MachineListItem>,
    #[serde(default)]
    pub quota: Option<MachineQuotaView>,
}

/// A preemptive data-access grant to create (ADR 1075 §12, data-access
/// program D1). `grantee_kind` is "org" | "principal"; `effective_to` is an
/// optional unix expiry. Mirrors the portal `POST /api/access/grants` body.
#[derive(Debug, Clone, Serialize)]
pub struct GrantSpec {
    pub producer: String,
    pub grantee_kind: String,
    pub grantee_id: String,
    pub justification: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_to: Option<i64>,
}

/// Add a publisher to a REGISTERED multi-publisher topic (ADR 1105,
/// PUBSUB-ORG-ADMIN). Lets a second same-org member (`writer`) also publish the
/// multi-publisher topic identified by `multipub_topic_id`. Single-owner roots
/// are never involved — co-publish only ever targets a deliberately-created
/// multi-publisher topic.
/// Mirrors the portal `POST /api/access/co-publish` body; the server enforces
/// at-most-own-org (`data.govern.org`) and records the audit.
#[derive(Debug, Clone, Serialize)]
pub struct CoPublishSpec {
    pub writer: String,
    pub multipub_topic_id: String,
    pub justification: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_to: Option<i64>,
}

/// Create a multi-publisher topic for an org (ADR 1105, PUBSUB-ORG-ADMIN)
/// — a deliberate govern-admin act. Mirrors the portal
/// `POST /api/access/multipub-topics` body; the server enforces at-most-own-org
/// on `org` and records the audit.
#[derive(Debug, Clone, Serialize)]
pub struct MultipubTopicSpec {
    pub org: String,
    pub display_name: String,
    pub justification: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persist: Option<bool>,
}

/// A multipub-topic row as the portal returns it (`POST`/`GET
/// /api/access/multipub-topics`). The `MULTI-PUBLISHER` class marker + owning
/// `org` are the discoverability surface; `publishers` is populated on the list
/// read (the declared writer anchors), empty on a create response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultipubTopicRow {
    pub topic_id: String,
    pub org: String,
    pub pool_handle: String,
    pub key_base: String,
    pub display_name: String,
    #[serde(default)]
    pub persist: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub publishers: Vec<String>,
}

/// A grant row as the portal returns it (subset consumed by the CLI).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantRow {
    pub id: String,
    pub grantee_kind: String,
    pub grantee_id: String,
    pub producer: String,
    #[serde(default)]
    pub justification: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub effective_to: Option<i64>,
    // Additive: the grant action ("write" for a co-publish grant). `serde(default)`
    // keeps this backward-compatible with view/read grant rows that omit it.
    #[serde(default)]
    pub action: String,
}

/// The derived machine-plane audience for one release-set, as
/// `GET /api/access/audience` returns it (mirrors the portal's audienceResponse).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudienceRow {
    pub release_set: String,
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub derived_at: String,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// One governance-admin nomination row (ADR 1102 decision 1 / U9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernAdminRow {
    pub id: String,
    pub org: String,
    pub identity_id: String,
    #[serde(default)]
    pub nominated_by: String,
    #[serde(default)]
    pub justification: String,
    #[serde(default)]
    pub status: String,
}

/// `GET /api/access/govern-admins` result: the rows plus which mode the org is
/// in ("bootstrap …" while it has no active nominations, "nominated …" once
/// narrowed to the listed admins).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernAdminsResult {
    #[serde(default)]
    pub org: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub admins: Vec<GovernAdminRow>,
}

/// The recipient-cert store's answer for one anchor (U7 publish-my-key):
/// either the current public leaf PEM, or an honest miss whose message names
/// the remedy (rotate — the re-mint persists).
#[derive(Debug, Clone)]
pub enum RecipientCert {
    Published { pem: String },
    Missing { message: String },
}

impl PortalClient {
    /// Build a client. `base_url` is the portal origin (e.g.
    /// `https://portal.example.net`; `api_key` is
    /// the operator Bearer token. Either-empty → disabled client.
    ///
    /// The request timeout defaults to [`DEFAULT_PORTAL_TIMEOUT_SECS`] and is
    /// overridable via `GOAT_PORTAL_TIMEOUT_SECS`. The old 5 s budget was
    /// tuned for a LAN-fast portal; over a degraded tactical mesh the portal
    /// answers but a single round-trip can take ~10 s (~3 s RTT with
    /// 50 % loss gives a 9 s response), so a 5 s client times out on a
    /// portal that is actually up and surfaces it as a false "unreachable". A
    /// generous ceiling costs nothing on a fast network — calls still return
    /// in ms — and is the difference between names+perms loading or not at the
    /// edge.
    pub fn new(base_url: String, api_key: String) -> Self {
        if base_url.is_empty() || api_key.is_empty() {
            return Self { inner: None };
        }
        let timeout = portal_timeout();
        let http = Client::builder()
            .timeout(timeout)
            // Connect (incl. the TLS handshake) is the slow leg on a lossy
            // mesh; allow it the full budget rather than a separate shorter
            // one, so a slow-but-up portal isn't cut off mid-handshake.
            .connect_timeout(timeout)
            .build()
            .unwrap_or_else(|_| Client::new());
        let base_url = base_url.trim_end_matches('/').to_string();
        Self {
            inner: Some(PortalInner {
                http,
                repair_resolve: None,
                repair_actor: None,
                // Onboard defaults to the portal origin; a profile with a distinct
                // `onboard_endpoint` re-points it via with_onboard_endpoint().
                onboard_base_url: base_url.clone(),
                onboard_endpoint_configured: false,
                base_url,
                api_key,
                orgs_seg: Arc::new(OnceLock::new()),
            }),
        }
    }

    /// Re-point the onboarding surface (`/api/onboard/*`) at a distinct origin —
    /// goat-id-console, which owns the onboarding engine post-cutover
    /// (docs/design/onboarding-extraction-plan.md). The portal `base_url` still
    /// serves the cert-mint (`/api/orgs`), whoami, and access surfaces. A CLI
    /// mint against the *portal's* onboard path would land in the portal store
    /// that nothing reads anymore, so re-pointing here is what makes
    /// `goat admin invite`/`queue` land in the live console store. Empty `url`
    /// or a disabled client is a no-op (keeps the base_url default).
    pub fn with_onboard_endpoint(mut self, url: &str) -> Self {
        let url = url.trim().trim_end_matches('/');
        if url.is_empty() {
            return self;
        }
        if let Some(inner) = self.inner.as_mut() {
            inner.onboard_base_url = url.to_string();
            inner.onboard_endpoint_configured = true;
        }
        self
    }

    /// Pin `host` to `addr` for this client's connections (reqwest
    /// `ClientBuilder::resolve`) — the goat-cli name-fallback resolver
    /// (ADR 1082) applies this ONLY after live DNS already failed and a
    /// validly-signed name snapshot answered, so live DNS is never
    /// shadowed. TLS SNI + certificate verification still use the
    /// hostname; only the socket address is overridden. No-op on a
    /// disabled client or a builder failure (keeps the default client).
    pub fn with_resolve(mut self, host: &str, addr: std::net::SocketAddr) -> Self {
        if let Some(inner) = self.inner.as_mut() {
            inner.repair_resolve = Some((host.to_owned(), addr));
            let timeout = Duration::from_secs(portal_timeout_secs());
            if let Ok(http) = Client::builder()
                .timeout(timeout)
                .connect_timeout(timeout)
                .resolve(host, addr)
                .build()
            {
                inner.http = http;
            }
        }
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// Name the human operator behind this client's shared operator key. Every
    /// request then carries the delegated-attribution header
    /// `X-Goat-Operator-Actor`, so portal-side sponsor/actor attribution
    /// records the person, not the shared "operator" label. The portal
    /// honors the header only for operator principals — holding the operator
    /// key is the trust boundary. The generic `operator` fallback is skipped
    /// (it adds nothing over the server default), as is a handle the header
    /// can't carry.
    pub fn with_operator_actor(mut self, actor: &str) -> Self {
        let actor = actor.trim();
        if actor.is_empty() || actor == "operator" {
            return self;
        }
        if let Some(inner) = self.inner.as_mut() {
            if let Ok(v) = reqwest::header::HeaderValue::from_str(actor) {
                inner.repair_actor = Some(v.clone());
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert("x-goat-operator-actor", v);
                let timeout = portal_timeout();
                if let Ok(http) = Client::builder()
                    .timeout(timeout)
                    .connect_timeout(timeout)
                    .default_headers(headers)
                    .build()
                {
                    inner.http = http;
                }
            }
        }
        self
    }

    /// The portal's base URL (e.g. `https://portal.example.net`). Used
    /// by goat-cli's `--narrate` to render the `POST <path>` line.
    /// Returns `None` if the client is disabled.
    pub fn base_url(&self) -> Option<&str> {
        self.inner.as_ref().map(|i| i.base_url.as_str())
    }

    /// The base URL the onboarding surface (`/api/onboard/*`) targets — the
    /// console post-cutover, or the portal `base_url` when no separate
    /// onboard endpoint was configured. Returns `None` if the client is
    /// disabled.
    pub fn onboard_base_url(&self) -> Option<&str> {
        self.inner.as_ref().map(|i| i.onboard_base_url.as_str())
    }

    /// True when this client is enabled and its onboard surface still falls
    /// back to the portal `base_url` (no distinct onboard endpoint was
    /// configured). Disabled clients report `false` so profile-missing paths
    /// stay clean.
    pub fn onboard_endpoint_is_fallback(&self) -> bool {
        self.inner
            .as_ref()
            .map(|inner| !inner.onboard_endpoint_configured)
            .unwrap_or(false)
    }

    /// Resolve (once, cached) the API path segment for the org/vendor surface
    /// (ADR 1052 Phase 2). Prefers the `orgs` surface; falls back to the
    /// deprecated `vendors` on a 404 from an un-migrated portal. The fleet is
    /// mixed during the migration window (some portals serve `/api/orgs*`, others may
    /// still be vendor-only), and the same client binary talks to either — so
    /// the segment is probed per client, not compiled in. A network error on
    /// the probe falls back to `vendors`, which every portal still serves.
    async fn seg(&self, inner: &PortalInner) -> &'static str {
        if let Some(&orgs) = inner.orgs_seg.get() {
            return if orgs { "orgs" } else { "vendors" };
        }
        let probe = format!("{}/api/orgs", inner.base_url);
        let orgs = match inner
            .http
            .get(&probe)
            .bearer_auth(&inner.api_key)
            .send()
            .await
        {
            // 404 ⇒ the portal doesn't register /api/orgs (legacy). Any other
            // status (200/401/403/5xx) ⇒ the route exists; use it.
            Ok(r) => r.status() != StatusCode::NOT_FOUND,
            Err(_) => false,
        };
        let _ = inner.orgs_seg.set(orgs);
        if orgs { "orgs" } else { "vendors" }
    }

    /// Build a `/api/<orgs|vendors><suffix>` URL with the resolved segment.
    /// `suffix` is the path AFTER the segment (e.g. `/{anchor}/topics`), or
    /// `""` for the bare listing.
    async fn vurl(&self, inner: &PortalInner, suffix: &str) -> String {
        format!("{}/api/{}{}", inner.base_url, self.seg(inner).await, suffix)
    }

    /// `GET /api/vendors` → all registered vendor prefixes. Returns
    /// an empty vec if the portal exists but has no vendors enrolled;
    /// `Err(Disabled)` if no client is configured.
    pub async fn list_vendors(&self) -> Result<Vec<Vendor>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = self.vurl(inner, "").await;
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {
                let env: VendorsEnvelope = resp
                    .json()
                    .await
                    .map_err(|e| PortalError::Decode(e.to_string()))?;
                Ok(env.into_rows())
            }
            s => {
                warn!(url = %url, status = %s, "list_vendors non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/aliases` → the slot_id↔human-alias map (ADR 1039) used by
    /// the display layer. Empty vec if the goatnet is per-vendor (no slots);
    /// `Err(Disabled)` if no client is configured. The caller caches this and
    /// degrades to raw UUIDs when it can't be fetched — aliases never block.
    pub async fn list_aliases(&self) -> Result<Vec<AliasRow>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/aliases", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {
                let env: AliasesEnvelope = resp
                    .json()
                    .await
                    .map_err(|e| PortalError::Decode(e.to_string()))?;
                Ok(env.aliases)
            }
            s => {
                warn!(url = %url, status = %s, "list_aliases non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/access/data-roles?email=<addr>` → the registry-derived
    /// `data.view.*` roles for a human principal (ADR 1074 resolver, build
    /// item B1). The portal owns the derivation (Kratos identity ↔ roster
    /// email-hash join + in-effect grant rows); this returns only the
    /// `roles` array. Auth is the client's bearer key, which MUST be
    /// lookup-authorized (an operator token or the console↔portal lookup
    /// secret — the resolver gates on `lookupAuthorized`). `Err(Disabled)`
    /// if no client is configured; a non-2xx (incl. 404 unknown principal)
    /// is surfaced so the caller can fail closed (leave the base scope).
    pub async fn fetch_data_roles(&self, email: &str) -> Result<Vec<String>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/data-roles", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .query(&[("email", email)])
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {
                let env: DataRolesEnvelope = resp
                    .json()
                    .await
                    .map_err(|e| PortalError::Decode(e.to_string()))?;
                Ok(env.roles)
            }
            s => {
                warn!(url = %url, status = %s, "fetch_data_roles non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/envelopes` → the anchor↔throughput-envelope map (ADR 1045)
    /// the reveal layer (`goat top`) caches to render each producer's band.
    /// Empty vec when no envelopes are declared; `Err(Disabled)` if no client
    /// is configured. The caller caches this and degrades to "no envelope"
    /// (an honest `—`, never a faked verdict) when it can't be fetched.
    pub async fn list_envelopes(&self) -> Result<Vec<EnvelopeRow>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/envelopes", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {
                let env: EnvelopesEnvelope = resp
                    .json()
                    .await
                    .map_err(|e| PortalError::Decode(e.to_string()))?;
                Ok(env.envelopes)
            }
            s => {
                warn!(url = %url, status = %s, "list_envelopes non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/vendors/{vendor}/topics` → all topics declared by
    /// `vendor`. Returns empty vec if the vendor exists but has no
    /// topics; `Err(NotFound)` if the vendor row itself is absent.
    /// Every org and its topics in ONE request (`GET /api/catalog`).
    ///
    /// WHY THIS EXISTS. The directory view used to call `list_vendors` and
    /// then `list_topics` once PER ORG, concurrently. With hundreds of orgs that is
    /// hundreds of simultaneous requests, and the portal's shared per-source rate limit
    /// refuses most of them, leaving the directory sparse rather than empty. The
    /// caller was not doing anything unreasonable per call — the shape of the
    /// call was the problem.
    ///
    /// Returns `Ok(None)` on 404 so a caller can fall back to the per-org
    /// walk: not every goatnet's portal serves this endpoint yet, and a
    /// surface that hard-fails against an older portal is worse than one that
    /// is briefly chattier.
    pub async fn catalog(&self) -> Result<Option<Catalog>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/catalog", inner.base_url.trim_end_matches('/'));
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<Catalog>()
                .await
                .map(Some)
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(non_2xx(resp).await),
        }
    }

    pub async fn list_topics(&self, vendor: &str) -> Result<Vec<Topic>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = self.vurl(inner, &format!("/{}/topics", vendor)).await;
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {
                let env: TopicsEnvelope = resp
                    .json()
                    .await
                    .map_err(|e| PortalError::Decode(e.to_string()))?;
                Ok(env.topics)
            }
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(vendor, url = %url, status = %s, "list_topics non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// Find a topic by its key_pattern (e.g.
    /// `org-1/radar1/RADAR-EXAMPLE/status`). Convenience wrapper around
    /// `list_topics`: extracts the vendor prefix from the leading slash
    /// segment, lists that vendor's topics, returns the row whose
    /// `key_pattern` matches exactly. Returns `Ok(None)` when the vendor
    /// or the specific key isn't registered. Used by the decoder to
    /// discover the topic's declared `format: protobuf:<Name>` so it
    /// can resolve the right message from a multi-message `.fds`
    /// (fixes the DRS-style "first message in FDS != actual wire
    /// message" failure mode where vendors land their topic with the
    /// dropdown placeholder format and protoc --include_imports puts
    /// a different message first in the descriptor).
    pub async fn find_topic_by_key(&self, key_pattern: &str) -> Result<Option<Topic>, PortalError> {
        let vendor = key_pattern.split('/').next().unwrap_or("");
        if vendor.is_empty() {
            return Ok(None);
        }
        match self.list_topics(vendor).await {
            Ok(topics) => Ok(topics.into_iter().find(|t| t.key_pattern == key_pattern)),
            Err(PortalError::NotFound { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// `GET /api/vendors/{vendor}/topics/{id}` → one topic.
    pub async fn get_topic(&self, vendor: &str, id: i64) -> Result<Topic, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = self
            .vurl(inner, &format!("/{}/topics/{}", vendor, id))
            .await;
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<Topic>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `POST /api/vendors` — admin: register a new vendor. Mirrors
    /// the Go reference at ops/goat-zenoh-portal/internal/pubsubstate/
    /// client.go CreateVendor. Returns `Err(Http {status: 409})` on
    /// already-exists (caller can map to "ok, exists" idempotency).
    /// Admin-gated server-side via the PORTAL_OPERATOR_API_KEY token
    /// scope; this client just submits the request and surfaces the
    /// portal's 401/403 if the token isn't admin.
    pub async fn create_vendor(
        &self,
        req: NewVendor,
    ) -> Result<(), PortalError> {
        // Same endpoint + admin gate as onboard_org; the only difference is
        // that `goat admin vendor create` does not render an onboarding bundle,
        // so it drops the response body. Delegate to keep one HTTP path.
        self.onboard_org(req).await.map(|_| ())
    }

    /// `POST /api/orgs` — admin: onboard an entity (org / unit / system /
    /// partner — ADR 1052, no longer "vendor"-specific) and RETURN the full
    /// onboarding bundle (api_key + bundle_url + uri_san + slot_id). The
    /// bundle-returning sibling of [`create_vendor`](Self::create_vendor)
    /// (the pre-ADR-1052 name kept until that rename track lands), used by the
    /// native fast-path onboard (`goat admin onboard --native`, P2.1). In
    /// per-uuid slot-pool mode (ADR 1025) `req.prefix` is the human
    /// handle and the portal allocates a slot anchor; the response's
    /// `slot_id`/`system_handle` carry the assignment. `req` reuses the
    /// pre-existing `NewVendor` request type (renamed wholesale by ADR 1052).
    ///
    /// Status mapping (verified against the Go reference, internal/api/handlers.go):
    /// 201 = fresh mint (success); 409 = the prefix/alias is already
    /// onboarded (the api_key + bundle are NOT regenerable — surfaced as
    /// `Http {status: 409}` so the caller can point at rotate/revoke); 200 =
    /// partial success (the row exists but ACL render / storage provision
    /// failed and the credential won't authorize writes) — surfaced as
    /// [`PortalError::Transient`] carrying the portal's `note_to_share`, the
    /// fail-loud answer to silent partials (the playbook's Phase 4.0a).
    /// `POST /api/access/grants` — create a preemptive data-access grant
    /// (ADR 1075 §12, D1 mutation verb). The key must be operator or the
    /// producer's custodian (server-enforced). Returns the created row.
    /// `POST /api/machines/enroll` — ADR 1096 stage 2: enroll one machine
    /// against a custodian's enrollment_grant, returning its minted bundle.
    /// Custodian-authed; the server applies the infra-vs-self-service gate per
    /// node-class and enforces the quota atomically. 201 = minted; a non-2xx
    /// (403 no-grant/gate, 409 quota/alias, 400 bad grammar) surfaces as
    /// [`PortalError::Http`] / [`PortalError::HttpWithBody`] so the caller can print the specific refusal.
    pub async fn enroll_machine(
        &self,
        req: &MachineEnrollSpec,
    ) -> Result<MachineEnrollResult, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/machines/enroll", inner.base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(req)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::CREATED => resp
                .json::<MachineEnrollResult>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => {
                warn!(url = %url, status = %s, "enroll_machine non-2xx");
                Err(non_2xx(resp).await)
            }
        }
    }

    /// Operator-only preview or atomic repair of legacy affiliation spellings.
    /// Mutations are never automatically retried. An older portal's 404 is an error.
    pub async fn repair_machine_affiliation(
        &self,
        spec: &AffiliationRepairSpec,
    ) -> Result<AffiliationRepairResult, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        // This mutation must never be replayed by redirect or transport policy.
        // Build separately so existing portal operations retain their policies.
        let timeout = portal_timeout();
        let mut builder = Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never());
        if let Some((host, addr)) = &inner.repair_resolve {
            builder = builder.resolve(host, *addr);
        }
        if let Some(actor) = &inner.repair_actor {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert("x-goat-operator-actor", actor.clone());
            builder = builder.default_headers(headers);
        }
        let http = builder
            .build()
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        let resp = http
            .post(format!(
                "{}/api/machines/affiliation/repair",
                inner.base_url
            ))
            .bearer_auth(&inner.api_key)
            .json(spec)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        if resp.status() != StatusCode::OK {
            return Err(non_2xx(resp).await);
        }
        resp.json::<AffiliationRepairResult>()
            .await
            .map_err(|e| PortalError::Decode(e.to_string()))
    }

    /// `POST /api/machines/grant` — operator-only: set (create or adjust) a
    /// custodian's machine enrollment quota. Include `affiliation` in `spec` to
    /// seed a new grant; omit it to bump only the quota of an existing one.
    /// Returns the resulting quota view (quota/used/remaining). A non-2xx (403
    /// non-operator, 400 missing-affiliation-on-first-grant) surfaces as
    /// [`PortalError::Http`] / [`PortalError::HttpWithBody`].
    pub async fn set_machine_grant(
        &self,
        spec: &MachineGrantSpec,
    ) -> Result<MachineQuotaView, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/machines/grant", inner.base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(spec)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {
                #[derive(Deserialize)]
                struct Wrap {
                    quota: MachineQuotaView,
                }
                resp.json::<Wrap>()
                    .await
                    .map(|w| w.quota)
                    .map_err(|e| PortalError::Decode(e.to_string()))
            }
            s => {
                warn!(url = %url, status = %s, "set_machine_grant non-2xx");
                Err(non_2xx(resp).await)
            }
        }
    }

    /// `GET /api/machines[?custodian=<handle>]` — ADR 1096 stage 5 custody
    /// surface. With no custodian an operator gets the whole fleet; a
    /// vendor-scoped caller always gets only its own machines (the server scopes
    /// it). A single-custodian view carries that custodian's quota.
    pub async fn list_machines(
        &self,
        custodian: Option<&str>,
    ) -> Result<MachineListResult, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        // Custodian handles are lowercase [a-z0-9-], so no percent-encoding needed.
        let url = match custodian {
            Some(c) if !c.is_empty() => format!("{}/api/machines?custodian={}", inner.base_url, c),
            _ => format!("{}/api/machines", inner.base_url),
        };
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<MachineListResult>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => {
                warn!(url = %url, status = %s, "list_machines non-2xx");
                Err(non_2xx(resp).await)
            }
        }
    }

    /// `POST /api/machines/{anchor}/mesh-key` — quota-neutral mesh-key re-mint
    /// for an already-enrolled machine (machine-mesh-alignment D1): the
    /// join-window recovery when the enroll-time key expired before the box
    /// joined. `exercise` scopes the key to an exercise's synthetic-site
    /// groups; `None` = the goatnet site scope. 200 = minted; a non-2xx (404
    /// not-a-machine, 403 custodian mismatch, 409 no mesh derivation for a
    /// legacy born-as class, 400 unknown exercise, 502 mint failure) surfaces
    /// as [`PortalError::Http`] / [`PortalError::HttpWithBody`] so the caller can print the specific refusal.
    pub async fn machine_mesh_key(
        &self,
        anchor: &str,
        exercise: Option<&str>,
    ) -> Result<MachineMeshKeyResult, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/machines/{}/mesh-key", inner.base_url, anchor);
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            exercise: Option<&'a str>,
        }
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&Body { exercise })
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<MachineMeshKeyResult>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => {
                warn!(url = %url, status = %s, "machine_mesh_key non-2xx");
                Err(non_2xx(resp).await)
            }
        }
    }

    /// `DELETE /api/machines/{anchor}` — retire an enrolled machine: cut its
    /// peer and any unconsumed setup key on the mgmt server, then free the
    /// slot and delete the anchor row (mesh-first, ADR 1031). Accepts the
    /// machine's handle as well as its 32-hex anchor. Quota returns by
    /// construction. A non-2xx (404 not-a-machine or already revoked, 403
    /// custodian mismatch, 409 the anchor still owns non-retired topics, 502
    /// the mesh cut failed and NOTHING was changed) surfaces as
    /// [`PortalError::Http`] / [`PortalError::HttpWithBody`].
    pub async fn revoke_machine(&self, anchor: &str) -> Result<MachineRevokeResult, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/machines/{}", inner.base_url, anchor);
        let resp = inner
            .http
            .delete(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<MachineRevokeResult>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => {
                warn!(url = %url, status = %s, "revoke_machine non-2xx");
                Err(non_2xx(resp).await)
            }
        }
    }

    /// `POST /api/machines/enroll-bulk` — enroll N machines under ONE
    /// custodian in one operator act (machine-mesh-alignment D7). The server
    /// returns 200 even with partial failures; the per-entry statuses in the
    /// result carry the act's outcome, so a 200 here is NOT "all minted" —
    /// check `failed`. A non-2xx (whole-act refusal: bad body, mixed
    /// custodians, over the 100-entry cap, no grant) surfaces as
    /// [`PortalError::Http`] / [`PortalError::HttpWithBody`].
    pub async fn enroll_machines_bulk(
        &self,
        req: &MachineBulkEnrollSpec,
    ) -> Result<MachineBulkEnrollResult, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/machines/enroll-bulk", inner.base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(req)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<MachineBulkEnrollResult>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => {
                warn!(url = %url, status = %s, "enroll_machines_bulk non-2xx");
                Err(non_2xx(resp).await)
            }
        }
    }

    pub async fn create_grant(&self, req: &GrantSpec) -> Result<GrantRow, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/grants", inner.base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(req)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => resp
                .json::<GrantRow>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => {
                warn!(url = %url, status = %s, "create_grant non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `POST /api/access/co-publish` — mint an ADR 1105 co-publish write grant:
    /// let `writer` also publish under one of its own org's anchors/topics. The
    /// server enforces at-most-own-org (`data.govern.org`) and audits it; the
    /// returned row is an ordinary grant row (`action = "write"`).
    pub async fn create_copublish_grant(
        &self,
        req: &CoPublishSpec,
    ) -> Result<GrantRow, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/co-publish", inner.base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(req)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => resp
                .json::<GrantRow>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => {
                warn!(url = %url, status = %s, "create_copublish_grant non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/access/co-publish-grants` — the materialized
    /// `zenoh_copublish_grants` ansible group_var for the ACL render
    /// (operator-only). Returned verbatim as JSON for capture into
    /// `group_vars/<env>/data-access.yml`.
    pub async fn copublish_grants_export(&self) -> Result<serde_json::Value, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/co-publish-grants", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<serde_json::Value>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => {
                warn!(url = %url, status = %s, "copublish_grants_export non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `POST /api/access/multipub-topics` — create a multi-publisher topic
    /// for an org (ADR 1105, a deliberate govern-admin act). Single-owner roots
    /// are never touched; this mints a separate, marked `multi-publisher` class.
    /// The server enforces at-most-own-org and audits it; the returned row
    /// carries the assigned `key_base` + `topic_id`.
    pub async fn create_multipub_topic(
        &self,
        req: &MultipubTopicSpec,
    ) -> Result<MultipubTopicRow, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/multipub-topics", inner.base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(req)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => resp
                .json::<MultipubTopicRow>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => {
                warn!(url = %url, status = %s, "create_multipub_topic non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/access/multipub-topics[?org=<org>]` — list/discover
    /// registered
    /// multi-publisher topics: the `MULTI-PUBLISHER` marker, owning org,
    /// key base, and declared publishers. Optionally narrowed to one org.
    pub async fn list_multipub_topics(
        &self,
        org: Option<&str>,
    ) -> Result<Vec<MultipubTopicRow>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/multipub-topics", inner.base_url);
        let mut req = inner.http.get(&url).bearer_auth(&inner.api_key);
        if let Some(o) = org {
            req = req.query(&[("org", o)]);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {
                #[derive(Deserialize)]
                struct Env {
                    #[serde(default)]
                    multipub_topics: Vec<MultipubTopicRow>,
                }
                let env: Env = resp
                    .json()
                    .await
                    .map_err(|e| PortalError::Decode(e.to_string()))?;
                Ok(env.multipub_topics)
            }
            s => {
                warn!(url = %url, status = %s, "list_multipub_topics non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/access/audience?release_set=<K>` — the derived machine-plane
    /// audience: the anchors a producer's DEK is wrapped to. Owner-facing "who
    /// can actually read my stream" (WS4). Operator/service-token gated like the
    /// other access reads.
    pub async fn audience(&self, release_set: &str) -> Result<AudienceRow, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/audience", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .query(&[("release_set", release_set)])
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<AudienceRow>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => {
                warn!(url = %url, status = %s, "audience non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/access/govern-admins[?org=<O>][&all=1]` — who administers an
    /// org's data governance, and whether the org is in bootstrap or nominated
    /// (narrowed) mode (ADR 1102 / U9).
    pub async fn govern_admins(
        &self,
        org: Option<&str>,
        all: bool,
    ) -> Result<GovernAdminsResult, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let mut url = format!("{}/api/access/govern-admins", inner.base_url);
        let mut q: Vec<(&str, &str)> = Vec::new();
        if let Some(o) = org {
            q.push(("org", o));
        }
        if all {
            q.push(("all", "1"));
        }
        if !q.is_empty() {
            url.push('?');
            url.push_str(
                &q.iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("&"),
            );
        }
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<GovernAdminsResult>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => {
                warn!(url = %url, status = %s, "govern_admins non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `POST /api/access/govern-admins` — nominate a governance admin for an
    /// org (at-most-own-scope; the nominee must be an active member of the
    /// org). Exactly one of `email` / `identity_id` selects the nominee. The
    /// FIRST active nomination narrows the org: from then on only nominated
    /// admins carry `data.govern.org:<O>`, on both planes.
    pub async fn nominate_govern_admin(
        &self,
        org: &str,
        email: Option<&str>,
        identity_id: Option<&str>,
        justification: &str,
    ) -> Result<GovernAdminRow, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/govern-admins", inner.base_url);
        #[derive(Serialize)]
        struct Body<'a> {
            org: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            email: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            identity_id: Option<&'a str>,
            justification: &'a str,
        }
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&Body {
                org,
                email,
                identity_id,
                justification,
            })
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => resp
                .json::<GovernAdminRow>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => {
                warn!(url = %url, status = %s, "nominate_govern_admin non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `POST /api/access/govern-admins/<id>/remove` — end-date a nomination.
    /// Removing an org's LAST active admin is refused server-side (HTTP 409,
    /// monotonic narrowing) — nominate a replacement first.
    pub async fn remove_govern_admin(&self, id: &str) -> Result<(), PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/govern-admins/{}/remove", inner.base_url, id);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => Ok(()),
            s => {
                warn!(url = %url, status = %s, "remove_govern_admin non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/access/recipient-cert?anchor=<a>` — the by-anchor PUBLIC
    /// recipient-cert store (U7). 200 → the current leaf PEM; 404 → an honest
    /// miss whose message carries the remedy (a leaf minted before the store
    /// existed publishes on its next rotation).
    pub async fn recipient_cert(&self, anchor: &str) -> Result<RecipientCert, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/recipient-cert", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .query(&[("anchor", anchor)])
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .text()
                .await
                .map(|pem| RecipientCert::Published { pem })
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::NOT_FOUND => {
                #[derive(Deserialize)]
                struct Err1 {
                    error: Err2,
                }
                #[derive(Deserialize)]
                struct Err2 {
                    #[serde(default)]
                    message: String,
                }
                let message = resp
                    .json::<Err1>()
                    .await
                    .map(|e| e.error.message)
                    .unwrap_or_else(|_| "no persisted recipient cert for this anchor".into());
                Ok(RecipientCert::Missing { message })
            }
            s => {
                warn!(url = %url, status = %s, "recipient_cert non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `POST /api/access/grants/<id>/revoke` — end-date a grant (D1 mutation
    /// verb). Deprecate-never-delete: the row stays for audit history. Idempotent
    /// re-revoke is the server's concern; a non-2xx surfaces as `Http`.
    pub async fn revoke_grant(&self, id: &str) -> Result<(), PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/grants/{}/revoke", inner.base_url, id);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => Ok(()),
            s => {
                warn!(url = %url, status = %s, "revoke_grant non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    pub async fn onboard_org(&self, req: NewVendor) -> Result<OnboardedOrg, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = self.vurl(inner, "").await;
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&req)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        let status = resp.status();
        match status {
            StatusCode::CREATED => resp
                .json::<OnboardedOrg>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::OK => {
                // 200 is only ever the partial-success path server-side. Decode
                // it for the note, then fail loud — a created-but-unrenderable
                // row hands out a credential that the router will silently deny.
                let body = resp
                    .json::<OnboardedOrg>()
                    .await
                    .map_err(|e| PortalError::Decode(e.to_string()))?;
                if body.note_to_share.is_empty() {
                    // A portal variant that returns 200 on full success — accept.
                    Ok(body)
                } else {
                    warn!(prefix = %req.prefix, url = %url, "onboard partial success (HTTP 200)");
                    Err(PortalError::Transient(format!(
                        "portal partial success (HTTP 200): {}. The row was created but the \
                         credential won't authorize writes until an operator clears it — revoke \
                         the row at the portal and re-run.",
                        body.note_to_share
                    )))
                }
            }
            s => {
                warn!(prefix = %req.prefix, url = %url, status = %s, "onboard_org non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// Mint a one-time candidate-onboarding invite (ADR 1062). POSTs the
    /// all-optional [`NewInvite`] to `/api/onboard/invite`; the response carries
    /// the one-time token + the nomination link (shown once). The server
    /// enforces the minimal-burden + escalated-role rules regardless of caller.
    pub async fn mint_invite(&self, req: &NewInvite) -> Result<InviteMinted, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/onboard/invite", inner.onboard_base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(req)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        let status = resp.status();
        if status == StatusCode::CREATED || status == StatusCode::OK {
            resp.json::<InviteMinted>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string()))
        } else {
            Err(PortalError::http(status, &resp))
        }
    }

    /// `POST /api/users/groups` — set a tenant identity's Ory login groups by
    /// email (the machine sibling of the console's user group patch). `groups`
    /// is the FULL desired set (empty clears). Operator-key; server-side this
    /// funnels through the same core as the web UI — same last-admin guard,
    /// audit, and best-effort role-change email. See
    /// docs/design/onboarding-notifications.md §"CLI/web parity".
    pub async fn patch_user_groups(
        &self,
        email: &str,
        groups: &[String],
        revoke_sessions: bool,
    ) -> Result<UserMutated, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/users/groups", inner.onboard_base_url);
        let body = serde_json::json!({
            "email": email,
            "groups": groups,
            "revoke_sessions": revoke_sessions,
        });
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<UserMutated>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `POST /api/onboard/operator-keys` — record a request for `handle`.
    ///
    /// Records only. The key is minted at APPROVAL, so nothing here grants
    /// anything, and a request that never reached an admin is a request that
    /// never worked rather than a credential loose in the world.
    pub async fn request_operator_key(
        &self,
        handle: &str,
        email: &str,
        env: &str,
        reason: &str,
    ) -> Result<OperatorKeyRow, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/onboard/operator-keys", inner.onboard_base_url);
        let body = serde_json::json!({
            "handle": handle, "email": email, "env": env, "reason": reason,
        });
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<OperatorKeyRow>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            _ => Err(non_2xx(resp).await),
        }
    }

    /// `GET /api/onboard/operator-keys?status=` — list requests by state.
    pub async fn list_operator_keys(
        &self,
        status: &str,
    ) -> Result<Vec<OperatorKeyRow>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/onboard/operator-keys", inner.onboard_base_url);
        let resp = inner
            .http
            .get(&url)
            .query(&[("status", status)])
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<Vec<OperatorKeyRow>>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `POST /api/onboard/operator-keys/{id}/{action}` — approve, reject, revoke.
    ///
    /// The server writes or removes the portal's key file as part of the same
    /// call and rolls the decision back if that fails, so a 200 here means the
    /// record and what the portal will accept already agree.
    pub async fn decide_operator_key(
        &self,
        id: &str,
        action: &str,
        note: &str,
    ) -> Result<OperatorKeyRow, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!(
            "{}/api/onboard/operator-keys/{}/{}",
            inner.onboard_base_url, id, action
        );
        let resp = inner
            .http
            .post(&url)
            // query() rather than interpolation: a note is free text, and an
            // unescaped one would truncate the request at the first ampersand.
            .query(&[("note", note)])
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<OperatorKeyRow>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `GET /api/onboard/operator-keys/{id}/collect?handle=` — the single
    /// delivery of the key value.
    ///
    /// Collecting twice is a 409, not a repeat: the console clears the value on
    /// read. The caller must be ready to tell somebody their key is
    /// unrecoverable rather than retrying.
    pub async fn collect_operator_key(
        &self,
        id: &str,
        handle: &str,
    ) -> Result<OperatorKeyDelivery, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!(
            "{}/api/onboard/operator-keys/{}/collect",
            inner.onboard_base_url, id
        );
        let resp = inner
            .http
            .get(&url)
            .query(&[("handle", handle)])
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<OperatorKeyDelivery>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            _ => Err(non_2xx(resp).await),
        }
    }

    /// `POST /api/onboard/operator-keys/reconcile` — compare the record against
    /// the directory the portal actually reads. Dry unless `apply`.
    pub async fn reconcile_operator_keys(
        &self,
        env: &str,
        apply: bool,
    ) -> Result<OperatorKeyReconcile, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!(
            "{}/api/onboard/operator-keys/reconcile",
            inner.onboard_base_url
        );
        let resp = inner
            .http
            .post(&url)
            .query(&[
                ("env", env),
                ("apply", if apply { "true" } else { "false" }),
            ])
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<OperatorKeyReconcile>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// Authorize an explicit reinstatement before the CLI uses privileged SSH.
    /// This endpoint neither changes state nor provides database credentials.
    pub async fn check_identity_reinstatement(
        &self,
        email: &str,
        reason: &str,
        env: &str,
    ) -> Result<(String, String), PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let resp = inner
            .http
            .post(format!(
                "{}/api/users/reinstatement-check",
                inner.onboard_base_url
            ))
            .bearer_auth(&inner.api_key)
            .json(&serde_json::json!({"email":email,"reason":reason,"env":env}))
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        if resp.status() != StatusCode::OK {
            return Err(PortalError::http(resp.status(), &resp));
        }
        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| PortalError::Decode(e.to_string()))?;
        if value.get("env").and_then(|v| v.as_str()) != Some(env) {
            return Err(PortalError::Decode(
                "reinstatement console goatnet mismatch".into(),
            ));
        }
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| PortalError::Decode("missing reinstatement identity".into()))?;
        let actor = value
            .get("actor")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| PortalError::Decode("missing reinstatement actor".into()))?;
        Ok((id.into(), actor.into()))
    }

    /// Use the exact identity authorized for reinstatement, not a second email resolution.
    pub async fn activate_reinstated_identity(&self, id: &str) -> Result<UserMutated, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let resp = inner
            .http
            .post(format!("{}/api/users/state", inner.onboard_base_url))
            .bearer_auth(&inner.api_key)
            .json(&serde_json::json!({"id":id,"state":"active"}))
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        if resp.status() != StatusCode::OK {
            return Err(PortalError::http(resp.status(), &resp));
        }
        resp.json::<UserMutated>()
            .await
            .map_err(|e| PortalError::Decode(e.to_string()))
    }

    /// `POST /api/users/state` — flip a tenant identity's activation state
    /// (`active` / `inactive`) by email. The reactivate-before-grant path;
    /// operator-key, same audit shape as the console.
    pub async fn patch_user_state(
        &self,
        email: &str,
        state: &str,
    ) -> Result<UserMutated, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/users/state", inner.onboard_base_url);
        let body = serde_json::json!({ "email": email, "state": state });
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<UserMutated>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `GET /api/onboard/proposals?status=<status>` — the review queue. `status`
    /// empty = all. Operator-gated (the admin view).
    pub async fn list_proposals(&self, status: &str) -> Result<Vec<Proposal>, PortalError> {
        self.list_proposals_filtered(status, None).await
    }

    /// Review queue narrowed by exact email at the server. Sponsor scoping
    /// still applies. Older consoles can ignore email; callers retain their
    /// local exact filter for that deployment transition.
    pub async fn list_proposals_filtered(
        &self,
        status: &str,
        email: Option<&str>,
    ) -> Result<Vec<Proposal>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/onboard/proposals", inner.onboard_base_url);
        let mut params = Vec::new();
        if !status.is_empty() {
            params.push(("status", status));
        }
        if let Some(email) = email {
            params.push(("email", email));
        }
        let resp = inner
            .http
            .get(&url)
            .query(&params)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<Vec<Proposal>>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `GET /api/onboard/invites?status=<status>` — the operator invite queue.
    /// `status` empty = all. Operator-gated.
    /// `GET /api/onboard/proposals/{id}` — one proposal with its manifest
    /// (`manifest_json`), returned raw: callers read single fields out of the
    /// manifest and a typed struct would drop every field it did not name.
    pub async fn get_proposal_detail(&self, id: &str) -> Result<serde_json::Value, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/onboard/proposals/{}", inner.onboard_base_url, id);
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<serde_json::Value>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// One JSON call against a portal route this client has no typed method
    /// for: `method` on `path` (starting `/api/`, query included), an optional
    /// JSON body, the decoded JSON answer (`null` for an empty 2xx). A non-2xx
    /// answer is [`PortalError::HttpWithBody`] when the body carried a message
    /// (its `error.code` kept), so a caller can map refusals by code. Used by
    /// `goat carriage` (ADR 1251), whose routes are new and versioned in one
    /// place (pod-carriage-and-provenance.md §7.5).
    pub async fn call_json(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}{}", inner.base_url, path);
        let mut req = inner
            .http
            .request(method, &url)
            .bearer_auth(&inner.api_key);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        if !resp.status().is_success() {
            warn!(url = %url, status = %resp.status(), "call_json non-2xx");
            return Err(non_2xx(resp).await);
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_slice(&bytes).map_err(|e| PortalError::Decode(e.to_string()))
    }

    pub async fn list_invites(&self, status: &str) -> Result<Vec<Invite>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let mut url = format!("{}/api/onboard/invites", inner.onboard_base_url);
        if !status.is_empty() {
            url.push_str(&format!("?status={status}"));
        }
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<Vec<Invite>>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `GET /api/onboard/ssh-keys?status=…` — the host-shell key queue.
    /// Empty status uses the server default (submitted); `all` selects every state.
    pub async fn list_ssh_keys(&self, status: &str) -> Result<Vec<SshKey>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let mut url = format!("{}/api/onboard/ssh-keys", inner.onboard_base_url);
        if !status.is_empty() {
            url.push_str(&format!("?status={status}"));
        }
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<Vec<SshKey>>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `POST /api/onboard/ssh-keys` — record a public key for `handle`.
    ///
    /// This is the "a fellow admin pushes it over for me" path: the route is
    /// operator-only, so submitting on someone's behalf is deliberate, not a
    /// hole. It GRANTS NOTHING — an admin still has to approve, and the
    /// installer only ever acts on approved rows.
    pub async fn submit_ssh_key(
        &self,
        handle: &str,
        pubkey: &str,
        env: &str,
        note: &str,
        email: &str,
    ) -> Result<SshKey, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/onboard/ssh-keys", inner.onboard_base_url);
        let body = serde_json::json!({
            "handle": handle, "pubkey": pubkey, "env": env, "note": note, "email": email,
        });
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => resp
                .json::<SshKey>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `POST /api/onboard/ssh-keys/{id}/{action}` — approve | reject | revoke.
    ///
    /// `installed` is deliberately NOT reachable from here: it is a statement
    /// of fact about a host, and only the installer that wrote the file is in
    /// a position to make it.
    pub async fn decide_ssh_key(&self, id: &str, action: &str) -> Result<SshKey, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!(
            "{}/api/onboard/ssh-keys/{id}/{action}",
            inner.onboard_base_url
        );
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<SshKey>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `POST /api/onboard/invites/{id}/revoke` — kill an open invite before use.
    /// Operator-gated; 409 if the invite is no longer open.
    /// `GET /api/org-registry/orgs` — the first-class org registry (ADR 1112),
    /// console-side. Operator or any authenticated machine-realm principal.
    pub async fn org_registry_list(&self) -> Result<Vec<OrgRegistryOrg>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/org-registry/orgs", inner.onboard_base_url);
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {
                #[derive(Deserialize)]
                struct Envelope {
                    #[serde(default)]
                    orgs: Vec<OrgRegistryOrg>,
                }
                resp.json::<Envelope>()
                    .await
                    .map(|e| e.orgs)
                    .map_err(|e| PortalError::Decode(e.to_string()))
            }
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `GET /api/org-registry/suggest?q=&nation=` — fuzzy candidates
    /// (ADR 1112 increment 2). 404 when `ORG_MATCH_ENABLED` is off on the
    /// goatnet — surface that verdict, don't mask it.
    pub async fn org_registry_suggest(
        &self,
        q: &str,
        nation: &str,
    ) -> Result<Vec<OrgRegistryCandidate>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/org-registry/suggest", inner.onboard_base_url);
        let mut query: Vec<(&str, &str)> = vec![("q", q)];
        if !nation.is_empty() {
            query.push(("nation", nation));
        }
        let resp = inner
            .http
            .get(&url)
            .query(&query)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {
                #[derive(Deserialize)]
                struct Envelope {
                    #[serde(default)]
                    candidates: Vec<OrgRegistryCandidate>,
                }
                resp.json::<Envelope>()
                    .await
                    .map(|e| e.candidates)
                    .map_err(|e| PortalError::Decode(e.to_string()))
            }
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `POST` helper for the org-registry mutation endpoints — all share the
    /// shape (JSON body in, `OrgRegistryOrg` out, operator-only server-side).
    async fn org_registry_post(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<OrgRegistryOrg, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/org-registry/{path}", inner.onboard_base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => resp
                .json::<OrgRegistryOrg>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `POST /api/org-registry/orgs` — register an org (optionally under a
    /// parent; `despite` records human-judged-distinct negative pairs).
    #[allow(clippy::too_many_arguments)]
    pub async fn org_registry_register(
        &self,
        id: &str,
        display_name: &str,
        kind: &str,
        nation: &str,
        parent: &str,
        provenance: &str,
        justification: &str,
        despite: &[String],
    ) -> Result<OrgRegistryOrg, PortalError> {
        self.org_registry_post(
            "orgs",
            serde_json::json!({
                "id": id, "display_name": display_name, "kind": kind,
                "nation": nation, "parent": parent, "provenance": provenance,
                "justification": justification, "despite": despite,
            }),
        )
        .await
    }

    /// `POST /api/org-registry/orgs/{id}/parent` — set (or clear, with an
    /// empty parent) the effective-dated containment edge.
    pub async fn org_registry_set_parent(
        &self,
        id: &str,
        parent: &str,
        justification: &str,
    ) -> Result<OrgRegistryOrg, PortalError> {
        self.org_registry_post(
            &format!("orgs/{id}/parent"),
            serde_json::json!({"parent": parent, "justification": justification}),
        )
        .await
    }

    /// `POST /api/org-registry/orgs/{id}/update` — correct an org's facts
    /// (kind / nation / composition / display name) in place. Absent fields are left
    /// unchanged (JSON nulls are omitted, matching the handler's pointer
    /// semantics); the server refuses empty kind/nation replacements.
    pub async fn org_registry_update(
        &self,
        id: &str,
        kind: Option<&str>,
        nation: Option<&str>,
        display_name: Option<&str>,
        // ADR 1171. Unlike kind/nation this MAY be sent empty: "not yet
        // declared" is a real state for this axis, and refusing to return to
        // it would make a wrong guess permanent.
        composition: Option<&str>,
        justification: &str,
    ) -> Result<OrgRegistryOrg, PortalError> {
        let mut body = serde_json::Map::new();
        if let Some(k) = kind {
            body.insert("kind".into(), k.into());
        }
        if let Some(n) = nation {
            body.insert("nation".into(), n.into());
        }
        if let Some(d) = display_name {
            body.insert("display_name".into(), d.into());
        }
        if let Some(c) = composition {
            body.insert("composition".into(), c.into());
        }
        body.insert("justification".into(), justification.into());
        self.org_registry_post(&format!("orgs/{id}/update"), serde_json::Value::Object(body))
            .await
    }

    /// `POST /api/org-registry/orgs/{id}/aliases` — record a name variant that
    /// resolves to this org. Accretes matcher recall; never re-keys anything.
    pub async fn org_registry_add_alias(
        &self,
        id: &str,
        alias: &str,
        kind: &str,
    ) -> Result<OrgRegistryOrg, PortalError> {
        self.org_registry_post(
            &format!("orgs/{id}/aliases"),
            serde_json::json!({"alias": alias, "kind": kind}),
        )
        .await
    }

    /// `POST /api/org-registry/orgs/{loser}/merge` — alias-absorb merge
    /// (ADR 1112 §8). Fail-closed server-side without the acknowledgment.
    pub async fn org_registry_merge(
        &self,
        loser: &str,
        into: &str,
        justification: &str,
        acknowledge_governance_union: bool,
    ) -> Result<OrgRegistryOrg, PortalError> {
        self.org_registry_post(
            &format!("orgs/{loser}/merge"),
            serde_json::json!({
                "into": into, "justification": justification,
                "acknowledge_governance_union": acknowledge_governance_union,
            }),
        )
        .await
    }

    /// `POST /api/org-registry/orgs/{id}/unmerge` — reactivate an absorbed
    /// org (parent edges are NOT auto-reverted; re-attach deliberately).
    pub async fn org_registry_unmerge(&self, id: &str) -> Result<OrgRegistryOrg, PortalError> {
        self.org_registry_post(&format!("orgs/{id}/unmerge"), serde_json::json!({}))
            .await
    }

    pub async fn revoke_invite(&self, id: &str) -> Result<(), PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/onboard/invites/{id}/revoke", inner.onboard_base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => Ok(()),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `POST /api/onboard/proposals/{id}/<action>` where action is one of
    /// `approve` / `reject` / `sponsor-approve` / `sponsor-reject`. Approve
    /// returns the manifest + command; the others return a minimal ack, so the
    /// caller can ignore the body for those. `note` rides the JSON body.
    /// Record an onboarding that happened OUTSIDE the review queue.
    ///
    /// `goat admin onboard` mints directly, and until this existed it wrote the
    /// operator manifest and nothing the console could see — so every lifecycle
    /// lane, all of which enumerate the console store, was blind to those
    /// people.
    ///
    /// Returns `true` when a row was created and `false` when one already
    /// existed for that email, which is a SUCCESS and not a conflict — a
    /// re-run of a mint records the same fact.
    pub async fn register_minted_proposal(
        &self,
        handle: &str,
        email: &str,
        env_name: &str,
        minted_by: &str,
        note: &str,
    ) -> Result<bool, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/onboard/register", inner.onboard_base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&serde_json::json!({
                "handle": handle,
                "email": email,
                "env": env_name,
                "minted_by": minted_by,
                "note": note,
            }))
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        let status = resp.status();
        if status == StatusCode::CREATED {
            Ok(true)
        } else if status == StatusCode::OK {
            Ok(false)
        } else {
            Err(PortalError::http(status, &resp))
        }
    }

    pub async fn decide_proposal(
        &self,
        id: &str,
        action: &str,
        note: &str,
    ) -> Result<ApproveResult, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/onboard/proposals/{id}/{action}", inner.onboard_base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&serde_json::json!({ "note": note }))
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        let status = resp.status();
        if status == StatusCode::OK {
            // approve returns the full ApproveResult; reject/sponsor-* return a
            // small ack that decodes into the same struct's defaults.
            resp.json::<ApproveResult>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string()))
        } else {
            Err(PortalError::http(status, &resp))
        }
    }

    /// `GET /api/onboard/proposals/{id}/repo-access` — the effective repo-access
    /// set (policy ⊕ overrides) for a proposal, plus the stored overrides + github
    /// username + token state. Read-scoped server-side like the proposal GET.
    pub async fn repo_access_get(&self, id: &str) -> Result<RepoAccessView, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/onboard/proposals/{id}/repo-access", inner.onboard_base_url);
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<RepoAccessView>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `GET /api/onboard/repo-access/policy` — the loaded on-rails repo-access
    /// policy (default owner + rules). Returned as a raw JSON value since the
    /// CLI only renders it (the rule shape uses the console's Go field names).
    pub async fn repo_access_policy(&self) -> Result<serde_json::Value, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/onboard/repo-access/policy", inner.onboard_base_url);
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<serde_json::Value>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `POST /api/onboard/proposals/{id}/repo-access` — REPLACE the stored override
    /// set with `add`/`remove` (the caller sends the full desired override). Returns
    /// the recomputed effective set. 409 if the proposal is already onboarded.
    pub async fn repo_access_set(
        &self,
        id: &str,
        add: Vec<RepoGrantRef>,
        remove: Vec<RepoGrantRef>,
    ) -> Result<RepoAccessView, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/onboard/proposals/{id}/repo-access", inner.onboard_base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&serde_json::json!({ "add": add, "remove": remove }))
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<RepoAccessView>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// GET a one-time onboarding-bundle tarball and write it to `dest`. `path`
    /// is the portal-relative `bundle_url` from [`OnboardedOrg`] /
    /// [`RotatedCert`] (e.g. `/api/vendor-bundle/<anchor>/<token>.tar.gz`); the
    /// portal base is prepended. The URL is token-gated in its path (no Bearer
    /// auth, matching the playbook's `curl`); it is one-time and ~24 h-lived.
    /// Bundles are small (a cert tarball), so the body is buffered, not streamed.
    pub async fn download_bundle(&self, path: &str, dest: &std::path::Path) -> Result<(), PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}{}", inner.base_url, path);
        let resp = inner
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {
                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| PortalError::Transient(format!("read bundle body: {e}")))?;
                std::fs::write(dest, &bytes)
                    .map_err(|e| PortalError::Transient(format!("write {}: {e}", dest.display())))
            }
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(url = %url, status = %s, "download_bundle non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `DELETE /api/vendors/{vendor}` — admin: revoke a vendor.
    /// Portal may refuse with 409 if the vendor has non-retired
    /// topics (caller must retire them first); we surface the
    /// status verbatim.
    pub async fn delete_vendor(&self, prefix: &str) -> Result<(), PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = self.vurl(inner, &format!("/{}", prefix)).await;
        let resp = inner
            .http
            .delete(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::NO_CONTENT => Ok(()),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(prefix, url = %url, status = %s, "delete_vendor non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `POST /api/access/peers` — batch resolve NetBird peer names to the
    /// person who owns each (ADR-free; `peer-identity-resolution.md` Option B,
    /// live).
    ///
    /// Resolution parses the deterministic peer name minted at onboard
    /// (`peer-<handle>[-<tag>]-<site>-<date>`) and looks the handle up in the
    /// registry — deterministic, not fuzzy. Peers whose names do NOT follow
    /// that convention resolve to nothing, and that is the honest answer: the
    /// name carries no handle, so nothing can be inferred from it.
    ///
    /// Returns the raw per-name object so a caller can read `handle`,
    /// `deep_links.id_console_user` (which embeds the Ory identity id), or
    /// whatever the portal adds later, without this client gating the shape.
    pub async fn access_peers(
        &self,
        names: &[String],
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        if names.is_empty() {
            return Ok(Default::default());
        }
        let url = format!("{}/api/access/peers", inner.base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&serde_json::json!({ "netbird_names": names }))
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        if !resp.status().is_success() {
            let s = resp.status();
            warn!(url = %url, status = %s, "access_peers non-2xx");
            return Err(PortalError::http(s, &resp));
        }
        resp.json()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))
    }

    /// `PUT /api/slots/{anchor}/identity` — record the person→slot linkage
    /// (`slots.identity_id`).
    ///
    /// Called after the Ory leg of an onboard, NOT at slot allocation: the
    /// native onboard mints the slot BEFORE the Kratos identity exists, so
    /// there is no UUID to send at allocation time. That ordering is
    /// deliberate — a failed login leg rolls the slot back, which is what
    /// makes the onboard atomic — so the linkage is recorded in a second
    /// call rather than by reordering the legs.
    ///
    /// Without it, ownership of a namespace falls back to a name join
    /// (alias↔handle), the derivation ADR 1051 and the identity
    /// fragility audit added `identity_id` to retire.
    ///
    /// Note the path is `/api/slots/...`, NOT the lexicon-mapped vendor
    /// segment — slots are not renamed by the display lexicon.
    pub async fn stamp_slot_identity(
        &self,
        anchor: &str,
        identity_id: &str,
    ) -> Result<(), PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/slots/{}/identity", inner.base_url, anchor);
        let resp = inner
            .http
            .put(&url)
            .bearer_auth(&inner.api_key)
            .json(&serde_json::json!({ "identity_id": identity_id }))
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::NO_CONTENT => Ok(()),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(anchor, url = %url, status = %s, "stamp_slot_identity non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `POST /api/vendors/{vendor}/rotate` — admin: re-issue a peer's
    /// cert in place (the "rotate" sibling of revocation). `vendor` is
    /// the existing anchor (= vendor prefix); no slot is allocated and
    /// the anchor / alias / write namespace / ACL are unchanged — only
    /// the leaf cert + portal api_key are fresh. Returns the new bundle
    /// URL + api_key. `Err(NotFound)` on a 404 (rotation re-issues an
    /// existing cert; it does not create — onboard a new one instead).
    pub async fn rotate_vendor(&self, prefix: &str) -> Result<RotatedCert, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = self.vurl(inner, &format!("/{}/rotate", prefix)).await;
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => resp
                .json::<RotatedCert>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(prefix, url = %url, status = %s, "rotate_vendor non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `POST /api/orgs/{vendor}/rekey` — admin: reissue only the portal api_key
    /// of an existing peer (goat#1193). The lighter sibling of [`rotate_vendor`]:
    /// the mTLS cert, anchor, alias, write namespace, and ACL are untouched, so
    /// there is no bundle to download and no peer restart. Returns the new key;
    /// `Err(NotFound)` on a 404 (rekey reissues, it does not create).
    pub async fn rekey_vendor(&self, prefix: &str) -> Result<RekeyedKey, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        // Hardcoded `orgs`, NOT vurl: rekey is an org-only surface with no
        // /api/vendors twin, so the segment is not a choice. vurl resolves the
        // segment through `seg()`, which caches `vendors` when the probe hits a
        // transport error — that would POST a route the portal never registers
        // and surface the 404 as "portal has no peer" for a live peer.
        let url = format!("{}/api/orgs/{}/rekey", inner.base_url, prefix);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => resp
                .json::<RekeyedKey>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(prefix, url = %url, status = %s, "rekey_vendor non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `POST /api/orgs/{prefix}/sponsorship` — admin: grant or revoke an
    /// identity's sponsorship annotation (ADR 1071), the case-by-case lever
    /// for identities that didn't get it by tier default at onboarding.
    /// Granting requires a valid `max_tier` (server-validated).
    pub async fn set_sponsorship(
        &self,
        prefix: &str,
        authorized: bool,
        max_tier: &str,
        email: Option<&str>,
    ) -> Result<(), PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = self.vurl(inner, &format!("/{}/sponsorship", prefix)).await;
        // Optional Ory-login binding: lets the id-admin-console resolve this
        // sponsor's web session to their registry identity (sponsor-scoped
        // console access). Omitted = no console binding, CLI/API unaffected.
        let mut body = serde_json::json!({ "authorized": authorized, "max_tier": max_tier });
        if let Some(e) = email {
            body["email"] = serde_json::json!(e);
        }
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => Ok(()),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(prefix, url = %url, status = %s, "set_sponsorship non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/health` — lightweight liveness probe. Returns Ok on
    /// any 2xx. Used by `goat status` to render a single portal-
    /// liveness glyph; deliberately ignores the response body so it
    /// doesn't have to track portal-schema drift.
    pub async fn health(&self) -> Result<(), PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/health", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(PortalError::http(resp.status(), &resp))
        }
    }

    /// `POST /api/vendors/{vendor}/topics` — create a topic. Mirrors
    /// the Go reference at ops/goat-zenoh-portal/internal/pubsubstate/
    /// client.go CreateTopic. Returns the new topic's ID, or
    /// `Err(NotFound)` mapped to `Err(Http {status: 409})` for
    /// already-exists. (Caller can `list_topics` to fetch the existing
    /// ID in the 409 case.)
    pub async fn create_topic(
        &self,
        vendor: &str,
        req: NewTopic,
    ) -> Result<i64, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = self.vurl(inner, &format!("/{}/topics", vendor)).await;
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&req)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => {
                let topic: Topic = resp
                    .json()
                    .await
                    .map_err(|e| PortalError::Decode(e.to_string()))?;
                Ok(topic.id)
            }
            s => {
                warn!(vendor, url = %url, status = %s, "create_topic non-2xx");
                Err(non_2xx(resp).await)
            }
        }
    }

    /// `DELETE /api/vendors/{vendor}/topics/{id}` — retire a topic.
    /// Portal-side semantics decide soft-vs-hard retention (the goat-
    /// cli passes `?hard=true` for the operator's explicit purge
    /// request; the portal may still refuse based on audit-tail policy).
    pub async fn delete_topic(
        &self,
        vendor: &str,
        id: i64,
        hard: bool,
    ) -> Result<(), PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = self
            .vurl(
                inner,
                &format!(
                    "/{}/topics/{}{}",
                    vendor,
                    id,
                    if hard { "?hard=true" } else { "" }
                ),
            )
            .await;
        let resp = inner
            .http
            .delete(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::NO_CONTENT => Ok(()),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(vendor, id, url = %url, status = %s, "delete_topic non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/vendors/{vendor}/topics/export?format=<fmt>` → the
    /// vendor's topics serialized as a `bulkFile` (the same shape
    /// `import_topics` consumes). `format` is `yaml` or `json`; the
    /// portal defaults to yaml. Returns the raw body verbatim so the
    /// caller can stream it to a file without re-serializing (preserving
    /// the portal's field order + indentation). The interactive
    /// state-as-code on-ramp; the canonical declarative path stays
    /// `pubsub-apply` + `pubsubstate.yaml`.
    pub async fn export_topics(&self, vendor: &str, format: &str) -> Result<String, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = self
            .vurl(
                inner,
                &format!("/{}/topics/export?format={}", vendor, format),
            )
            .await;
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .text()
                .await
                .map_err(|e| PortalError::Transient(format!("read body: {e}"))),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(vendor, url = %url, status = %s, "export_topics non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// List authenticated portal references for metadata-only audit history.
    /// Fetch with `audit_metadata`; no client connection to MinIO is required.
    pub async fn audit_presign(
        &self,
        since: Option<&str>,
        key: Option<&str>,
    ) -> Result<AuditPresign, PortalError> {
        self.history_presign("audit", since, key, None).await
    }

    /// Payload history from the separately governed replay store. No fallback
    /// to a legacy server route: missing support is a rollout error.
    pub async fn replay_presign(
        &self,
        since: Option<&str>,
        key: Option<&str>,
    ) -> Result<AuditPresign, PortalError> {
        self.history_presign("replay", since, key, None).await
    }

    /// Paginated, metadata-only history. No raw S3 credentials are exposed.
    pub async fn audit_history_page(
        &self,
        since: &str,
        after: Option<&str>,
    ) -> Result<AuditPresign, PortalError> {
        self.history_presign("audit", Some(since), None, after)
            .await
    }

    pub async fn audit_metadata(
        &self,
        objects: &[PresignedObject],
    ) -> anyhow::Result<crate::audit_tail::PresignedFetch> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let base = url::Url::parse(&inner.base_url)?;
        anyhow::ensure!(
            base.scheme() == "https" || (cfg!(test) && base.host_str() == Some("127.0.0.1")),
            "metadata portal must use HTTPS"
        );
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let mut out = crate::audit_tail::PresignedFetch::default();
        let mut total_bytes = 0usize;
        let mut total_rows = 0usize;
        for object in objects {
            let result: anyhow::Result<Vec<crate::audit_event::AuditEvent>> = async {
                anyhow::ensure!(
                    matches!(
                        object.source.as_str(),
                        "legacy-projection" | "metadata-projection"
                    ),
                    "history server does not support metadata projection"
                );
                let url = base.join(&object.url)?;
                anyhow::ensure!(
                    url.origin() == base.origin()
                        && url.path() == "/api/audit/object"
                        && url.username().is_empty()
                        && url.password().is_none(),
                    "invalid projection origin or route"
                );
                let mut response = http
                    .get(url)
                    .bearer_auth(&inner.api_key)
                    .send()
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "portal request failed (no HTTP status): {}",
                            e.without_url()
                        )
                    })?;
                let status = response.status();
                if !status.is_success() {
                    let error = non_2xx(response).await;
                    anyhow::bail!(
                        "portal HTTP {status}: {}",
                        error.server_message().unwrap_or("audit metadata request refused")
                    );
                }
                let mut body = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|e| {
                    anyhow::anyhow!("portal body read failed: {}", e.without_url())
                })? {
                    total_bytes += chunk.len();
                    anyhow::ensure!(
                        total_bytes <= 64 * 1024 * 1024,
                        "metadata projection aggregate byte limit exceeded"
                    );
                    body.extend_from_slice(&chunk);
                }
                let mut events = Vec::new();
                for line in body.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
                    let raw: serde_json::Value = serde_json::from_slice(line)?;
                    anyhow::ensure!(
                        raw.get("payload_bytes_b64").is_none(),
                        "body field in metadata projection"
                    );
                    total_rows += 1;
                    anyhow::ensure!(
                        total_rows <= 100_000,
                        "metadata projection aggregate row limit exceeded"
                    );
                    events.push(serde_json::from_value(raw)?);
                }
                Ok(events)
            }
            .await;
            match result {
                Ok(events) => out.events.extend(events),
                Err(error) => out.skipped.push(crate::audit_tail::SkippedObject {
                    object: object.key.clone(),
                    reason: error.to_string(),
                }),
            }
        }
        Ok(out)
    }

    async fn history_presign(
        &self,
        store: &str,
        since: Option<&str>,
        key: Option<&str>,
        after: Option<&str>,
    ) -> Result<AuditPresign, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/{store}/presign", inner.base_url);
        let mut params: Vec<(&str, &str)> = vec![("projection", "1")];
        if let Some(after) = after {
            params.push(("after", after));
        }
        if let Some(s) = since {
            if !s.is_empty() {
                params.push(("since", s));
            }
        }
        if let Some(k) = key {
            if !k.is_empty() {
                params.push(("key", k));
            }
        }
        let base = url::Url::parse(&inner.base_url).map_err(|e| PortalError::Transient(e.to_string()))?;
        if base.scheme() != "https" && !(cfg!(test) && base.host_str() == Some("127.0.0.1")) {
            return Err(PortalError::Transient("history portal requires HTTPS".into()));
        }
        let http = Client::builder().timeout(std::time::Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none()).build()
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        let resp = http
            .get(&url)
            .query(&params)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<AuditPresign>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            _ => Err(non_2xx(resp).await),
        }
    }

    /// `POST /api/vendors/{vendor}/topics/import` — apply a `bulkFile`
    /// (the shape `export_topics` produces). `content_type` is
    /// `application/yaml` or `application/json` (drives the portal's
    /// parse). `mode` is `additive` (create/update only) or `replace`
    /// (also retire topics absent from the file). `dry_run` previews
    /// the diff without applying. The portal returns the same
    /// `ImportDiff` for both the preview and the applied run.
    pub async fn import_topics(
        &self,
        vendor: &str,
        body: String,
        content_type: &str,
        mode: &str,
        dry_run: bool,
    ) -> Result<ImportDiff, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = self
            .vurl(
                inner,
                &format!(
                    "/{}/topics/import?mode={}&dry_run={}",
                    vendor, mode, dry_run
                ),
            )
            .await;
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .header("Content-Type", content_type.to_string())
            .body(body)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<ImportDiff>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(vendor, url = %url, status = %s, "import_topics non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/vendors/{vendor}/topics/{id}/schemas` → all schema
    /// rows for the topic. Empty vec means "no schema body uploaded
    /// yet" (the row is parented to a vendor + topic but topic_schemas
    /// has no entries); `Err(NotFound)` means the topic itself doesn't
    /// exist.
    pub async fn list_schemas(
        &self,
        vendor: &str,
        topic_id: i64,
    ) -> Result<Vec<SchemaMeta>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = self
            .vurl(inner, &format!("/{}/topics/{}/schemas", vendor, topic_id))
            .await;
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {
                let env: SchemasEnvelope = resp
                    .json()
                    .await
                    .map_err(|e| PortalError::Decode(e.to_string()))?;
                Ok(env.schemas)
            }
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `POST /api/vendors/{vendor}/topics/{id}/schemas` — upload a schema
    /// body as a multipart form. `req.format` is `protobuf-fds` (a
    /// serialized FileDescriptorSet) or `protobuf-source` (raw `.proto`
    /// bytes the portal compiles in-process on ingress, F-158 — no
    /// client-side protoc). `req.version` must match `^v\d+$`;
    /// `req.compatibility` is one of backward | forward | none. Returns
    /// the created schema row. The portal's 409 "version already exists"
    /// surfaces as `Err(Http { status: 409 })`; an unknown vendor/topic
    /// as `Err(NotFound)`.
    pub async fn upload_schema(
        &self,
        vendor: &str,
        topic_id: i64,
        req: NewSchema,
    ) -> Result<SchemaMeta, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = self
            .vurl(inner, &format!("/{}/topics/{}/schemas", vendor, topic_id))
            .await;
        let part = reqwest::multipart::Part::bytes(req.bytes).file_name(req.file_name);
        let form = reqwest::multipart::Form::new()
            .text("version", req.version)
            .text("compatibility", req.compatibility)
            .text("format", req.format)
            .part("file", part);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => resp
                .json()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(vendor, topic_id, url = %url, status = %s, "upload_schema non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/schemas/render/{key}[?version=v2]` → the structured
    /// message + enum tree for a topic's schema body. `version = None`
    /// selects the latest. Used by `goat schema show` (render one) and
    /// `goat schema diff` (render two and compare). `Err(NotFound)` when
    /// the topic exists but has no schema body uploaded (or no such
    /// version).
    pub async fn render_schema(
        &self,
        key_pattern: &str,
        version: Option<&str>,
    ) -> Result<RenderResponse, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let mut url = format!("{}/api/schemas/render/{}", inner.base_url, key_pattern);
        if let Some(v) = version {
            url.push_str("?version=");
            url.push_str(v);
        }
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(key = %key_pattern, url = %url, status = %s, "render_schema non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/schemas/<key_pattern>` → raw FileDescriptorSet bytes
    /// for the latest schema version of the topic identified by
    /// `key_pattern`. Preserves panoscope's existing FetchOutcome
    /// shape verbatim so the migration is a one-line import swap.
    pub async fn fetch_fds(&self, key_pattern: &str) -> FetchOutcome {
        let Some(inner) = &self.inner else {
            return FetchOutcome::Disabled;
        };
        let url = format!("{}/api/schemas/{}", inner.base_url, key_pattern);
        match inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                if status == StatusCode::OK {
                    match resp.bytes().await {
                        Ok(b) => FetchOutcome::Ready(b.to_vec()),
                        Err(e) => FetchOutcome::Transient(format!("body read: {e}")),
                    }
                } else if status == StatusCode::NOT_FOUND {
                    FetchOutcome::NoSchema
                } else {
                    warn!(key = %key_pattern, url = %url, status = %status, "fetch_fds non-2xx");
                    FetchOutcome::Transient(format!("portal returned {status}"))
                }
            }
            Err(e) => {
                warn!(key = %key_pattern, error = %e, "fetch_fds network error");
                FetchOutcome::Transient(e.to_string())
            }
        }
    }

    /// `GET /api/topics/decoded?key=<key>&n=<n>` → up to `n` recent
    /// samples, each decoded server-side against the topic's declared
    /// `format`. The portal sets `decoded` on success and `error` on a
    /// per-sample decode failure. Used by the stethoscope decode-rate
    /// check: schema-body only proves the descriptor parses and the
    /// declared name exists — it never proves a single wire byte
    /// conforms to that message. This endpoint closes that gap by
    /// surfacing the actual decode success ratio on live samples.
    pub async fn decoded_samples(
        &self,
        key_pattern: &str,
        n: usize,
    ) -> Result<Vec<DecodedSample>, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!(
            "{}/api/topics/decoded?key={}&n={}",
            inner.base_url, key_pattern, n
        );
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {
                let env: DecodedEnvelope = resp
                    .json()
                    .await
                    .map_err(|e| PortalError::Decode(e.to_string()))?;
                Ok(env.samples)
            }
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(key = %key_pattern, url = %url, status = %s, "decoded_samples non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/access/who?id=<identity>` — Data-Access COP forward query
    /// (what this identity may write/read + the reason at each layer). The
    /// portal is self-or-operator scoped: a vendor token may query only its
    /// own prefix, else `Err(Http { status: 403 })`.
    pub async fn access_who(&self, id: &str) -> Result<AccessWho, PortalError> {
        self.access_get("who", &[("id", id)]).await
    }

    /// `GET /api/access/topic?key=<key>` — reverse query (who may write/read
    /// this topic + who receives it if released). Operator-only server-side.
    pub async fn access_topic(&self, key: &str) -> Result<AccessTopic, PortalError> {
        self.access_get("topic", &[("key", key)]).await
    }

    /// `GET /api/access/roster` — the BULK forward query: a slim cross-layer
    /// summary (W/R counts + reconcile state + on-mesh / last-activity) for
    /// every identity, assembled once. The responsiveness path for `goat who`:
    /// one call seeds the whole roster instead of one `who` round-trip per
    /// selected row over the lossy mesh. Operator-only server-side.
    pub async fn access_roster(&self) -> Result<AccessRoster, PortalError> {
        self.access_get("roster", &[]).await
    }

    /// `GET /api/access/why?id=<identity>&key=<key>` — single allow/deny
    /// trace across mesh → identity → Zenoh ACL → release. Self-or-operator
    /// scoped on `id`.
    pub async fn access_why(&self, id: &str, key: &str) -> Result<AccessWhy, PortalError> {
        self.access_get("why", &[("id", id), ("key", key)]).await
    }

    /// Shared GET+decode for the `/api/access/*` read surface.
    /// `GET /api/whoami` — resolve the presented key's OWN principal
    /// (org prefix / alias for a vendor key; actor handle for a per-person
    /// operator key). The CLI's `me` selector resolves through this.
    pub async fn whoami(&self) -> Result<Whoami, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/whoami", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    async fn access_get<T: for<'de> Deserialize<'de>>(
        &self,
        sub: &str,
        params: &[(&str, &str)],
    ) -> Result<T, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/{}", inner.base_url, sub);
        let resp = inner
            .http
            .get(&url)
            .query(params)
            .bearer_auth(&inner.api_key)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => {
                warn!(sub, url = %url, status = %s, "access_get non-2xx");
                Err(PortalError::http(s, &resp))
            }
        }
    }

    /// `GET /api/access/probation` — the anomaly-probation list (ADR 1068).
    /// `all=false` returns just the open (undecided) entries; `all=true` the
    /// full history. Operator-only server-side.
    pub async fn probation_list(&self, all: bool) -> Result<Vec<Probation>, PortalError> {
        let params: &[(&str, &str)] = if all { &[("all", "true")] } else { &[] };
        let resp: ProbationList = self.access_get("probation", params).await?;
        Ok(resp.probation)
    }

    /// `POST /api/access/probation` — hold an entity on probation. `409` maps to
    /// `PortalError::Http` (already held). Operator-only.
    pub async fn probation_hold(&self, req: &ProbationHold) -> Result<Probation, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/probation", inner.base_url);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(req)
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => resp
                .json::<Probation>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            s => Err(PortalError::http(s, &resp)),
        }
    }

    /// `POST /api/access/probation/{id}/decide` — record a terminal disposition
    /// (`repaired` | `purged`) + note. Returns the recorded decision + the
    /// suggested follow-up command. Operator-only.
    pub async fn probation_decide(
        &self,
        id: &str,
        disposition: &str,
        note: &str,
    ) -> Result<ProbationDecided, PortalError> {
        let inner = self.inner.as_ref().ok_or(PortalError::Disabled)?;
        let url = format!("{}/api/access/probation/{}/decide", inner.base_url, id);
        let resp = inner
            .http
            .post(&url)
            .bearer_auth(&inner.api_key)
            .json(&serde_json::json!({ "disposition": disposition, "note": note }))
            .send()
            .await
            .map_err(|e| PortalError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<ProbationDecided>()
                .await
                .map_err(|e| PortalError::Decode(e.to_string())),
            StatusCode::NOT_FOUND => Err(PortalError::not_found(&resp)),
            s => Err(PortalError::http(s, &resp)),
        }
    }
}

/// One anomaly-probation entry (ADR 1068). Mirrors the portal's
/// `registry.Probation` (internal/registry/probation.go).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Probation {
    #[serde(default)]
    pub id: String,
    /// The held entity's stable reference. `ref` is a Rust keyword, so the
    /// field is `ref_` mapped to the wire `ref`.
    #[serde(default, rename = "ref")]
    pub ref_: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub held_at: i64,
    #[serde(default)]
    pub held_by: String,
    #[serde(default)]
    pub cure_by: i64,
    #[serde(default)]
    pub disposition: String,
    #[serde(default)]
    pub decided_by: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub expired: bool,
}

/// `POST /api/access/probation` body — hold an entity.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ProbationHold {
    #[serde(rename = "ref")]
    pub ref_: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub cure_by: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

/// `GET /api/access/probation` envelope.
#[derive(Debug, Clone, Default, Deserialize)]
struct ProbationList {
    #[serde(default)]
    probation: Vec<Probation>,
}

/// `POST /api/access/probation/{id}/decide` result.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProbationDecided {
    #[serde(default)]
    pub disposition: String,
    #[serde(default)]
    pub next: String,
}

/// One write/read permission with its deciding layer + record. Mirrors the
/// portal's `access.Access` (internal/access/query.go).
#[derive(Debug, Clone, Deserialize)]
pub struct AccessEntry {
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub layer: String,
    #[serde(default)]
    pub reason: String,
}

/// `GET /api/access/who` result — the forward query.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccessWho {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub resolved: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub state: String,
    /// Identity-grammar affiliation (ADR 1043): org kind. Empty = none declared.
    #[serde(default)]
    pub affiliation: String,
    /// Org nation (ISO code). Empty = none declared.
    #[serde(default)]
    pub nation: String,
    /// Why a stray didn't reconcile (untracked / unregistered-key /
    /// no-declared-member). Empty for non-strays.
    #[serde(default)]
    pub recon_reason: String,
    #[serde(default)]
    pub on_mesh: bool,
    /// Where a person's on-mesh answer came from: `peer` (their own enrolled
    /// device, matched by the peer-name custody link) / `org` (org-spine
    /// roll-up — describes the org, not the person) / `none` (nothing
    /// attributable — render "unattributed", never a confident "no").
    /// Empty for non-person identities and pre-field portals.
    #[serde(default)]
    pub mesh_attribution: String,
    /// HOW this person's write anchor was attributed to them: `recorded`
    /// (slots.identity_id, stamped at onboard — ground truth) or `tag-join`
    /// (the slot alias equals the identity's org short-tag — a NAME join, and
    /// coarse: org-mates sharing a tag share the anchor). Empty on a
    /// pre-field portal or when no anchor resolved.
    ///
    /// A consumer that renders an anchor without this distinction is telling
    /// an operator that ownership is a fact when it may be a guess — the
    /// indistinguishability ADR 1051 and the fragility audit added
    /// the recorded linkage to retire.
    #[serde(default)]
    pub anchor_linkage: String,
    #[serde(default)]
    pub writes: Vec<AccessEntry>,
    #[serde(default)]
    pub reads: Vec<AccessEntry>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// One identity's slim cross-layer summary from `GET /api/access/roster`.
/// Mirrors the portal's `access.RosterRow` (internal/access/query.go): the
/// W/R counts + reconcile state + on-mesh / last-activity for the bulk roster,
/// so the CLI renders the whole fleet's permissions from one call.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccessRosterEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub state: String,
    /// Identity-grammar affiliation (ADR 1043): org kind `vendor`/`organic`/
    /// `partner`. Empty = none declared (rendered `·`, never guessed).
    #[serde(default)]
    pub affiliation: String,
    /// Org nation (ISO code). Empty = none declared.
    #[serde(default)]
    pub nation: String,
    /// Why a stray (state=out_of_band) didn't reconcile: `untracked` /
    /// `unregistered-key` / `no-declared-member`. Empty for non-strays. The
    /// safe-sweep signal — `unregistered-key` is the "don't blind-cut" class.
    #[serde(default)]
    pub recon_reason: String,
    #[serde(default)]
    pub on_mesh: bool,
    /// See [`AccessWho::mesh_attribution`]: `peer` / `org` / `none` (persons only).
    #[serde(default)]
    pub mesh_attribution: String,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub last_seen_unix: i64,
    #[serde(default)]
    pub writes: usize,
    #[serde(default)]
    pub reads: usize,
    #[serde(default)]
    pub write_anchor: String,
    /// Furthest onboarding step with positive evidence behind it, keyed off
    /// this row's write anchor (the portal's `/api/progress` `stage`
    /// vocabulary: `registered` → `bundle-minted` → `bundle-downloaded` →
    /// `mesh-enrolled` → `topics-registered` → `hello-world-published` →
    /// `publishing`).
    ///
    /// Empty means the portal found no progress row for the anchor, OR the
    /// portal predates the field — render that as UNKNOWN, never as "did not
    /// start". The COP carries it so "who is here" and "who never finished
    /// arriving" are one question rather than two surfaces.
    #[serde(default)]
    pub onboarding_stage: String,
}

/// `GET /api/access/roster` result — the bulk forward query over every
/// identity-bearing spine node, one summary row each.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccessRoster {
    #[serde(default)]
    pub rows: Vec<AccessRosterEntry>,
    #[serde(default)]
    pub notes: Vec<String>,
    /// Declared-registry provenance (roster.json's `generated_at` +
    /// `source_digest`), so the CLI can report roster age and detect a portal
    /// serving a stale copy. `None` when the portal predates the stamp.
    #[serde(default)]
    pub freshness: Option<RosterFreshness>,
}

/// Provenance of the declared registry the portal reconciled against. Mirrors
/// the portal's `access.RosterFreshness` (internal/access/query.go).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RosterFreshness {
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub source_digest: String,
    #[serde(default)]
    pub member_count: i64,
    #[serde(default)]
    pub loaded_unix: i64,
}

/// `GET /api/whoami` result — the caller's own resolved principal.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Whoami {
    #[serde(default)]
    pub operator: bool,
    /// Person handle for a per-person operator key; empty for
    /// vendor/org principals.
    #[serde(default)]
    pub actor: String,
    /// Org prefix for a vendor/org principal; empty for operators.
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub alias: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccessWriter {
    #[serde(default)]
    pub identity: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub layer: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccessReader {
    #[serde(default)]
    pub identity: String,
    #[serde(default)]
    pub layer: String,
    #[serde(default)]
    pub reason: String,
}

/// `GET /api/access/topic` result — the reverse query.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccessTopic {
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub found: bool,
    #[serde(default)]
    pub writers: Vec<AccessWriter>,
    #[serde(default)]
    pub readers: Vec<AccessReader>,
    #[serde(default)]
    pub receivers: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// One layer's verdict in a why-trace.
#[derive(Debug, Clone, Deserialize)]
pub struct AccessStep {
    #[serde(default)]
    pub layer: String,
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub reason: String,
}

/// `GET /api/access/why` result — the single allow/deny trace.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccessWhy {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub resolved: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub write_verdict: String,
    #[serde(default)]
    pub read_verdict: String,
    #[serde(default)]
    pub steps: Vec<AccessStep>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// One sample from `/api/topics/decoded`. Exactly one of `decoded` /
/// `error` is meaningful per sample: `decoded` is the server-side
/// protobuf→JSON result (present on success), `error` is the per-sample
/// decode failure string (e.g. `invalid wire type: …`). Both absent
/// means the sample carried no payload.
#[derive(Debug, Clone, Deserialize)]
pub struct DecodedSample {
    #[serde(default)]
    pub decoded: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
}

impl DecodedSample {
    /// True when the portal decoded this sample's bytes against the
    /// declared format without error.
    pub fn is_decoded(&self) -> bool {
        self.decoded.is_some() && self.error.is_none()
    }
}

#[derive(Debug, Deserialize)]
struct DecodedEnvelope {
    #[serde(default)]
    samples: Vec<DecodedSample>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #1309 — reqwest was built with `default-features = false` and no
    /// `gzip` feature, so the client never sent `Accept-Encoding: gzip`
    /// and a gzip-compressing portal response would have been handed to
    /// `resp.json()` as raw, undecoded bytes (a decode error, not silent
    /// corruption, but still a request that should have worked). This
    /// proves both halves: the request carries the header, and a
    /// gzip-encoded response decodes transparently.
    #[tokio::test]
    async fn whoami_requests_gzip_and_decodes_a_gzip_response() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buf = [0; 1024];
                let n = socket.read(&mut buf).await.unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buf[..n]);
                assert!(bytes.len() < 16384);
                if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(bytes).unwrap();

            let body = br#"{"operator":true,"actor":"fixture","vendor":"","display_name":"Fixture Operator","alias":"fixture"}"#;
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(body).unwrap();
            let gz = encoder.finish().unwrap();

            let mut response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                gz.len()
            )
            .into_bytes();
            response.extend_from_slice(&gz);
            socket.write_all(&response).await.unwrap();
            request
        });

        let client = PortalClient::new(format!("http://{addr}"), "fixture-key".into());
        let who = tokio::time::timeout(std::time::Duration::from_secs(5), client.whoami())
            .await
            .unwrap()
            .expect("gzip-encoded whoami response decodes");

        let request = tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        let headers = request.to_ascii_lowercase();
        assert!(
            headers
                .lines()
                .any(|l| l.starts_with("accept-encoding:") && l.contains("gzip")),
            "request did not carry an Accept-Encoding header with gzip:\n{request}"
        );
        assert_eq!(who.actor, "fixture");
        assert_eq!(who.display_name, "Fixture Operator");
    }

    #[tokio::test]
    async fn proposal_email_query_is_encoded_and_uses_onboard_endpoint() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (status, email) in [
            ("archived", Some("person+tag&other=#@example.test")),
            ("", Some("person+tag@example.test")),
            ("sponsor_approved", None),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buf = [0; 1024];
                    let n = socket.read(&mut buf).await.unwrap();
                    assert_ne!(n, 0);
                    bytes.extend_from_slice(&buf[..n]);
                    assert!(bytes.len() < 16384);
                    if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]").await.unwrap();
                String::from_utf8(bytes).unwrap()
            });
            let client = PortalClient::new("http://127.0.0.1:1".into(), "fixture-key".into())
                .with_onboard_endpoint(&endpoint);
            let rows = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                if email.is_some() {
                    client.list_proposals_filtered(status, email).await
                } else {
                    client.list_proposals(status).await
                }
            })
            .await
            .unwrap()
            .unwrap();
            assert!(rows.is_empty());
            let request = tokio::time::timeout(std::time::Duration::from_secs(5), server)
                .await
                .unwrap()
                .unwrap();
            let target = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let url = reqwest::Url::parse(&format!("http://fixture{target}")).unwrap();
            assert_eq!(url.path(), "/api/onboard/proposals");
            let pairs: Vec<_> = url.query_pairs().into_owned().collect();
            let mut expected = Vec::new();
            if !status.is_empty() {
                expected.push(("status".into(), status.into()));
            }
            if let Some(email) = email {
                expected.push(("email".into(), email.into()));
            }
            assert_eq!(pairs, expected);
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture-key"));
        }
    }

    #[test]
    fn salvage_message_shapes() {
        assert_eq!(
            salvage_message(r#"{"error":{"message":"no mesh mapping for kid"}}"#).as_deref(),
            Some("no mesh mapping for kid")
        );
        assert_eq!(
            salvage_message(r#"{"error":"quota exhausted"}"#).as_deref(),
            Some("quota exhausted")
        );
        assert_eq!(
            salvage_message(r#"{"message":"bad handle"}"#).as_deref(),
            Some("bad handle")
        );
        assert_eq!(
            salvage_message("plain refusal text").as_deref(),
            Some("plain refusal text")
        );
        // The portal's own refusals are RFC 7807 problem JSON; `detail` is the
        // line a person reads (found by the e2e goat suite: the portal named the
        // missing org and goat printed only the status).
        assert_eq!(
            salvage_message(r#"{"type":"urn:goat:portal:problem:notfound","title":"Not Found","status":404,"detail":"no org with prefix \"nope\""}"#).as_deref(),
            Some(r#"no org with prefix "nope""#)
        );
        // HTML, whitespace, and message-less JSON salvage nothing — the
        // caller falls back to the bare status rather than dumping noise.
        assert_eq!(salvage_message("<html><body>502</body></html>"), None);
        assert_eq!(salvage_message("   "), None);
        assert_eq!(salvage_message(r#"{"code":42}"#), None);
    }

    #[test]
    fn salvage_message_sanitizes_for_terminal() {
        // Control characters (incl. the ANSI escape byte) collapse to spaces.
        let m = salvage_message("bad\u{1b}[31mred\u{1b}[0m\nline").unwrap();
        assert!(!m.chars().any(|c| c.is_control()), "{m:?}");
        // An echoed bearer credential is redacted before terminal display.
        let m = salvage_message("auth Bearer vp_supersecret rejected").unwrap();
        assert!(!m.contains("vp_supersecret"), "{m}");
        assert!(m.contains("[redacted]"), "{m}");
        // JSON-derived messages are capped like every other path.
        let long = format!(r#"{{"message":"{}"}}"#, "x".repeat(1000));
        let m = salvage_message(&long).unwrap();
        assert!(m.chars().count() <= 401, "len {}", m.chars().count());
    }

    /// Format characters are not control characters, but a bidi override
    /// reorders what the operator reads, a zero-width joiner hides a split,
    /// and a tag character carries invisible text. None may reach the
    /// terminal; the visible text around them is kept.
    #[test]
    fn salvage_message_strips_format_characters() {
        let body = "grant \u{202e}desufer\u{202c} for op\u{200d}erator \u{feff}x\u{e0041}\u{ad}y";
        let m = salvage_message(body).unwrap();
        for bad in [
            '\u{202e}',
            '\u{202c}',
            '\u{200d}',
            '\u{feff}',
            '\u{e0041}',
            '\u{ad}',
        ] {
            assert!(!m.contains(bad), "{m:?} still carries U+{:04X}", bad as u32);
        }
        assert!(
            m.contains("grant") && m.contains("desufer") && m.contains("erator"),
            "{m:?}"
        );

        // Letters, marks, spacing, and symbols are untouched: the predicate
        // is the format category, not "anything unusual".
        let kept = "Forțele Aeriene Române\u{0301} café — 100 % ok";
        assert_eq!(salvage_message(kept).as_deref(), Some(kept));

        // The predicate is the category, not a guess at what looks odd.
        for (c, want) in [
            ('\u{202e}', true),
            ('\u{e0041}', true),
            ('\u{fff9}', true),
            ('\u{ad}', true),
            ('A', false),
            ('\u{a0}', false),
            ('\u{301}', false),
            ('\u{fffc}', false),
        ] {
            assert_eq!(is_format(c), want, "U+{:04X}", c as u32);
        }
    }

    /// The error body is read up to the cap and no further: a server or a
    /// proxy that answers a refusal with a huge body cannot make the client
    /// buffer all of it for a message that keeps 400 characters. A short
    /// body arrives whole, and the capped read still salvages the message.
    #[tokio::test]
    async fn read_capped_stops_at_the_cap() {
        let big = "y".repeat(3 * 1024);
        let resp = reqwest::Response::from(
            http::Response::builder()
                .status(500)
                .body(big.clone())
                .unwrap(),
        );
        assert_eq!(
            read_capped(resp, 1024).await.as_deref(),
            Some(&big.as_bytes()[..1024])
        );

        let resp = reqwest::Response::from(
            http::Response::builder()
                .status(500)
                .body("short".to_string())
                .unwrap(),
        );
        assert_eq!(
            read_capped(resp, 1024).await.as_deref(),
            Some(b"short".as_slice())
        );

        let resp =
            reqwest::Response::from(http::Response::builder().status(500).body(big).unwrap());
        let err = non_2xx(resp).await;
        assert_eq!(
            err.http_status(),
            Some(reqwest::StatusCode::INTERNAL_SERVER_ERROR)
        );
        assert!(err
            .server_message()
            .is_some_and(|m| m.chars().count() <= 401));
    }

    /// Every separator an echoed `Authorization` header realistically uses,
    /// not just the ASCII space. Each of these leaked the credential before.
    #[test]
    fn redact_bearer_covers_the_separators_headers_actually_use() {
        for input in [
            "auth Bearer vp_supersecret rejected",
            "auth bearer vp_supersecret rejected",
            "auth Bearer=vp_supersecret rejected",
            "auth Bearer:vp_supersecret rejected",
            "auth Bearer\u{00a0}vp_supersecret rejected",
            "auth Bearer  vp_supersecret rejected",
            "Bearer vp_supersecret",
        ] {
            let out = redact_bearer(input);
            assert!(!out.contains("vp_supersecret"), "{input:?} -> {out:?}");
            assert!(out.contains("[redacted]"), "{input:?} -> {out:?}");
        }

        // Text without a credential is returned unchanged — including the
        // shapes that only look like the keyword.
        for input in [
            "no credential here",
            "nonbearer vp_notatoken",
            "Bearertoken vp_notatoken",
            "trailing Bearer",
        ] {
            assert_eq!(redact_bearer(input), input, "{input:?}");
        }

        // Two echoed headers in one body: both go.
        let out = redact_bearer("Bearer aaa then Bearer bbb");
        assert_eq!(out, "Bearer [redacted] then Bearer [redacted]");
    }

    #[test]
    fn salvage_message_takes_a_json_string_root() {
        assert_eq!(
            salvage_message(r#""quota exceeded""#).as_deref(),
            Some("quota exceeded")
        );
        // Same sanitisation as every other shape: an empty or blank string
        // root salvages nothing, and a credential inside one is redacted.
        assert_eq!(salvage_message("\"\""), None);
        assert_eq!(salvage_message("\"   \""), None);
        assert_eq!(
            salvage_message(r#""bad key vp_abc123""#).as_deref(),
            Some("bad key [redacted]")
        );
    }

    /// An echoed key is redacted even with no `Bearer` keyword in front of
    /// it: a validation error that quotes the value, or a reflected query
    /// parameter, must not print the credential.
    #[test]
    fn redact_api_keys_covers_echoed_operator_and_vendor_keys() {
        for (input, want) in [
            ("key op_abc123 rejected", "key [redacted] rejected"),
            ("key vp_abc-123_x rejected", "key [redacted] rejected"),
            ("api_key=op_abc123", "api_key=[redacted]"),
            ("(vp_abc123)", "([redacted])"),
            ("op_abc123.", "[redacted]."),
            ("op_a vp_b", "[redacted] [redacted]"),
        ] {
            assert_eq!(redact_api_keys(input), want, "{input:?}");
        }
        // Shapes that only look like a key are left alone.
        for input in ["stop_x", "op_", "vp_ trailing", "OP_UPPER", "loop_vp_x"] {
            assert_eq!(redact_api_keys(input), input, "{input:?}");
        }
        // The salvage chain runs it after the bearer pass.
        let m = salvage_message(r#"{"error":"unknown key op_abc123"}"#).unwrap();
        assert_eq!(m, "unknown key [redacted]");
    }

    #[test]
    fn portal_error_status_and_message_accessors() {
        let bare = PortalError::status(reqwest::StatusCode::FORBIDDEN);
        let body = PortalError::HttpWithBody {
            status: reqwest::StatusCode::FORBIDDEN,
            code: Some("self_only".into()),
            message: "custodian mismatch".into(),
            path: "/api/machines/x".into(),
            retry_after: None,
        };
        assert_eq!(bare.http_status(), body.http_status());
        assert_eq!(bare.server_message(), None);
        assert_eq!(body.server_message(), Some("custodian mismatch"));
        assert_eq!(bare.error_code(), None);
        assert_eq!(body.error_code(), Some("self_only"));
        assert_eq!(PortalError::Transient("dns".into()).http_status(), None);
    }

    #[test]
    fn salvage_code_reads_the_onboard_envelope_code() {
        let env = r#"{"error":{"code":"operator_only","message":"no"}}"#;
        assert_eq!(salvage_code(env).as_deref(), Some("operator_only"));
        // No envelope, or a code that is not a plain identifier: nothing to match on.
        assert_eq!(salvage_code(r#"{"error":"no"}"#), None);
        assert_eq!(salvage_code("plain text"), None);
        assert_eq!(salvage_code(r#"{"error":{"code":"Self Only","message":"x"}}"#), None);
        assert_eq!(salvage_code(r#"{"error":{"code":"","message":"x"}}"#), None);
        assert_eq!(salvage_code(r#"{"error":{"code":"a[31m","message":"x"}}"#), None);
    }

    // Regression for the `goat schema show`/`diff` decode failure that
    // surfaced as `E_PORTAL_UNREACHABLE` ("portal transient: decode: ..."):
    // the live `GET /api/schemas/render/{key}` (HTTP 200) emits
    // `proto_source` as a per-file `map[string]string` and `enums` as
    // `null` when a schema has no enums — but `RenderResponse.proto_source`
    // was typed `String` (map is not a string) and the collection fields
    // rejected a present `null` (serde `default` only fills *absent* keys).
    // This payload is a trimmed copy of a real response for a
    // namespaced topic. Both fields must decode without error.
    #[test]
    fn govern_admins_result_decodes_bootstrap_and_nominated() {
        let boot: GovernAdminsResult = serde_json::from_str(
            r#"{"org":"acme","mode":"bootstrap (every registry member of the org governs — ADR 1102 bootstrap rule)","admins":[]}"#,
        ).expect("bootstrap decodes");
        assert_eq!(boot.org, "acme");
        assert!(boot.mode.starts_with("bootstrap"));
        assert!(boot.admins.is_empty());
        let nom: GovernAdminsResult = serde_json::from_str(
            r#"{"org":"acme","mode":"nominated (narrowed: only the active admins listed govern)","admins":[
                {"id":"gadm-abc","org":"acme","identity_id":"uuid-1","nominated_by":"operator","justification":"handover","status":"active"}
            ]}"#,
        ).expect("nominated decodes");
        assert_eq!(nom.admins.len(), 1);
        assert_eq!(nom.admins[0].id, "gadm-abc");
        assert_eq!(nom.admins[0].identity_id, "uuid-1");
        assert_eq!(nom.admins[0].status, "active");
    }

    #[test]
    fn govern_admin_row_tolerates_absent_optional_fields() {
        let row: GovernAdminRow = serde_json::from_str(
            r#"{"id":"gadm-9","org":"acme","identity_id":"uuid-2","status":"active"}"#,
        ).expect("minimal row decodes");
        assert_eq!(row.nominated_by, "");
        assert_eq!(row.justification, "");
    }

    #[test]
    fn render_response_decodes_map_proto_source_and_null_enums() {
        let body = r#"{
            "key": "f52455d54703f0207090a335861e1d0a/tak-test/v1",
            "vendor": "f52455d54703f0207090a335861e1d0a",
            "format": "protobuf:TakTest",
            "primary_message": "TakTest",
            "proto_source": {"taktest.proto": "syntax = \"proto3\";\n\nmessage TakTest {\n  string uid = 1;\n}\n"},
            "rendered": {
                "files": [{"name": "taktest.proto", "package": "toorbo.taktest.v1", "syntax": "proto3"}],
                "messages": [{"full_name": "toorbo.taktest.v1.TakTest", "name": "TakTest", "fields": [
                    {"name": "uid", "number": 1, "cardinality": "singular", "type": "string"}
                ]}],
                "enums": null
            },
            "schema": {"version": "v1", "compatibility": "none", "all_versions": ["v1"], "size_bytes": 637, "format_storage": "protobuf-fds"}
        }"#;
        let resp: RenderResponse = serde_json::from_str(body).expect("live render payload must decode");
        assert_eq!(resp.primary_message, "TakTest");
        assert_eq!(
            resp.proto_source.get("taktest.proto").map(|s| s.starts_with("syntax")),
            Some(true)
        );
        assert_eq!(resp.rendered.messages.len(), 1);
        assert!(resp.rendered.enums.is_empty()); // null → default, not an error
        assert_eq!(resp.schema.all_versions, vec!["v1"]);
    }

    // The nil-map / no-schema-body shape: proto_source and the rendered
    // collections all arrive as `null`. Must still decode to empty rather
    // than erroring.
    #[test]
    fn render_response_tolerates_all_null_collections() {
        let body = r#"{
            "key": "acme/topic/v1",
            "proto_source": null,
            "rendered": {"files": null, "messages": null, "enums": null},
            "schema": {"version": "v1", "all_versions": null}
        }"#;
        let resp: RenderResponse = serde_json::from_str(body).expect("null collections must decode");
        assert!(resp.proto_source.is_empty());
        assert!(resp.rendered.messages.is_empty());
        assert!(resp.schema.all_versions.is_empty());
    }

    // Onboarding-extraction cutover: the onboard surface (`/api/onboard/*`) can
    // target a distinct origin (goat-id-console) while the portal `base_url`
    // keeps the cert-mint/whoami/access surfaces. Default = same as base_url
    // (back-compat); with_onboard_endpoint re-points only the onboard surface.
    #[test]
    fn onboard_endpoint_defaults_to_base_then_repoints() {
        let portal = "https://portal.zagros.netbird.zagros.example";
        let console = "https://onboard.zagros.netbird.zagros.example";

        // No override: onboard falls back to the portal base (pre-cutover).
        let c = PortalClient::new(portal.to_string(), "k".to_string());
        assert_eq!(c.base_url(), Some(portal));
        assert_eq!(c.onboard_base_url(), Some(portal));
        assert!(c.onboard_endpoint_is_fallback());

        // Re-pointed: onboard surface moves, portal base is untouched. Trailing
        // slash is trimmed to match the base_url normalization.
        let c = PortalClient::new(portal.to_string(), "k".to_string())
            .with_onboard_endpoint(&format!("{console}/"));
        assert_eq!(c.base_url(), Some(portal));
        assert_eq!(c.onboard_base_url(), Some(console));
        assert!(!c.onboard_endpoint_is_fallback());

        // Empty override is a no-op (keeps the base fallback).
        let c = PortalClient::new(portal.to_string(), "k".to_string()).with_onboard_endpoint("  ");
        assert_eq!(c.onboard_base_url(), Some(portal));
        assert!(c.onboard_endpoint_is_fallback());

        // Disabled client (empty key) stays disabled through the builder.
        let c = PortalClient::new(portal.to_string(), String::new()).with_onboard_endpoint(console);
        assert!(!c.is_enabled());
        assert_eq!(c.onboard_base_url(), None);
        assert!(!c.onboard_endpoint_is_fallback());
    }

    // ADR 1052 Phase 2: the listing dual-read prefers `orgs`, falls back to the
    // deprecated `vendors`, and tolerates the migrated portal emitting BOTH
    // (identical rows) without a duplicate-field error.
    #[test]
    fn listing_envelope_prefers_orgs_falls_back_to_vendors() {
        // Legacy portal: only `vendors`.
        let legacy: VendorsEnvelope =
            serde_json::from_str(r#"{"vendors":[{"prefix":"acme"}]}"#).unwrap();
        assert_eq!(legacy.into_rows().into_iter().map(|v| v.prefix).collect::<Vec<_>>(), vec!["acme"]);

        // Migrated portal: BOTH keys present (dual-emit) — must parse, prefer orgs.
        let dual: VendorsEnvelope = serde_json::from_str(
            r#"{"orgs":[{"prefix":"neworg"}],"vendors":[{"prefix":"neworg"}]}"#,
        )
        .unwrap();
        assert_eq!(dual.into_rows().into_iter().map(|v| v.prefix).collect::<Vec<_>>(), vec!["neworg"]);

        // Org-only (future Phase-3 portal that stopped emitting vendors).
        let org_only: VendorsEnvelope =
            serde_json::from_str(r#"{"orgs":[{"prefix":"o1"}]}"#).unwrap();
        assert_eq!(org_only.into_rows().len(), 1);

        // Empty listing.
        let empty: VendorsEnvelope = serde_json::from_str(r#"{}"#).unwrap();
        assert!(empty.into_rows().is_empty());
    }

    // ADR 1052 Phase 3: a Topic row deserializes its anchor from `org_prefix`
    // (primary) OR the deprecated `vendor_prefix` (alias) — so a Phase-4 portal
    // that drops `vendor_prefix` still populates the field. Serialized output
    // stays `vendor_prefix` (no `--json` break / no consumer churn).
    #[test]
    fn topic_reads_org_prefix_or_vendor_prefix() {
        // Migrated portal: both keys present (dual-emit), identical value.
        let dual: Topic = serde_json::from_str(
            r#"{"id":1,"org_prefix":"acme","vendor_prefix":"acme","key_pattern":"acme/foo/v1"}"#,
        )
        .unwrap();
        assert_eq!(dual.vendor_prefix, "acme");

        // Phase-4 portal: only `org_prefix` — the alias keeps the field populated.
        let org_only: Topic = serde_json::from_str(
            r#"{"id":2,"org_prefix":"neworg","key_pattern":"neworg/bar/v1"}"#,
        )
        .unwrap();
        assert_eq!(org_only.vendor_prefix, "neworg");

        // Legacy portal: only `vendor_prefix` (the field's own name).
        let legacy: Topic = serde_json::from_str(
            r#"{"id":3,"vendor_prefix":"oldco","key_pattern":"oldco/baz/v1"}"#,
        )
        .unwrap();
        assert_eq!(legacy.vendor_prefix, "oldco");

        // Serialize side is unchanged: emits `vendor_prefix`, not `org_prefix`.
        let out = serde_json::to_string(&legacy).unwrap();
        assert!(out.contains("\"vendor_prefix\":\"oldco\""));
        assert!(!out.contains("org_prefix"));
    }

    #[tokio::test]
    async fn disabled_client_short_circuits_all_calls() {
        let c = PortalClient::new(String::new(), String::new());
        assert!(!c.is_enabled());
        assert!(matches!(c.list_topics("drs").await, Err(PortalError::Disabled)));
        assert!(matches!(c.get_topic("drs", 1).await, Err(PortalError::Disabled)));
        assert!(matches!(c.list_schemas("drs", 1).await, Err(PortalError::Disabled)));
        assert!(matches!(c.fetch_fds("drs/foo/bar/v1").await, FetchOutcome::Disabled));
    }

    #[tokio::test]
    async fn missing_api_key_disables_client() {
        let c = PortalClient::new("https://portal.example".into(), String::new());
        assert!(!c.is_enabled());
    }

    // P2.1 — the native onboard keeps the whole createVendorResponse. Decode the
    // slot-pool (ADR 1025) shape the server emits: handle→anchor remap +
    // one-time api_key/bundle_url/uri_san.
    #[test]
    fn onboarded_org_decodes_slot_pool_response() {
        let body = r#"{
            "prefix":"e7a9c75b1f2e4d3c8a9b0c1d2e3f4a5b",
            "api_key":"sk-abc123",
            "bundle_url":"/api/vendor-bundle/e7a9c75b1f2e4d3c8a9b0c1d2e3f4a5b/tok99.tar.gz",
            "bundle_token":"tok99",
            "uri_san":"urn:goat:zagros:org=e7a9c75b1f2e4d3c8a9b0c1d2e3f4a5b",
            "approved":true,
            "slot_id":"e7a9c75b1f2e4d3c8a9b0c1d2e3f4a5b",
            "system_handle":"fra-land-bde",
            "note_to_share":"Per-uuid slot onboarded."
        }"#;
        let o: OnboardedOrg = serde_json::from_str(body).unwrap();
        assert_eq!(o.system_handle, "fra-land-bde");
        assert_eq!(o.slot_id, o.prefix);
        assert!(o.bundle_url.ends_with(".tar.gz"));
        assert!(o.uri_san.starts_with("urn:goat:zagros:org="));
        assert!(o.approved);
    }

    // The per-vendor shape: no slot_id/system_handle, defaults hold.
    #[test]
    fn onboarded_org_tolerates_missing_slot_fields() {
        let o: OnboardedOrg = serde_json::from_str(
            r#"{"prefix":"acme","api_key":"k","bundle_url":"/api/vendor-bundle/acme/t.tar.gz"}"#,
        )
        .unwrap();
        assert_eq!(o.prefix, "acme");
        assert!(o.slot_id.is_empty());
        assert!(o.note_to_share.is_empty());
        assert!(!o.approved);
    }

    // goat#1193 — key-only reissue decodes the minimal server body: the anchor
    // it belongs to and the fresh api_key, and nothing else. Rekey mints no
    // cert, so there are deliberately no bundle fields to carry.
    #[test]
    fn rekeyed_key_decodes_minimal_body() {
        let body = r#"{"prefix":"acme","api_key":"vp_acme_newkey","note_to_share":"Portal key reissued in place."}"#;
        let k: RekeyedKey = serde_json::from_str(body).unwrap();
        assert_eq!(k.prefix, "acme");
        assert_eq!(k.api_key, "vp_acme_newkey");
    }

    #[tokio::test]
    async fn disabled_client_short_circuits_rekey() {
        let c = PortalClient::new(String::new(), String::new());
        assert!(matches!(c.rekey_vendor("acme").await, Err(PortalError::Disabled)));
    }

    #[tokio::test]
    async fn disabled_client_short_circuits_onboard_and_download() {
        let c = PortalClient::new(String::new(), String::new());
        let req = NewVendor {
            prefix: "x".into(),
            display_name: String::new(),
            synthetic: false,
            auto_approve: true,
            sponsor_authorized: false,
            sponsor_max_tier: String::new(),
            sponsor_email: String::new(),
            ..Default::default()
        };
        assert!(matches!(c.onboard_org(req).await, Err(PortalError::Disabled)));
        assert!(matches!(
            c.download_bundle("/api/vendor-bundle/x/t.tar.gz", std::path::Path::new("/tmp/none"))
                .await,
            Err(PortalError::Disabled)
        ));
    }

    // ADR 1096 stage 2 — decode the machine-enroll 201 bundle the portal emits.
    #[test]
    fn machine_enroll_result_decodes() {
        let body = r#"{
            "slot_id":"a1b2c3d4e5f60718293a4b5c6d7e8f90",
            "system_handle":"fra-sensor-01",
            "custodian":"alice",
            "node_class":"SENSOR",
            "autonomy":"supervised",
            "affiliation":"usareur-af",
            "api_key":"sk-machine-xyz",
            "bundle_url":"/api/vendor-bundle/a1b2c3d4e5f60718293a4b5c6d7e8f90/tok.tar.gz",
            "bundle_token":"tok",
            "uri_san":"urn:goat:zagros:org=a1b2c3d4e5f60718293a4b5c6d7e8f90",
            "quota_remaining":3,
            "note_to_share":"Deliver the cert tarball OOB."
        }"#;
        let r: MachineEnrollResult = serde_json::from_str(body).unwrap();
        assert_eq!(r.custodian, "alice");
        assert_eq!(r.node_class, "SENSOR");
        assert_eq!(r.autonomy, "supervised");
        assert_eq!(r.affiliation, "usareur-af");
        assert_eq!(r.slot_id, "a1b2c3d4e5f60718293a4b5c6d7e8f90");
        assert_eq!(r.quota_remaining, 3);
        assert!(!r.api_key.is_empty());
    }

    // custodian/display_name are skipped when None (self-service body).
    #[test]
    fn machine_enroll_spec_omits_none_fields() {
        let spec = MachineEnrollSpec {
            machine_handle: "proc-1".into(),
            node_class: "PROCESSOR".into(),
            autonomy: "autonomous".into(),
            custodian: None,
            display_name: None,
            workloads: None,
            exercise: None,
            keyless: None,
            adapters: None,
            receive: None,
            pods: None,
        };
        let j = serde_json::to_string(&spec).unwrap();
        assert!(!j.contains("custodian"), "custodian must be omitted when None: {j}");
        assert!(!j.contains("display_name"), "display_name must be omitted when None: {j}");
        assert!(!j.contains("workloads"), "workloads must be omitted when None: {j}");
        assert!(!j.contains("exercise"), "exercise must be omitted when None: {j}");
        assert!(j.contains("\"node_class\":\"PROCESSOR\""));
        // present when set
        let spec2 = MachineEnrollSpec {
            custodian: Some("bob".into()),
            ..spec
        };
        assert!(serde_json::to_string(&spec2).unwrap().contains("\"custodian\":\"bob\""));
    }

    #[tokio::test]
    async fn disabled_client_short_circuits_enroll_machine() {
        let c = PortalClient::new(String::new(), String::new());
        let spec = MachineEnrollSpec {
            machine_handle: "x".into(),
            node_class: "SENSOR".into(),
            autonomy: "manual".into(),
            custodian: None,
            display_name: None,
            workloads: None,
            exercise: None,
            keyless: None,
            adapters: None,
            receive: None,
            pods: None,
        };
        assert!(matches!(c.enroll_machine(&spec).await, Err(PortalError::Disabled)));
        assert!(matches!(c.list_machines(None).await, Err(PortalError::Disabled)));
        assert!(matches!(c.list_machines(Some("alice")).await, Err(PortalError::Disabled)));
        assert!(matches!(
            c.machine_mesh_key("a1b2c3d4e5f60718293a4b5c6d7e8f90", None).await,
            Err(PortalError::Disabled)
        ));
        let bulk = MachineBulkEnrollSpec {
            custodian: None,
            exercise: None,
            machines: vec![],
        };
        assert!(matches!(c.enroll_machines_bulk(&bulk).await, Err(PortalError::Disabled)));
    }

    // ADR 1096 stage 5 — decode a single-custodian machine list (machines + quota).
    #[test]
    fn machine_list_result_decodes() {
        let body = r#"{
            "machines":[
                {"slot_id":"aa","system_handle":"alice-sensor-1","custodian":"alice",
                 "node_class":"SENSOR","autonomy":"supervised","affiliation":"usareur-af","enrolled_at":1700},
                {"slot_id":"bb","system_handle":"alice-eff-1","custodian":"alice",
                 "node_class":"EFFECTOR","autonomy":"remote","affiliation":"usareur-af","enrolled_at":1701}
            ],
            "quota":{"custodian":"alice","quota":5,"used":2,"remaining":3}
        }"#;
        let r: MachineListResult = serde_json::from_str(body).unwrap();
        assert_eq!(r.machines.len(), 2);
        assert_eq!(r.machines[0].node_class, "SENSOR");
        let q = r.quota.expect("single-custodian view carries quota");
        assert_eq!(q.remaining, 3);
        assert_eq!(q.used, 2);
    }

    // Fleet view: machines present, quota omitted; and a fully empty body.
    #[test]
    fn machine_list_tolerates_missing_quota_and_empty() {
        let fleet: MachineListResult =
            serde_json::from_str(r#"{"machines":[{"slot_id":"x","system_handle":"h","custodian":"c","node_class":"PLATFORM","autonomy":"manual","affiliation":"u"}]}"#).unwrap();
        assert_eq!(fleet.machines.len(), 1);
        assert!(fleet.quota.is_none());
        assert_eq!(fleet.machines[0].enrolled_at, 0); // defaulted

        let empty: MachineListResult = serde_json::from_str("{}").unwrap();
        assert!(empty.machines.is_empty());
        assert!(empty.quota.is_none());
    }

    // machine-mesh-alignment — the enroll 201 now carries the mesh half
    // (groups + one-off key + note) and an optional exercise scope; a body
    // without them (older portal) still decodes with empty defaults.
    #[test]
    fn machine_enroll_result_decodes_mesh_fields_and_scope() {
        let body = r#"{
            "slot_id":"a1b2c3d4e5f60718293a4b5c6d7e8f90",
            "system_handle":"fra-sensor-01",
            "custodian":"alice",
            "node_class":"SENSOR-FEED",
            "autonomy":"supervised",
            "affiliation":"usareur-af",
            "api_key":"sk-machine-xyz",
            "bundle_url":"/api/vendor-bundle/a1b2c3d4e5f60718293a4b5c6d7e8f90/tok.tar.gz",
            "mesh_groups":["ZAGROS-SENSOR-FEEDS","ZAGROS-SITE-A"],
            "mesh_setup_key":"nbsk-one-off",
            "mesh_note":"single-use, 24h join window",
            "mesh_scope":"lion-strike"
        }"#;
        let r: MachineEnrollResult = serde_json::from_str(body).unwrap();
        assert_eq!(r.mesh_groups, vec!["ZAGROS-SENSOR-FEEDS", "ZAGROS-SITE-A"]);
        assert_eq!(r.mesh_setup_key, "nbsk-one-off");
        assert_eq!(r.mesh_scope, "lion-strike");

        let bare: MachineEnrollResult = serde_json::from_str(
            r#"{"slot_id":"a","system_handle":"h","custodian":"c","node_class":"KID",
                "autonomy":"manual","affiliation":"u","api_key":"k","bundle_url":"/b"}"#,
        )
        .unwrap();
        assert!(bare.mesh_groups.is_empty());
        assert!(bare.mesh_setup_key.is_empty());
        assert!(bare.mesh_scope.is_empty());
    }

    // machine-mesh-alignment D1 — decode the mesh-key re-mint 200, with and
    // without a minted key (mint-disabled portals return note-only).
    #[test]
    fn machine_mesh_key_result_decodes() {
        let body = r#"{
            "slot_id":"a1b2c3d4e5f60718293a4b5c6d7e8f90",
            "system_handle":"fra-sensor-01",
            "node_class":"SENSOR-FEED",
            "mesh_groups":["ZAGROS-SENSOR-FEEDS"],
            "mesh_setup_key":"nbsk-fresh",
            "mesh_note":"single-use, 24h join window",
            "mesh_scope":"lion-strike"
        }"#;
        let r: MachineMeshKeyResult = serde_json::from_str(body).unwrap();
        assert_eq!(r.system_handle, "fra-sensor-01");
        assert_eq!(r.mesh_groups, vec!["ZAGROS-SENSOR-FEEDS"]);
        assert_eq!(r.mesh_setup_key, "nbsk-fresh");
        assert_eq!(r.mesh_scope, "lion-strike");

        let off: MachineMeshKeyResult = serde_json::from_str(
            r#"{"slot_id":"a","system_handle":"h","node_class":"KID",
                "mesh_groups":["G"],"mesh_note":"mesh-key minting is off on this portal"}"#,
        )
        .unwrap();
        assert!(off.mesh_setup_key.is_empty());
        assert!(!off.mesh_note.is_empty());
        assert!(off.mesh_scope.is_empty());
    }

    // machine-mesh-alignment D7 — the bulk request omits None custodian/exercise
    // (self-service, site scope) and carries them when set.
    #[test]
    fn machine_bulk_enroll_spec_omits_none_fields() {
        let spec = MachineBulkEnrollSpec {
            custodian: None,
            exercise: None,
            machines: vec![MachineBulkSpec {
                machine_handle: "fra-sensor-01".into(),
                node_class: "SENSOR-FEED".into(),
                autonomy: "supervised".into(),
                display_name: None,
                workloads: None,
                keyless: None,
                adapters: None,
                receive: None,
                pods: None,
            }],
        };
        let j = serde_json::to_string(&spec).unwrap();
        assert!(!j.contains("custodian"), "custodian must be omitted when None: {j}");
        assert!(!j.contains("exercise"), "exercise must be omitted when None: {j}");
        assert!(!j.contains("display_name"), "display_name must be omitted when None: {j}");
        assert!(j.contains("\"machine_handle\":\"fra-sensor-01\""));

        let spec2 = MachineBulkEnrollSpec {
            custodian: Some("alice".into()),
            exercise: Some("lion-strike".into()),
            ..spec
        };
        let j2 = serde_json::to_string(&spec2).unwrap();
        assert!(j2.contains("\"custodian\":\"alice\""));
        assert!(j2.contains("\"exercise\":\"lion-strike\""));
    }

    // machine-mesh-alignment D7 — decode a mixed bulk result (one minted with
    // its full bundle, one failed with the single-door error vocabulary).
    #[test]
    fn machine_bulk_enroll_result_decodes_mixed() {
        let body = r#"{
            "custodian":"alice",
            "minted":1,
            "failed":1,
            "quota_remaining":3,
            "entries":[
                {"machine_handle":"fra-sensor-01","status":"minted","result":{
                    "slot_id":"a1b2c3d4e5f60718293a4b5c6d7e8f90","system_handle":"fra-sensor-01",
                    "custodian":"alice","node_class":"SENSOR-FEED","autonomy":"supervised",
                    "affiliation":"usareur-af","api_key":"sk-1","bundle_url":"/b1",
                    "mesh_groups":["ZAGROS-SENSOR-FEEDS"],"mesh_setup_key":"nbsk-1","mesh_note":"n"}},
                {"machine_handle":"bad-box","status":"failed",
                 "error_code":"reserved_node_class","error":"SENSOR is RESERVED"}
            ]
        }"#;
        let r: MachineBulkEnrollResult = serde_json::from_str(body).unwrap();
        assert_eq!(r.custodian, "alice");
        assert_eq!((r.minted, r.failed, r.quota_remaining), (1, 1, 3));
        assert_eq!(r.entries.len(), 2);
        let minted = r.entries[0].result.as_ref().expect("minted entry carries the bundle");
        assert_eq!(minted.mesh_setup_key, "nbsk-1");
        assert!(r.entries[0].error_code.is_empty());
        assert_eq!(r.entries[1].status, "failed");
        assert_eq!(r.entries[1].error_code, "reserved_node_class");
        assert!(r.entries[1].result.is_none());
    }
}
