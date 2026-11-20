// `goat audit` — operator-action audit.
//
// Two halves live here:
//   - the EMIT side (this module's `emit` + helpers), called best-effort from
//     every operator surface at the `goat admin` chokepoint (§4 intent layer):
//     build an OperatorActionV1 -> prost-encode -> age-encrypt to the operator
//     roster -> Zenoh put on ops/audit/operator/<actor>. Encryption is the §7
//     confidentiality layer: even a mis-set read ACL leaks only ciphertext.
//   - the READ side (`goat audit ops`): subscribe/query the stream, decrypt with
//     the operator age identity, prost-decode, render a readable table.
//
// BEST-EFFORT INVARIANT: an emit failure (no recipients configured, bus
// unreachable, session/encrypt error) MUST NEVER fail the operator action it
// describes. On any failure we warn on stderr AND append the plaintext JSON of
// the action to a local fallback log so the record is never silently lost.
//
// goat-cli deliberately does NOT enable Zenoh `unstable` (see Cargo.toml +
// CLAUDE.md) — everything here is stable Zenoh 1.9.0 (put / declare_subscriber
// / get).

use crate::profile::Profile;
use clap::{Args, Subcommand};
use prost::Message as _;
use std::io::Write as _;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ZENOH_OPEN_TIMEOUT: Duration = Duration::from_secs(5);
/// How long `goat audit ops` waits for the rocksdb-backed storage replies.
/// Storage replies are local to the router; a denial returns zero replies and
/// we hit the empty-stream end quickly.
const AUDIT_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Wire type — hand-written prost-derive struct. Keep field tags in lock-step
// with ops/zenoh-trial-peers/operator-action-audit/proto/operator_action_v1.proto
// (there is NO build.rs codegen — same convention as fabric-health / time-health).
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message)]
pub struct OperatorActionV1 {
    /// The PERSON who acted (operator handle) — never a shared label. Also the
    /// `<actor>` sub-key of `ops/audit/operator/<actor>`.
    #[prost(string, tag = "1")]
    pub actor: String,
    /// goat-cli | portal | ansible | bundle-create | host | minio.
    #[prost(string, tag = "2")]
    pub surface: String,
    /// Semantic verb: admin.onboard | admin.revoke | vendor.create | ...
    #[prost(string, tag = "3")]
    pub action: String,
    /// What it acted on: handle | vendor prefix | cert CN | topic key | host.
    #[prost(string, tag = "4")]
    pub target: String,
    /// goatnet: zagros | zagros-prod | ...
    #[prost(string, tag = "5")]
    pub env: String,
    /// started | ok | failed.
    #[prost(string, tag = "6")]
    pub outcome: String,
    /// Human error string on a failed outcome (empty otherwise).
    #[prost(string, tag = "7")]
    pub error_msg: String,
    /// Where it ran: operator-laptop hostname / mgmt-host id.
    #[prost(string, tag = "8")]
    pub host: String,
    /// Source path / working directory the action was launched from.
    #[prost(string, tag = "9")]
    pub src: String,
    /// SHA-256 hex of a REDACTED argument summary — never raw secrets.
    #[prost(string, tag = "10")]
    pub args_digest: String,
    /// Emit time, unix epoch milliseconds.
    #[prost(int64, tag = "11")]
    pub ts_unix_ms: i64,
}

