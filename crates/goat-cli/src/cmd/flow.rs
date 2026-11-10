// `goat flow` — is data flowing, and when did it last?
//
// Rationale, alternatives, and the full blind-spot analysis live in
// ADR 1121. What follows is only the set of invariants you cannot recover
// by reading the code, because breaking one of them silently turns this
// command back into the misleading surfaces it replaced.
//
// 1. ENGAGEMENT (which observers we query — a cost decision, a fallback
//    ladder) is separate from RESOLUTION (what we print given N answers — a
//    truth decision, always a merge). Never collapse them into one pass.
//
// 2. `last` = MAX across answering tiers, labelled with the tier that
//    produced it. Understating recency is the dangerous direction, so a
//    merge can only improve the answer; a fallback can regress it.
//
// 3. Absence needs BOTH halves: `can_prove_absence()` (whole tier — is this
//    kind of observer able to look backwards over a stated horizon, and did
//    it answer completely this run) AND `covers(key)` (this key
//    specifically — the audit tail is healthy while structurally blind to
//    `volatile/**`). Either half alone stamps `no-record` on topics nobody
//    actually examined.
//
// 4. Horizons come from what an observer ACTUALLY served, never from what
//    the operator asked for. The portal silently clamps `--since` to 7d.
//
// 5. One timebase per frame. Ages are measured against the snapshot's
//    `generated_at_ms` (panoscope's server clock, not the laptop's — ADR
//    1037 says we cannot assume UTC confidence), so any tier stamping local
//    time must be offset onto it first.
//
// 6. Absence of an observation is never a zero. `last_event_ms == 0` is
//    "unobserved" and maps to `None`; an unknown last-publish is OMITTED
//    from `--json` rather than serialized as 0 or null.
//
// This module also owns the panoscope wire models: `/api/topics` (every
// visible topic, ungrouped — what this command reads) and `/api/snapshot`
// with the per-topic fold over it, which `goat ui` imports and which this
// command falls back to on a panoscope that predates `/api/topics`. A second
// copy would drift, and once did.

use crate::cmd::ui::DeclaredTopic;
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use zenoh::key_expr::KeyExpr;

/// Ties within this many ms are treated as the same instant and broken by
/// evidence durability rather than by raw ordering — two observers of the
/// same publish will not agree to the millisecond.
const SKEW_TOLERANCE_MS: i64 = 2_000;

/// Hard cap on the live tier's sample count. Hitting it downgrades the tier
/// to `Partial` rather than silently reporting a bounded count as complete.
const LIVE_SAMPLE_CAP: u64 = 50_000;

/// Default flowing/idle boundary.
const DEFAULT_FRESH_SECS: i64 = 60;

/// Default live-window length, in seconds.
const DEFAULT_LIVE_WINDOW_SECS: u64 = 3;

/// What the live tier subscribes to when the operator names no key.
///
/// It used to be `**`. Subscribing to the whole keyspace in one declaration is
/// an infrastructure grant bound to named certificates, because a router blocks
/// per push to slow `**` subscribers and can stop fabric-wide delivery. A
/// `goat flow` run from an ordinary operator laptop would be one of those
/// subscribers, and an ordinary laptop is not named.
///
/// `*/**` is the same coverage without the grant: it matches every key with at
/// least one chunk — i.e. everything anyone actually publishes — and the ACL
/// rule that replaced the universal `**` grant allows it (the rule IS `*/**`).
/// So the command keeps showing the whole fabric, and stops being one of the
/// subscriptions that can stall it by name.
const DEFAULT_LIVE_SCOPE: &str = "*/**";

/// The portal's own hard clamp on an audit presign window
/// (`auditPresignMaxWindow`). Used only as a fallback bound when the portal's
/// echoed floor can't be parsed — the echoed floor is authoritative.
const AUDIT_MAX_WINDOW_MS: i64 = 7 * 24 * 3_600_000;

/// Default `--watch` repaint interval, in seconds. One frame is ~N
/// concurrent portal reads plus a snapshot GET, so a fast cadence would
/// spend more time fetching than displaying.
const DEFAULT_WATCH_INTERVAL_SECS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum LiveMode {
    /// Engage the live tier only when every other engaged tier is
    /// unreachable (the denied-edge case: router up, portal/panoscope gone).
    Auto,
    /// Always engage, as corroboration alongside the other tiers.
    Always,
    /// Never engage.
    Never,
}

#[derive(Args, Debug)]
pub struct FlowArgs {
    /// Key expression to report on (e.g. `acme/**`). Default: every
    /// topic any engaged observer knows about.
    pub key_expr: Option<String>,

    /// Also read the audit-tail over this window: a duration (`90m`, `24h`,
    /// `7d`, `2h30m`) or an RFC3339 timestamp. This is the only tier that
    /// can prove a topic has NOT published — without it a silent topic reads
    /// `unknown`, not `no-record`. The portal serves at most 7 days and says
    /// so on the frame when it clamps you.
    #[arg(long)]
    pub since: Option<String>,

    /// When to engage a bounded live subscribe as an observer.
    #[arg(long, value_enum, default_value_t = LiveMode::Auto)]
    pub live: LiveMode,

    /// How long the live tier listens, in seconds.
    #[arg(long, value_name = "SECS", default_value_t = DEFAULT_LIVE_WINDOW_SECS)]
    pub live_window: u64,

    /// A topic whose last publish is within this many seconds reads
    /// `flowing`; older reads `idle`.
    #[arg(long, value_name = "SECS", default_value_t = DEFAULT_FRESH_SECS)]
    pub fresh: i64,

    /// Repaint on an interval until Ctrl-C. On a terminal the frame
    /// repaints in place; under a pipe each frame is appended with a
    /// separator, so `goat flow --watch | tee flow.log` keeps a history.
    ///
    /// (There is no `--once`: a bare `goat flow` is already the one-frame
    /// scriptable snapshot. `top`/`who` need `--once` because they default
    /// to an interactive screen; this command defaults to the snapshot.)
    #[arg(long)]
    pub watch: bool,

    /// Seconds between `--watch` repaints.
    #[arg(long, value_name = "SECS", default_value_t = DEFAULT_WATCH_INTERVAL_SECS)]
    pub interval: u64,

    /// Machine output: one JSON object carrying the rows AND a `sources`
    /// array describing every tier, including the ones that were skipped —
    /// so a script can tell "the audit tail said no" from "the audit tail
    /// was never asked". The stable machine contract.
    #[arg(long, conflicts_with = "watch")]
    pub json: bool,

    /// Override the panoscope snapshot endpoint. Default: derived from the
    /// profile's portal_endpoint (`portal.*` → `panoscope.*`), also honors
    /// `$GOAT_PANOSCOPE_ENDPOINT`.
    #[arg(long)]
    pub snapshot_endpoint: Option<String>,

    /// Show every topic any engaged observer knows about — declared,
    /// idle, no-record, and unknown rows included. Default shows only
    /// what is actually `flowing` right now; the full honesty-graded
    /// universe (this flag) is what `--json` always carries regardless,
    /// so scripts never lose the distinction this flag hides from the
    /// human table.
    #[arg(long)]
    pub all: bool,
}

pub async fn run(ctx: &super::Ctx, args: FlowArgs) -> anyhow::Result<()> {
    // NO TTY gate, deliberately. `crate::tui::require_tty` exists for
    // full-screen surfaces that READ INPUT and would hang a script waiting
    // for a keypress — its only other callers are `top`, `who`, `ui`, and
    // the two `--tui` lenses. `goat flow --watch` reads no input, and the
    // sibling line-oriented watchers (`health --watch`, `presence --watch`)
    // are likewise ungated. Gating here would also forbid
    // `goat flow --watch | tee flow.log`, which is one of the reasons this
    // command uses a repaint loop instead of a TUI in the first place —
    // refusing it would defeat the design (ADR 1121 D5). Non-TTY output
    // takes the append-with-separator path in `watch_loop` instead.
    let reg = crate::alias::load_aliases(&ctx.profile).await;
    let filter = match &args.key_expr {
        Some(k) => Some(
            crate::alias::resolve_key_volatile_aware(&reg, k.trim())
                .map_err(|e| anyhow::anyhow!("cannot resolve `{k}`: {e}"))?,
        ),
        None => None,
    };

    if !args.watch {
        let report = collect(ctx, &args, filter.as_deref()).await?;
        if args.json {
            println!("{}", serde_json::to_string(&report)?);
        } else {
            print!("{}", render_frame(&report, ctx.output.color, !args.all));
        }
        return Ok(());
    }

    watch_loop(ctx, &args, filter.as_deref()).await
}

/// Repaint on an interval until Ctrl-C. A PLAIN loop, deliberately not a
/// ratatui screen (ADR 1121 D5): `flow` polls HTTP on a seconds cadence
/// rather than draining a Zenoh stream, so there is nothing to decouple a
/// draw loop from; an alternate screen would additionally destroy the
/// scrollback an operator watching a topic go quiet actually wants, and make
/// the command impossible to pipe. It also keeps this file free of
/// `StderrSilencer`, which is unix-only — so the Windows cross-compile leg
/// stays trivial.
///
/// The in-place repaint is gated on `ctx.output.color` (i.e. TTY-ness, minus
/// `NO_COLOR`/`--color never`), NOT on a TTY refusal: under a pipe the loop
/// still runs and appends each frame, which is what makes `| tee` work.
async fn watch_loop(
    ctx: &super::Ctx,
    args: &FlowArgs,
    filter: Option<&str>,
) -> anyhow::Result<()> {
    let mut ticker = tokio::time::interval(Duration::from_secs(args.interval.max(1)));
    // A frame is several network reads and can outrun the interval. The
    // default `Burst` would then fire the backlog back-to-back, turning a
    // slow fabric into an unthrottled fetch loop against the portal —
    // exactly when it is least able to take it.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut frame: u64 = 0;
    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = tokio::signal::ctrl_c() => break,
        }
        frame += 1;

        // Ctrl-C races the WHOLE frame, not just the wait before it. A frame
        // can take seconds, and an operator hitting Ctrl-C mid-collect
        // expects it to land now, not after the fetch finishes.
        let collected = tokio::select! {
            r = collect(ctx, args, filter) => r,
            _ = tokio::signal::ctrl_c() => break,
        };

        let report = match collected {
            Ok(r) => r,
            // The FIRST frame failing means the setup is broken (no profile,
            // no endpoint) — fail fast with the real exit code. After that,
            // a watch is a monitoring surface: a transient portal blip must
            // not tear it down, or it will always die exactly when the
            // fabric is interesting.
            Err(e) if frame == 1 => return Err(e),
            Err(e) => {
                eprintln!("frame {frame}: {e:#} — retrying at the next tick");
                continue;
            }
        };

        let body = render_frame(&report, ctx.output.color, !args.all);
        if ctx.output.color {
            // Home + clear, so the frame repaints in place.
            print!("\x1b[H\x1b[2J{body}");
        } else {
            // Non-TTY: append with a separator so `| tee` keeps a readable
            // history instead of a pile of clobbered frames.
            print!("── frame {frame} ──\n{body}\n");
        }
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Observers
// ---------------------------------------------------------------------------

/// Which observer answered a field.
// NO `PartialOrd`/`Ord`, deliberately — see `evidence_rank`. A derived `<`
// would order these by declaration position, which is meaningless, and would
// silently compile in exactly the places where the right ordering differs
// per field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Source {
    None,
    /// A bounded live subscribe saw at least one sample.
    LiveWindow,
    /// panoscope `/api/snapshot` per-topic edge state.
    Panoscope,
    /// The Object-Lock audit tail, via portal-presigned NDJSON.
    Audit,
    /// The portal registry's own `topics.last_seen_at` column.
    PortalRegistry,
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Source::None => "—",
            Source::LiveWindow => "live",
            Source::Panoscope => "panoscope",
            Source::Audit => "audit",
            Source::PortalRegistry => "portal-reg",
        }
    }

    /// Tie-break rank for the `last` field ONLY — durability of the
    /// evidence. Deliberately NOT a `PartialOrd` impl over the whole type:
    /// the ordering that is right for `last` is wrong for `eps` (where a
    /// live window is the most direct measurement), so overloading `<`
    /// would invite exactly the bug this comment exists to prevent.
    fn evidence_rank(self) -> u8 {
        match self {
            Source::Audit => 4,
            Source::PortalRegistry => 3,
            Source::Panoscope => 2,
            Source::LiveWindow => 1,
            Source::None => 0,
        }
    }

    /// THE load-bearing predicate. True only for tiers that can look
    /// BACKWARDS over a STATED horizon and report "nothing there".
    /// panoscope cannot — it never states how far back its memory reaches,
    /// so its silence bounds nothing. A live window cannot — silence is not
    /// absence.
    fn can_prove_absence(self) -> bool {
        matches!(self, Source::Audit | Source::PortalRegistry)
    }
}

/// One tier's word on one topic. Every field is `Option` because "didn't
/// answer" and "answered zero" are different facts, and the fold must never
/// conflate them — conflating them is precisely the `publish_count: 0` bug.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TierObservation {
    pub last_ms: Option<i64>,
    pub eps: Option<f64>,
    pub pubs: Option<usize>,
    pub subs: Option<usize>,
}

/// Why a tier contributed what it did. Distinguishing these IS the command.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TierState {
    /// Engaged, answered.
    Ok(String),
    /// Not engaged this run.
    Skipped(String),
    /// Engaged and failed. An UNREACHABLE observer is not an idle topic.
    Unreachable(String),
    /// Answered, but known-incomplete. A `Partial` tier LOSES
    /// `can_prove_absence` for this run.
    Partial(String),
}