impl OperatorActionV1 {
    /// Construct a new action with `host` / `src` / `ts` auto-populated from the
    /// environment. The caller fills the semantic fields (actor/surface/action/
    /// target/env/outcome) and an already-redacted `args_digest` (see
    /// [`args_digest`]). `error_msg` defaults empty; set it on a failure.
    pub fn new(
        actor: impl Into<String>,
        surface: impl Into<String>,
        action: impl Into<String>,
        target: impl Into<String>,
        env: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Self {
        Self {
            actor: actor.into(),
            surface: surface.into(),
            action: action.into(),
            target: target.into(),
            env: env.into(),
            outcome: outcome.into(),
            error_msg: String::new(),
            host: host(),
            src: src(),
            args_digest: String::new(),
            ts_unix_ms: ts_unix_ms(),
        }
    }

    /// Builder: attach a SHA-256 args digest of an already-redacted summary.
    pub fn with_args_digest(mut self, redacted_summary: &str) -> Self {
        self.args_digest = args_digest(redacted_summary);
        self
    }

    /// Builder: attach an error message (use on `outcome = "failed"`).
    pub fn with_error(mut self, msg: impl Into<String>) -> Self {
        self.error_msg = msg.into();
        self
    }
}

// ---------------------------------------------------------------------------
// Actor handle + topic key
// ---------------------------------------------------------------------------

/// The per-person operator handle for the `actor` field + the `<actor>` topic
/// sub-key. Resolution order (highest first):
///   1. `$GOAT_OPERATOR_HANDLE`
///   2. `operator_handle = "..."` in the active context's profile.toml
///   3. the active profile context name
///   4. `"operator"` (the shared legacy fallback)
///
/// The result is kebab-validated; an invalid candidate falls back to the next
/// source, and a fully invalid chain falls back to `"operator"` rather than
/// writing a malformed actor / topic key.
pub fn actor_handle(profile: &Profile) -> String {
    if let Ok(h) = std::env::var("GOAT_OPERATOR_HANDLE") {
        if is_kebab_handle(&h) {
            return h;
        }
    }
    if let Some(h) = profile_operator_handle(profile) {
        if is_kebab_handle(&h) {
            return h;
        }
    }
    if is_kebab_handle(&profile.context) {
        return profile.context.clone();
    }
    "operator".to_string()
}

/// Read `operator_handle = "..."` from the active context's profile.toml.
/// Returns `None` when the profile / key is absent. Same lightweight one-key
/// extractor pattern as `profile::read_current_context` / the profile.toml
/// endpoint readers (no full TOML parser pulled for one field).
fn profile_operator_handle(profile: &Profile) -> Option<String> {
    // Delegates to the shared, correct field reader instead of a local
    // line-scan — a leftover copy of the prefix-collision bug fixed
    // elsewhere in crate::profile::extract_toml_field would otherwise have
    // let a line like `operator_handle_prev = "..."` ahead of the real key
    // corrupt the actor id attributed to every audit record.
    let body = std::fs::read_to_string(profile.profile_toml_path()).ok()?;
    crate::profile::extract_toml_field(&body, "operator_handle").filter(|s| !s.is_empty())
}

/// The Zenoh topic key for an actor: `ops/audit/operator/<actor_handle>`.
pub fn topic_key(actor_handle: &str) -> String {
    format!("ops/audit/operator/{actor_handle}")
}

/// Kebab-handle validator: `^[a-z0-9][a-z0-9-]*$`, 1..=64 chars, no leading/
/// trailing hyphen, no path separators. Defends the `<actor>` topic segment
/// against a stray `..`/`/` and keeps the actor field a clean slug.
pub(crate) fn is_kebab_handle(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 64 {
        return false;
    }
    let first = b[0];
    let last = b[b.len() - 1];
    if first == b'-' || last == b'-' {
        return false;
    }
    s.bytes()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

// ---------------------------------------------------------------------------
// Environment helpers (host / src / ts / args_digest)
// ---------------------------------------------------------------------------

/// Operator-laptop / mgmt-host identifier. std-only (no `hostname` crate) so
/// the Windows-gnu cross-check stays green: `$HOSTNAME` (set on most Unix
/// shells), then `$COMPUTERNAME` (Windows), else `"unknown-host"`.
fn host() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "unknown-host".to_string())
}

/// Where the action was launched from — the current working directory.
fn src() -> String {
    std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

/// Emit time, unix epoch milliseconds.
fn ts_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// SHA-256 hex of an already-redacted argument summary the caller passes.
/// NEVER feed raw secrets — the caller is responsible for redaction; this only
/// makes the (already-safe) summary fixed-width + tamper-evident.
fn args_digest(redacted_summary: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(redacted_summary.as_bytes());
    let out = h.finalize();
    let mut hex = String::with_capacity(out.len() * 2);
    for byte in out {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

// ---------------------------------------------------------------------------
// age encryption to the operator roster (§7)
// ---------------------------------------------------------------------------

/// Parse the operator age roster from `$GOAT_OPERATOR_AGE_RECIPIENTS`.
/// Recipients are X25519 `age1...` strings, comma- AND/OR newline-separated
/// (mirrors the Go `parseRecipients` semantics at ops/enrollment/cmd/ca-init,
/// which splits on commas; we also accept newlines so a roster file content can
/// be pasted in directly). Returns the parsed recipients, or an error naming
/// the offending entry. An empty/unset env => `Ok(empty)` so the caller can take
/// the best-effort fallback path rather than treating "not configured" as fatal.
fn load_recipients() -> anyhow::Result<Vec<age::x25519::Recipient>> {
    let raw = std::env::var("GOAT_OPERATOR_AGE_RECIPIENTS").unwrap_or_default();
    let mut out = Vec::new();
    for part in raw.split([',', '\n', '\r']) {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let r = age::x25519::Recipient::from_str(p)
            .map_err(|e| anyhow::anyhow!("invalid age recipient `{p}`: {e}"))?;
        out.push(r);
    }
    Ok(out)
}

/// Encrypt `plaintext` to the given X25519 recipients, binary (non-armored)
/// age format. Mirrors the Go `encryptAgePEM` shape (age.Encrypt -> write ->
/// close) at ops/enrollment/cmd/ca-init/main.go.
fn encrypt_to_roster(
    plaintext: &[u8],
    recipients: &[age::x25519::Recipient],
) -> anyhow::Result<Vec<u8>> {
    if recipients.is_empty() {
        anyhow::bail!("no age recipients configured (GOAT_OPERATOR_AGE_RECIPIENTS)");
    }
    // age 0.11 takes an iterator of `&dyn Recipient` and returns a Result.
    let recs = recipients
        .iter()
        .map(|r| r as &dyn age::Recipient);
    let encryptor = age::Encryptor::with_recipients(recs)
        .map_err(|e| anyhow::anyhow!("age encryptor: {e}"))?;
    let mut out = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut out)
        .map_err(|e| anyhow::anyhow!("age wrap output: {e}"))?;
    writer
        .write_all(plaintext)
        .map_err(|e| anyhow::anyhow!("age write: {e}"))?;
    writer.finish().map_err(|e| anyhow::anyhow!("age finish: {e}"))?;
    Ok(out)
}

/// Decrypt an age ciphertext with the X25519 identity at `identity_path`.
/// Mirrors the Go `loadAgeIdentities` + `age.Decrypt` path. The identity file
/// holds one or more `AGE-SECRET-KEY-1...` lines.
fn decrypt_from_identity(
    ciphertext: &[u8],
    identity_path: &std::path::Path,
) -> anyhow::Result<Vec<u8>> {
    use std::io::Read as _;
    let body = std::fs::read_to_string(identity_path)
        .map_err(|e| anyhow::anyhow!("read age identity {}: {e}", identity_path.display()))?;
    let mut identities: Vec<age::x25519::Identity> = Vec::new();
    for line in body.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        // Only X25519 identity lines are ours; ignore anything else.
        if let Ok(id) = age::x25519::Identity::from_str(l) {
            identities.push(id);
        }
    }
    if identities.is_empty() {
        anyhow::bail!(
            "no AGE-SECRET-KEY identity found in {}",
            identity_path.display()
        );
    }
    let decryptor = age::Decryptor::new(ciphertext)
        .map_err(|e| anyhow::anyhow!("age decryptor: {e}"))?;
    let mut reader = decryptor
        .decrypt(identities.iter().map(|i| i as &dyn age::Identity))
        .map_err(|e| anyhow::anyhow!("age decrypt (not encrypted to your key?): {e}"))?;
    let mut out = Vec::new();
    reader
        .read_to_end(&mut out)
        .map_err(|e| anyhow::anyhow!("age read: {e}"))?;
    Ok(out)
}

/// Resolve the operator age identity file path for the READ side: prefer
/// `$GOAT_OPERATOR_AGE_IDENTITY`, fall back to `$GOAT_DEV_CA_AGE_IDENTITY`
/// (the dev-CA operator identity that already decrypts the roster-encrypted
/// bundle keys).
fn operator_identity_path() -> Option<std::path::PathBuf> {
    std::env::var_os("GOAT_OPERATOR_AGE_IDENTITY")
        .or_else(|| std::env::var_os("GOAT_DEV_CA_AGE_IDENTITY"))
        .map(std::path::PathBuf::from)
}

// ---------------------------------------------------------------------------
// EMIT side (best-effort)
// ---------------------------------------------------------------------------

/// Emit one OperatorActionV1 to `ops/audit/operator/<actor>` (best-effort).
///
/// The on-wire payload is `age(roster, prost_encode(action))` — the whole
/// serialized message is encrypted (§7). Open a stable Zenoh mTLS session, put
/// with `CongestionControl::Block`, close.
///
/// BEST-EFFORT: every failure path (no recipients, encrypt error, bus
/// unreachable, put error) is caught, warned on stderr, and the plaintext JSON
/// of the action is appended to the local fallback log so the record survives.
/// This function NEVER returns an error — the caller's own command exit is
/// unaffected.
/// `goat audit emit` — publish one record supplied on the command line.
///
/// Refuses an unknown outcome rather than putting an uninterpretable value on a
/// stream other tools grade: the reader has three arms and a fourth is a silent
/// hole in every rollup.
async fn run_emit(ctx: &super::Ctx, args: AuditEmitArgs) -> anyhow::Result<()> {
    const OUTCOMES: [&str; 3] = ["started", "ok", "failed"];
    if !OUTCOMES.contains(&args.outcome.as_str()) {
        anyhow::bail!(
            "--outcome must be one of {}, got {:?}",
            OUTCOMES.join(" | "),
            args.outcome
        );
    }
    if !is_kebab_handle(&args.actor) {
        anyhow::bail!(
            "--actor must be a kebab operator handle, got {:?} — a malformed actor \
             writes a malformed topic key and the record becomes unfindable",
            args.actor
        );
    }

    let mut action = OperatorActionV1::new(
        &args.actor,
        &args.surface,
        &args.action,
        &args.target,
        &args.env,
        &args.outcome,
    );
    if let Some(d) = args.args_digest.as_deref() {
        action = action.with_args_digest(d);
    }
    if let Some(e) = args.error.as_deref() {
        action = action.with_error(e);
    }

    // Deliberately the fallible path, not `emit`. `emit` is best-effort because
    // it wraps an operator command that has already DONE its work — swallowing
    // the failure there is right. Here the emit IS the work, so a caller that
    // gets exit 0 must be safe to mark the row forwarded.
    try_emit(&ctx.profile, &action).await
}

pub async fn emit(profile: &Profile, action: OperatorActionV1) {
    if let Err(e) = try_emit(profile, &action).await {
        eprintln!(
            "warning: operator-action audit emit failed ({e}); recording to local fallback log"
        );
        if let Err(fe) = append_fallback(profile, &action) {
            eprintln!("warning: operator-action audit fallback log also failed: {fe}");
        }
    }
}

/// The fallible inner emit. Split out so `emit` can keep the best-effort
/// contract in one place.
async fn try_emit(profile: &Profile, action: &OperatorActionV1) -> anyhow::Result<()> {
    let recipients = load_recipients()?;
    let plaintext = action.encode_to_vec();
    let ciphertext = encrypt_to_roster(&plaintext, &recipients)?;

    let key = topic_key(&action.actor);
    let router = profile.router_endpoint_label();
    let session = crate::cmd::pubsub::open_session_for(profile).await?;
    let put = session
        .put(&key, ciphertext)
        .congestion_control(zenoh::qos::CongestionControl::Block)
        .await;
    let _ = session.close().await;
    put.map_err(|e| crate::classify::classify_zenoh_err(e, &router, "emit an audit record"))?;
    Ok(())
}

/// Append the plaintext JSON of an action to the local fallback log so a failed
/// emit never silently loses the record. Path:
/// `<profile dir>/audit/operator-actions.log`. Dir created 0700, file 0600.
fn append_fallback(profile: &Profile, action: &OperatorActionV1) -> anyhow::Result<()> {
    let dir = profile.dir.join("audit");
    std::fs::create_dir_all(&dir)?;
    set_mode(&dir, 0o700);
    let path = dir.join("operator-actions.log");
    let line = fallback_json(action);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    set_mode(&path, 0o600);
    writeln!(f, "{line}")?;
    Ok(())
}

/// One JSON object for the fallback log. Hand-built (the wire type is a prost
/// struct, not serde) so the local record is human-greppable.
fn fallback_json(a: &OperatorActionV1) -> String {
    // serde_json handles the escaping; build a flat map of strings + the ts.
    let obj = serde_json::json!({
        "actor": a.actor,
        "surface": a.surface,
        "action": a.action,
        "target": a.target,
        "env": a.env,
        "outcome": a.outcome,
        "error_msg": a.error_msg,
        "host": a.host,
        "src": a.src,
        "args_digest": a.args_digest,
        "ts_unix_ms": a.ts_unix_ms,
    });
    obj.to_string()
}

/// Set Unix file mode; no-op on non-Unix (Windows ACLs are out of scope for
/// the fallback-log hardening, and the audit feature must still build there).
#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}
#[cfg(not(unix))]
fn set_mode(_path: &std::path::Path, _mode: u32) {}

// ---------------------------------------------------------------------------
// READ side — `goat audit ops`
// ---------------------------------------------------------------------------

#[derive(Subcommand, Debug)]
pub enum AuditCmd {
    /// Emit ONE operator-action record on behalf of another surface.
    ///
    /// This exists because the on-wire record is `age(protobuf(OperatorActionV1))`
    /// and this crate holds the only implementation of that encoding — the proto
    /// warns that any change must be mirrored field-for-field, tag-for-tag, or
    /// the wire breaks silently. A surface written in another language (herd,
    /// ADR 1131) would otherwise need a second copy of the format to be
    /// auditable at all, and two copies of a wire format is how one of them
    /// drifts.
    ///
    /// `--actor` is the PERSON the action belongs to, NOT whoever runs this
    /// command. That is the whole point: a forwarder emitting herd's records
    /// must attribute them to the person who opened the session, or attribution
    /// collapses onto the forwarder and the audit answers nothing.
    ///
    /// SAID PLAINLY: emitting on behalf of another surface is a TRUSTED
    /// operation. Anyone able to run this already holds a roster identity, and
    /// nothing here proves the named actor really did the thing. What the record
    /// does carry is `surface`, `host` and `src` — so a forged actor is still
    /// traceable to the box that emitted it. Do not extend this to accept a
    /// caller-supplied host or src.
    Emit(AuditEmitArgs),
    /// Read the operator-action audit trail (`ops/audit/operator/**`): query
    /// the rocksdb-backed store, decrypt each event with your operator age
    /// identity, and print a readable table. Samples not encrypted to your key
    /// render an `<undecryptable>` row rather than aborting the whole read.
    Ops(AuditOpsArgs),
}

#[derive(Args, Debug)]
pub struct AuditEmitArgs {
    /// The person this action belongs to (kebab operator handle).
    #[arg(long)]
    pub actor: String,
    /// Emitting surface, e.g. `herd`.
    #[arg(long)]
    pub surface: String,
    /// Semantic verb, e.g. `gear.session.open`.
    #[arg(long)]
    pub action: String,
    /// What was acted on — node id, host, topic.
    #[arg(long, default_value = "")]
    pub target: String,
    /// goatnet the action landed in.
    #[arg(long)]
    pub env: String,
    /// started | ok | failed
    #[arg(long)]
    pub outcome: String,
    /// Failure detail; use with `--outcome failed`.
    #[arg(long)]
    pub error: Option<String>,
    /// An ALREADY-REDACTED argument summary. Hashed before it goes on the wire;
    /// never pass raw secrets.
    #[arg(long)]
    pub args_digest: Option<String>,
}

#[derive(Args, Debug)]
pub struct AuditOpsArgs {
    /// Restrict to one actor (the `<actor>` sub-key). Default: all actors.
    /// An action approved in the identity console and executed by the
    /// onboard-runner (an offboard, a hold release) is filed under the
    /// approver's session identity, `kratos-<identity id>`; pass that form to
    /// see one admin's console approvals.
    #[arg(long)]
    pub actor: Option<String>,
    /// Keep the subscription open and print events as they arrive (live tail)
    /// instead of a one-shot query of stored events.
    #[arg(long)]
    pub watch: bool,
    /// Raw output: one JSON object per decoded event on stdout.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(ctx: &super::Ctx, op: AuditCmd) -> anyhow::Result<()> {
    match op {
        AuditCmd::Ops(args) => run_ops(ctx, args).await,
        AuditCmd::Emit(args) => run_emit(ctx, args).await,
    }
}

/// A decoded (or undecryptable) audit row, ready to render. The decoded
/// variant is boxed so the enum stays small (the OperatorActionV1 body dwarfs
/// the undecryptable key).
enum Row {
    Decoded(Box<OperatorActionV1>),
    Undecryptable { key: String },
}

async fn run_ops(ctx: &super::Ctx, args: AuditOpsArgs) -> anyhow::Result<()> {
    // Validate an explicit --actor the same way the emit side validates it, so a
    // stray `..`/`/` can't widen the key expression.
    let pattern = match args.actor.as_deref() {
        Some(a) => {
            if !is_kebab_handle(a) {
                anyhow::bail!("invalid --actor `{a}`: expected a kebab handle ([a-z0-9-])");
            }
            topic_key(a)
        }
        None => "ops/audit/operator/**".to_string(),
    };

    let identity = operator_identity_path().ok_or_else(|| {
        anyhow::anyhow!(
            "no operator age identity (set $GOAT_OPERATOR_AGE_IDENTITY or $GOAT_DEV_CA_AGE_IDENTITY) \
             — required to decrypt the operator-action trail"
        )
    })?;

    let router = ctx.profile.router_endpoint_label();
    let session = crate::cmd::pubsub::open_session_for(&ctx.profile).await?;

    if args.watch {
        let subscriber = session
            .declare_subscriber(pattern.clone())
            .await
            .map_err(|e| crate::classify::classify_zenoh_err(e, &router, "declare a subscriber"))?;
        eprintln!("watching `{pattern}` (Ctrl-C to stop)");
        if !args.json {
            print_header();
        }
        // Same rule as the health/presence/pub-sub watch loops: a Ctrl-C
        // ends the process, so the only way out of this loop is the
        // subscription dying, and that is a failure, not a clean stop.
        let recv_error = loop {
            let sample = match subscriber.recv_async().await {
                Ok(s) => s,
                Err(e) => break e.to_string(),
            };
            let key = sample.key_expr().as_str().to_string();
            let payload = sample.payload().to_bytes();
            let row = decode_sample(&key, &payload, &identity);
            print_row(&row, args.json);
        };
        let _ = session.close().await;
        anyhow::bail!("audit watch on `{pattern}` ended (connection lost, not Ctrl-C) — {recv_error}");
    }

    // One-shot: query the rocksdb-backed storage for stored events. Mirrors
    // pubsub's read-back shape: QueryTarget::All (storage queryables aren't
    // flagged "complete") + no consolidation.
    use zenoh::query::{ConsolidationMode, QueryTarget};
    let replies = session
        .get(&pattern)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(AUDIT_QUERY_TIMEOUT)
        .await
        .map_err(|e| crate::classify::classify_zenoh_err(e, &router, "query the audit storage"))?;

    let mut rows: Vec<Row> = Vec::new();
    while let Ok(reply) = replies.recv_async().await {
        let Ok(sample) = reply.result() else { continue };
        let key = sample.key_expr().as_str().to_string();
        let payload = sample.payload().to_bytes();
        rows.push(decode_sample(&key, &payload, &identity));
    }
    let _ = session.close().await;

    // Sort decoded rows by timestamp (oldest first); undecryptable rows sink to
    // the end (no timestamp to order on).
    rows.sort_by_key(|r| match r {
        Row::Decoded(a) => a.ts_unix_ms,
        Row::Undecryptable { .. } => i64::MAX,
    });

    if args.json {
        for r in &rows {
            print_row(r, true);
        }
        // stderr-only so --json's stdout stays clean NDJSON (main.rs routes
        // tracing to stderr for the same reason) — but the caveat below
        // still needs to reach a human, since zero rows here is otherwise
        // indistinguishable from a genuinely empty audit trail.
        if rows.is_empty() {
            eprintln!(
                "note: zero rows — this could be a genuinely empty audit trail, OR a router \
                 ACL denial silently returning nothing. It does not by itself prove \
                 nothing was ever emitted; run `goat doctor --topic <key>` to check write access."
            );
        }
    } else {
        print_header();
        for r in &rows {
            print_row(r, false);
        }
        let decoded = rows
            .iter()
            .filter(|r| matches!(r, Row::Decoded(_)))
            .count();
        eprintln!("{decoded} event(s) decoded, {} total", rows.len());
        if rows.is_empty() {
            eprintln!(
                "  (zero rows — could be a genuinely empty trail, or a silent router ACL \
                 denial, F-177. Not proof nothing was ever emitted.)"
            );
        }
    }
    Ok(())
}

/// Decrypt + decode one sample into a renderable row. A decrypt failure (the
/// sample isn't encrypted to this identity) yields `Undecryptable` rather than
/// aborting the whole read.
fn decode_sample(key: &str, payload: &[u8], identity: &std::path::Path) -> Row {
    match decrypt_from_identity(payload, identity) {
        Ok(plaintext) => match OperatorActionV1::decode(plaintext.as_slice()) {
            Ok(a) => Row::Decoded(Box::new(a)),
            Err(_) => Row::Undecryptable {
                key: key.to_string(),
            },
        },
        Err(_) => Row::Undecryptable {
            key: key.to_string(),
        },
    }
}

fn print_header() {
    println!(
        "{:<16} {:<24} {:<18} {:<20} {:<8} {:<12} HOST",
        "ACTOR", "TS", "ACTION", "TARGET", "OUTCOME", "SURFACE"
    );
}

fn print_row(row: &Row, json: bool) {
    match row {
        Row::Decoded(a) => {
            if json {
                println!("{}", fallback_json(a));
            } else {
                println!(
                    "{:<16} {:<24} {:<18} {:<20} {:<8} {:<12} {}",
                    actor_cell(&a.actor),
                    fmt_ts(a.ts_unix_ms),
                    truncate(&a.action, 18),
                    truncate(&a.target, 20),
                    truncate(&a.outcome, 8),
                    truncate(&a.surface, 12),
                    a.host,
                );
            }
        }
        Row::Undecryptable { key } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"undecryptable": true, "key": key})
                );
            } else {
                println!("{key}  <undecryptable: not encrypted to your key>");
            }
        }
    }
}