impl TierState {
    fn label(&self) -> &'static str {
        match self {
            TierState::Ok(_) => "ok",
            TierState::Skipped(_) => "skipped",
            TierState::Unreachable(_) => "unreachable",
            TierState::Partial(_) => "partial",
        }
    }

    fn detail(&self) -> &str {
        match self {
            TierState::Ok(d)
            | TierState::Skipped(d)
            | TierState::Unreachable(d)
            | TierState::Partial(d) => d,
        }
    }

    fn is_ok(&self) -> bool {
        matches!(self, TierState::Ok(_))
    }

    fn is_unreachable(&self) -> bool {
        matches!(self, TierState::Unreachable(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TierResult {
    pub source: Source,
    pub state: TierState,
    /// How far back this tier can speak to. `None` = unbounded or unknown.
    pub horizon_ms: Option<i64>,
    pub horizon_note: Option<String>,
    pub topics: BTreeMap<String, TierObservation>,
}

impl TierResult {
    fn skipped(source: Source, detail: impl Into<String>) -> Self {
        Self {
            source,
            state: TierState::Skipped(detail.into()),
            horizon_ms: None,
            horizon_note: None,
            topics: BTreeMap::new(),
        }
    }

    fn unreachable(source: Source, detail: impl Into<String>) -> Self {
        Self {
            source,
            state: TierState::Unreachable(detail.into()),
            horizon_ms: None,
            horizon_note: None,
            topics: BTreeMap::new(),
        }
    }

    /// A tier proves absence only if it CAN, and only if it answered
    /// completely this run. `Partial` deliberately forfeits the power.
    ///
    /// This is a WHOLE-TIER property. It is necessary but NOT sufficient —
    /// always pair it with `covers(key)`, which is the per-key half.
    fn proves_absence(&self) -> bool {
        self.source.can_prove_absence() && self.state.is_ok()
    }

    /// Whether this tier's silence about `key` is EVIDENCE about `key`.
    ///
    /// The per-key half of proving absence, and it exists because a tier can
    /// be healthy in aggregate while being structurally blind to particular
    /// keys. The audit tail is the case that forced it: it is `Ok` over its
    /// whole window while never seeing `volatile/**` at all — the audit
    /// subscriber drops those samples before hashing or upload (ADR 1032).
    /// Without this check, every volatile topic would be stamped
    /// `no-record`, which is precisely the "we could not see" → "there was
    /// nothing" collapse this command exists to prevent.
    fn covers(&self, key: &str) -> bool {
        // FAIL CLOSED on anything that is not exactly one topic. Observers
        // are keyed by concrete observed keys, so "no tier had an entry for
        // this pattern" is a property of how we look things up, not evidence
        // about the fabric. Without this, a registered `acme/entity/*/v1`
        // (the portal explicitly permits a `*` segment) reads `no-record`
        // while its own children are flowing one line above.
        if !is_concrete_key(key) {
            return false;
        }
        match self.source {
            // Blind to the non-persisted class by construction.
            Source::Audit => !key_intersects("volatile/**", key),
            // The registry can only speak to rows it actually holds.
            Source::PortalRegistry => self.topics.contains_key(key),
            _ => false,
        }
    }

    fn provides(&self) -> Vec<&'static str> {
        if !(self.state.is_ok() || matches!(self.state, TierState::Partial(_))) {
            return Vec::new();
        }
        match self.source {
            Source::Panoscope => vec!["last", "eps", "pubs", "subs"],
            Source::Audit => vec!["last"],
            Source::LiveWindow => vec!["last", "eps"],
            Source::PortalRegistry => vec!["last"],
            Source::None => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------

/// The four-state honesty vocabulary. See the module header — the
/// `Unknown`/`NoRecord` split is the reason this command exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FlowState {
    /// A tier has a last-publish within the freshness window.
    Flowing,
    /// A tier has a last-publish, and it is older than the window.
    Idle,
    /// A tier that CAN prove absence looked over its stated horizon and
    /// found nothing. Rendered "none in 24h" — never "never".
    NoRecord,
    /// Nothing could answer. NOT "no data" — "no observer".
    Unknown,
}

impl FlowState {
    fn label(self) -> &'static str {
        match self {
            FlowState::Flowing => "flowing",
            FlowState::Idle => "idle",
            FlowState::NoRecord => "no-record",
            FlowState::Unknown => "unknown",
        }
    }

    /// ANSI attribute for the state cell. The inversion is deliberate:
    /// `unknown` is YELLOW because a blind observer demands operator
    /// action, while `idle` is DIM because a quiet topic is a fact, not a
    /// fault. Mirrors `goat health` dimming idle/unmetered so they can
    /// never read as a false green.
    fn ansi(self) -> &'static str {
        match self {
            FlowState::Flowing => "32",
            FlowState::Idle | FlowState::NoRecord => "2",
            FlowState::Unknown => "33",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AsOfSource {
    /// panoscope's server clock, from the snapshot envelope.
    PanoscopeGeneratedAt,
    /// This machine's clock — used only when no tier carried its own.
    LocalClock,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct FlowRow {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
    pub declared: bool,
    pub state: FlowState,
    /// ABSENT when unknown. Never 0, never null — encoding the absence
    /// structurally is what stops this from becoming the next
    /// `publish_count: 0`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_publish_unix_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_age_ms: Option<i64>,
    pub last_source: Source,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eps_source: Option<Source>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pubs: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subs: Option<usize>,
    /// Every tier that had anything to say about this key — so a row is
    /// never ambiguous, even when the tiers disagree.
    pub observed_by: Vec<Source>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SourceReport {
    pub source: Source,
    pub state: &'static str,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizon_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizon_note: Option<String>,
    pub provides: Vec<&'static str>,
    pub can_prove_absence: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct FlowReport {
    pub as_of_unix_ms: i64,
    pub as_of_source: AsOfSource,
    pub fresh_secs: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// Always carries an entry for EVERY tier, including skipped ones.
    pub sources: Vec<SourceReport>,
    pub rows: Vec<FlowRow>,
}

/// Everything already fetched. No HTTP, no clock, no Zenoh — so the fold
/// below is pure and fully testable, the same shape as `merge_roster` /
/// `aggregate_live` in `cmd/ui.rs`.
pub(crate) struct FlowInputs {
    pub as_of_unix_ms: i64,
    pub as_of_source: AsOfSource,
    pub fresh_ms: i64,
    pub declared: Vec<DeclaredTopic>,
    pub tiers: Vec<TierResult>,
    pub filter: Option<String>,
}

// ---------------------------------------------------------------------------
// The pure fold — the heart of the command
// ---------------------------------------------------------------------------

pub(crate) fn resolve_flow(inputs: &FlowInputs) -> FlowReport {
    let declared_by_key: BTreeMap<&str, &DeclaredTopic> = inputs
        .declared
        .iter()
        .map(|d| (d.key.as_str(), d))
        .collect();

    // Universe = declared keys ∪ every key any tier mentioned, filtered by
    // key-expression intersection (not prefix match — `org11x/**` must
    // catch `org11x/obs/v1` and nothing else).
    let mut keys: BTreeSet<&str> = BTreeSet::new();
    for d in &inputs.declared {
        keys.insert(d.key.as_str());
    }
    for t in &inputs.tiers {
        for k in t.topics.keys() {
            keys.insert(k.as_str());
        }
    }
    if let Some(f) = &inputs.filter {
        keys.retain(|k| key_intersects(f, k));
    }

    let rows: Vec<FlowRow> = keys
        .into_iter()
        .map(|k| {
            resolve_one_key(
                k,
                declared_by_key.get(k).copied(),
                &inputs.tiers,
                inputs.as_of_unix_ms,
                inputs.fresh_ms,
            )
        })
        .collect();

    let sources = inputs
        .tiers
        .iter()
        .map(|t| SourceReport {
            source: t.source,
            state: t.state.label(),
            detail: t.state.detail().to_string(),
            horizon_ms: t.horizon_ms,
            horizon_note: t.horizon_note.clone(),
            provides: t.provides(),
            can_prove_absence: t.proves_absence(),
        })
        .collect();

    FlowReport {
        as_of_unix_ms: inputs.as_of_unix_ms,
        as_of_source: inputs.as_of_source,
        fresh_secs: inputs.fresh_ms / 1000,
        filter: inputs.filter.clone(),
        sources,
        rows,
    }
}

fn resolve_one_key(
    key: &str,
    declared: Option<&DeclaredTopic>,
    tiers: &[TierResult],
    as_of_ms: i64,
    fresh_ms: i64,
) -> FlowRow {
    // Concrete keys resolve by exact lookup — every observer is keyed by the
    // concrete key it actually saw, so this is both correct and cheap.
    //
    // A DECLARED PATTERN (the portal permits a `*` segment) is never a key any
    // observer reports, so an exact lookup
    // finds nothing and the row would render as if unobserved. Resolve it
    // through the children it covers instead: newest child wins `last`,
    // rates and counts sum. The children still appear as their own rows;
    // this one summarises them.
    let answers: Vec<(Source, TierObservation)> = if is_concrete_key(key) {
        tiers
            .iter()
            .filter_map(|t| t.topics.get(key).map(|o| (t.source, o.clone())))
            .collect()
    } else {
        tiers
            .iter()
            .filter_map(|t| {
                let mut merged = TierObservation::default();
                let mut matched = false;
                for (k, o) in t.topics.iter().filter(|(k, _)| key_intersects(key, k)) {
                    let _ = k;
                    matched = true;
                    if let Some(ms) = o.last_ms {
                        merged.last_ms = Some(merged.last_ms.map_or(ms, |c: i64| c.max(ms)));
                    }
                    if let Some(e) = o.eps {
                        *merged.eps.get_or_insert(0.0) += e;
                    }
                    if let Some(p) = o.pubs {
                        *merged.pubs.get_or_insert(0) += p;
                    }
                    if let Some(s) = o.subs {
                        *merged.subs.get_or_insert(0) += s;
                    }
                }
                matched.then_some((t.source, merged))
            })
            .collect()
    };

    // `last` = max across answering tiers, attributed to the most durable
    // observer that saw essentially the same instant.
    //
    // TWO passes, deliberately. A single fold comparing each tier against
    // the running best lets near-ties CHAIN: with A at t, B at t+1.5s and C
    // at t+3s, B is "within skew" of both, so attribution walks A→B→C even
    // though A and C are 3s apart. Taking the max first, then choosing among
    // only those within tolerance OF THE MAX, is independent of tier order
    // and cannot chain.
    let max_ms = answers.iter().filter_map(|(_, o)| o.last_ms).max();
    let best: Option<(Source, i64)> = max_ms.map(|m| {
        let src = answers
            .iter()
            .filter(|(_, o)| o.last_ms.is_some_and(|ms| m - ms <= SKEW_TOLERANCE_MS))
            .max_by_key(|(s, _)| s.evidence_rank())
            .map(|(s, _)| *s)
            .unwrap_or(Source::None);
        (src, m)
    });

    // `eps`: prefer panoscope's real EMA over a short live count. This is
    // the opposite preference from `last`, which is exactly why
    // `evidence_rank` is not a blanket Ord impl.
    let eps = answers
        .iter()
        .find(|(s, o)| *s == Source::Panoscope && o.eps.is_some())
        .or_else(|| {
            answers
                .iter()
                .find(|(s, o)| *s == Source::LiveWindow && o.eps.is_some())
        })
        .map(|(s, o)| (*s, o.eps.unwrap_or_default()));

    let pano = answers.iter().find(|(s, _)| *s == Source::Panoscope);

    let state = match best {
        Some((_, ms)) => {
            if as_of_ms.saturating_sub(ms) <= fresh_ms {
                FlowState::Flowing
            } else {
                FlowState::Idle
            }
        }
        // Nothing has a timestamp. Only a tier that CAN prove absence, that
        // answered COMPLETELY, *and* that can actually SEE this key may
        // downgrade the row from "no observer" to "observed, and there was
        // nothing". All three conditions, or it stays `unknown`.
        None => {
            if tiers.iter().any(|t| t.proves_absence() && t.covers(key)) {
                FlowState::NoRecord
            } else {
                FlowState::Unknown
            }
        }
    };

    // For a `no-record` row the attribution is the tier that did the
    // proving, so the renderer can say "none in <its horizon>".
    let last_source = match (state, best) {
        (_, Some((s, _))) => s,
        (FlowState::NoRecord, None) => tiers
            .iter()
            .find(|t| t.proves_absence() && t.covers(key))
            .map(|t| t.source)
            .unwrap_or(Source::None),
        _ => Source::None,
    };

    FlowRow {
        key: key.to_string(),
        org: declared
            .map(|d| d.vendor.clone())
            .filter(|v| !v.is_empty())
            .or_else(|| key.split('/').next().filter(|s| !s.is_empty()).map(String::from)),
        declared: declared.is_some(),
        state,
        last_publish_unix_ms: best.map(|(_, ms)| ms),
        // Clamped at 0: `as_of` is frozen at the snapshot instant, but the
        // live tier keeps listening for seconds AFTER it, so a genuinely
        // fresh sighting is legitimately newer than the frame's clock. Left
        // negative it renders as `—` (`fmt_age`), the same glyph as "no
        // timestamp at all" — so the freshest rows on the frame would read
        // as the emptiest. `saturating_sub` does not help here: on i64 it
        // saturates at MIN, not 0.
        last_age_ms: best.map(|(_, ms)| (as_of_ms - ms).max(0)),
        last_source,
        eps: eps.map(|(_, v)| v),
        eps_source: eps.map(|(s, _)| s),
        pubs: pano.and_then(|(_, o)| o.pubs),
        subs: pano.and_then(|(_, o)| o.subs),
        observed_by: answers.iter().map(|(s, _)| *s).collect(),
    }
}

/// True when `key` names exactly one topic: it parses as a key expression
/// AND contains no wildcard chunk.
///
/// The distinction is load-bearing in two places, and both were bugs before
/// it existed. panoscope's node arrays are NOT clean key lists — `publishes`
/// is passed through the same server-side singleton fold as the edges (so it
/// can contain the display string `"<ns>/* (folded xN)"`), and `subscribes`
/// is verbatim, so the audit subscriber's default `**` subscription shows up
/// as an entry. Admitting either as a topic produces a phantom row; and
/// because `**` IS a legal key expression, that phantom survives every
/// filter, so `goat flow <one-key>` would print a row the operator did not
/// ask for. A pattern must also never be eligible for an absence verdict:
/// no observer is keyed by patterns, so "nothing matched this pattern
/// exactly" is not evidence that nothing published.
fn is_concrete_key(key: &str) -> bool {
    KeyExpr::new(key).is_ok()
        && !key
            .split('/')
            .any(|c| c == "*" || c == "**" || c.contains("$*"))
}

/// True when `key` falls under `pattern` (Zenoh key-expression
/// intersection). Falls back to exact match if either side fails to parse,
/// so a malformed filter narrows rather than silently widening.
fn key_intersects(pattern: &str, key: &str) -> bool {
    match (
        KeyExpr::new(pattern.to_string()),
        KeyExpr::new(key.to_string()),
    ) {
        (Ok(p), Ok(k)) => p.intersects(&k),
        _ => pattern == key,
    }
}

// ---------------------------------------------------------------------------
// panoscope /api/snapshot wire model — owned here, imported by `goat ui`
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct Snapshot {
    #[serde(default)]
    pub nodes: Vec<SnapNode>,
    #[serde(default)]
    pub edges: Vec<SnapEdge>,
    /// panoscope's own clock at snapshot time. Ages are computed against
    /// THIS, not the local clock — see the module header on clock
    /// discipline. Zero when an older panoscope omits it.
    #[serde(default)]
    pub generated_at_ms: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SnapNode {
    #[serde(default)]
    pub peer_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub vendor: Option<String>,
    #[serde(default)]
    pub role_class: Option<String>,
    #[serde(default)]
    pub publishes: Vec<String>,
    #[serde(default)]
    pub subscribes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SnapEdge {
    pub topic: String,
    /// The raw EMA. NOTE: this only decays when an event ARRIVES
    /// (`EdgeState::record_event`), so a topic that published once an hour
    /// ago carries a permanently non-zero `ema_eps` in every snapshot.
    /// Reading it as a current rate is a lie — prefer `projected_eps`.
    #[serde(default)]
    pub ema_eps: f64,
    /// The EMA projected forward to the snapshot instant — panoscope emits
    /// this precisely "for read paths so a quiet edge fades over time".
    /// Absent on older panoscopes, hence `Option`.
    #[serde(default)]
    pub projected_eps: Option<f64>,
    #[serde(default)]
    pub last_event_ms: u64,
    /// Set when panoscope has server-side FOLDED a fan-out group into one
    /// synthetic edge to bound the wire payload. The `topic` on such an edge
    /// is not a key expression at all — it is the display string
    /// `"<ns>/* (folded xN)"`, which fails to parse as a key expression, so
    /// it can never match a filter or attach to a declared row. Callers must
    /// exclude these from per-key resolution rather than treat them as
    /// topics. Carries how many real topics the fold collapsed.
    #[serde(default)]
    pub folded_singletons_count: Option<u64>,
}

impl SnapEdge {
    /// The honest current rate: the projection when panoscope offers it,
    /// falling back to the undecayed EMA only for older servers that don't.
    pub(crate) fn rate(&self) -> f64 {
        self.projected_eps.unwrap_or(self.ema_eps)
    }

    /// True when this edge is panoscope's synthetic fold of many topics
    /// rather than one real topic.
    pub(crate) fn is_folded(&self) -> bool {
        self.folded_singletons_count.is_some()
    }
}

/// One topic's worth of a folded snapshot. The single place the
/// `/api/snapshot` edge/node arrays are reduced per topic — `goat flow` and
/// `goat ui` both consume this rather than each walking the arrays, which
/// is how they drifted apart in the first place (`ui` summed the undecayed
/// `ema_eps` and dropped `last_event_ms` entirely).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TopicFold {
    pub eps: f64,
    /// `None` when panoscope has no observation — never 0, which would read
    /// as epoch-1970 rather than "unobserved".
    pub last_event_ms: Option<u64>,
    pub pubs: usize,
    pub subs: usize,
}

/// Result of folding a whole snapshot: real topics, plus what panoscope
/// collapsed and therefore is NOT reporting per-topic.
pub(crate) struct SnapshotFold {
    pub topics: BTreeMap<String, TopicFold>,
    /// Synthetic fan-out edges skipped, and how many real topics they hid.
    pub folded_edges: u64,
    pub folded_topics: u64,
}

/// How many real topics a folded display string stands for, parsed out of
/// panoscope's `"<ns>/* (folded xN)"` suffix. **0 when the string is not a
/// fold** — a plain wildcard subscription like `**` hides no topics, so it
/// must contribute nothing to the disclosed count.
fn fold_count(s: &str) -> u64 {
    s.rsplit_once("(folded x")
        .and_then(|(_, tail)| tail.trim_end_matches(')').parse().ok())
        .unwrap_or(0)
}

/// Fold the snapshot per topic. Pure — no clock, no HTTP — so both callers
/// can unit-test against it.
pub(crate) fn fold_snapshot(snap: &Snapshot) -> SnapshotFold {
    let mut topics: BTreeMap<String, TopicFold> = BTreeMap::new();
    let mut folded_edges = 0u64;
    let mut folded_topics = 0u64;

    for e in &snap.edges {
        // A folded edge's `topic` is a DISPLAY STRING ("<ns>/* (folded xN)"),
        // not a key expression — it cannot parse, so it can never match a
        // filter or attach to a declared row. Admitting it would produce a
        // bogus row unfiltered and silently drop the whole namespace when
        // filtered. Count it and disclose it instead of inventing keys.
        if e.is_folded() {
            folded_edges += 1;
            folded_topics += e.folded_singletons_count.unwrap_or(0);
            continue;
        }
        let row = topics.entry(e.topic.clone()).or_default();
        // `rate()` prefers the projection: raw `ema_eps` only decays when an
        // event ARRIVES, so summing it reports a quiet topic as busy.
        row.eps += e.rate();
        if e.last_event_ms > 0 {
            row.last_event_ms = Some(
                row.last_event_ms
                    .map_or(e.last_event_ms, |cur| cur.max(e.last_event_ms)),
            );
        }
    }
    // The node arrays need the SAME guard as the edges, and for two reasons
    // rather than one. `publishes` runs through panoscope's singleton fold,
    // so it can carry `"<ns>/* (folded xN)"` display strings; `subscribes`
    // is verbatim, so it carries whatever peers actually subscribed to —
    // and the audit subscriber subscribes to `**` by default. Neither names
    // a topic. `**` is worse than the unparseable fold string because it IS
    // a valid key expression, so it would pass every filter and appear as a
    // phantom row on a query for one specific key.
    for n in &snap.nodes {
        for t in &n.publishes {
            if !is_concrete_key(t) {
                folded_topics += fold_count(t);
                continue;
            }
            topics.entry(t.clone()).or_default().pubs += 1;
        }
        for t in &n.subscribes {
            if !is_concrete_key(t) {
                continue;
            }
            topics.entry(t.clone()).or_default().subs += 1;
        }
    }
    SnapshotFold {
        topics,
        folded_edges,
        folded_topics,
    }
}

/// panoscope `/api/topics`: every topic the caller may see, one row per key,
/// never grouped (docs/design/panoscope-view-model.md).
#[derive(Debug, Deserialize)]
pub(crate) struct TopicList {
    /// panoscope's clock when its model was built — the frame's timebase.
    #[serde(default)]
    pub built_at_ms: u64,
    /// Matching topics before panoscope's row limit.
    #[serde(default)]
    pub total: usize,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub topics: Vec<TopicListRow>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TopicListRow {
    pub key: String,
    #[serde(default)]
    pub publishers: Vec<String>,
    #[serde(default)]
    pub projected_eps: f64,
    #[serde(default)]
    pub last_event_ms: u64,
    #[serde(default)]
    pub subscriber_count: usize,
}

/// Per-topic facts from `/api/topics`. Pure. Rows whose key is not concrete
/// are dropped the same way the snapshot fold drops them.
pub(crate) fn fold_topic_list(list: &TopicList) -> BTreeMap<String, TopicFold> {
    list.topics
        .iter()
        .filter(|r| is_concrete_key(&r.key))
        .map(|r| {
            (
                r.key.clone(),
                TopicFold {
                    eps: r.projected_eps,
                    last_event_ms: (r.last_event_ms > 0).then_some(r.last_event_ms),
                    pubs: r.publishers.len(),
                    subs: r.subscriber_count,
                },
            )
        })
        .collect()
}

/// GET `<endpoint>/api/topics`. `Ok(None)` when panoscope predates the
/// endpoint (404), so the caller can fall back to `/api/snapshot`.
pub(crate) async fn fetch_topic_list(endpoint: &str, token: &str) -> anyhow::Result<Option<TopicList>> {
    let url = format!("{}/api/topics?limit=50000", endpoint.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let mut req = client.get(&url);
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        anyhow::bail!("panoscope /api/topics returned {}", resp.status());
    }
    Ok(Some(resp.json().await?))
}

/// GET `<endpoint>/api/snapshot`. Any transport / decode error returns
/// `Err` so callers degrade rather than fail — the snapshot is an
/// enhancement, not a hard dependency.
pub(crate) async fn fetch_snapshot(endpoint: &str, token: &str) -> anyhow::Result<Snapshot> {
    let url = format!("{}/api/snapshot", endpoint.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let mut req = client.get(&url);
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("panoscope /api/snapshot returned {}", resp.status());
    }
    Ok(resp.json().await?)
}

// ---------------------------------------------------------------------------
// Fetchers — the only impure code
// ---------------------------------------------------------------------------

async fn collect(
    ctx: &super::Ctx,
    args: &FlowArgs,
    filter: Option<&str>,
) -> anyhow::Result<FlowReport> {
    // A missing profile is a structural failure, not a degraded read:
    // `portal_client()` errors ONLY when the profile is absent or
    // unreadable (an unreachable portal still constructs fine), so `?` here
    // gives the house `E_PROFILE_MISSING` / exit 3 without swallowing a
    // genuine portal outage — that one degrades per-tier below.
    let portal = ctx.profile.portal_client()?;
    let token = ctx.profile.portal_token().unwrap_or_default();

    let (declared, declared_status) = if portal.is_enabled() {
        crate::cmd::ui::fetch_declared(&portal).await
    } else {
        (
            Vec::new(),
            crate::cmd::ui::DeclaredFetchStatus {
                not_queried: true,
                ..Default::default()
            },
        )
    };

    let (pano_tier, snap_generated_at) = fetch_panoscope_tier(
        args.snapshot_endpoint.as_deref(),
        &ctx.profile,
        &token,
    )
    .await;
    // Sampled immediately after the snapshot returns, so it is within one
    // round-trip of the instant panoscope stamped `generated_at_ms`. The
    // difference is this machine's skew against the frame's reference clock,
    // which the live tier needs to report ages on the same basis.
    let local_at_snapshot = now_unix_ms();
    let clock_offset_ms = match snap_generated_at {
        Some(g) if g > 0 => g.saturating_sub(local_at_snapshot),
        _ => 0,
    };

    let audit_tier = fetch_audit_tier(ctx, args.since.as_deref(), filter).await;
    let portal_tier = portal_registry_tier(&declared, &declared_status);

    let engaged_unreachable = [&pano_tier, &audit_tier]
        .iter()
        .filter(|t| !matches!(t.state, TierState::Skipped(_)))
        .all(|t| t.state.is_unreachable())
        && [&pano_tier, &audit_tier]
            .iter()
            .any(|t| !matches!(t.state, TierState::Skipped(_)));

    let live_tier = match args.live {
        LiveMode::Never => TierResult::skipped(Source::LiveWindow, "not engaged (--live never)"),
        LiveMode::Always => fetch_live_tier(ctx, filter, args.live_window, clock_offset_ms).await,
        LiveMode::Auto if engaged_unreachable => {
            fetch_live_tier(ctx, filter, args.live_window, clock_offset_ms).await
        }
        LiveMode::Auto => TierResult::skipped(
            Source::LiveWindow,
            "not engaged (--live auto; another observer answered)",
        ),
    };

    // Prefer panoscope's server clock over this machine's — an operator's
    // NTP drift must not silently skew every age on the frame.
    let (as_of_unix_ms, as_of_source) = match snap_generated_at {
        Some(ms) if ms > 0 => (ms, AsOfSource::PanoscopeGeneratedAt),
        _ => (now_unix_ms(), AsOfSource::LocalClock),
    };

    let tiers = vec![pano_tier, audit_tier, live_tier, portal_tier];

    // The command answered nothing at all — that is a failure, not a report.
    if tiers.iter().all(|t| {
        t.state.is_unreachable() || matches!(t.state, TierState::Skipped(_))
    }) && tiers.iter().any(|t| t.state.is_unreachable())
    {
        let causes: Vec<String> = tiers
            .iter()
            .filter(|t| t.state.is_unreachable())
            .map(|t| format!("{}: {}", t.source.label(), t.state.detail()))
            .collect();
        anyhow::bail!(
            "no observer could answer — every engaged source is unreachable ({})",
            causes.join("; ")
        );
    }

    Ok(resolve_flow(&FlowInputs {
        as_of_unix_ms,
        as_of_source,
        fresh_ms: args.fresh.max(0).saturating_mul(1000),
        declared,
        tiers,
        filter: filter.map(String::from),
    }))
}

/// panoscope tier. Returns the tier plus the snapshot's own clock, which
/// the caller uses as the frame's `as-of`.
async fn fetch_panoscope_tier(
    endpoint_override: Option<&str>,
    profile: &crate::profile::Profile,
    token: &str,
) -> (TierResult, Option<i64>) {
    let endpoint = match crate::cmd::inspect::resolve_panoscope_endpoint(endpoint_override, profile)
    {
        Ok(e) => e,
        Err(e) => {
            return (
                TierResult::skipped(Source::Panoscope, format!("no endpoint: {e}")),
                None,
            )
        }
    };
    let to_observations = |folded: BTreeMap<String, TopicFold>| -> BTreeMap<String, TierObservation> {
        folded
            .into_iter()
            .map(|(k, f)| {
                (
                    k,
                    TierObservation {
                        last_ms: f.last_event_ms.map(|ms| ms as i64),
                        eps: Some(f.eps),
                        pubs: (f.pubs > 0).then_some(f.pubs),
                        subs: (f.subs > 0).then_some(f.subs),
                    },
                )
            })
            .collect()
    };

    // `/api/topics` answers per key with nothing grouped away; an older
    // panoscope without it answers 404 and the snapshot fold is used.
    let (generated, topics, state) = match fetch_topic_list(&endpoint, token).await {
        Ok(Some(list)) => {
            let topics = to_observations(fold_topic_list(&list));
            // Never a silent cap: a row limit is a known-incomplete answer.
            let state = if list.truncated {
                TierState::Partial(format!(
                    "{} of {} topics (panoscope's row limit)",
                    topics.len(),
                    list.total
                ))
            } else {
                TierState::Ok(format!("{} topics", topics.len()))
            };
            (list.built_at_ms as i64, topics, state)
        }
        Ok(None) => {
            let snap = match fetch_snapshot(&endpoint, token).await {
                Ok(s) => s,
                Err(e) => {
                    return (
                        TierResult::unreachable(Source::Panoscope, format!("{e}")),
                        None,
                    )
                }
            };
            let fold = fold_snapshot(&snap);
            let folded_edges = fold.folded_edges;
            let folded_topics = fold.folded_topics;
            let topics = to_observations(fold.topics);
            // Never a silent cap: if panoscope folded a fan-out group away,
            // say so on the frame rather than letting the operator read a
            // short list as complete coverage.
            let detail = if folded_edges > 0 {
                format!(
                    "{} topics; {} more folded by panoscope into {} group(s) and not \
                     resolvable per-key",
                    topics.len(),
                    folded_topics,
                    folded_edges
                )
            } else {
                format!("{} topics", topics.len())
            };
            (snap.generated_at_ms as i64, topics, TierState::Ok(detail))
        }
        Err(e) => {
            return (
                TierResult::unreachable(Source::Panoscope, format!("{e}")),
                None,
            )
        }
    };
    (
        TierResult {
            source: Source::Panoscope,
            state,
            horizon_ms: None,
            horizon_note: Some(
                "back to the start of panoscope's configured audit-replay window; \
                 that window is not exposed on the wire, so this client cannot \
                 state where its memory actually begins"
                    .to_string(),
            ),
            topics,
        },
        Some(generated),
    )
}

/// Audit-tail tier — the only observer that can prove a topic has NOT
/// published, and only when it answers completely.
async fn fetch_audit_tier(
    ctx: &super::Ctx,
    since: Option<&str>,
    filter: Option<&str>,
) -> TierResult {
    let Some(since) = since else {
        return TierResult::skipped(
            Source::Audit,
            "not engaged (pass --since <window> to read the audit tail)",
        );
    };
    // Still `**` when nothing is named, and deliberately so: this tier reads
    // the portal's audit API over HTTP, not the fabric. The F-287 fix 8 grant
    // governs what a peer may SUBSCRIBE to on the router; it says nothing about
    // an authenticated read of the recorded tail, which is a different door
    // with its own authorization.
    let pattern = filter.unwrap_or("**");
    let window = match super::replay::fetch_events_detailed(ctx, Some(since), pattern).await {
        Ok(v) => v,
        Err(e) => return TierResult::unreachable(Source::Audit, format!("{e}")),
    };

    let mut topics: BTreeMap<String, TierObservation> = BTreeMap::new();
    for ev in &window.events {
        let row = topics.entry(ev.key_expr.clone()).or_default();
        if let Some(ms) = ev.ts_unix_ms() {
            row.last_ms = Some(row.last_ms.map_or(ms, |cur: i64| cur.max(ms)));
        }
    }

    // The horizon must come from the floor the portal ACTUALLY applied, not
    // from what the operator asked for. The portal silently clamps to 7d and
    // echoes back the effective floor; deriving the horizon from `--since`
    // would print "none in 200h" after examining only 168h — a
    // proof-of-absence claim over 32 hours nobody looked at.
    let effective_floor_ms = rfc3339_to_unix_ms(&window.effective_since);
    let horizon_ms = effective_floor_ms
        .map(|floor| now_unix_ms().saturating_sub(floor))
        .or_else(|| since_to_ms(since).map(|ms| ms.min(AUDIT_MAX_WINDOW_MS)));
    let clamped = was_clamped(since, effective_floor_ms);
    let mut note = "the window the portal actually served (it clamps to a 7d maximum); \
                    excludes volatile/** (ADR 1032)"
        .to_string();
    if clamped {
        note.push_str(" — your --since was clamped");
    }

    if !window.skipped.is_empty() {
        // Same reasoning as truncation: objects the read never saw leave the
        // window unable to prove absence (#1301).
        return TierResult {
            source: Source::Audit,
            state: TierState::Partial(format!(
                "{} of {} audit objects could not be read — narrow --since; {} event(s) visible",
                window.skipped.len(),
                window.object_count,
                window.events.len()
            )),
            horizon_ms,
            horizon_note: Some(note),
            topics,
        };
    }
    if window.truncated {
        // A truncated window cannot support "there was nothing" — it only
        // supports "there was nothing in the part we could see".
        return TierResult {
            source: Source::Audit,
            state: TierState::Partial(format!(
                "window truncated to {} audit objects — narrow --since; {} event(s) visible",
                window.object_count,
                window.events.len()
            )),
            horizon_ms,
            horizon_note: Some(note),
            topics,
        };
    }

    let served = horizon_ms.map_or_else(|| since.to_string(), fmt_horizon);
    TierResult {
        source: Source::Audit,
        state: TierState::Ok(format!(
            "{} event(s) over {served}{}",
            window.events.len(),
            if clamped { " (clamped from your --since)" } else { "" }
        )),
        horizon_ms,
        horizon_note: Some(note),
        topics,
    }
}

/// Did the portal serve a narrower window than the operator asked for?
///
/// Two request shapes, and BOTH comparisons are deliberately
/// clock-independent:
///   - a DURATION is compared against the portal's known maximum, never
///     against the computed horizon — the horizon is measured with this
///     machine's clock against the portal's, so a laptop even slightly
///     behind would report a clamp that never happened;
///   - an RFC3339 TIMESTAMP is compared against the floor the portal echoed
///     back. Both are absolute instants, so again no local clock enters.
///
/// The timestamp arm exists because without it `--since <rfc3339 older than
/// 7d>` was silently clamped with no note. The horizon stayed honest either
/// way (it is derived from the echoed floor) — the operator simply was never
/// told their start point had moved.
fn was_clamped(since: &str, effective_floor_ms: Option<i64>) -> bool {
    match since_to_ms(since) {
        Some(requested_span) => requested_span > AUDIT_MAX_WINDOW_MS,
        None => match (rfc3339_to_unix_ms(since), effective_floor_ms) {
            // Slack absorbs the portal's second-truncation of its echo.
            (Some(requested), Some(served)) => served > requested + 60_000,
            _ => false,
        },
    }
}

/// Parse the portal's RFC3339 `since` echo into unix ms.
fn rfc3339_to_unix_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|t| t.timestamp_millis())
}

/// Bounded live subscribe. Proves "flowing NOW" and nothing else — it can
/// never prove absence, so a silent window leaves rows `unknown`.
/// `clock_offset_ms` converts this machine's clock into the frame's
/// reference clock (panoscope's, when it answered), so the live tier's
/// stamps are comparable with the other tiers' rather than off by the
/// operator's NTP drift.
///
/// It removes SKEW, not ELAPSED TIME: the frame's `as_of` is frozen at the
/// snapshot instant while this tier keeps listening for seconds afterwards,
/// so a sighting here is legitimately newer than `as_of`. The resolver
/// clamps the resulting age at zero.
async fn fetch_live_tier(
    ctx: &super::Ctx,
    filter: Option<&str>,
    window_secs: u64,
    clock_offset_ms: i64,
) -> TierResult {
    let window = window_secs.max(1);
    let pattern = filter.unwrap_or(DEFAULT_LIVE_SCOPE).to_string();
    let fabric_wide = is_fabric_wide_pattern(&pattern);

    // Say it BEFORE the subscription, because the fabric will not say it
    // after. A router refuses a declaration by dropping it: no error reaches
    // the client, the subscriber looks healthy, and the tier reports an honest
    // "nothing heard" that an operator reads as "nothing is flowing".
    if fabric_wide {
        eprintln!(
            "goat flow: `{pattern}` asks the router for the WHOLE keyspace. That is a \
             named grant (zenoh_fabric_wide_read) held by inspection surfaces and \
             reviewed integrations, not by an ordinary identity. If this identity is \
             not named, the router REFUSES the subscription silently and the live tier \
             below reads `0 event(s)` — which is indistinguishable from a quiet fabric. \
             Scope the run (`goat flow <anchor>/**`, `goat flow ops/**`) or ask for the \
             grant."
        );
    }

    let session = match super::pubsub::open_session_readonly(&ctx.profile).await {
        Ok(s) => s,
        Err(e) => return TierResult::unreachable(Source::LiveWindow, format!("{e}")),
    };
    let subscriber = match session.declare_subscriber(pattern.clone()).await {
        Ok(s) => s,
        Err(e) => {
            let _ = session.close().await;
            return TierResult::unreachable(
                Source::LiveWindow,
                format!("declare subscriber on `{pattern}`: {e}"),
            );
        }
    };

    let mut counts: BTreeMap<String, (u64, i64)> = BTreeMap::new();
    let mut total: u64 = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(window);
    let mut capped = false;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            recv = subscriber.recv_async() => {
                match recv {
                    // Count only — never buffer payloads. The tier answers
                    // "did anything arrive", not "what was in it".
                    Ok(sample) => {
                        let key = sample.key_expr().as_str().to_string();
                        let e = counts.entry(key).or_insert((0, 0));
                        e.0 += 1;
                        e.1 = now_unix_ms().saturating_add(clock_offset_ms);
                        total += 1;
                        if total >= LIVE_SAMPLE_CAP {
                            capped = true;
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }
    let _ = session.close().await;

    let topics: BTreeMap<String, TierObservation> = counts
        .into_iter()
        .map(|(k, (n, last))| {
            (
                k,
                TierObservation {
                    last_ms: Some(last),
                    eps: Some(n as f64 / window as f64),
                    ..Default::default()
                },
            )
        })
        .collect();

    let detail = format!(
        "watched {window}s on `{pattern}` · {total} event(s) on {} key(s)",
        topics.len()
    );
    // Two different silences, and the operator cannot tell them apart from the
    // count alone: nothing published, or the declaration was refused. Name the
    // second one whenever it is possible.
    let mut horizon_note = String::from(
        "a silence this short is NOT evidence of absence — this tier can only \
         prove \"flowing now\", never \"last flowed\"",
    );
    if filter.is_none() {
        horizon_note.push_str(
            "; scoped to `ops/**` by default (fabric control + health) — pass a key \
             expression to watch tenant data",
        );
    }
    if fabric_wide && total == 0 {
        horizon_note.push_str(
            "; `0 event(s)` on a fabric-wide pattern ALSO reads this way when the \
             router refused the subscription for want of the named `**` grant",
        );
    }
    TierResult {
        source: Source::LiveWindow,
        state: if capped {
            TierState::Partial(format!("{detail}; hit the {LIVE_SAMPLE_CAP}-sample cap"))
        } else {
            TierState::Ok(detail)
        },
        horizon_ms: Some(window as i64 * 1000),
        horizon_note: Some(horizon_note),
        topics,
    }
}

/// True when `pattern` names the ENTIRE keyspace in one declaration — which is
/// the literal `**`, and nothing else.
///
/// Not "has a leading wildcard": `**/basemap_vendor/**` and `*/geo/track/v1/*` are
/// filters, they are allowed for every peer, and warning about them would train
/// operators to ignore the warning. The ACL grants `**` to named certificates
/// only; every other shape either matches the rule
/// `*/**` or is enumerated in `zenoh_peer_cross_tenant_read`.
fn is_fabric_wide_pattern(pattern: &str) -> bool {
    pattern == "**"
}

/// The portal registry's own `topics.last_seen_at`. Populated the day the
/// audit tailer stamps it (the deferred follow-on named in schema.sql);
/// until then this tier honestly reports that it has no column to read.
///
/// `status` names what the vendor-topic fan-out that built `declared`
/// actually managed to read — a rate-limited/unreachable vendor's topics are
/// silently missing from `declared` itself (there is nothing there to
/// report absence of), so without `status` this tier could not distinguish
/// "the goatnet has nothing registered" from "we couldn't read some/all of
/// the registry" — both used to render as the same generic message.
fn portal_registry_tier(
    declared: &[DeclaredTopic],
    status: &crate::cmd::ui::DeclaredFetchStatus,
) -> TierResult {
    let topics: BTreeMap<String, TierObservation> = declared
        .iter()
        .filter_map(|d| {
            d.last_seen_at.map(|secs| {
                (
                    d.key.clone(),
                    TierObservation {
                        // UNIT BOUNDARY: the portal stores and emits
                        // `last_seen_at` in unix SECONDS (both web surfaces
                        // render it as `new Date(v * 1000)`, and the writer
                        // that will eventually populate it documents "Unix
                        // seconds"). Everything inside the resolver is
                        // milliseconds. Convert here, once, at the edge.
                        last_ms: Some(secs.saturating_mul(1000)),
                        ..Default::default()
                    },
                )
            })
        })
        .collect();

    // The unreachable-vendor caveat, shared by every branch below. Built
    // once so "empty because nothing's registered", "empty because we never
    // asked", and "empty because we asked and it failed" never silently
    // collapse into the same sentence.
    let caveat = if status.not_queried {
        Some(" — no portal configured for this profile, never queried".to_string())
    } else if status.roster_unreachable {
        Some(" — the vendor roster itself was unreachable (portal list_vendors failed)".to_string())
    } else if status.vendors_unreachable > 0 {
        Some(format!(
            " — {}/{} vendor(s) unreachable while listing topics (rate-limited or errored); \
             their declared topics are missing from this read",
            status.vendors_unreachable, status.total_vendors
        ))
    } else {
        None
    };

    if declared.is_empty() {
        return TierResult::skipped(
            Source::PortalRegistry,
            match caveat {
                Some(c) => format!("no topic roster available{c}"),
                // Only say "empty" outright when the read was actually
                // complete — otherwise this is indistinguishable from a
                // portal outage (or, before `not_queried` existed, a portal
                // that was never even asked) that happened to return zero
                // rows.
                None => "no topic roster available (portal reports zero declared topics)".to_string(),
            },
        );
    }
    if topics.is_empty() {
        return TierResult::skipped(
            Source::PortalRegistry,
            format!(
                "none of the {} declared topic(s) carry last_seen_at{}",
                declared.len(),
                caveat.unwrap_or_default()
            ),
        );
    }
    TierResult {
        source: Source::PortalRegistry,
        state: TierState::Ok(format!(
            "{} topic(s) carry last_seen_at{}",
            topics.len(),
            caveat.unwrap_or_default()
        )),
        horizon_ms: None,
        horizon_note: Some("the registry's own monotonic last-publish stamp".to_string()),
        topics,
    }
}

/// Single-topic resolution for `goat topic show`, which needs an answer
/// without a fabric-wide sweep. Engages only the two cheap observers —
/// panoscope (one GET) and the registry stamp already on the `Topic` row
/// (free) — and returns `(last_publish, flow_state)` pre-rendered.
///
/// Deliberately does NOT engage the audit tail or a live subscribe:
/// `topic show` is a one-round-trip metadata read, and silently turning it
/// into a multi-second audit fetch would be a worse surprise than an
/// `unknown`. The `unknown` line names `goat flow --since` as the way to
/// actually check.
pub(crate) async fn last_publish_for(
    ctx: &super::Ctx,
    topic: &panoscope_core::portal::Topic,
) -> (String, String) {
    let token = ctx.profile.portal_token().unwrap_or_default();
    let (pano, generated) = fetch_panoscope_tier(None, &ctx.profile, &token).await;
    let declared = vec![DeclaredTopic {
        key: topic.key_pattern.clone(),
        vendor: topic.vendor_prefix.clone(),
        format: topic.format.clone(),
        schema_ref: topic.schema_ref.clone(),
        description: topic.description.clone(),
        last_seen_at: topic.last_seen_at,
    }];
    // No vendor fan-out happened here (the row came pre-resolved from the
    // caller), so the read is complete by construction.
    let portal_tier =
        portal_registry_tier(&declared, &crate::cmd::ui::DeclaredFetchStatus::default());
    let unreachable_detail = pano
        .state
        .is_unreachable()
        .then(|| format!("panoscope unreachable: {}", pano.state.detail()));

    let (as_of_unix_ms, as_of_source) = match generated {
        Some(ms) if ms > 0 => (ms, AsOfSource::PanoscopeGeneratedAt),
        _ => (now_unix_ms(), AsOfSource::LocalClock),
    };
    let report = resolve_flow(&FlowInputs {
        as_of_unix_ms,
        as_of_source,
        fresh_ms: DEFAULT_FRESH_SECS * 1000,
        declared,
        // Scope the resolve to THIS topic. Without the filter the universe is
        // every key any tier mentions, and picking a row out of it by
        // position would report a different topic's timestamp entirely.
        filter: Some(topic.key_pattern.clone()),
        tiers: vec![pano, portal_tier],
    });

    render_single(&report, &topic.key_pattern, unreachable_detail.as_deref())
}

/// Pick the requested topic's row out of a report and render it for
/// `goat topic show`. Split out from `last_publish_for` so it is testable
/// without HTTP — the untested version of this selected `rows.first()`,
/// which is the alphabetically-first key on the whole fabric rather than
/// the topic the operator asked about.
fn render_single(
    report: &FlowReport,
    key: &str,
    unreachable_detail: Option<&str>,
) -> (String, String) {
    // Match by key, never by position.
    let Some(row) = report.rows.iter().find(|r| r.key == key) else {
        return ("unknown".to_string(), FlowState::Unknown.label().to_string());
    };

    let last = match (row.state, row.last_publish_unix_ms, row.last_age_ms) {
        (FlowState::Unknown, _, _) => {
            let why = unreachable_detail.map(String::from).unwrap_or_else(|| {
                "no observer could answer — `goat flow --since 24h` reads the audit tail"
                    .to_string()
            });
            format!("unknown — {why}")
        }
        (FlowState::NoRecord, _, _) => "no record in the registry's window".to_string(),
        (_, Some(ms), age) => {
            let ts = chrono::DateTime::from_timestamp_millis(ms)
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
                .unwrap_or_else(|| ms.to_string());
            match age {
                Some(a) => format!(
                    "{ts} ({}, via {})",
                    crate::health_wire::fmt_age(a),
                    row.last_source.label()
                ),
                None => ts,
            }
        }
        _ => "unknown".to_string(),
    };
    (last, row.state.label().to_string())
}

/// Best-effort parse of the `--since` window into ms, for the reported
/// horizon. Mirrors the duration grammar `goat replay` accepts; an RFC3339
/// timestamp yields `None` (the horizon is then the timestamp itself, which
/// the detail line already states).
pub(crate) fn since_to_ms(since: &str) -> Option<i64> {
    let s = since.trim();
    if s.is_empty() || s.contains('T') {
        return None;
    }
    let mut total: i64 = 0;
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: i64 = num.parse().ok()?;
        num.clear();
        total += match c {
            's' => n * 1000,
            'm' => n * 60_000,
            'h' => n * 3_600_000,
            'd' => n * 86_400_000,
            _ => return None,
        };
    }
    if !num.is_empty() || total == 0 {
        return None;
    }
    Some(total)
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn paint(color: bool, ansi: &str, s: &str) -> String {
    if color {
        format!("\x1b[{ansi}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub(crate) fn render_frame(report: &FlowReport, color: bool, active_only: bool) -> String {
    let mut out = String::new();

    let as_of = chrono::DateTime::from_timestamp_millis(report.as_of_unix_ms)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| "unknown".to_string());
    let clock = match report.as_of_source {
        AsOfSource::PanoscopeGeneratedAt => "panoscope clock",
        AsOfSource::LocalClock => "local clock",
    };
    out.push_str(&format!(
        "flow · as-of {as_of} ({clock}) · fresh ≤{}s",
        report.fresh_secs
    ));
    if let Some(f) = &report.filter {
        out.push_str(&format!(" · filter {f}"));
    }
    if active_only {
        out.push_str(" · flowing only (pass --all for the full universe)");
    }
    out.push_str("\n\n");

    // `--all` (the historical default) shows every row this report knows
    // about, honesty grading included — that is the whole point of this
    // command and stays true, unfiltered, in `report` itself and in
    // `--json` regardless of this flag. The default human table instead
    // answers "what's actually flowing right now", which is what an
    // operator glancing at the command usually wants; the full
    // declared/idle/no-record/unknown universe is one flag away, not a
    // second command.
    let rows: Vec<&FlowRow> = if active_only {
        report.rows.iter().filter(|r| r.state == FlowState::Flowing).collect()
    } else {
        report.rows.iter().collect()
    };

    if rows.is_empty() {
        let empty = if report.rows.is_empty() {
            // The universe itself is empty — no rows exist to filter.
            match &report.filter {
                Some(f) => format!("(no topics match `{f}`)\n"),
                None => "(no topics)\n".to_string(),
            }
        } else {
            // Rows exist, the default active-only filter just excluded all of
            // them — but "not Flowing" also covers `Unknown` ("no observer
            // could answer"), which must NEVER render as a confirmed absence.
            // Saying "nothing flowing" here would be exactly the misreading
            // this command exists to prevent, just relocated into the
            // active-only headline instead of a per-row state.
            let total = report.rows.len();
            let unknown = report.rows.iter().filter(|r| r.state == FlowState::Unknown).count();
            if unknown == total {
                format!(
                    "(no observer could answer for {total} known topic(s) — state unknown, NOT \
                     confirmed absent; pass --all to see them, or check `sources` below)\n"
                )
            } else if unknown > 0 {
                format!(
                    "({total} topic(s) known, none flowing right now ({unknown} unknown — no \
                     observer could answer for them, not confirmed absent); pass --all to see \
                     them)\n"
                )
            } else {
                format!("({total} topic(s) known, none flowing right now — pass --all to see them)\n")
            }
        };
        out.push_str(&empty);
    } else {
        let key_w = rows.iter().map(|r| r.key.len()).max().unwrap_or(3).max(3);
        let state_w = 9;
        let last_cells: Vec<String> = rows.iter().map(|r| last_cell(r, report)).collect();
        let last_w = last_cells.iter().map(|s| s.len()).max().unwrap_or(4).max(4);

        out.push_str(&format!(
            "{:<key_w$}  {:<state_w$}  {:<last_w$}  {:>8}  {}\n",
            "key", "state", "last", "rate", "src"
        ));
        for (r, last) in rows.iter().zip(last_cells.iter()) {
            let rate = match r.eps {
                Some(v) => format!("{v:.1}/s"),
                None => "—".to_string(),
            };
            let src = if r.last_source == Source::None {
                "—".to_string()
            } else {
                src_label(r.last_source, report)
            };
            // The state cell is padded BEFORE colouring so the escape bytes
            // don't count toward the column width.
            let state_cell = format!("{:<state_w$}", r.state.label());
            out.push_str(&format!(
                "{:<key_w$}  {}  {:<last_w$}  {:>8}  {}\n",
                r.key,
                paint(color, r.state.ansi(), &state_cell),
                last,
                rate,
                src
            ));
        }
    }

    out.push_str("\nsources\n");
    let name_w = report
        .sources
        .iter()
        .map(|s| s.source.label().len())
        .max()
        .unwrap_or(6);
    let state_w = report
        .sources
        .iter()
        .map(|s| s.state.len())
        .max()
        .unwrap_or(6);
    for s in &report.sources {
        let painted = if s.state == "unreachable" {
            paint(color, "33", &format!("{:<state_w$}", s.state))
        } else {
            format!("{:<state_w$}", s.state)
        };
        out.push_str(&format!(
            "  {:<name_w$}  {}  {}\n",
            s.source.label(),
            painted,
            s.detail
        ));
    }

    // The legend explains `unknown`/`no-record`, so it's only worth printing
    // when those states could actually be on screen (`--all`) OR are hidden
    // by the active-only filter — an `unknown` row disappearing from the
    // table is exactly the case the legend exists to prevent operators from
    // misreading as "confirmed nothing," so it must not go quiet right when
    // it matters most.
    let hides_unknown_rows =
        active_only && report.rows.iter().any(|r| r.state == FlowState::Unknown);
    if !active_only || hides_unknown_rows {
        // The legend is the contract. Without it `unknown` reads as "no data",
        // which is the exact misreading this command was built to end.
        out.push_str(
            "\nⓘ unknown = no observer could answer — NOT \"no data\".\n  \
             no-record = an observer that can see backwards looked over its stated horizon\n  \
             and found nothing — an absence verdict always names the horizon it covers.\n",
        );
        // Only suggest `--since` when it would actually help — i.e. the audit
        // tier was never engaged. If it WAS engaged and came back unreachable or
        // partial, telling the operator to do the thing they just did is noise
        // that reads as the command not having noticed. (An operator-gated audit
        // tail that rejects the token must not produce a frame asking for
        // `--since 24h` on a run that passed it.)
        // (`match` rather than `is_none_or`: that is stable since 1.82 and this
        // crate's MSRV is 1.75.)
        let audit_not_engaged = match report.sources.iter().find(|s| s.source == Source::Audit) {
            Some(s) => s.state == "skipped",
            None => true,
        };
        if !report.sources.iter().any(|s| s.can_prove_absence) && audit_not_engaged {
            out.push_str(
                "  no engaged observer can prove absence — add `--since 24h` to read the audit tail.\n",
            );
        }
    }
    out
}

/// The `last` cell. A `no-record` row renders the PROVING TIER'S HORIZON
/// ("none in 24h"), never the word "never" — the horizon is the honest
/// scope of the claim.
fn last_cell(r: &FlowRow, report: &FlowReport) -> String {
    match (r.state, r.last_age_ms) {
        (FlowState::NoRecord, _) => {
            let horizon = report
                .sources
                .iter()
                .find(|s| s.source == r.last_source)
                .and_then(|s| s.horizon_ms);
            match horizon {
                Some(ms) => format!("none in {}", fmt_horizon(ms)),
                None => "none in window".to_string(),
            }
        }
        (_, Some(age)) => crate::health_wire::fmt_age(age),
        _ => "—".to_string(),
    }
}

/// The live tier's label carries its window, so a `live` attribution can
/// never be read as a claim about anything outside it.
fn src_label(source: Source, report: &FlowReport) -> String {
    if source == Source::LiveWindow {
        if let Some(ms) = report
            .sources
            .iter()
            .find(|s| s.source == Source::LiveWindow)
            .and_then(|s| s.horizon_ms)
        {
            return format!("live({}s)", ms / 1000);
        }
    }
    source.label().to_string()
}

/// Render a horizon in the coarsest unit that describes it to within ~1%.
///
/// Horizons are DERIVED — `now` minus the portal's second-truncated floor —
/// so they land on values like 86_413s rather than exactly 24h. Requiring an
/// exact modulo printed `none in 86413s` where the operator asked for `24h`,
/// which reads as a different window rather than the same one measured
/// honestly. Rounding keeps the value truthful and the label legible.
fn fmt_horizon(ms: i64) -> String {
    let s = (ms + 500) / 1000;
    for (unit, secs) in [("d", 86_400i64), ("h", 3_600), ("m", 60)] {
        if s >= secs {
            let n = (s + secs / 2) / secs;
            // Only claim the coarser unit when rounding to it stays within
            // 1% of the real span; otherwise drop to the finer one.
            if (n * secs - s).abs() * 100 <= s {
                return format!("{n}{unit}");
            }
        }
    }
    format!("{s}s")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs_last(ms: i64) -> TierObservation {
        TierObservation {
            last_ms: Some(ms),
            ..Default::default()
        }
    }

    fn tier(source: Source, state: TierState, topics: &[(&str, TierObservation)]) -> TierResult {
        TierResult {
            source,
            state,
            horizon_ms: Some(86_400_000),
            horizon_note: None,
            topics: topics
                .iter()
                .map(|(k, o)| (k.to_string(), o.clone()))
                .collect(),
        }
    }

    fn inputs(tiers: Vec<TierResult>, declared: &[&str]) -> FlowInputs {
        FlowInputs {
            as_of_unix_ms: 1_000_000,
            as_of_source: AsOfSource::LocalClock,
            fresh_ms: 60_000,
            declared: declared
                .iter()
                .map(|k| DeclaredTopic {
                    key: k.to_string(),
                    vendor: k.split('/').next().unwrap_or("").to_string(),
                    format: String::new(),
                    schema_ref: String::new(),
                    description: String::new(),
                    last_seen_at: None,
                })
                .collect(),
            tiers,
            filter: None,
        }
    }

    fn row<'a>(r: &'a FlowReport, key: &str) -> &'a FlowRow {
        r.rows.iter().find(|x| x.key == key).unwrap()
    }

    // --- the case strict fallback gets wrong -------------------------------

    #[test]
    fn audit_answers_when_panoscope_is_silent_after_a_restart() {
        // panoscope has no observation of this topic (sampler drop, or it
        // fell outside panoscope's replay window); the audit tail still holds
        // the row from 2h ago. Strict panoscope-first fallback would print
        // "no data" here.
        let report = resolve_flow(&inputs(
            vec![
                tier(Source::Panoscope, TierState::Ok("0 topics".into()), &[]),
                tier(
                    Source::Audit,
                    TierState::Ok("1 event".into()),
                    &[("v/t/v1", obs_last(1_000_000 - 7_200_000))],
                ),
            ],
            &["v/t/v1"],
        ));
        let r = row(&report, "v/t/v1");
        assert_eq!(r.state, FlowState::Idle);
        assert_eq!(r.last_source, Source::Audit);
        assert_eq!(r.last_publish_unix_ms, Some(1_000_000 - 7_200_000));
    }

    #[test]
    fn last_is_the_max_across_tiers_not_the_first_to_answer() {
        let report = resolve_flow(&inputs(
            vec![
                tier(
                    Source::Panoscope,
                    TierState::Ok("1".into()),
                    &[("v/t/v1", obs_last(990_000))],
                ),
                tier(
                    Source::Audit,
                    TierState::Ok("1".into()),
                    &[("v/t/v1", obs_last(500_000))],
                ),
            ],
            &[],
        ));
        let r = row(&report, "v/t/v1");
        assert_eq!(r.last_publish_unix_ms, Some(990_000));
        assert_eq!(r.last_source, Source::Panoscope);
        assert_eq!(r.observed_by, vec![Source::Panoscope, Source::Audit]);
    }

    // --- the honesty vocabulary -------------------------------------------

    #[test]
    fn unreachable_panoscope_yields_unknown_never_idle() {
        let report = resolve_flow(&inputs(
            vec![TierResult::unreachable(Source::Panoscope, "503")],
            &["v/t/v1"],
        ));
        assert_eq!(row(&report, "v/t/v1").state, FlowState::Unknown);
    }

    #[test]
    fn silent_topic_is_unknown_when_no_tier_can_prove_absence() {
        // panoscope answered fine — it just has nothing on this key. It
        // never states how far back it can see, so its silence bounds
        // nothing: this is NOT "no-record".
        let report = resolve_flow(&inputs(
            vec![tier(Source::Panoscope, TierState::Ok("0 topics".into()), &[])],
            &["v/t/v1"],
        ));
        assert_eq!(row(&report, "v/t/v1").state, FlowState::Unknown);
    }

    // --- regressions from the audit --------------------------

    #[test]
    fn topic_show_selects_the_requested_key_not_the_first_row() {
        // REGRESSION: `last_publish_for` used `rows.first()` on an unfiltered
        // report, so `goat topic show zeta/…` printed the timestamp of the
        // alphabetically-first key on the whole fabric. Confidently wrong is
        // worse than the `publish_count: 0` it replaced.
        let report = resolve_flow(&inputs(
            vec![tier(
                Source::Panoscope,
                TierState::Ok("2 topics".into()),
                &[
                    ("acme/alerts/v1", obs_last(1_000_000 - 3_000)),
                    ("zeta/tracks/v1", obs_last(1_000_000 - 21_600_000)),
                ],
            )],
            &[],
        ));
        let (last, state) = render_single(&report, "zeta/tracks/v1", None);
        assert_eq!(state, "idle");
        assert!(last.contains("6h ago"), "got: {last}");
        // The other topic's freshness must not leak in.
        assert!(!last.contains("3s ago"), "leaked acme/alerts/v1: {last}");
    }

    #[test]
    fn topic_show_is_unknown_when_the_key_is_absent_from_the_report() {
        let report = resolve_flow(&inputs(vec![], &[]));
        let (last, state) = render_single(&report, "v/missing/v1", None);
        assert_eq!(state, "unknown");
        assert_eq!(last, "unknown");
    }

    #[test]
    fn audit_never_proves_absence_for_volatile_keys_it_cannot_see() {
        // REGRESSION: `proves_absence()` is a whole-tier property but was
        // applied per-key. The audit tail is `Ok` across its window while
        // being structurally blind to `volatile/**` (ADR 1032), so every
        // volatile topic was stamped `no-record` — the exact
        // "we could not see" -> "there was nothing" collapse this command
        // exists to prevent.
        let report = resolve_flow(&inputs(
            vec![tier(Source::Audit, TierState::Ok("0 events".into()), &[])],
            &["volatile/acme/stream/x", "acme/persisted/v1"],
        ));
        assert_eq!(
            row(&report, "volatile/acme/stream/x").state,
            FlowState::Unknown,
            "the audit tail cannot see volatile/** — its silence proves nothing"
        );
        // A persisted key in the same run is still legitimately provable.
        assert_eq!(row(&report, "acme/persisted/v1").state, FlowState::NoRecord);
    }

    #[test]
    fn everything_except_star_star_is_not_a_key_expression() {
        // The rule you reach for when you want "any wildcard filter, but not
        // the whole keyspace" is `**/*/**`. It reads correctly and it is NOT A
        // KEY EXPRESSION: zenoh canonizes `**/*` to `*/**`, so the string is
        // refused — by a publisher, by a subscriber, and by the router's ACL
        // config, whose `key_exprs` deserialize as OwnedKeyExpr. A router
        // handed that rule does not start.
        //
        // This test exists because the ACL design turned on the question and a
        // string-level evaluator says nothing about it. If a future zenoh makes
        // `**/*/**` canonical, this test fails and the ACL can be simplified —
        // that is exactly when someone should look again.
        use zenoh::key_expr::{KeyExpr, OwnedKeyExpr};
        assert!(KeyExpr::new("**/*/**").is_err(), "**/*/** must not parse");
        assert!(serde_json::from_str::<OwnedKeyExpr>("\"**/*/**\"").is_err());
        assert_eq!(
            KeyExpr::autocanonize("**/*/**".to_string()).unwrap().as_str(),
            "*/**",
            "and its canon form is */**, which refuses every **-leading filter"
        );
        assert!(KeyExpr::new("**/**").is_err(), "**/** canonizes to **");
        // What IS expressible: the whole keyspace, one chunk down, and a
        // cross-tenant filter naming a literal chunk.
        for ok in ["**", "*/**", "**/basemap_vendor/**", "**/airtracks/v1/*", "*/geo/track/v1/*"] {
            assert!(KeyExpr::new(ok).is_ok(), "{ok} should parse");
        }
    }

    #[test]
    fn only_literal_star_star_reads_as_the_whole_keyspace() {
        // The warning fires for the ONE shape the router grants by name. It
        // must not fire for a filter: `**/basemap_vendor/**` is one vendor across
        // tenants, `*/geo/track/v1/*` is one topic across tenants, and both are
        // allowed for every peer. A warning on those
        // would be noise, and noise is how a real warning gets ignored.
        assert!(is_fabric_wide_pattern("**"));
        for p in [
            "*/**",
            "**/basemap_vendor/**",
            "**/airtracks/v1/*",
            "*/geo/track/v1/*",
            "*/*/*/health/v1",
            "ops/**",
            "acme/*/tracks/v1",
        ] {
            assert!(!is_fabric_wide_pattern(p), "{p} is a filter, not the keyspace");
        }
    }

    #[test]
    fn the_default_live_scope_needs_no_grant_and_still_sees_everything() {
        // `*/**` matches every key with at least one chunk — everything anyone
        // publishes — and it is what the peer read rule itself grants, so a
        // bare `goat flow` covers the fabric without holding the named `**`
        // grant. The regression this guards: quietly restoring `**` as the
        // default would put every run back among the subscribers that stall a router,
        // and would return nothing, silently, for every operator who is not named.
        assert!(!is_fabric_wide_pattern(DEFAULT_LIVE_SCOPE));
        assert_eq!(DEFAULT_LIVE_SCOPE, "*/**");
    }

    #[test]
    fn portal_registry_converts_seconds_to_milliseconds() {
        // REGRESSION: the portal stores/emits `last_seen_at` in unix SECONDS
        // (both web surfaces render it as `v * 1000`); the resolver is all
        // milliseconds. Copying it across unconverted dated every stamp to
        // 1970.
        let declared = vec![DeclaredTopic {
            key: "v/t/v1".into(),
            vendor: "v".into(),
            format: String::new(),
            schema_ref: String::new(),
            description: String::new(),
            last_seen_at: Some(1_786_250_531), // 10-digit => seconds
        }];
        let t = portal_registry_tier(&declared, &crate::cmd::ui::DeclaredFetchStatus::default());
        assert!(t.state.is_ok());
        assert_eq!(
            t.topics.get("v/t/v1").unwrap().last_ms,
            Some(1_786_250_531_000)
        );
    }

    // --- regressions from the pre-push gate --------------------

    #[test]
    fn is_concrete_key_rejects_every_wildcard_shape() {
        assert!(is_concrete_key("acme/obs/v1"));
        assert!(!is_concrete_key("**")); // the audit subscriber's default
        assert!(!is_concrete_key("*"));
        assert!(!is_concrete_key("acme/*/v1")); // portal permits this shape
        assert!(!is_concrete_key("acme/**"));
        assert!(!is_concrete_key("acme/entity/*/v1"));
        // A folded display string is not even a key expression.
        assert!(!is_concrete_key("15f1/aircraft/*/v1 (folded x270)"));
    }

    #[test]
    fn node_arrays_never_admit_wildcards_or_fold_strings_as_topics() {
        // panoscope runs `publishes` through the same singleton fold as the
        // edges, and copies `subscribes` verbatim — and the audit subscriber
        // subscribes to `**`. Admitting either invents a topic; `**` is the
        // dangerous one because it IS a legal key expression and so passes
        // every filter.
        let snap = Snapshot {
            nodes: vec![SnapNode {
                peer_id: "p".into(),
                display_name: None,
                vendor: None,
                role_class: None,
                publishes: vec![
                    "acme/real/v1".into(),
                    "15f1/aircraft/*/v1 (folded x270)".into(),
                ],
                subscribes: vec!["**".into(), "acme/real/v1".into()],
            }],
            edges: vec![],
            generated_at_ms: 1_000,
        };
        let f = fold_snapshot(&snap);
        assert_eq!(
            f.topics.keys().collect::<Vec<_>>(),
            vec!["acme/real/v1"],
            "only the one real key may become a topic row"
        );
        // The fold's hidden coverage is disclosed, not silently dropped.
        assert_eq!(f.folded_topics, 270);
    }

    #[test]
    fn a_wildcard_row_can_never_be_stamped_no_record() {
        // Before: a declared `acme/entity/*/v1` found no exact tier entry, so
        // the audit tier's `covers()` returned true and the row rendered
        // `no-record` while its own children were flowing.
        let report = resolve_flow(&inputs(
            vec![tier(
                Source::Audit,
                TierState::Ok("1 event".into()),
                &[("acme/entity/abc/v1", obs_last(1_000_000 - 3_000))],
            )],
            &["acme/entity/*/v1"],
        ));
        let pattern = row(&report, "acme/entity/*/v1");
        // Resolved through the children it covers — flowing, not no-record.
        assert_eq!(pattern.state, FlowState::Flowing);
        assert_eq!(pattern.last_publish_unix_ms, Some(1_000_000 - 3_000));
        // …and the concrete child still stands on its own.
        assert_eq!(row(&report, "acme/entity/abc/v1").state, FlowState::Flowing);
    }

    #[test]
    fn an_unobserved_wildcard_row_is_unknown_not_no_record() {
        let report = resolve_flow(&inputs(
            vec![tier(Source::Audit, TierState::Ok("0 events".into()), &[])],
            &["acme/entity/*/v1"],
        ));
        assert_eq!(
            row(&report, "acme/entity/*/v1").state,
            FlowState::Unknown,
            "no observer is keyed by patterns, so silence proves nothing"
        );
    }

    #[test]
    fn a_live_sighting_newer_than_the_frame_clock_never_ages_negative() {
        // `as_of` freezes at the snapshot instant while the live tier keeps
        // listening, so a fresh sighting is legitimately NEWER than as_of.
        // Left negative, `fmt_age` renders `—` — the same glyph as "no
        // timestamp at all", so the freshest row would read as the emptiest.
        let report = resolve_flow(&inputs(
            vec![tier(
                Source::LiveWindow,
                TierState::Ok("watched 30s".into()),
                &[("v/t/v1", obs_last(1_000_000 + 33_000))],
            )],
            &[],
        ));
        let r = row(&report, "v/t/v1");
        assert_eq!(r.last_age_ms, Some(0));
        assert_eq!(r.state, FlowState::Flowing);
        assert!(crate::health_wire::fmt_age(r.last_age_ms.unwrap()) != "—");
    }

    #[test]
    fn an_rfc3339_since_older_than_the_portal_max_is_reported_as_clamped() {
        // served floor the portal moved the start
        // point a month forward. Previously this produced no note at all,
        // because the clamp check only understood durations.
        let requested = "2026-07-01T00:00:00Z";
        let served = rfc3339_to_unix_ms("2026-08-02T00:00:00Z");
        assert!(was_clamped(requested, served));

        // Inside the window: served floor matches the request, no note.
        let ok = "2026-08-08T00:00:00Z";
        assert!(!was_clamped(ok, rfc3339_to_unix_ms(ok)));

        // Second-truncation in the echo must not read as a clamp.
        assert!(!was_clamped(ok, rfc3339_to_unix_ms(ok).map(|m| m + 999)));

        // Durations still resolve against the portal's known maximum, with
        // no clock involved.
        assert!(was_clamped("8d", None));
        assert!(!was_clamped("24h", None));
        assert!(!was_clamped("7d", None)); // exactly the maximum

        // No echo and an unparseable request — claim nothing.
        assert!(!was_clamped("garbage", None));
    }

    #[test]
    fn fold_count_reads_the_hidden_topic_count_off_the_display_string() {
        assert_eq!(fold_count("15f1/aircraft/*/v1 (folded x270)"), 270);
        assert_eq!(fold_count("**"), 0);
        assert_eq!(fold_count("acme/real/v1"), 0);
    }

    #[test]
    fn fold_topic_list_keeps_every_concrete_key_and_its_counts() {
        let list: TopicList = serde_json::from_value(serde_json::json!({
            "generation": 7,
            "built_at_ms": 1_000,
            "total": 3,
            "truncated": false,
            "topics": [
                {"key": "acme/cam/chunk/0001", "publishers": ["p1"], "projected_eps": 2.5,
                 "total_events": 10, "last_event_ms": 900, "subscriber_count": 2},
                {"key": "acme/shared/v1", "publishers": ["p1", "p2"], "projected_eps": 1.0,
                 "last_event_ms": 0, "subscriber_count": 0},
                {"key": "acme/**", "publishers": ["p3"]}
            ]
        }))
        .unwrap();
        let f = fold_topic_list(&list);
        assert_eq!(f.len(), 2, "a wildcard row is not a topic");
        let chunk = &f["acme/cam/chunk/0001"];
        assert_eq!((chunk.eps, chunk.last_event_ms, chunk.pubs, chunk.subs), (2.5, Some(900), 1, 2));
        let shared = &f["acme/shared/v1"];
        assert_eq!(shared.pubs, 2);
        assert_eq!(shared.last_event_ms, None, "0 is unobserved, never epoch");
    }

    #[test]
    fn fold_snapshot_skips_folded_edges_and_reports_what_it_hid() {
        let snap = Snapshot {
            nodes: vec![],
            edges: vec![
                SnapEdge {
                    topic: "acme/real/v1".into(),
                    ema_eps: 9.0,
                    projected_eps: Some(2.0),
                    last_event_ms: 500,
                    folded_singletons_count: None,
                },
                SnapEdge {
                    topic: "org11x/lattice/entity/track/* (folded x19500)".into(),
                    ema_eps: 40.0,
                    projected_eps: Some(40.0),
                    last_event_ms: 900,
                    folded_singletons_count: Some(19_500),
                },
            ],
            generated_at_ms: 1_000,
        };
        let f = fold_snapshot(&snap);
        // The synthetic row is not a topic and must not appear as one…
        assert_eq!(f.topics.len(), 1);
        assert!(f.topics.contains_key("acme/real/v1"));
        // …but the coverage it hid is disclosed rather than silently dropped.
        assert_eq!(f.folded_edges, 1);
        assert_eq!(f.folded_topics, 19_500);
        // The real row uses the decayed rate, not the raw EMA.
        assert_eq!(f.topics["acme/real/v1"].eps, 2.0);
        assert_eq!(f.topics["acme/real/v1"].last_event_ms, Some(500));
    }

    #[test]
    fn org_falls_back_to_the_key_prefix_when_the_declared_vendor_is_empty() {
        let mut i = inputs(vec![], &[]);
        i.declared = vec![DeclaredTopic {
            key: "acme/t/v1".into(),
            vendor: String::new(), // portal row with no org prefix
            format: String::new(),
            schema_ref: String::new(),
            description: String::new(),
            last_seen_at: None,
        }];
        let report = resolve_flow(&i);
        assert_eq!(row(&report, "acme/t/v1").org.as_deref(), Some("acme"));
    }

    #[test]
    fn portal_registry_distinguishes_no_roster_from_no_stamps() {
        // Saying "no row carried last_seen_at" when the portal was
        // unreachable asserts something about rows we never saw.
        let none = portal_registry_tier(&[], &crate::cmd::ui::DeclaredFetchStatus::default());
        assert!(none.state.detail().contains("no topic roster available"));

        let declared = vec![DeclaredTopic {
            key: "v/t/v1".into(),
            vendor: "v".into(),
            format: String::new(),
            schema_ref: String::new(),
            description: String::new(),
            last_seen_at: None,
        }];
        let unstamped = portal_registry_tier(&declared, &crate::cmd::ui::DeclaredFetchStatus::default());
        assert!(unstamped.state.detail().contains("carry last_seen_at"));
    }

    #[test]
    fn portal_registry_tier_names_a_disabled_portal_distinctly_from_a_failed_query() {
        // REGRESSION: `!portal.is_enabled()` used to pass the plain default
        // status, and the empty-declared branch's fallback text ("portal
        // reports zero declared topics") asserts a query outcome for a
        // portal that was never asked at all.
        let status = crate::cmd::ui::DeclaredFetchStatus { not_queried: true, ..Default::default() };
        let t = portal_registry_tier(&[], &status);
        let detail = t.state.detail();
        assert!(detail.contains("never queried"), "got: {detail}");
        assert!(!detail.contains("reports zero declared topics"), "got: {detail}");
    }

    #[test]
    fn portal_registry_tier_names_an_unreachable_vendor_roster() {
        let status =
            crate::cmd::ui::DeclaredFetchStatus { roster_unreachable: true, ..Default::default() };
        let t = portal_registry_tier(&[], &status);
        assert!(t.state.detail().contains("vendor roster itself was unreachable"));
    }

    #[test]
    fn portal_registry_tier_names_partial_vendor_failures_on_an_otherwise_ok_read() {
        // A read that DID produce topics must still say so when some vendors
        // failed — an incomplete-but-present read is the exact case that
        // used to render identically to a complete one.
        let declared = vec![DeclaredTopic {
            key: "v/t/v1".into(),
            vendor: "v".into(),
            format: String::new(),
            schema_ref: String::new(),
            description: String::new(),
            last_seen_at: Some(1_786_250_000),
        }];
        let status = crate::cmd::ui::DeclaredFetchStatus {
            vendors_unreachable: 3,
            total_vendors: 10,
            ..Default::default()
        };
        let t = portal_registry_tier(&declared, &status);
        let detail = t.state.detail();
        assert!(detail.contains("carry last_seen_at"));
        assert!(detail.contains("3/10 vendor(s) unreachable"), "got: {detail}");

        // The complementary property: a genuinely complete read (the default
        // status) must NOT print any caveat at all.
        let clean = portal_registry_tier(&declared, &crate::cmd::ui::DeclaredFetchStatus::default());
        assert!(!clean.state.detail().contains("unreachable"), "got: {}", clean.state.detail());
    }

    #[test]
    fn folded_panoscope_edges_are_excluded_and_counted_not_treated_as_topics() {
        // REGRESSION: panoscope folds fan-out groups into a synthetic edge
        // whose `topic` is a DISPLAY STRING, not a key expression. Admitting
        // it produced a bogus row unfiltered, and — because the string fails
        // to parse — silently dropped the whole namespace when filtered.
        let folded = SnapEdge {
            topic: "org11x/lattice/entity/track/* (folded x19500)".into(),
            ema_eps: 5.0,
            projected_eps: Some(5.0),
            last_event_ms: 900_000,
            folded_singletons_count: Some(19_500),
        };
        assert!(folded.is_folded());
        // The synthetic string is genuinely not a key expression, which is
        // why it can never match a filter.
        assert!(!key_intersects("org11x/**", &folded.topic));
    }

    #[test]
    fn audit_ok_and_empty_proves_no_record() {
        let report = resolve_flow(&inputs(
            vec![tier(Source::Audit, TierState::Ok("0 events".into()), &[])],
            &["v/t/v1"],
        ));
        let r = row(&report, "v/t/v1");
        assert_eq!(r.state, FlowState::NoRecord);
        // Attributed to the proving tier so the renderer can state its horizon.
        assert_eq!(r.last_source, Source::Audit);
    }

    #[test]
    fn truncated_audit_window_can_never_prove_no_record() {
        let report = resolve_flow(&inputs(
            vec![tier(
                Source::Audit,
                TierState::Partial("window truncated".into()),
                &[],
            )],
            &["v/t/v1"],
        ));
        assert_eq!(row(&report, "v/t/v1").state, FlowState::Unknown);
        assert!(!report.sources[0].can_prove_absence);
    }

    #[test]
    fn a_live_window_alone_never_proves_no_record() {
        let report = resolve_flow(&inputs(
            vec![tier(
                Source::LiveWindow,
                TierState::Ok("watched 3s · 0 events".into()),
                &[],
            )],
            &["v/t/v1"],
        ));
        assert_eq!(row(&report, "v/t/v1").state, FlowState::Unknown);
    }

    #[test]
    fn fresh_boundary_splits_flowing_from_idle() {
        let report = resolve_flow(&inputs(
            vec![tier(
                Source::Panoscope,
                TierState::Ok("2".into()),
                &[
                    ("v/fresh/v1", obs_last(1_000_000 - 59_000)),
                    ("v/stale/v1", obs_last(1_000_000 - 61_000)),
                ],
            )],
            &[],
        ));
        assert_eq!(row(&report, "v/fresh/v1").state, FlowState::Flowing);
        assert_eq!(row(&report, "v/stale/v1").state, FlowState::Idle);
    }

    // --- tie-break + field preference -------------------------------------

    #[test]
    fn chaining_fold_and_two_pass_disagree_and_two_pass_is_right() {
        // THE discriminating case. Audit@base, Live@base+1.5s, Pano@base+3s,
        // tolerance 2s.
        //   two-pass: max = base+3s; only Pano and Live are within tolerance
        //             of it; Pano outranks Live  -> Panoscope.
        //   chaining: Audit(rank 4) absorbs Live (1.5s away), then absorbs
        //             Pano (1.5s from the running max) -> Audit, even though
        //             Audit's own reading is 3s stale.
        // Asserting Panoscope is what pins the two-pass rewrite.
        let base = 1_000_000i64;
        let report = resolve_flow(&inputs(
            vec![
                tier(Source::Audit, TierState::Ok("1".into()), &[("v/t/v1", obs_last(base))]),
                tier(
                    Source::LiveWindow,
                    TierState::Ok("1".into()),
                    &[("v/t/v1", obs_last(base + 1_500))],
                ),
                tier(
                    Source::Panoscope,
                    TierState::Ok("1".into()),
                    &[("v/t/v1", obs_last(base + 3_000))],
                ),
            ],
            &[],
        ));
        let r = row(&report, "v/t/v1");
        assert_eq!(r.last_source, Source::Panoscope, "a chaining fold returns Audit here");
        assert_eq!(r.last_publish_unix_ms, Some(base + 3_000));
    }

    #[test]
    fn near_ties_do_not_chain_across_the_tolerance() {
        // A at t, live at t+1.5s, audit at t+3s. Live is within tolerance of
        // both, so a single running-max fold would let attribution walk from
        // panoscope through live to audit even though panoscope and audit are
        // 3s apart. Only tiers within tolerance OF THE MAX may compete.
        let base = 1_000_000i64;
        let report = resolve_flow(&inputs(
            vec![
                tier(
                    Source::Panoscope,
                    TierState::Ok("1".into()),
                    &[("v/t/v1", obs_last(base))],
                ),
                tier(
                    Source::LiveWindow,
                    TierState::Ok("1".into()),
                    &[("v/t/v1", obs_last(base + 1_500))],
                ),
                tier(
                    Source::Audit,
                    TierState::Ok("1".into()),
                    &[("v/t/v1", obs_last(base + 3_000))],
                ),
            ],
            &[],
        ));
        let r = row(&report, "v/t/v1");
        assert_eq!(r.last_publish_unix_ms, Some(base + 3_000));
        // audit IS the max here, so it wins on its own merits…
        assert_eq!(r.last_source, Source::Audit);

        // …and with the roles reversed, the 3s-stale audit tier must NOT
        // capture attribution from the actually-freshest observer.
        let report2 = resolve_flow(&inputs(
            vec![
                tier(
                    Source::Audit,
                    TierState::Ok("1".into()),
                    &[("v/t/v1", obs_last(base))],
                ),
                tier(
                    Source::LiveWindow,
                    TierState::Ok("1".into()),
                    &[("v/t/v1", obs_last(base + 3_000))],
                ),
            ],
            &[],
        ));
        let r2 = row(&report2, "v/t/v1");
        assert_eq!(r2.last_publish_unix_ms, Some(base + 3_000));
        assert_eq!(r2.last_source, Source::LiveWindow);
    }

    #[test]
    fn near_tie_breaks_toward_the_more_durable_evidence() {
        let report = resolve_flow(&inputs(
            vec![
                tier(
                    Source::Panoscope,
                    TierState::Ok("1".into()),
                    &[("v/t/v1", obs_last(999_000))],
                ),
                tier(
                    Source::Audit,
                    TierState::Ok("1".into()),
                    &[("v/t/v1", obs_last(998_500))],
                ),
            ],
            &[],
        ));
        let r = row(&report, "v/t/v1");
        // Within the 2s skew tolerance → audit wins attribution, and the
        // later of the two timestamps is kept.
        assert_eq!(r.last_source, Source::Audit);
        assert_eq!(r.last_publish_unix_ms, Some(999_000));
    }

    #[test]
    fn eps_prefers_panoscope_while_last_still_takes_the_freshest_timestamp() {
        // The two fields resolve by DIFFERENT rules, which is exactly why
        // `evidence_rank` is not a blanket `Ord`: for `eps` panoscope's real
        // EMA beats a 3s sample count, but for `last` a live sighting from
        // 5s later is simply more recent and wins.
        let pano = TierObservation {
            eps: Some(12.0),
            ..obs_last(995_000)
        };
        let live = TierObservation {
            eps: Some(3.0),
            ..obs_last(1_000_000)
        };
        let report = resolve_flow(&inputs(
            vec![
                tier(Source::Panoscope, TierState::Ok("1".into()), &[("v/t/v1", pano)]),
                tier(Source::LiveWindow, TierState::Ok("1".into()), &[("v/t/v1", live)]),
            ],
            &[],
        ));
        let r = row(&report, "v/t/v1");
        assert_eq!(r.eps_source, Some(Source::Panoscope));
        assert_eq!(r.eps, Some(12.0));
        assert_eq!(r.last_source, Source::LiveWindow);
        assert_eq!(r.last_publish_unix_ms, Some(1_000_000));
    }

    #[test]
    fn a_live_sighting_inside_the_skew_tolerance_defers_to_durable_evidence() {
        // 500ms apart is the SAME publish seen twice, not a newer one — so
        // attribution goes to the more durable observer while the later of
        // the two timestamps is kept.
        let report = resolve_flow(&inputs(
            vec![
                tier(
                    Source::Panoscope,
                    TierState::Ok("1".into()),
                    &[("v/t/v1", obs_last(999_000))],
                ),
                tier(
                    Source::LiveWindow,
                    TierState::Ok("1".into()),
                    &[("v/t/v1", obs_last(999_500))],
                ),
            ],
            &[],
        ));
        let r = row(&report, "v/t/v1");
        assert_eq!(r.last_source, Source::Panoscope);
        assert_eq!(r.last_publish_unix_ms, Some(999_500));
    }

    // --- the JSON contract -------------------------------------------------

    #[test]
    fn unknown_row_omits_last_publish_entirely() {
        // The whole point: never 0, never null — absent. This is the
        // structural fix for `publish_count: 0`.
        let report = resolve_flow(&inputs(
            vec![TierResult::unreachable(Source::Panoscope, "503")],
            &["v/t/v1"],
        ));
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("last_publish_unix_ms"));
        assert!(!json.contains("last_age_ms"));
        assert!(json.contains("\"state\":\"unknown\""));
    }

    #[test]
    fn sources_array_lists_every_tier_including_skipped_ones() {
        let report = resolve_flow(&inputs(
            vec![
                tier(Source::Panoscope, TierState::Ok("1 topic".into()), &[]),
                TierResult::skipped(Source::Audit, "not engaged"),
                TierResult::skipped(Source::LiveWindow, "not engaged"),
                TierResult::skipped(Source::PortalRegistry, "no column"),
            ],
            &[],
        ));
        assert_eq!(report.sources.len(), 4);
        let audit = report.sources.iter().find(|s| s.source == Source::Audit).unwrap();
        assert_eq!(audit.state, "skipped");
        assert!(!audit.can_prove_absence);
        assert!(audit.provides.is_empty());
    }

    // --- filtering ---------------------------------------------------------

    #[test]
    fn filter_intersects_key_expressions_rather_than_prefix_matching() {
        let mut i = inputs(
            vec![tier(
                Source::Panoscope,
                TierState::Ok("3".into()),
                &[
                    ("org11x/obs/v1", obs_last(999_000)),
                    ("org11x-other/obs/v1", obs_last(999_000)),
                    ("acme/obs/v1", obs_last(999_000)),
                ],
            )],
            &[],
        );
        i.filter = Some("org11x/**".to_string());
        let report = resolve_flow(&i);
        let keys: Vec<&str> = report.rows.iter().map(|r| r.key.as_str()).collect();
        // `org11x-other/...` must NOT match — a prefix match would catch it.
        assert_eq!(keys, ["org11x/obs/v1"]);
    }

    #[test]
    fn declared_but_unobserved_topics_still_appear() {
        let report = resolve_flow(&inputs(
            vec![tier(Source::Panoscope, TierState::Ok("0".into()), &[])],
            &["v/declared/v1"],
        ));
        let r = row(&report, "v/declared/v1");
        assert!(r.declared);
        assert_eq!(r.org.as_deref(), Some("v"));
        assert!(r.observed_by.is_empty());
    }

    #[test]
    fn undeclared_but_observed_topics_still_appear() {
        let report = resolve_flow(&inputs(
            vec![tier(
                Source::Panoscope,
                TierState::Ok("1".into()),
                &[("rogue/t/v1", obs_last(999_000))],
            )],
            &[],
        ));
        let r = row(&report, "rogue/t/v1");
        assert!(!r.declared);
        assert_eq!(r.org.as_deref(), Some("rogue"));
    }

    // --- wire model + helpers ---------------------------------------------

    #[test]
    fn snap_edge_rate_prefers_projected_over_undecayed_ema() {
        // The bug: `ema_eps` only decays when an event ARRIVES, so a quiet
        // topic reports a stale non-zero rate forever.
        let quiet = SnapEdge {
            topic: "t".into(),
            ema_eps: 12.4,
            projected_eps: Some(0.0),
            last_event_ms: 5,
            folded_singletons_count: None,
        };
        assert_eq!(quiet.rate(), 0.0);
        // Older panoscopes omit the projection — fall back rather than lie
        // in the other direction by reporting zero.
        let legacy = SnapEdge {
            topic: "t".into(),
            ema_eps: 12.4,
            projected_eps: None,
            last_event_ms: 5,
            folded_singletons_count: None,
        };
        assert_eq!(legacy.rate(), 12.4);
    }

    #[test]
    fn since_to_ms_parses_the_replay_duration_grammar() {
        assert_eq!(since_to_ms("1h"), Some(3_600_000));
        assert_eq!(since_to_ms("90m"), Some(5_400_000));
        assert_eq!(since_to_ms("2h30m"), Some(9_000_000));
        assert_eq!(since_to_ms("7d"), Some(604_800_000));
        // An RFC3339 timestamp has no fixed horizon.
        assert_eq!(since_to_ms("2026-08-07T18:00:00Z"), None);
        assert_eq!(since_to_ms("garbage"), None);
        assert_eq!(since_to_ms("12"), None);
    }

    #[test]
    fn fmt_horizon_reads_as_the_window_the_operator_asked_for() {
        assert_eq!(fmt_horizon(86_400_000), "1d");
        assert_eq!(fmt_horizon(604_800_000), "7d");
        assert_eq!(fmt_horizon(3_600_000), "1h");
        assert_eq!(fmt_horizon(5_400_000), "90m");
        assert_eq!(fmt_horizon(3_000), "3s");
        // The real shape: horizons are DERIVED (now minus the portal's
        // second-truncated floor), so they never land on an exact modulo.
        // `--since 24h` must still read "1d", not "86413s".
        assert_eq!(fmt_horizon(86_413_000), "1d");
        assert_eq!(fmt_horizon(3_607_000), "1h");
        // …but a genuinely odd span must not be rounded into a lie: 12.5h
        // would have to round to 13h (4% off), so it drops to the exact
        // finer unit instead.
        assert_eq!(fmt_horizon(45_000_000), "750m");
        assert_eq!(fmt_horizon(9_000), "9s");
    }

    // --- rendering ---------------------------------------------------------

    #[test]
    fn no_record_renders_the_horizon_never_the_word_never() {
        let report = resolve_flow(&inputs(
            vec![tier(Source::Audit, TierState::Ok("0 events".into()), &[])],
            &["v/t/v1"],
        ));
        let frame = render_frame(&report, false, false);
        assert!(frame.contains("none in 1d"), "frame was:\n{frame}");
        assert!(!frame.to_lowercase().contains("never published"));
    }

    #[test]
    fn frame_does_not_nudge_toward_since_when_audit_was_already_engaged() {
        // An operator-gated audit tail can reject the token, so the tier comes back
        // `unreachable`; the frame must not then tell the operator to pass the
        // `--since` they just passed. Suggest it only when it would actually change anything.
        let report = resolve_flow(&inputs(
            vec![
                tier(Source::Panoscope, TierState::Ok("0".into()), &[]),
                TierResult::unreachable(Source::Audit, "not an operator key"),
            ],
            &["v/t/v1"],
        ));
        let frame = render_frame(&report, false, false);
        assert!(!frame.contains("--since 24h"), "frame was:\n{frame}");
        // The row itself still refuses to claim absence.
        assert_eq!(row(&report, "v/t/v1").state, FlowState::Unknown);
    }

    #[test]
    fn frame_nudges_toward_since_when_nothing_can_prove_absence() {
        let report = resolve_flow(&inputs(
            vec![tier(Source::Panoscope, TierState::Ok("0".into()), &[])],
            &["v/t/v1"],
        ));
        let frame = render_frame(&report, false, false);
        assert!(frame.contains("--since 24h"), "frame was:\n{frame}");
    }

    #[test]
    fn live_source_label_carries_its_window() {
        let mut t = tier(
            Source::LiveWindow,
            TierState::Ok("watched 3s".into()),
            &[("v/t/v1", obs_last(999_000))],
        );
        t.horizon_ms = Some(3_000);
        let report = resolve_frame_helper(t);
        let frame = render_frame(&report, false, false);
        assert!(frame.contains("live(3s)"), "frame was:\n{frame}");
    }

    fn resolve_frame_helper(t: TierResult) -> FlowReport {
        resolve_flow(&inputs(vec![t], &[]))
    }

    #[test]
    fn colored_state_cell_keeps_column_alignment() {
        let report = resolve_flow(&inputs(
            vec![tier(
                Source::Panoscope,
                TierState::Ok("1".into()),
                &[("v/t/v1", obs_last(999_000))],
            )],
            &[],
        ));
        let plain = render_frame(&report, false, false);
        let colored = render_frame(&report, true, false);
        // Colour adds escape bytes but must not shift the visible columns:
        // the padding is applied before the escapes are wrapped around it.
        assert!(colored.contains("\x1b[32m"));
        assert!(plain.contains("flowing  "));
        assert!(colored.contains("flowing  \x1b[0m"));
    }

    /// A representative multi-state frame, asserted as a whole. Locks the
    /// column layout and the legend together — the pieces are individually
    /// covered above, but an operator reads them as one page, and this is
    /// the test that fails if the page stops making sense.
    #[test]
    fn representative_frame_layout() {
        const AS_OF: i64 = 1_786_250_531_000;
        let pano = tier(
            Source::Panoscope,
            TierState::Ok("3 topics".into()),
            &[
                (
                    "orgalfa/observations/v1",
                    TierObservation {
                        eps: Some(12.4),
                        ..obs_last(AS_OF - 2_000)
                    },
                ),
                (
                    "orgalfa/tracks/v1",
                    TierObservation {
                        eps: Some(0.0),
                        ..obs_last(AS_OF - 360_000)
                    },
                ),
            ],
        );
        let mut audit = tier(
            Source::Audit,
            TierState::Ok("41 event(s) over 24h".into()),
            &[("org/radar1/status", obs_last(AS_OF - 11_520_000))],
        );
        audit.horizon_ms = Some(86_400_000);

        let mut i = inputs(
            vec![
                pano,
                audit,
                TierResult::skipped(Source::LiveWindow, "not engaged (--live auto)"),
                TierResult::skipped(
                    Source::PortalRegistry,
                    "no topic roster available (portal unreachable or empty)",
                ),
            ],
            &["orgalfa/observations/v1", "orgalfa/cmd/v1", "notional/never/v1"],
        );
        i.as_of_unix_ms = AS_OF;
        i.as_of_source = AsOfSource::PanoscopeGeneratedAt;
        let report = resolve_flow(&i);
        let frame = render_frame(&report, false, false);
        println!("\n{frame}");

        // Every state in the vocabulary appears exactly where expected.
        assert!(frame.contains("orgalfa/observations/v1  flowing"));
        assert!(frame.contains("orgalfa/tracks/v1        idle"));
        assert!(frame.contains("orgalfa/cmd/v1           no-record  none in 1d"));
        assert!(frame.contains("notional/never/v1        no-record  none in 1d"));
        // The header row and the widest key share a column origin.
        assert!(frame.contains("key                      state"));
        // Sources footer carries every tier, skipped ones included.
        assert!(frame.contains("live        skipped"));
        assert!(frame.contains("portal-reg  skipped"));

        // The same report, active-only: only the flowing row survives, the
        // rest of the universe (still fully present in `report` itself,
        // unaffected by this rendering-only filter) drops out of the human
        // table, and the legend explaining unknown/no-record goes with it
        // since there is nothing on screen for it to explain.
        let active = render_frame(&report, false, true);
        assert!(active.contains("orgalfa/observations/v1  flowing"));
        assert!(!active.contains("orgalfa/tracks/v1"));
        assert!(!active.contains("orgalfa/cmd/v1"));
        assert!(!active.contains("notional/never/v1"));
        assert!(!active.contains("no observer could answer"));
        assert!(active.contains("flowing only (pass --all for the full universe)"));
        assert_eq!(report.rows.len(), 5, "active_only must not mutate the report itself");
    }

    #[test]
    fn active_only_never_claims_unknown_rows_are_a_confirmed_absence() {
        // REGRESSION: an earlier version of this filter said "nothing flowing
        // right now" for a topic that was actually `unknown` (no observer
        // could answer) — exactly the "unknown reads as no data" misreading
        // this whole command exists to prevent, just relocated into the
        // active-only headline. No engaged observer here, so the row is
        // Unknown, not NoRecord/Idle.
        let mut i = inputs(
            vec![TierResult::skipped(Source::LiveWindow, "not engaged (--live auto)")],
            &["quiet/topic/v1"],
        );
        i.as_of_unix_ms = 1_786_250_531_000;
        let report = resolve_flow(&i);
        assert!(!report.rows.is_empty(), "the topic is still in the universe");
        assert_eq!(report.rows[0].state, FlowState::Unknown);
        let frame = render_frame(&report, false, true);
        assert!(!frame.contains("nothing flowing right now"), "must not claim confirmed absence");
        assert!(frame.contains("no observer could answer for 1 known topic(s)"));
        assert!(frame.contains("NOT confirmed absent"));
        // An unknown row is hidden by the filter — the legend that explains
        // what "unknown" means must stay visible, not go quiet right when it
        // matters most.
        assert!(frame.contains("no observer could answer"));
    }

    #[test]
    fn active_only_says_none_flowing_only_when_that_is_actually_known() {
        // A tier answered and found the topic stale (Idle), not Unknown — a
        // genuine "checked, it's just not flowing right now" case, so the
        // headline may say so plainly and the unknown/no-record legend has
        // nothing to explain here.
        const AS_OF: i64 = 1_786_250_531_000;
        let pano = tier(
            Source::Panoscope,
            TierState::Ok("1 topic".into()),
            &[("acme/tracks/v1", obs_last(AS_OF - 360_000))],
        );
        let mut i = inputs(vec![pano], &["acme/tracks/v1"]);
        i.as_of_unix_ms = AS_OF;
        let report = resolve_flow(&i);
        assert_eq!(report.rows[0].state, FlowState::Idle);
        let frame = render_frame(&report, false, true);
        assert!(frame.contains("1 topic(s) known, none flowing right now — pass --all"));
        assert!(!frame.contains("unknown"), "nothing unknown here — the legend has nothing to add");
    }

    #[test]
    fn empty_roster_renders_the_house_empty_marker() {
        let report = resolve_flow(&inputs(vec![], &[]));
        assert!(render_frame(&report, false, false).contains("(no topics)"));
        let mut i = inputs(vec![], &[]);
        i.filter = Some("nope/**".to_string());
        assert!(render_frame(&resolve_flow(&i), false, false).contains("(no topics match `nope/**`)"));
    }
}