/// Render a unix-ms timestamp as a compact UTC `YYYY-MM-DDTHH:MM:SSZ`. No
/// chrono dep — the fabric pins time deps tightly; a hand-rolled civil-time
/// conversion keeps the read surface dependency-free.
fn fmt_ts(ms: i64) -> String {
    if ms <= 0 {
        return "-".to_string();
    }
    let secs = ms / 1000;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (h, m, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Days-since-Unix-epoch -> (year, month, day). Howard Hinnant's civil-from-days
/// algorithm (public domain) — exact, no leap-table.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The ACTOR column. A console session actor (`kratos-<identity id>`) is shown
/// whole: cut to the column width it would keep only the prefix and eight hex
/// digits, too little to tell two admins apart or to look the identity up.
fn actor_cell(actor: &str) -> String {
    if actor.starts_with(SESSION_ACTOR_PREFIX) {
        actor.to_string()
    } else {
        truncate(actor, 16)
    }
}

/// The actor prefix for a person identified by their identity-console session
/// (the Kratos identity id), rather than by an operator handle. It cannot
/// collide with an onboarded operator handle: the manifest schema caps a
/// handle at 32 characters, and `kratos-<uuid>` is 43.
pub(crate) const SESSION_ACTOR_PREFIX: &str = "kratos-";

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

// Re-export the open-timeout constant so a future caller can reference the same
// budget; silences dead-code on the const while keeping it documented.
#[allow(dead_code)]
const _OPEN_TIMEOUT: Duration = ZENOH_OPEN_TIMEOUT;

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_with_context(ctx: &str) -> Profile {
        Profile {
            dir: std::env::temp_dir().join("goat-audit-test-noexist"),
            context: ctx.to_string(),
        }
    }

    #[test]
    fn topic_key_is_per_actor_under_ops_audit() {
        assert_eq!(
            topic_key("maintainer-operator"),
            "ops/audit/operator/maintainer-operator"
        );
    }

    #[test]
    fn actor_handle_falls_back_to_context_then_operator() {
        // No env, no profile.toml -> the context name (when kebab-valid).
        // GOAT_OPERATOR_HANDLE is a higher layer; skip rather than mutate the
        // global env (racy under parallel tests).
        if std::env::var_os("GOAT_OPERATOR_HANDLE").is_some() {
            return;
        }
        let p = profile_with_context("maintainer-operator");
        assert_eq!(actor_handle(&p), "maintainer-operator");

        // A context name that isn't kebab-valid falls through to "operator".
        let p_bad = profile_with_context("Not_A_Handle");
        assert_eq!(actor_handle(&p_bad), "operator");
    }

    #[test]
    fn actor_handle_reads_profile_toml_over_context() {
        if std::env::var_os("GOAT_OPERATOR_HANDLE").is_some() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let p = Profile {
            dir: tmp.path().to_path_buf(),
            context: "default".to_string(),
        };
        std::fs::create_dir_all(p.context_dir()).unwrap();
        std::fs::write(
            p.profile_toml_path(),
            "router_endpoint = \"tls/x:7447\"\noperator_handle = \"jane-operator\"\n",
        )
        .unwrap();
        // profile.toml wins over the context name ("default").
        assert_eq!(actor_handle(&p), "jane-operator");
    }

    #[test]
    fn session_actor_is_shown_whole_and_filterable() {
        let id = "kratos-11111111-2222-4333-8444-555555555555";
        assert_eq!(actor_cell(id), id);
        assert_eq!(
            actor_cell("maintainer-operator-with-a-long-name"),
            "maintainer-oper…"
        );
        // --actor accepts the session form, so one admin's console approvals
        // can be listed by their identity.
        assert!(is_kebab_handle(id));
        assert_eq!(topic_key(id), format!("ops/audit/operator/{id}"));
    }

    #[test]
    fn is_kebab_handle_accepts_and_rejects() {
        assert!(is_kebab_handle("maintainer-operator"));
        assert!(is_kebab_handle("op1"));
        assert!(is_kebab_handle("a"));
        assert!(!is_kebab_handle(""));
        assert!(!is_kebab_handle("-leading"));
        assert!(!is_kebab_handle("trailing-"));
        assert!(!is_kebab_handle("Has_Underscore"));
        assert!(!is_kebab_handle("path/sep"));
        assert!(!is_kebab_handle(".."));
    }

    #[test]
    fn args_digest_is_deterministic_and_hex() {
        let a = args_digest("onboard handle=maintainer-operator env=zagros");
        let b = args_digest("onboard handle=maintainer-operator env=zagros");
        assert_eq!(a, b, "same input -> same digest");
        assert_eq!(a.len(), 64, "sha256 hex is 64 chars");
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        // Different input -> different digest.
        assert_ne!(a, args_digest("onboard handle=other env=zagros"));
    }

    #[test]
    fn age_round_trips_protobuf_body() {
        // Generate an ephemeral X25519 identity, encrypt to its recipient, and
        // decrypt back — the exact §7 path (whole encoded message encrypted).
        let identity = age::x25519::Identity::generate();
        let recipient = identity.to_public();

        let action = OperatorActionV1::new(
            "maintainer-operator",
            "goat-cli",
            "admin.onboard",
            "jane",
            "zagros",
            "ok",
        )
        .with_args_digest("onboard handle=jane env=zagros");
        let plaintext = action.encode_to_vec();

        let ciphertext = encrypt_to_roster(&plaintext, &[recipient]).expect("encrypt");
        assert_ne!(ciphertext, plaintext, "wire bytes are ciphertext, not plaintext");

        // Write the identity to a temp file and decrypt through the file path
        // (exercises decrypt_from_identity end-to-end, the read-side helper).
        // `to_string()` returns a secrecy-wrapped `AGE-SECRET-KEY-1...` string.
        use age::secrecy::ExposeSecret as _;
        let tmp = tempfile::tempdir().unwrap();
        let id_path = tmp.path().join("id.txt");
        std::fs::write(&id_path, identity.to_string().expose_secret()).unwrap();

        let back = decrypt_from_identity(&ciphertext, &id_path).expect("decrypt");
        assert_eq!(back, plaintext, "round-trip restores the protobuf body");
        let decoded = OperatorActionV1::decode(back.as_slice()).expect("decode");
        assert_eq!(decoded.actor, "maintainer-operator");
        assert_eq!(decoded.action, "admin.onboard");
        assert_eq!(decoded.outcome, "ok");
    }

    #[test]
    fn encrypt_to_roster_errors_with_no_recipients() {
        let err = encrypt_to_roster(b"x", &[]).unwrap_err();
        assert!(err.to_string().contains("no age recipients"));
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        // 1970-01-01 is day 0.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // > verify fmt_ts wraps it correctly.
        let ms = 1_781_740_800_000; // 2026-06-17T00:00:00Z (sanity: a 2026 date)
        let s = fmt_ts(ms);
        assert!(s.starts_with("2026-"), "got {s}");
        assert!(s.ends_with('Z'));
    }

    #[test]
    fn fmt_ts_handles_zero_and_negative() {
        assert_eq!(fmt_ts(0), "-");
        assert_eq!(fmt_ts(-5), "-");
    }
}
