// `goat top` — the live fabric dashboard screen (ADR 1044).
//
// FOLLOWS tui/tail.rs's structure exactly: a producer async task owns the
// Zenoh Session + the subscribers and feeds an unbounded mpsc channel; a
// synchronous ratatui event loop on `spawn_blocking` drains the channel each
// tick and redraws; terminal teardown is guaranteed even on error/panic.
//
// DIFFERENCE FROM tail.rs: tail is a scrollback log (a bounded ring of past
// samples). `goat top` is a CURRENT-STATE dashboard — latest-wins per key with
// a last-seen instant, bounded by the live key set (no unbounded history).
// A reading not refreshed within its staleness window is dropped (footprints)
// or dimmed-with-age (domain chips), NEVER rendered as live. This is the
// env-health honesty discipline: the dashboard never shows a green it can't
// ground.
//
// Decode reuse: the wire types + fmt_age/short_name come from
// crate::health_wire (shared with `goat health`). Color is mapped to a ratatui
// Style here (NOT health.rs::colorize, which emits ANSI for line output).

use crate::cmd::top::TopSort;
use crate::health_wire::{
    fmt_age, short_name, EnvDetail, EnvHealthV1, FabricHealthV1, ProducerFootprintV1,
    FOOTPRINT_PREFIX, HEALTH_PREFIX,
};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use panoscope_core::alias::AliasRegistry;
use panoscope_core::census::{query_census_mirror_first, Census};
use panoscope_core::envelope::EnvelopeRegistry;
use prost::Message as _;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, Clear, Gauge, Paragraph, Row, Table},
    Terminal,
};
use std::collections::HashMap;
use std::io::stdout;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

/// Input-poll cadence — also the redraw interval when a beat arrives with no
/// key event. Independent of `--interval`, which sets the staleness clock.
const TICK: Duration = Duration::from_millis(120);

/// A domain or footprint chip is rendered stale (dim + age) once its last beat
/// is older than this multiple of the publish window. ~2.5× tolerates one
/// missed beat without flapping; past it, we stop asserting a live color.
const STALE_FACTOR: u32 = 3;
/// Fallback staleness window (secs) when a beat carries no `window_secs`
/// (e.g. the fabric weather's footprint side). Mesh/time/presence publish
/// ~30s; fabric ~5s — this is only the floor for the footprint table.
const DEFAULT_WINDOW_SECS: u64 = 5;

/// The env-health domains rendered in the header weather-strip, in order.
/// `peer` is faults the router we own measured on peers we do not own (ADR 1232).
const DOMAINS: [&str; 5] = ["mesh", "fabric", "time", "presence", "peer"];

// --- messages from the producer task to the UI loop -------------------------

enum TopMsg {
    /// A fabric weather beat (`ops/fabric/health/<site>`).
    Fabric(Box<FabricHealthV1>),
    /// A per-producer footprint (`ops/fabric/footprint/<peer>`).
    Footprint(Box<ProducerFootprintV1>),
    /// An EnvHealthV1 beat from mesh/time/presence/peer (domain is in the envelope).
    Env(Box<EnvHealthV1>),
    /// A refreshed pub/sub census (who-consumes-what), read from the router
    /// adminspace. Fail-soft: an EMPTY census means the grant is deploy-pending
    /// or no data — the view model renders `—`, never a fabricated 0.
    Census(Box<Census>),
    Error(String),
}

/// Producer: own the session, declare one subscriber per key, forward every
/// decoded reading to the UI channel. Mirrors tail.rs::produce. A declare or
/// recv error becomes a `TopMsg::Error` the UI surfaces rather than stalling.
#[allow(clippy::disallowed_methods)] // census poller runs as its own task (below)
async fn produce(session: zenoh::Session, router: String, tx: UnboundedSender<TopMsg>) {
    // Our own ZID, so the census can filter this read-only session's own
    // in-process subscriptions out of the count (else `goat top` would render
    // itself as a subscriber on every producer).
    let own_zid = session.zid().to_string();
    // ops/fabric/** carries BOTH the weather (health) and the footprints.
    let fabric_sub = match session.declare_subscriber("ops/fabric/**").await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(TopMsg::Error(crate::classify::classify_zenoh_err_line(
                e,
                &router,
                "declare a subscriber on ops/fabric/**",
            )));
            return;
        }
    };
    // The EnvHealthV1 domains. One subscriber on ops/{mesh,time,presence,peer}
    // would need a brace key-expr; declare them separately for clarity.
    let mesh_sub = match session.declare_subscriber("ops/mesh/health/**").await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(TopMsg::Error(crate::classify::classify_zenoh_err_line(
                e,
                &router,
                "declare a subscriber on ops/mesh/health/**",
            )));
            return;
        }
    };
    let time_sub = match session.declare_subscriber("ops/time/health/**").await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(TopMsg::Error(crate::classify::classify_zenoh_err_line(
                e,
                &router,
                "declare a subscriber on ops/time/health/**",
            )));
            return;
        }
    };
    let presence_sub = match session.declare_subscriber("ops/presence/health/**").await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(TopMsg::Error(crate::classify::classify_zenoh_err_line(
                e,
                &router,
                "declare a subscriber on ops/presence/health/**",
            )));
            return;
        }
    };
    let peer_sub = match session.declare_subscriber("ops/peer/health/**").await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(TopMsg::Error(crate::classify::classify_zenoh_err_line(
                e,
                &router,
                "declare a subscriber on ops/peer/health/**",
            )));
            return;
        }
    };

    // The census is a POLLED surface (an adminspace query returning ~1000+
    // entries), not a subscription. Run it in its OWN task so its up-to-3s round
    // trip NEVER blocks this weather/producer loop. It previously lived in the
    // select! below with an immediate first tick, so at startup the census query
    // stalled the first render (the weather bar + producer table) by seconds —
    // and every 5s poll hitched the UI. As its own task it refreshes
    // concurrently; results arrive via TopMsg::Census. Fail-soft — a query error
    // or an ungranted ACL returns an empty Census, rendered honestly as `—`.
    {
        let census_session = session.clone();
        let census_tx = tx.clone();
        let census_own_zid = own_zid.clone();
        tokio::spawn(async move {
            let mut census_tick = tokio::time::interval(Duration::from_secs(5));
            loop {
                census_tick.tick().await;
                // Mirror-first (the one-crawler contract): read the lazy
                // crawler's published census when fresh+populated — a single
                // storage-backed GET — else the direct adminspace walk
                // (today's behavior; also the census-blind-mirror fallback).
                // max_age = 3x the crawler's 60s interval.
                let census = query_census_mirror_first(
                    &census_session,
                    &census_own_zid,
                    Duration::from_secs(3),
                    Duration::from_secs(180),
                )
                .await;
                if census_tx.send(TopMsg::Census(Box::new(census))).is_err() {
                    break; // UI loop gone — stop polling.
                }
            }
        });
    }

    loop {
        let msg = tokio::select! {
            r = fabric_sub.recv_async() => match r {
                Ok(sample) => decode_fabric(&sample),
                Err(e) => Some(TopMsg::Error(
                    crate::classify::classify_zenoh_err_line(e, &router, "receive a fabric-health sample"),
                )),
            },
            r = mesh_sub.recv_async() => match r {
                Ok(sample) => decode_env_sample(&sample, "mesh"),
                Err(e) => Some(TopMsg::Error(
                    crate::classify::classify_zenoh_err_line(e, &router, "receive a mesh-health sample"),
                )),
            },
            r = time_sub.recv_async() => match r {
                Ok(sample) => decode_env_sample(&sample, "time"),
                Err(e) => Some(TopMsg::Error(
                    crate::classify::classify_zenoh_err_line(e, &router, "receive a time-health sample"),
                )),
            },
            r = presence_sub.recv_async() => match r {
                Ok(sample) => decode_env_sample(&sample, "presence"),
                Err(e) => Some(TopMsg::Error(
                    crate::classify::classify_zenoh_err_line(e, &router, "receive a presence-health sample"),
                )),
            },
            r = peer_sub.recv_async() => match r {
                Ok(sample) => decode_env_sample(&sample, "peer"),
                Err(e) => Some(TopMsg::Error(
                    crate::classify::classify_zenoh_err_line(e, &router, "receive a peer-health sample"),
                )),
            },
        };
        if let Some(m) = msg {
            if tx.send(m).is_err() {
                return; // UI gone — stop.
            }
        }
    }
}

/// Decode a sample off `ops/fabric/**` into either a weather or a footprint
/// message by key prefix. Returns None for an unrecognized key.
fn decode_fabric(sample: &zenoh::sample::Sample) -> Option<TopMsg> {
    let key = sample.key_expr().as_str();
    let bytes = sample.payload().to_bytes();
    if key.starts_with(HEALTH_PREFIX) {
        match FabricHealthV1::decode(bytes.as_ref()) {
            Ok(h) => Some(TopMsg::Fabric(Box::new(h))),
            Err(e) => Some(TopMsg::Error(format!("decode FabricHealthV1 from `{key}`: {e}"))),
        }
    } else if key.starts_with(FOOTPRINT_PREFIX) {
        match ProducerFootprintV1::decode(bytes.as_ref()) {
            Ok(f) => Some(TopMsg::Footprint(Box::new(f))),
            Err(e) => Some(TopMsg::Error(format!("decode ProducerFootprintV1 from `{key}`: {e}"))),
        }
    } else {
        None
    }
}

/// Decode an EnvHealthV1 sample, keeping it only if its domain matches the
/// subscriber it came from (defends against a stray cross-domain publish).
fn decode_env_sample(sample: &zenoh::sample::Sample, domain: &str) -> Option<TopMsg> {
    let bytes = sample.payload().to_bytes();
    match EnvHealthV1::decode(bytes.as_ref()) {
        Ok(env) if env.domain == domain => Some(TopMsg::Env(Box::new(env))),
        Ok(_) => None,
        Err(e) => Some(TopMsg::Error(format!(
            "decode EnvHealthV1 from `{}`: {e}",
            sample.key_expr().as_str()
        ))),
    }
}

// --- entry point -------------------------------------------------------------

// The disallowed-methods clippy lint (clippy.toml / ADR 0709 D7 layer 3) pins
// goat-relay's hot-path tasks to one shard via spawn_local. goat-cli is an
// admin-path operator binary on the multi-thread runtime — the producer (Zenoh
// subscriber) must run on a worker thread so the spawn_blocking ratatui loop
// can block on input polling without starving it. Same shape as the Tail lens.
#[allow(clippy::disallowed_methods)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    session: zenoh::Session,
    router: String,
    registry: AliasRegistry,
    envelopes: EnvelopeRegistry,
    site: Option<String>,
    sort: TopSort,
    interval_secs: u64,
    filter: Option<String>,
) -> anyhow::Result<()> {
    let (ui_tx, ui_rx) = mpsc::unbounded_channel::<TopMsg>();
    let producer = tokio::spawn(produce(session, router, ui_tx.clone()));
    drop(ui_tx); // the producer holds the live clone.

    let handle = tokio::task::spawn_blocking(move || {
        run_loop(ui_rx, registry, envelopes, site, sort, interval_secs, filter)
    });
    let result = handle
        .await
        .map_err(|e| anyhow::anyhow!("top TUI thread panicked: {e}"))?;

    producer.abort();
    result
}

/// Collection window floor for `--once`: long enough to catch at least one
/// fabric-health beat (it publishes every ~5s).
const ONCE_MIN_WINDOW_SECS: u64 = 6;

/// Batch snapshot (`goat top --once`): collect beats for one bounded window,
/// render ONE plain-text frame to stdout, exit. No terminal, no TTY gate — the
/// scriptable / CI counterpart. Same producer task + view model as the
/// interactive screen; only the renderer differs.
#[allow(clippy::disallowed_methods)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_once(
    session: zenoh::Session,
    router: String,
    registry: AliasRegistry,
    envelopes: EnvelopeRegistry,
    site: Option<String>,
    sort: TopSort,
    interval_secs: u64,
    filter: Option<String>,
) -> anyhow::Result<()> {
    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<TopMsg>();
    let producer = tokio::spawn(produce(session, router, ui_tx.clone()));
    drop(ui_tx);

    let window = Duration::from_secs(interval_secs.max(ONCE_MIN_WINDOW_SECS));
    let mut state = TopState::new(site, sort, interval_secs);
    if let Some(f) = filter {
        state.filter = f;
    }

    // Fold every beat that arrives within the collection window. Use a deadline
    // so a quiet fabric still produces a frame (with honest unmetered/idle).
    let deadline = tokio::time::Instant::now() + window;
    // Track whether ANY real reading ever arrived — distinct from
    // `state.status`, which is also set by a late/transient recv or decode
    // error after plenty of good data already came in. Only the "zero beats,
    // ever" case (e.g. every declare_subscriber failed at startup, so the
    // producer sent one Error and returned immediately) is a hard failure
    // for `--once`'s documented scriptable/CI contract.
    let mut received_any = false;
    loop {
        let now = Instant::now();
        match tokio::time::timeout_at(deadline, ui_rx.recv()).await {
            Ok(Some(msg)) => match msg {
                TopMsg::Fabric(h) => {
                    state.apply_fabric(*h, now);
                    received_any = true;
                }
                TopMsg::Footprint(f) => {
                    state.apply_footprint(*f, now);
                    received_any = true;
                }
                TopMsg::Env(e) => {
                    state.apply_env(*e, now);
                    received_any = true;
                }
                TopMsg::Census(c) => {
                    state.apply_census(*c);
                    received_any = true;
                }
                TopMsg::Error(e) => state.status = Some(e),
            },
            // Window elapsed (timeout) or producer gone (channel closed) — done.
            Ok(None) | Err(_) => break,
        }
    }
    producer.abort();

    let now = Instant::now();
    // Honesty stays intact in batch: expiry + the view model's stale-collapse
    // run exactly as in the live loop, so a beat that arrived early in the
    // window but is now stale reads idle, not a faked live color.
    state.expire(now);
    let vm = state.view_model(&registry, &envelopes, now);
    print!("{}", render_plain(&vm, sort));
    if let Some(e) = &state.status {
        eprintln!("⚠ {e}");
    }
    // Before this check, a total failure (every subscription failed to
    // declare) still printed a plausible-looking near-empty frame and
    // returned Ok(()) — indistinguishable from a genuinely quiet-but-healthy
    // fabric to a script checking the exit code, which is exactly the
    // scenario `--once` documents itself as being for.
    if !received_any {
        if let Some(e) = &state.status {
            anyhow::bail!("goat top --once collected zero readings — {e}");
        }
    }
    Ok(())
}

// --- live state (latest-wins, bounded, honest about staleness) ---------------

/// A domain reading kept latest-wins, with the local instant it arrived (the
/// honest staleness clock — we do NOT trust the beat's own timestamp to decide
/// liveness) and the publish window it declared.
struct DomainBeat {
    env: EnvHealthV1,
    seen: Instant,
}

/// A producer footprint kept latest-wins per peer anchor, with arrival instant.
struct FootprintEntry {
    fp: ProducerFootprintV1,
    seen: Instant,
}

/// Which modal overlay is drawn on top of the live table. Modal: while an
/// overlay is open the table keeps updating underneath, but keys route to the
/// overlay (close / browse) — it's a read-only lens, never a mode that can
/// mutate the fabric.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Overlay {
    /// No overlay — the normal live table.
    None,
    /// The `?` legend: what the weather chips, colors, columns, and bands mean.
    Help,
    /// The `↵` producer drill-down: the selected producer's full footprint plus
    /// its live consumer fan-out from the census (who this producer reaches).
    Detail,
}

struct TopState {
    site_filter: Option<String>,
    sort: TopSort,
    /// Staleness window in seconds (from `--interval`-independent publish
    /// cadence): a key older than STALE_FACTOR × its own window is stale.
    interval_secs: u64,
    fabric: Option<DomainBeat>,
    /// mesh/time/presence keyed by domain name.
    env: HashMap<String, DomainBeat>,
    /// footprints keyed by peer anchor (the canonical UUID/handle string).
    footprints: HashMap<String, FootprintEntry>,
    /// Selected producer row, by peer anchor (stable across refreshes).
    selected_peer: Option<String>,
    paused: bool,
    status: Option<String>,
    /// Active producer-table filter (case-insensitive substring of a peer's
    /// alias or anchor). Empty = no filter — the htop incremental-filter idiom
    /// (`/` to edit, narrows live, `Esc` clears).
    filter: String,
    /// True while the operator is typing into the filter (keys route to the
    /// filter buffer, not to commands).
    editing_filter: bool,
    sorted_cache: Vec<String>,
    /// The raw fabric weather (held separately from the `fabric` chip form so
    /// the class-load row has the real ClassHealthV1 rows).
    fabric_raw: Option<FabricHealthV1>,
    /// Latest pub/sub census (who-consumes-what), refreshed on its own cadence.
    /// Default-empty until the first refresh lands; an EMPTY census is honest
    /// "unknown" — the consumer column renders `—`, never a fabricated 0.
    census: Census,
    /// The active modal overlay (legend / producer drill-down), or `None`.
    overlay: Overlay,
}

// --- the pure view model -----------------------------------------------------
//
// `TopState::view_model(now)` projects the live state into a renderer-agnostic
// snapshot. Both the ratatui interactive draw and the `--once` plain-text
// renderer consume ONLY this — so the two surfaces can never disagree about the
// honesty discipline (a chip that is stale/ungrounded is computed once, here).

/// One domain chip: name, the honesty STATE STRING to show (already
/// stale-collapsed to "idle" when the beat aged out, or "unmetered" when no
/// beat was ever seen), n, and an optional stale age string.
struct ChipVM {
    domain: String,
    state: String,
    sample_count: u64,
    stale_age: Option<String>,
}

/// One fabric class-load row.
struct ClassVM {
    class: String,
    sample_rate: f64,
    target_sample_rate: f64,
    ratio: f64,
    color: String,
}

/// The fabric class-load pane state: either rows, or an honest absent/stale note.
enum ClassPaneVM {
    Rows(Vec<ClassVM>),
    /// No fabric beat yet — "unmetered" note.
    Absent,
    /// Fabric weather stale — show the age, hide the bars (never faked).
    Stale(String),
}

/// One axis's envelope breach, for the drill-down band explainer: which axis
/// (msgs/s or B/s), the verdict, the allotment it breached, and by how much.
/// Computed only for off-nominal axes — a producer inside its envelope has none.
struct BandAxis {
    axis: &'static str,
    verdict: &'static str,
    color: &'static str,
    current: f64,
    /// The allotment edge this axis breached: `max` for OVER, `high` for HIGH,
    /// `min` for LOW.
    limit: f64,
    /// Signed distance past the edge: `current - limit` (positive OVER/HIGH,
    /// negative LOW).
    delta: f64,
    /// `current` as a percent of `limit` (limit > 0).
    pct: f64,
}

/// Per-axis envelope breaches for a producer's live rates. Each axis that is
/// out of band (OVER `max`, HIGH over `high`, or LOW under a non-zero `min`)
/// contributes one `BandAxis`; an in-envelope / no-envelope producer gets none.
/// Mirrors `Envelope::axis_band` (envelope.rs) but keeps the numbers so the
/// drill-down can show the allotment and the overage, not just the verdict.
fn band_axes(env: Option<&panoscope_core::envelope::Envelope>, msgs: f64, bytes: f64) -> Vec<BandAxis> {
    let Some(e) = env else { return Vec::new() };
    let mut out = Vec::new();
    for (axis, rate, min, high, max) in [
        ("msgs/s", msgs, e.min_msgs_per_s, e.high_msgs_per_s, e.max_msgs_per_s),
        ("B/s", bytes, e.min_bytes_per_s, e.high_bytes_per_s, e.max_bytes_per_s),
    ] {
        let (verdict, color, limit) = if max > 0.0 && rate > max {
            ("OVER", "red", max)
        } else if high > 0.0 && rate > high {
            ("HIGH", "amber", high)
        } else if min > 0.0 && rate < min {
            ("LOW", "cyan", min)
        } else {
            continue;
        };
        out.push(BandAxis {
            axis,
            verdict,
            color,
            current: rate,
            limit,
            delta: rate - limit,
            pct: if limit > 0.0 { rate / limit * 100.0 } else { 0.0 },
        });
    }
    out
}

/// One producer table row, fully resolved for display.
struct ProducerVM {
    /// 1-based DISPLAY position in the current sort order — the `#` a top-style
    /// user expects (top row always #1, numbers read in order). This is NOT the
    /// meter's ProducerFootprintV1.rank (a fixed byte-rate rank); numbering by
    /// rank under a msgs/s sort makes the column read jumbled.
    display_pos: usize,
    peer_display: String,
    sample_rate: f64,
    byte_rate: f64,
    shares: String,
    near_limit: bool,
    path: String,
    latency: String,
    /// Throughput-envelope band label (ADR 1045): OVER / HIGH / ok / LOW, or
    /// `—` when no envelope is known (honest no-verdict, never a faked green).
    band_label: String,
    /// Honesty-palette color for the band ("red"/"amber"/"green"/"cyan"), or
    /// "" for the no-envelope `—`.
    band_color: String,
    /// Per-axis envelope breaches for the drill-down (which axis is out of band,
    /// the allotment it breached, and by how much). Empty for `ok`/`—` bands.
    band_axes: Vec<BandAxis>,
    /// Distinct-subscriber fan-out from the pub/sub census (how many consumers
    /// this producer's output reaches), or `None` when the census is empty
    /// (grant deploy-pending / no data). `None` renders `—`, never a faked 0.
    subscribers: Option<usize>,
    /// peer anchor (for selection match in the interactive renderer).
    anchor: String,
}

struct TotalsVM {
    /// Total live footprints in the window (unfiltered) — the true fabric
    /// producer count.
    active: usize,
    /// Producers shown after the active filter (== producers.len()). Equals
    /// `active` when no filter is set.
    shown: usize,
    total_msgs: f64,
    total_bytes: f64,
    direct: usize,
    relayed: usize,
    alive: u32,
    dropped: u32,
}

struct ViewModel {
    chips: Vec<ChipVM>,
    classes: ClassPaneVM,
    producers: Vec<ProducerVM>,
    totals: TotalsVM,
    /// The active producer filter, if any — surfaced so a narrowed view is
    /// never mistaken for the whole fabric.
    filter: Option<String>,
}

impl TopState {
    fn new(site_filter: Option<String>, sort: TopSort, interval_secs: u64) -> Self {
        Self {
            site_filter,
            sort,
            interval_secs: interval_secs.max(1),
            fabric: None,
            env: HashMap::new(),
            footprints: HashMap::new(),
            selected_peer: None,
            paused: false,
            status: None,
            filter: String::new(),
            editing_filter: false,
            sorted_cache: Vec::new(),
            fabric_raw: None,
            census: Census::default(),
            overlay: Overlay::None,
        }
    }

    /// True when a footprint passes the active filter — case-insensitive
    /// substring of the peer's rendered name (alias) OR its raw anchor. An
    /// empty filter passes everyone. Matching the alias is what lets an
    /// operator type the handle they SEE on a slot-pool goatnet, where
    /// the anchor is a UUID; matching the anchor covers per-vendor names.
    fn matches_filter(&self, registry: &AliasRegistry, anchor: &str) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        let needle = self.filter.to_lowercase();
        registry.render(anchor).to_lowercase().contains(&needle)
            || anchor.to_lowercase().contains(&needle)
    }

    /// `true` if the scope passes the optional site filter.
    fn scope_ok(&self, scope: &str) -> bool {
        match &self.site_filter {
            Some(s) => scope == s,
            None => true,
        }
    }

    fn apply_fabric(&mut self, h: FabricHealthV1, now: Instant) {
        if !self.scope_ok(&h.site) {
            return;
        }
        self.fabric = Some(DomainBeat { env: fabric_as_env(&h), seen: now });
        // Stash the real fabric weather for the class-load row.
        self.fabric_raw = Some(h);
    }

    fn apply_env(&mut self, env: EnvHealthV1, now: Instant) {
        if !self.scope_ok(&env.scope) {
            return;
        }
        self.env.insert(env.domain.clone(), DomainBeat { env, seen: now });
    }

    fn apply_footprint(&mut self, fp: ProducerFootprintV1, now: Instant) {
        self.footprints
            .insert(fp.peer.clone(), FootprintEntry { fp, seen: now });
    }

    /// Replace the census latest-wins (a full snapshot per refresh, not a
    /// delta). An empty snapshot legitimately overwrites a prior one — the
    /// fabric's consumer set really can be unknown again.
    fn apply_census(&mut self, census: Census) {
        self.census = census;
    }

    /// Drop footprints not refreshed within their staleness window so a
    /// producer that went quiet ages out rather than lingering as a false-live
    /// row. Domain chips are NOT dropped (we want to show their stale age) —
    /// only footprints, which have no "stale but visible" rendering.
    fn expire(&mut self, now: Instant) {
        let window = self.footprint_stale_window();
        self.footprints
            .retain(|_, e| now.duration_since(e.seen) < window);
    }

    fn footprint_stale_window(&self) -> Duration {
        // Footprints ride the fabric cadence; floor at DEFAULT_WINDOW_SECS.
        let w = self
            .fabric_raw
            .as_ref()
            .map(|h| h.window_secs as u64)
            .filter(|w| *w > 0)
            .unwrap_or(DEFAULT_WINDOW_SECS);
        Duration::from_secs(w.max(self.interval_secs) * STALE_FACTOR as u64)
    }

    /// Is a domain beat stale (older than STALE_FACTOR × its own window)?
    fn is_stale(beat: &DomainBeat, now: Instant) -> bool {
        let w = (beat.env.window_secs as u64).max(1);
        now.duration_since(beat.seen) >= Duration::from_secs(w * STALE_FACTOR as u64)
    }

    /// Footprint peer anchors that pass the active filter, sorted by the
    /// current sort key. The SINGLE source of producer order — both the
    /// rendered table and the selection cache read it, so a filtered-out peer
    /// can never be selected. Takes the alias registry so the filter can match
    /// the displayed handle. Pure over (self, registry) so it's unit-testable.
    fn sorted_peers(&self, registry: &AliasRegistry) -> Vec<String> {
        let mut peers: Vec<&FootprintEntry> = self
            .footprints
            .values()
            .filter(|e| self.matches_filter(registry, &e.fp.peer))
            .collect();
        match self.sort {
            TopSort::Msgs => peers.sort_by(|a, b| {
                b.fp.sample_rate
                    .partial_cmp(&a.fp.sample_rate)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.fp.peer.cmp(&b.fp.peer))
            }),
            TopSort::Bytes => peers.sort_by(|a, b| {
                b.fp.byte_rate
                    .partial_cmp(&a.fp.byte_rate)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.fp.peer.cmp(&b.fp.peer))
            }),
            TopSort::Peer => peers.sort_by(|a, b| a.fp.peer.cmp(&b.fp.peer)),
        }
        peers.into_iter().map(|e| e.fp.peer.clone()).collect()
    }

    fn cycle_sort(&mut self) {
        self.sort = match self.sort {
            TopSort::Msgs => TopSort::Bytes,
            TopSort::Bytes => TopSort::Peer,
            TopSort::Peer => TopSort::Msgs,
        };
    }

    fn select_next(&mut self) {
        if self.sorted_cache.is_empty() {
            return;
        }
        let i = self.current_index().map(|i| i + 1).unwrap_or(0);
        let i = i.min(self.sorted_cache.len() - 1);
        self.selected_peer = self.sorted_cache.get(i).cloned();
    }

    fn select_prev(&mut self) {
        if self.sorted_cache.is_empty() {
            return;
        }
        let i = self.current_index().map(|i| i.saturating_sub(1)).unwrap_or(0);
        self.selected_peer = self.sorted_cache.get(i).cloned();
    }

    fn current_index(&self) -> Option<usize> {
        let sel = self.selected_peer.as_ref()?;
        self.sorted_cache.iter().position(|p| p == sel)
    }

    /// Project the live state into the renderer-agnostic view model, applying
    /// the honesty discipline ONCE (stale-collapse, ungrounded → idle/unmetered)
    /// so both renderers stay consistent. Pure over (self, registry, now).
    fn view_model(
        &self,
        registry: &AliasRegistry,
        envelopes: &EnvelopeRegistry,
        now: Instant,
    ) -> ViewModel {
        // Domain chips, in the canonical strip order.
        let chips = DOMAINS
            .iter()
            .map(|domain| {
                let beat = if *domain == "fabric" {
                    self.fabric.as_ref()
                } else {
                    self.env.get(*domain)
                };
                match beat {
                    None => ChipVM {
                        domain: domain.to_string(),
                        state: "unmetered".to_string(),
                        sample_count: 0,
                        stale_age: None,
                    },
                    Some(b) => {
                        let stale = TopState::is_stale(b, now);
                        let age_ms = now.duration_since(b.seen).as_millis() as i64;
                        ChipVM {
                            domain: domain.to_string(),
                            // Stale beats collapse to idle — never re-assert the
                            // last live color.
                            state: if stale {
                                "idle".to_string()
                            } else {
                                b.env.overall_color.clone()
                            },
                            sample_count: b.env.sample_count,
                            stale_age: stale.then(|| fmt_age(age_ms)),
                        }
                    }
                }
            })
            .collect();

        // Fabric class-load pane.
        let classes = match &self.fabric_raw {
            None => ClassPaneVM::Absent,
            Some(h) => {
                let stale = self
                    .fabric
                    .as_ref()
                    .map(|b| TopState::is_stale(b, now))
                    .unwrap_or(false);
                if stale {
                    let age = self
                        .fabric
                        .as_ref()
                        .map(|b| fmt_age(now.duration_since(b.seen).as_millis() as i64))
                        .unwrap_or_else(|| "—".to_string());
                    ClassPaneVM::Stale(age)
                } else {
                    ClassPaneVM::Rows(
                        h.classes
                            .iter()
                            .map(|c| ClassVM {
                                class: c.class.clone(),
                                sample_rate: c.sample_rate,
                                target_sample_rate: c.target_sample_rate,
                                ratio: if c.target_sample_rate > 0.0 {
                                    (c.sample_rate / c.target_sample_rate).clamp(0.0, 1.0)
                                } else {
                                    0.0
                                },
                                color: c.color.clone(),
                            })
                            .collect(),
                    )
                }
            }
        };

        // Producer rows, in current sort order, joined to mesh path/latency.
        let mesh = self.env.get("mesh").map(|b| &b.env);
        let producers: Vec<ProducerVM> = self
            .sorted_peers(registry)
            .iter()
            .filter_map(|peer| self.footprints.get(peer))
            .enumerate()
            .map(|(i, e)| {
                let fp = &e.fp;
                let (path, latency) = mesh_join(fp, registry, mesh);
                // Envelope band (ADR 1045): the worst-axis verdict for this
                // producer's LIVE rate, or an honest `—` when no envelope is
                // declared (never a faked green).
                let (band_label, band_color) = match envelopes.band(&fp.peer, fp.sample_rate, fp.byte_rate) {
                    Some(b) => (b.label().to_string(), b.color().to_string()),
                    None => ("—".to_string(), String::new()),
                };
                // Per-axis breach detail for the drill-down (allotment + overage).
                let band_axes = band_axes(envelopes.get(&fp.peer), fp.sample_rate, fp.byte_rate);
                // Consumer fan-out from the census: the distinct subscriber ZIDs
                // reached by anything this producer publishes under its anchor.
                // An empty census is honest "unknown" (→ `—`), NEVER a faked 0.
                let subscribers = if self.census.is_empty() {
                    None
                } else {
                    Some(self.census.fanout_for_anchor(&fp.peer).len())
                };
                ProducerVM {
                    // `#` = display position in the current sort (1-based), so
                    // the top row is always #1 and the column reads in order.
                    display_pos: i + 1,
                    peer_display: registry.render(&fp.peer),
                    sample_rate: fp.sample_rate,
                    byte_rate: fp.byte_rate,
                    shares: shares_label(fp),
                    near_limit: fp.near_limit,
                    path,
                    latency,
                    band_label,
                    band_color,
                    band_axes,
                    subscribers,
                    anchor: fp.peer.clone(),
                }
            })
            .collect();
        let shown = producers.len();

        let (direct, relayed) = mesh_link_counts(self);
        let (alive, dropped) = presence_counts(self);
        let totals = TotalsVM {
            active: self.footprints.len(),
            shown,
            total_msgs: self.footprints.values().map(|e| e.fp.sample_rate).sum(),
            total_bytes: self.footprints.values().map(|e| e.fp.byte_rate).sum(),
            direct,
            relayed,
            alive,
            dropped,
        };

        let filter = if self.filter.is_empty() {
            None
        } else {
            Some(self.filter.clone())
        };

        ViewModel { chips, classes, producers, totals, filter }
    }
}

/// Plain-text one-frame render of the view model (the `--once` / `top -bn1`
/// renderer). Honesty discipline in text: print the state STRING (idle/
/// unmetered are honest "unconfirmed", never a faked green), the grounding n,
/// and a stale age when stale. Pure → unit-testable.
fn render_plain(vm: &ViewModel, sort: TopSort) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    // Weather strip — one line.
    let mut chip_parts: Vec<String> = Vec::new();
    for c in &vm.chips {
        let mut s = format!("{} {} n={}", c.domain.to_uppercase(), c.state.to_uppercase(), c.sample_count);
        if let Some(age) = &c.stale_age {
            let _ = write!(s, " (stale {age})");
        }
        chip_parts.push(s);
    }
    let _ = writeln!(out, "ENV  {}", chip_parts.join("   ·   "));
    out.push('\n');

    // Fabric class load.
    match &vm.classes {
        ClassPaneVM::Absent => {
            let _ = writeln!(out, "FABRIC CLASS LOAD: unmetered — no fabric-health beat (is goat-fabric-health running?)");
        }
        ClassPaneVM::Stale(age) => {
            let _ = writeln!(out, "FABRIC CLASS LOAD: stale ({age}) — bars hidden, not faked");
        }
        ClassPaneVM::Rows(rows) => {
            let _ = writeln!(out, "FABRIC CLASS LOAD");
            for c in rows {
                // Same fixed-width rate field as the interactive gauge (shared
                // via class_rate_field) so the two renderers never drift.
                let _ = writeln!(
                    out,
                    "  {:<3} {:<12}{} {:>4.0}%  {}",
                    c.class,
                    class_label(&c.class),
                    class_rate_field(c),
                    c.ratio * 100.0,
                    c.color.to_uppercase()
                );
            }
        }
    }
    out.push('\n');

    // Producer table. The title carries the filter + shown/total so a narrowed
    // view never reads as the whole fabric (BAND = envelope verdict, ADR 1045).
    match &vm.filter {
        Some(f) => {
            let _ = writeln!(
                out,
                "PRODUCERS ({} of {} shown · filter \"{}\" · sort {})",
                vm.totals.shown,
                vm.totals.active,
                f,
                sort_label(sort)
            );
        }
        None => {
            let _ = writeln!(out, "PRODUCERS ({} active · sort {})", vm.totals.active, sort_label(sort));
        }
    }
    let _ = writeln!(
        out,
        "  {:<3} {:<22} {:>8} {:>10} {:>5} {:<5} {:<18} {:<2} {:<18} {:>10}",
        "#", "PEER", "MSG/S", "B/S", "SUBS", "BAND", "SHARES", "", "PATH", "LATENCY"
    );
    if vm.producers.is_empty() {
        let note = if vm.filter.is_some() {
            "  (no producers match the filter)"
        } else {
            "  (no producer footprints in this window)"
        };
        let _ = writeln!(out, "{note}");
    }
    for p in &vm.producers {
        let near = if p.near_limit { "!" } else { "" };
        let _ = writeln!(
            out,
            "  {:<3} {:<22} {:>8.0} {:>10.0} {:>5} {:<5} {:<18} {:<2} {:<18} {:>10}",
            p.display_pos,
            truncate(&p.peer_display, 22),
            p.sample_rate,
            p.byte_rate,
            subs_label(p.subscribers),
            p.band_label.to_uppercase(),
            truncate(&p.shares, 18),
            near,
            truncate(&p.path, 18),
            p.latency,
        );
    }
    out.push('\n');

    // Totals.
    let t = &vm.totals;
    let _ = writeln!(
        out,
        "TOTALS  peers {}  {:.0} msgs/s  {:.0} B/s  links {} direct / {} relayed  presence {} alive / {} dropped",
        t.active, t.total_msgs, t.total_bytes, t.direct, t.relayed, t.alive, t.dropped
    );
    out
}

/// Render a producer's consumer fan-out for display: the count when the census
/// is known, or an honest `—` when it is empty (grant deploy-pending / no data).
/// Never a fabricated 0 — an unknown consumer set must READ unknown.
fn subs_label(subscribers: Option<usize>) -> String {
    match subscribers {
        Some(n) => n.to_string(),
        None => "—".to_string(),
    }
}

/// Wrap a FabricHealthV1 as the EnvHealthV1 chip shape for the weather strip
/// (domain "fabric"); the class rows are read from the raw form separately.
fn fabric_as_env(h: &FabricHealthV1) -> EnvHealthV1 {
    EnvHealthV1 {
        domain: "fabric".to_string(),
        scope: h.site.clone(),
        ts_unix_ms: h.ts_unix_ms,
        overall_color: h.overall_color.clone(),
        sample_count: h.classes.iter().map(|c| c.sample_count).sum(),
        window_secs: h.window_secs,
        detail: None,
    }
}

/// Map a 5-state honesty color string to a ratatui style. green/amber/red are
/// the grounded colors; idle/unmetered (ungrounded) are dim — never a false
/// green. Style::fg, NOT health.rs::colorize (which emits ANSI for stdout).
fn color_style(state: &str) -> Style {
    match state {
        "green" => Style::default().fg(Color::Green),
        "amber" => Style::default().fg(Color::Yellow),
        "red" => Style::default().fg(Color::Red),
        // cyan = the envelope LOW band (under-floor fault — a distinct kind of
        // off-nominal from amber's "approaching ceiling"; ADR 1045).
        "cyan" => Style::default().fg(Color::Cyan),
        // idle = UNCONFIRMED, unmetered = no signal — both honestly dim.
        "idle" | "unmetered" => Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
        // "" = the envelope `—` (no envelope declared) → dim, an honest no-verdict.
        "" => Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
        _ => Style::default().fg(Color::Gray),
    }
}

/// Compact per-class share string, e.g. "C2 80% · C0 20%", from a footprint's
/// shares (by msgs/s share of the producer's own total). Pure + testable.
fn shares_label(fp: &ProducerFootprintV1) -> String {
    let total: f64 = fp.shares.iter().map(|s| s.sample_rate).sum();
    if total <= 0.0 || fp.shares.is_empty() {
        return "—".to_string();
    }
    let mut parts: Vec<(String, f64)> = fp
        .shares
        .iter()
        .map(|s| (s.class.clone(), s.sample_rate / total))
        .collect();
    parts.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    parts
        .iter()
        .take(3)
        .map(|(c, frac)| format!("{c} {:.0}%", frac * 100.0))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Best-effort join of a producer's footprint to its mesh-link path + latency.
///
/// HONESTY (supervisor's flag, ADR 1044 §4.3): the footprint `peer` is a slot
/// anchor (a UUID or a human handle), while a mesh link keys on a netbird
/// `peer_fqdn` (a hostname) — DIFFERENT namespaces. We match only when the
/// alias-rendered footprint name equals the mesh link's short name; otherwise
/// we render "—". We NEVER guess a path: a wrong join is worse than an honest
/// blank. Returns (path, latency).
fn mesh_join(
    fp: &ProducerFootprintV1,
    registry: &AliasRegistry,
    mesh: Option<&EnvHealthV1>,
) -> (String, String) {
    let Some(env) = mesh else {
        return ("—".to_string(), "—".to_string());
    };
    let Some(EnvDetail::Mesh(detail)) = &env.detail else {
        return ("—".to_string(), "—".to_string());
    };
    // The names we can compare a mesh short-name against: the peer's alias and
    // the raw anchor (in case the anchor itself IS a hostname-like label).
    let alias = registry.alias_of(&fp.peer);
    let anchor = &fp.peer;
    let matched = detail.links.iter().find(|l| {
        let sn = short_name(&l.peer_fqdn);
        sn == anchor.as_str() || alias.as_deref() == Some(sn)
    });
    match matched {
        None => ("—".to_string(), "—".to_string()),
        Some(l) if !l.connected => ("offline".to_string(), "—".to_string()),
        Some(l) => {
            let path = if l.direct {
                "direct P2P".to_string()
            } else {
                format!("relayed {}", relay_label(&l.relay_address))
            };
            let latency = if l.latency_ms > 0.0 {
                format!("{:.0} ms", l.latency_ms)
            } else {
                "unmeasured".to_string()
            };
            (path, latency)
        }
    }
}

/// Relay short label: "via relay-dub" from "rels://relay-dub.example.net:33073".
fn relay_label(addr: &str) -> String {
    let host = addr.trim_start_matches("rels://").trim_start_matches("rel://");
    match host.split(['.', ':']).next() {
        Some(h) if !h.is_empty() => format!("via {h}"),
        _ => "via relay".to_string(),
    }
}

/// Canonical human short-name for a fabric class code, shown beside the C# in
/// the class-load row as a legend. DISPLAY-ONLY (ADR 1039 spirit) — the wire
/// key + ACL stay C0..C3; this is a client-side map. Names per
/// fabric-health-surface.md / zenoh-qos-and-prioritization.md. An unknown code
/// (e.g. a future C4) falls back to "" so it renders as just the bare code.
fn class_label(class: &str) -> &'static str {
    match class {
        "C0" => "control",
        "C1" => "mission",
        "C2" => "situational",
        "C3" => "bulk",
        _ => "",
    }
}

/// The fixed-width "current / target msgs/s" load field. Numbers are
/// right-aligned into columns and the whole field is padded to a constant
/// width, so the status column lines up across every class row. This is
/// load-bearing for the interactive gauge: ratatui centers a Gauge label by
/// its width, so rows of differing width shift horizontally — the ragged
/// C2-"situational" / 6-digit-target look. Widths: current `{:>6}`, target
/// `{:>7}` (200000 fits), whole field padded to 24.
fn class_rate_field(c: &ClassVM) -> String {
    let s = if c.target_sample_rate > 0.0 {
        format!("{:>6.0} /{:>7.0} msgs/s", c.sample_rate, c.target_sample_rate)
    } else {
        format!("{:>6.0} msgs/s", c.sample_rate)
    };
    format!("{s:<24}")
}

/// The complete, FIXED-WIDTH class-load label used by the interactive gauge.
/// Every row is exactly the same width (code 3 + name 12 + rate 24 + status 9,
/// plus separators), which is what makes ratatui's centered gauge labels line
/// up into columns. Also used by the plain `--once` path so the two never drift.
fn format_class_label(c: &ClassVM) -> String {
    format!(
        "{:<3} {:<12}{}{:<9}",
        c.class,
        class_label(&c.class),
        class_rate_field(c),
        c.color.to_uppercase()
    )
}

// --- the loop ----------------------------------------------------------------

/// Set up the terminal, run the event loop, and guarantee teardown even on
/// error so a panic doesn't leave the operator's terminal in raw mode. Mirrors
/// tail.rs::run_loop.
#[allow(clippy::too_many_arguments)]
fn run_loop(
    ui_rx: UnboundedReceiver<TopMsg>,
    registry: AliasRegistry,
    envelopes: EnvelopeRegistry,
    site: Option<String>,
    sort: TopSort,
    interval_secs: u64,
    filter: Option<String>,
) -> anyhow::Result<()> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let outcome = event_loop(&mut terminal, ui_rx, &registry, &envelopes, site, sort, interval_secs, filter);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    outcome.map_err(|e| anyhow::anyhow!("top TUI: {e}"))
}

#[allow(clippy::too_many_arguments)]
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    mut ui_rx: UnboundedReceiver<TopMsg>,
    registry: &AliasRegistry,
    envelopes: &EnvelopeRegistry,
    site: Option<String>,
    sort: TopSort,
    interval_secs: u64,
    filter: Option<String>,
) -> std::io::Result<()> {
    let mut state = TopState::new(site, sort, interval_secs);
    if let Some(f) = filter {
        state.filter = f;
    }

    loop {
        let now = Instant::now();
        // Drain everything the producer queued since the last tick.
        while let Ok(msg) = ui_rx.try_recv() {
            if state.paused {
                // While paused, still drain (don't let the channel grow
                // unbounded) but don't fold into the live model.
                if let TopMsg::Error(e) = msg {
                    state.status = Some(e);
                }
                continue;
            }
            match msg {
                TopMsg::Fabric(h) => state.apply_fabric(*h, now),
                TopMsg::Footprint(f) => state.apply_footprint(*f, now),
                TopMsg::Env(e) => state.apply_env(*e, now),
                TopMsg::Census(c) => state.apply_census(*c),
                TopMsg::Error(e) => state.status = Some(e),
            }
        }

        if !state.paused {
            state.expire(now);
        }
        // Recompute the sorted producer order + keep the selection valid.
        state.sorted_cache = state.sorted_peers(registry);
        if state.current_index().is_none() {
            state.selected_peer = state.sorted_cache.first().cloned();
        }

        let vm = state.view_model(registry, envelopes, now);
        terminal.draw(|f| draw(f, &state, &vm))?;

        if event::poll(TICK)? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                // Ctrl-C always quits, even mid-filter.
                if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(());
                }
                if state.overlay != Overlay::None {
                    // Modal overlay open (legend / drill-down). Esc/Enter/q/?
                    // close it; ↑/↓ (j/k) still move the selection so the detail
                    // pane can browse producer-to-producer without closing. The
                    // live table keeps updating underneath — this is a lens.
                    match k.code {
                        KeyCode::Esc
                        | KeyCode::Enter
                        | KeyCode::Char('q')
                        | KeyCode::Char('?') => state.overlay = Overlay::None,
                        KeyCode::Down | KeyCode::Char('j') => state.select_next(),
                        KeyCode::Up | KeyCode::Char('k') => state.select_prev(),
                        _ => {}
                    }
                } else if state.editing_filter {
                    // Filter input mode (htop incremental-filter idiom): printable
                    // keys edit the live buffer, the table narrows on the next
                    // draw. Enter applies + leaves the input line; Esc clears.
                    match k.code {
                        KeyCode::Esc => {
                            state.filter.clear();
                            state.editing_filter = false;
                        }
                        KeyCode::Enter => state.editing_filter = false,
                        KeyCode::Backspace => {
                            state.filter.pop();
                        }
                        KeyCode::Char(c) => state.filter.push(c),
                        KeyCode::Down => state.select_next(),
                        KeyCode::Up => state.select_prev(),
                        _ => {}
                    }
                } else {
                    match (k.code, k.modifiers) {
                        (KeyCode::Char('q'), _)
                        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(()),
                        // Esc clears an active filter first (htop), else quits.
                        (KeyCode::Esc, _) => {
                            if state.filter.is_empty() {
                                return Ok(());
                            }
                            state.filter.clear();
                        }
                        // `/` (and htop's `\`) enter the incremental filter.
                        (KeyCode::Char('/'), _) | (KeyCode::Char('\\'), _) => {
                            state.editing_filter = true
                        }
                        (KeyCode::Char('s'), _) => state.cycle_sort(),
                        (KeyCode::Char('p'), _) => state.paused = !state.paused,
                        // `?` opens the legend; `↵` drills into the selected
                        // producer (no-op with nothing selected — empty fabric).
                        (KeyCode::Char('?'), _) => state.overlay = Overlay::Help,
                        (KeyCode::Enter, _) if state.selected_peer.is_some() => {
                            state.overlay = Overlay::Detail;
                        }
                        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => state.select_next(),
                        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => state.select_prev(),
                        _ => {}
                    }
                }
            }
        }
    }
}

// --- rendering ---------------------------------------------------------------

fn draw(f: &mut ratatui::Frame, state: &TopState, vm: &ViewModel) {
    // Footer is totals + help (2 lines), plus a third line when a producer/
    // decode error is held — surfaced, never swallowed.
    let footer_h = if state.status.is_some() { 3 } else { 2 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header weather-strip
            Constraint::Length(class_rows_height(&vm.classes)), // fabric class-load
            Constraint::Min(0),    // producer table
            Constraint::Length(footer_h), // totals + help (+ held error)
        ])
        .split(f.area());

    draw_weather(f, chunks[0], vm);
    draw_classes(f, chunks[1], vm);
    draw_producers(f, chunks[2], state, vm);
    draw_footer(f, chunks[3], state, vm);

    // Modal overlays draw last, on top of the live table (which keeps updating
    // underneath). Read-only lenses — closed with Esc/Enter/q/?.
    match state.overlay {
        Overlay::None => {}
        Overlay::Help => draw_help_overlay(f),
        Overlay::Detail => draw_detail_overlay(f, state, vm),
    }
}

/// A centered rectangle `pct_w` × `pct_h` percent of `area` — the popup frame
/// for the modal overlays.
fn centered_rect(pct_w: u16, pct_h: u16, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_h) / 2),
            Constraint::Percentage(pct_h),
            Constraint::Percentage((100 - pct_h) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_w) / 2),
            Constraint::Percentage(pct_w),
            Constraint::Percentage((100 - pct_w) / 2),
        ])
        .split(vert[1])[1]
}

/// The `?` legend overlay: what the weather chips, honesty colors, producer
/// columns, and envelope bands mean. Laid out as an aligned two-column grid
/// (term | meaning, one concept per row) so it scans cleanly. Static reference
/// text — kept in lock-step with the surfaces it describes.
fn draw_help_overlay(f: &mut ratatui::Frame) {
    let area = centered_rect(76, 88, f.area());
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" goat top — legend  (Esc/? closes) ");
    f.render_widget(Paragraph::new(legend_lines()).block(block), area);
}

/// The legend's lines, apart from drawing so a test can read them.
fn legend_lines() -> Vec<Line<'static>> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);
    // Fixed-width term column → the meaning column lines up into a grid.
    let sec = |s: &str| Line::from(Span::styled(s.to_string(), bold));
    let row = |term: &str, meaning: &str| {
        Line::from(vec![
            Span::raw("  "),
            Span::raw(format!("{term:<11}")),
            Span::raw("  "),
            Span::raw(meaning.to_string()),
        ])
    };
    // Same grid, but the term cell carries its honesty color (the swatch).
    let crow = |term: &str, color: &str, meaning: &str| {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{term:<11}"), color_style(color)),
            Span::raw("  "),
            Span::raw(meaning.to_string()),
        ])
    };
    let blank = || Line::from("");
    vec![
        sec("fabric weather — one honesty-graded chip per domain"),
        row("MESH", "WireGuard link health (measured latency)"),
        row("FABRIC", "Zenoh class-load vs declared envelopes"),
        row("TIME", "internal-sync coherence + UTC confidence"),
        row("PRESENCE", "peers alive on the bus right now"),
        row("PEER", "faults our router measured on peers we do not own"),
        row(
            "",
            "amber on trust: a client refuses our certificate; candidates, not faults",
        ),
        blank(),
        sec("honesty colors — a color means exactly what it says"),
        crow("green", "green", "healthy"),
        crow("amber", "amber", "degrading"),
        crow("red", "red", "unhealthy"),
        crow("idle", "idle", "no beat lately — maybe deliberately off, not a fault"),
        crow("unmetered", "unmetered", "never measured"),
        Line::from(Span::styled(
            "  absence is not illness: idle/unmetered are unconfirmed, never a red alarm",
            dim,
        )),
        blank(),
        sec("producer columns"),
        row("#", "rank in the current sort"),
        row("PEER", "publisher (alias or anchor)"),
        row("MSG/S B/S", "publish throughput"),
        row("SUBS", "distinct consumers reached (census); — = not yet measured"),
        row("BAND", "envelope verdict: OVER / HIGH / ok / LOW / — (none)"),
        row("SHARES", "per-class message mix"),
        row("PATH LAT", "mesh route + RTT (— when unmeasured)"),
        blank(),
        sec("keys"),
        row("↵", "drill into selected producer (allotment + consumers)"),
        row("↑/↓ j/k", "move selection"),
        row("/", "filter producers"),
        row("s", "cycle sort"),
        row("p", "pause refresh"),
        row("? · q/Esc", "legend · quit"),
        blank(),
        Line::from(Span::styled(
            "  live, read-only — goat top never writes to the fabric",
            dim,
        )),
    ]
}

/// The `↵` producer drill-down: the selected producer's full footprint plus its
/// live consumer fan-out from the census (the exact ZIDs it reaches). Honest
/// throughout — an empty census shows "no consumers observed", never a faked 0.
fn draw_detail_overlay(f: &mut ratatui::Frame, state: &TopState, vm: &ViewModel) {
    let area = centered_rect(72, 72, f.area());
    f.render_widget(Clear, area);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);

    let Some(anchor) = state.selected_peer.as_deref() else {
        f.render_widget(
            Paragraph::new(Span::styled("no producer selected", dim))
                .block(Block::default().borders(Borders::ALL).title(" producer detail ")),
            area,
        );
        return;
    };
    let Some(p) = vm.producers.iter().find(|p| p.anchor == anchor) else {
        f.render_widget(
            Paragraph::new(Span::styled(
                "selected producer left the window (expired)",
                color_style("idle"),
            ))
            .block(Block::default().borders(Borders::ALL).title(" producer detail ")),
            area,
        );
        return;
    };

    // Published topics under this anchor + the distinct consumers they reach.
    // Both read straight off the census (`publications` is the publish-side
    // fact; `fanout_for_anchor` intersects it with every subscription).
    let mut topics: Vec<&String> = state
        .census
        .publications
        .keys()
        .filter(|ke| ke.split('/').next() == Some(anchor))
        .collect();
    topics.sort();
    let consumers = state.census.fanout_for_anchor(anchor);

    // Left label column, fixed-width so the values line up (was `:<10`, which
    // jammed a 10-char label like "throughput" against its value).
    let field = |k: &str, v: String| {
        Line::from(vec![Span::styled(format!("  {k:<11}"), dim), Span::raw(v)])
    };
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(format!("  {}", p.peer_display), bold)));
    lines.push(Line::from(Span::styled(format!("  anchor     {anchor}"), dim)));
    lines.push(Line::from(""));
    lines.push(field("throughput", format!("{:.0} msgs/s   {:.0} B/s", p.sample_rate, p.byte_rate)));
    lines.push(Line::from(vec![
        Span::styled(format!("  {:<11}", "band"), dim),
        Span::styled(
            if p.band_label.is_empty() { "—".to_string() } else { p.band_label.clone() },
            color_style(&p.band_color),
        ),
    ]));
    // When an axis is out of its envelope, show the allotment it breached and by
    // how much — "12 msgs/s · max 5 · +7 over (240%)" — one line per off-nominal
    // axis. In-envelope / no-envelope producers add nothing here.
    for a in &p.band_axes {
        let edge = if a.verdict == "LOW" { "floor" } else if a.verdict == "HIGH" { "high" } else { "max" };
        let over = if a.delta >= 0.0 { "over" } else { "under" };
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(format!("{:<7}", a.axis), dim),
            Span::raw(format!("{}  ·  ", fmt_rate(a.current))),
            Span::styled(a.verdict.to_string(), color_style(a.color)),
            Span::raw(format!(
                " {} {}  ·  {}{} {} ({:.0}%)",
                edge,
                fmt_rate(a.limit),
                if a.delta >= 0.0 { "+" } else { "−" },
                fmt_rate(a.delta.abs()),
                over,
                a.pct,
            )),
        ]));
    }
    lines.push(field("shares", if p.shares.is_empty() { "—".into() } else { p.shares.clone() }));
    lines.push(field("path", if p.path.is_empty() { "—".into() } else { p.path.clone() }));
    lines.push(field("latency", if p.latency.is_empty() { "—".into() } else { p.latency.clone() }));
    lines.push(Line::from(""));

    // Consumer fan-out — the subscribe-side half of the pub/sub picture.
    lines.push(Line::from(Span::styled(
        format!("  consumers reached: {}", consumers.len()),
        bold,
    )));
    if state.census.is_empty() {
        lines.push(Line::from(Span::styled(
            "  census empty — consumer set not yet measured (not a confident 0)",
            color_style("unmetered"),
        )));
    } else if consumers.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no subscriber's key-expression intersects this producer's topics",
            dim,
        )));
    } else {
        for z in consumers.iter().take(6) {
            lines.push(Line::from(Span::styled(format!("    ← {z}"), dim)));
        }
        if consumers.len() > 6 {
            lines.push(Line::from(Span::styled(
                format!("    … +{} more", consumers.len() - 6),
                dim,
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  published topics: {}", topics.len()),
        bold,
    )));
    if topics.is_empty() {
        lines.push(Line::from(Span::styled(
            "  none observed in the census window",
            dim,
        )));
    } else {
        for t in topics.iter().take(5) {
            lines.push(Line::from(Span::styled(format!("    • {}", truncate(t, 60)), dim)));
        }
        if topics.len() > 5 {
            lines.push(Line::from(Span::styled(format!("    … +{} more", topics.len() - 5), dim)));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑/↓ browse producers · Esc/↵ closes",
        dim,
    )));
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" producer detail  (↑/↓ browse · Esc closes) ");
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn class_rows_height(classes: &ClassPaneVM) -> u16 {
    let n = match classes {
        ClassPaneVM::Rows(rows) => rows.len() as u16,
        _ => 0,
    };
    // border (2) + one line per class, min 3 so the block + a note always show.
    (n + 2).max(3)
}

fn draw_weather(f: &mut ratatui::Frame, area: ratatui::layout::Rect, vm: &ViewModel) {
    let mut spans: Vec<Span> = vec![Span::styled("env  ", Style::default().add_modifier(Modifier::BOLD))];
    for (i, chip) in vm.chips.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   ·   "));
        }
        spans.push(Span::styled(
            format!("{} ", chip.domain.to_uppercase()),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        // The state string is already stale-collapsed in the view model;
        // color it via Style::fg (idle/unmetered dim, never a false green).
        spans.push(Span::styled(chip.state.to_uppercase(), color_style(&chip.state)));
        spans.push(Span::styled(
            format!(" n={}", chip.sample_count),
            Style::default().add_modifier(Modifier::DIM),
        ));
        if let Some(age) = &chip.stale_age {
            spans.push(Span::styled(
                format!(" (stale {age})"),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            ));
        }
    }
    let p = Paragraph::new(Line::from(spans))
        .block(Block::default().borders(Borders::ALL).title("goat top — fabric weather"));
    f.render_widget(p, area);
}

fn draw_classes(f: &mut ratatui::Frame, area: ratatui::layout::Rect, vm: &ViewModel) {
    let block = Block::default().borders(Borders::ALL).title("fabric class load");
    let rows = match &vm.classes {
        ClassPaneVM::Absent => {
            f.render_widget(
                Paragraph::new(Span::styled(
                    "no fabric-health beat yet — is goat-fabric-health running?",
                    color_style("unmetered"),
                ))
                .block(block),
                area,
            );
            return;
        }
        ClassPaneVM::Stale(age) => {
            f.render_widget(
                Paragraph::new(Span::styled(
                    format!("fabric weather is stale ({age}) — bars hidden, not faked"),
                    color_style("idle"),
                ))
                .block(block),
                area,
            );
            return;
        }
        ClassPaneVM::Rows(rows) => rows,
    };
    let inner = block.inner(area);
    f.render_widget(block, area);
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); rows.len().max(1)])
        .split(inner);
    for (i, c) in rows.iter().enumerate() {
        if i >= areas.len() {
            break;
        }
        // Fixed-width label so the centered gauge labels line up into columns
        // (see format_class_label — ratatui centers a Gauge label by its width,
        // so every row must be identical width or the status column goes ragged).
        let label = format_class_label(c);
        let gauge = Gauge::default()
            .gauge_style(color_style(&c.color))
            .ratio(c.ratio)
            .label(label);
        f.render_widget(gauge, areas[i]);
    }
}

fn draw_producers(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &TopState,
    vm: &ViewModel,
) {
    // Numeric columns (MSG/S, B/S, LATENCY) right-align on both header and data
    // so a magnitude column reads as one — matching the `--once` renderer's
    // `{:>}` formatting. The rest stay left.
    let header = Row::new(vec![
        Cell::from("#"),
        Cell::from("PEER"),
        rcell("MSG/S"),
        rcell("B/S"),
        rcell("SUBS"),
        Cell::from("BAND"),
        Cell::from("SHARES"),
        Cell::from(""),
        Cell::from("PATH"),
        rcell("LATENCY"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));
    let rows: Vec<Row> = vm
        .producers
        .iter()
        .map(|p| {
            let selected = state.selected_peer.as_deref() == Some(p.anchor.as_str());
            producer_row(p, selected)
        })
        .collect();

    // Title carries the active filter + shown/total so a narrowed pane never
    // reads as the whole fabric. While editing the filter, show the live buffer
    // with a cursor so the operator sees what they're typing.
    let title = if state.editing_filter {
        format!("producers · filter: {}▏ (Esc clears · Enter applies)", state.filter)
    } else if let Some(f) = &vm.filter {
        format!(
            "producers ({} of {} shown · filter \"{}\" · / edit · sort: {})",
            vm.totals.shown,
            vm.totals.active,
            f,
            sort_label(state.sort),
        )
    } else {
        format!(
            "producers ({} active · / filter · sort: {} · s cycles)",
            vm.totals.active,
            sort_label(state.sort),
        )
    };
    let widths = [
        Constraint::Length(3),
        Constraint::Length(22),
        Constraint::Length(9),
        Constraint::Length(11),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(20),
        Constraint::Length(4),
        Constraint::Length(20),
        Constraint::Length(11),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(table, area);
}

/// A right-aligned table cell — for numeric/magnitude columns so the header
/// label sits over a right-aligned column of values that line up on the ones
/// digit (ratatui defaults cells to left-aligned).
fn rcell<'a>(s: impl Into<String>) -> Cell<'a> {
    Cell::from(Text::from(s.into()).alignment(Alignment::Right))
}

/// The SUBS cell: a right-aligned consumer count, or a DIM `—` when the census
/// is empty (the grant is deploy-pending / no data). The dim styling matches the
/// idle/unmetered honesty palette — an unknown consumer set reads as unknown,
/// never a confident 0.
fn subs_cell<'a>(subscribers: Option<usize>) -> Cell<'a> {
    let cell = rcell(subs_label(subscribers));
    match subscribers {
        Some(_) => cell,
        None => cell.style(color_style("unmetered")),
    }
}

fn producer_row<'a>(p: &ProducerVM, selected: bool) -> Row<'a> {
    let near = if p.near_limit { "⚠" } else { "" };
    let cells = vec![
        Cell::from(format!("{}", p.display_pos)),
        Cell::from(truncate(&p.peer_display, 22)),
        rcell(format!("{:.0}", p.sample_rate)),
        rcell(format!("{:.0}", p.byte_rate)),
        // Consumer fan-out from the census, or an honest `—` when the census is
        // empty (grant deploy-pending) — dim, never a fabricated 0.
        subs_cell(p.subscribers),
        // The envelope band keeps its own honesty color even under the row's
        // selection/near-limit style.
        Cell::from(p.band_label.clone()).style(color_style(&p.band_color)),
        Cell::from(truncate(&p.shares, 20)),
        Cell::from(near.to_string()),
        Cell::from(truncate(&p.path, 20)),
        rcell(p.latency.clone()),
    ];
    let style = if selected {
        Style::default().add_modifier(Modifier::REVERSED).add_modifier(Modifier::BOLD)
    } else if p.near_limit {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    Row::new(cells).style(style)
}

fn draw_footer(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &TopState,
    vm: &ViewModel,
) {
    let t = &vm.totals;
    let totals = Line::from(vec![
        Span::raw(format!("peers {}   ", t.active)),
        Span::raw(format!("{:.0} msgs/s   {:.0} B/s   ", t.total_msgs, t.total_bytes)),
        Span::raw(format!("links {} direct / {} relayed   ", t.direct, t.relayed)),
        Span::raw(format!("presence {} alive / {} dropped", t.alive, t.dropped)),
    ]);
    let paused = if state.paused { "  [PAUSED]" } else { "" };
    let help_text = if state.editing_filter {
        " type to filter producers · Enter applies · Esc clears · matches alias or anchor ".to_string()
    } else {
        format!(" q/Esc quit · ? legend · ↵ detail · / filter · s sort · p pause{paused} · ↑/↓ select · live, read-only ")
    };
    let help = Line::from(vec![Span::styled(
        help_text,
        Style::default().add_modifier(Modifier::DIM),
    )]);
    let mut lines = vec![totals, help];
    // Surface a held producer/decode error — honesty ethos: a subscriber error
    // is shown, never silently swallowed. Keeps the last error visible until
    // the next one replaces it.
    if let Some(e) = &state.status {
        lines.push(Line::from(Span::styled(
            format!(" ⚠ {}", truncate(e, 100)),
            Style::default().fg(Color::Red),
        )));
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// Direct vs relayed connected link counts from the mesh domain (0/0 absent).
fn mesh_link_counts(state: &TopState) -> (usize, usize) {
    let Some(b) = state.env.get("mesh") else { return (0, 0) };
    let Some(EnvDetail::Mesh(d)) = &b.env.detail else { return (0, 0) };
    let connected: Vec<_> = d.links.iter().filter(|l| l.connected).collect();
    let relayed = connected.iter().filter(|l| !l.direct).count();
    (connected.len() - relayed, relayed)
}

/// Alive / dropped-recent counts from the presence domain (0/0 absent).
fn presence_counts(state: &TopState) -> (u32, u32) {
    let Some(b) = state.env.get("presence") else { return (0, 0) };
    let Some(EnvDetail::Presence(d)) = &b.env.detail else { return (0, 0) };
    (d.alive_total, d.dropped_recent)
}

fn sort_label(sort: TopSort) -> &'static str {
    match sort {
        TopSort::Msgs => "msgs/s",
        TopSort::Bytes => "B/s",
        TopSort::Peer => "peer",
    }
}

/// Compact rate: integer when whole, else one decimal — envelope allotments can
/// be fractional (e.g. 0.5 msgs/s), where `{:.0}` would misleadingly print 0/1.
fn fmt_rate(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health_wire::{
        ClassHealthV1, ClassShareV1, MeshDetailV1, MeshLinkHealthV1, TimeDetailV1,
    };

    fn fp(peer: &str, msgs: f64, bytes: f64, near: bool) -> ProducerFootprintV1 {
        ProducerFootprintV1 {
            peer: peer.to_string(),
            ts_unix_ms: 0,
            sample_rate: msgs,
            byte_rate: bytes,
            shares: vec![],
            rank: 0,
            near_limit: near,
        }
    }

    fn fabric(site: &str, window: u32, classes: Vec<ClassHealthV1>) -> FabricHealthV1 {
        FabricHealthV1 {
            site: site.to_string(),
            ts_unix_ms: 1,
            classes,
            overall_color: "green".to_string(),
            active_producers: 1,
            window_secs: window,
        }
    }

    #[test]
    fn sort_by_msgs_descending_with_peer_tiebreak() {
        let mut st = TopState::new(None, TopSort::Msgs, 2);
        let now = Instant::now();
        st.apply_footprint(fp("b", 10.0, 5.0, false), now);
        st.apply_footprint(fp("a", 30.0, 1.0, false), now);
        st.apply_footprint(fp("c", 30.0, 9.0, false), now); // tie with a on msgs
        let order = st.sorted_peers(&AliasRegistry::empty());
        // 30s first (a before c by peer-name tiebreak), then 10.
        assert_eq!(order, vec!["a", "c", "b"]);
    }

    #[test]
    fn sort_by_bytes() {
        let mut st = TopState::new(None, TopSort::Bytes, 2);
        let now = Instant::now();
        st.apply_footprint(fp("a", 30.0, 1.0, false), now);
        st.apply_footprint(fp("b", 10.0, 99.0, false), now);
        assert_eq!(st.sorted_peers(&AliasRegistry::empty()), vec!["b", "a"]);
    }

    #[test]
    fn sort_by_peer_name() {
        let mut st = TopState::new(None, TopSort::Peer, 2);
        let now = Instant::now();
        st.apply_footprint(fp("zeta", 99.0, 99.0, false), now);
        st.apply_footprint(fp("alpha", 1.0, 1.0, false), now);
        assert_eq!(st.sorted_peers(&AliasRegistry::empty()), vec!["alpha", "zeta"]);
    }

    #[test]
    fn cycle_sort_rotates() {
        let mut st = TopState::new(None, TopSort::Msgs, 2);
        st.cycle_sort();
        assert_eq!(st.sort, TopSort::Bytes);
        st.cycle_sort();
        assert_eq!(st.sort, TopSort::Peer);
        st.cycle_sort();
        assert_eq!(st.sort, TopSort::Msgs);
    }

    #[test]
    fn footprints_expire_past_stale_window() {
        let mut st = TopState::new(None, TopSort::Msgs, 2);
        let start = Instant::now();
        // window 5s, interval 2 → window = max(5,2)=5, ×3 = 15s stale window.
        st.apply_fabric(fabric("s", 5, vec![]), start);
        st.apply_footprint(fp("a", 10.0, 1.0, false), start);
        // 10s later — still fresh.
        let t10 = start + Duration::from_secs(10);
        st.expire(t10);
        assert!(st.footprints.contains_key("a"));
        // 16s later — past the 15s window, expired.
        let t16 = start + Duration::from_secs(16);
        st.expire(t16);
        assert!(!st.footprints.contains_key("a"));
    }

    #[test]
    fn domain_beat_staleness_by_window() {
        let env = EnvHealthV1 {
            domain: "mesh".to_string(),
            scope: "s".to_string(),
            ts_unix_ms: 1,
            overall_color: "green".to_string(),
            sample_count: 3,
            window_secs: 30,
            detail: None,
        };
        let start = Instant::now();
        let beat = DomainBeat { env, seen: start };
        // fresh at 60s (< 90 = 30×3), stale at 91s.
        assert!(!TopState::is_stale(&beat, start + Duration::from_secs(60)));
        assert!(TopState::is_stale(&beat, start + Duration::from_secs(91)));
    }

    #[test]
    fn color_map_dims_ungrounded_never_false_green() {
        assert_eq!(color_style("green").fg, Some(Color::Green));
        assert_eq!(color_style("amber").fg, Some(Color::Yellow));
        assert_eq!(color_style("red").fg, Some(Color::Red));
        // idle + unmetered both dim — the honesty guard.
        assert_eq!(color_style("idle").fg, Some(Color::DarkGray));
        assert_eq!(color_style("unmetered").fg, Some(Color::DarkGray));
        assert_ne!(color_style("idle").fg, Some(Color::Green));
    }

    #[test]
    fn site_filter_drops_other_scopes() {
        let mut st = TopState::new(Some("zagros".to_string()), TopSort::Msgs, 2);
        let now = Instant::now();
        st.apply_fabric(fabric("nova-a", 5, vec![]), now);
        assert!(st.fabric.is_none(), "wrong-site fabric beat dropped");
        st.apply_fabric(fabric("zagros", 5, vec![]), now);
        assert!(st.fabric.is_some(), "matching-site fabric beat kept");
    }

    #[test]
    fn shares_label_top_classes_by_share() {
        let mut f = fp("a", 10.0, 1.0, false);
        f.shares = vec![
            ClassShareV1 { class: "C0".to_string(), sample_rate: 2.0, byte_rate: 0.0 },
            ClassShareV1 { class: "C2".to_string(), sample_rate: 8.0, byte_rate: 0.0 },
        ];
        // C2 is the larger share → first.
        assert_eq!(shares_label(&f), "C2 80% · C0 20%");
        // empty shares → honest dash.
        assert_eq!(shares_label(&fp("b", 1.0, 1.0, false)), "—");
    }

    #[test]
    fn mesh_join_honest_dash_when_no_namespace_match() {
        // Footprint peer is a UUID anchor; mesh link keys on a hostname fqdn —
        // different namespaces, so the join MUST NOT match (honest "—").
        let env = EnvHealthV1 {
            domain: "mesh".to_string(),
            scope: "s".to_string(),
            ts_unix_ms: 1,
            overall_color: "amber".to_string(),
            sample_count: 1,
            window_secs: 30,
            detail: Some(EnvDetail::Mesh(MeshDetailV1 {
                links: vec![MeshLinkHealthV1 {
                    peer_fqdn: "thor.zagros.example.net".to_string(),
                    peer_ip: "100.127.0.1".to_string(),
                    color: "amber".to_string(),
                    connected: true,
                    direct: false,
                    conn_type: "Relayed".to_string(),
                    latency_ms: 0.0,
                    relay_address: "rels://relay-rla.x:33073".to_string(),
                    handshake_age_ms: 1000,
                    transition_count: 0,
                }],
                vantage_host: "thor".to_string(),
                peers_total: 1,
                peers_connected: 1,
            })),
        };
        let reg = AliasRegistry::empty();
        let (path, lat) = mesh_join(&fp("e7a9c75b1234567890abcdef01234567", 1.0, 1.0, false), &reg, Some(&env));
        assert_eq!(path, "—");
        assert_eq!(lat, "—");
    }

    #[test]
    fn mesh_join_matches_on_short_name_when_anchor_is_hostname() {
        // When the footprint peer anchor IS a hostname-like label that matches
        // the mesh link short-name, we join and render the real path.
        let env = EnvHealthV1 {
            domain: "mesh".to_string(),
            scope: "s".to_string(),
            ts_unix_ms: 1,
            overall_color: "amber".to_string(),
            sample_count: 1,
            window_secs: 30,
            detail: Some(EnvDetail::Mesh(MeshDetailV1 {
                links: vec![MeshLinkHealthV1 {
                    peer_fqdn: "synth-01.zagros.example.net".to_string(),
                    peer_ip: "100.127.0.5".to_string(),
                    color: "green".to_string(),
                    connected: true,
                    direct: true,
                    conn_type: "P2P".to_string(),
                    latency_ms: 12.0,
                    relay_address: String::new(),
                    handshake_age_ms: 1000,
                    transition_count: 0,
                }],
                vantage_host: "thor".to_string(),
                peers_total: 1,
                peers_connected: 1,
            })),
        };
        let reg = AliasRegistry::empty();
        let (path, lat) = mesh_join(&fp("synth-01", 1.0, 1.0, false), &reg, Some(&env));
        assert_eq!(path, "direct P2P");
        assert_eq!(lat, "12 ms");
    }

    #[test]
    fn mesh_join_dash_when_mesh_absent() {
        let reg = AliasRegistry::empty();
        let (path, lat) = mesh_join(&fp("a", 1.0, 1.0, false), &reg, None);
        assert_eq!(path, "—");
        assert_eq!(lat, "—");
    }

    #[test]
    fn relay_label_extracts_short_name() {
        assert_eq!(relay_label("rels://relay-dub.zagros.example:33073"), "via relay-dub");
        assert_eq!(relay_label(""), "via relay");
    }

    #[test]
    fn class_label_maps_known_codes_and_falls_back() {
        assert_eq!(class_label("C0"), "control");
        assert_eq!(class_label("C1"), "mission");
        assert_eq!(class_label("C2"), "situational");
        assert_eq!(class_label("C3"), "bulk");
        // A future / unknown class renders as just its bare code (no panic, no
        // wrong name) — graceful.
        assert_eq!(class_label("C4"), "");
        assert_eq!(class_label("weird"), "");
    }

    #[test]
    fn class_load_row_shows_code_and_canonical_name() {
        let mut st = TopState::new(None, TopSort::Msgs, 2);
        let now = Instant::now();
        let cls = |code: &str, color: &str| ClassHealthV1 {
            class: code.to_string(),
            sample_rate: 4.0,
            byte_rate: 100.0,
            target_sample_rate: 5000.0,
            target_byte_rate: 9000.0,
            headroom_pct: 0.9,
            color: color.to_string(),
            sample_count: 50,
        };
        st.apply_fabric(
            fabric("zagros", 5, vec![cls("C0", "green"), cls("C2", "green"), cls("C4", "green")]),
            now,
        );
        let vm = st.view_model(&AliasRegistry::empty(), &EnvelopeRegistry::empty(), now);
        let out = render_plain(&vm, TopSort::Msgs);
        // Code + canonical name side by side (the class-load row is the legend).
        assert!(out.contains("C0  control"), "C0 control in the row:\n{out}");
        assert!(out.contains("C2  situational"), "C2 situational in the row:\n{out}");
        // Unknown C4 still renders its bare code (graceful fallback).
        assert!(out.contains("C4"), "unknown class code still rendered:\n{out}");
    }

    fn class_vm(code: &str, cur: f64, target: f64, color: &str) -> ClassVM {
        ClassVM {
            class: code.to_string(),
            sample_rate: cur,
            target_sample_rate: target,
            ratio: cur / target.max(1.0),
            color: color.to_string(),
        }
    }

    /// The bug this guards: the class rows in `goat top` were horizontally
    /// ragged because the "N/M msgs/s" field was variable-width, so the status
    /// column drifted (and the ratatui gauge CENTERS the label by width, so
    /// unequal-width rows shift too). Every class-load label must be the exact
    /// same width, with `msgs/s` and the status word at identical offsets —
    /// across small + large + differing-name magnitudes.
    #[test]
    fn class_load_labels_are_column_aligned() {
        // The exact magnitudes from the ragged screenshot: 1-digit vs 2-digit
        // current, 4- to 6-digit target, short + long names.
        let rows = [
            class_vm("C0", 3.0, 5000.0, "green"),
            class_vm("C1", 0.0, 20000.0, "idle"),
            class_vm("C2", 11.0, 50000.0, "green"),
            class_vm("C3", 0.0, 200000.0, "idle"),
        ];
        let labels: Vec<String> = rows.iter().map(format_class_label).collect();

        // 1. Every label is the exact same character width.
        let w0 = labels[0].chars().count();
        for l in &labels {
            assert_eq!(l.chars().count(), w0, "labels differ in width:\n{labels:#?}");
        }
        // 2. `msgs/s` starts at the same column in every row.
        let msgs = labels[0].find("msgs/s").expect("msgs/s present");
        for l in &labels {
            assert_eq!(l.find("msgs/s"), Some(msgs), "msgs/s column drifts:\n{l}");
        }
        // 3. The status word starts at the same column in every row.
        assert_eq!(labels[0].find("GREEN"), labels[2].find("GREEN"));
        assert_eq!(labels[1].find("IDLE"), labels[3].find("IDLE"));
        assert_eq!(
            labels[0].find("GREEN"),
            labels[1].find("IDLE"),
            "status column drifts between GREEN and IDLE rows:\n{labels:#?}"
        );
    }

    /// Prove the alignment survives the ACTUAL interactive render (gauge labels
    /// are centered by ratatui, so this checks the real terminal buffer, not
    /// just the format string): the status word sits in the same buffer column
    /// on every class row.
    #[test]
    fn class_load_columns_align_in_interactive_buffer() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let mut st = TopState::new(None, TopSort::Msgs, 2);
        let now = Instant::now();
        let cls = |code: &str, cur: f64, target: f64, color: &str| ClassHealthV1 {
            class: code.to_string(),
            sample_rate: cur,
            byte_rate: 100.0,
            target_sample_rate: target,
            target_byte_rate: 9000.0,
            headroom_pct: 0.9,
            color: color.to_string(),
            sample_count: 50,
        };
        st.apply_fabric(
            fabric(
                "zagros",
                5,
                vec![
                    cls("C0", 3.0, 5000.0, "green"),
                    cls("C1", 0.0, 20000.0, "idle"),
                    cls("C2", 11.0, 50000.0, "green"),
                    cls("C3", 0.0, 200000.0, "idle"),
                ],
            ),
            now,
        );
        let vm = st.view_model(&AliasRegistry::empty(), &EnvelopeRegistry::empty(), now);
        let mut term = Terminal::new(TestBackend::new(140, 30)).expect("test terminal");
        term.draw(|f| draw(f, &st, &vm)).expect("draw");
        let screen = buffer_to_string(term.backend().buffer());
        // Each rendered class row contains its status; assert the status word is
        // at the same column on the C0 (GREEN) and C2 (GREEN) rows, and the C1
        // (IDLE) and C3 (IDLE) rows — the columns that used to drift.
        let col_of = |needle: &str, after: &str| -> Option<usize> {
            screen.lines().find_map(|line| {
                if line.contains(after) {
                    line.find(needle)
                } else {
                    None
                }
            })
        };
        // control (C0) and situational (C2) both GREEN — status column must match.
        let c0 = col_of("GREEN", "control").expect("C0 row GREEN");
        let c2 = col_of("GREEN", "situational").expect("C2 row GREEN");
        assert_eq!(c0, c2, "status column drifts between control and situational:\n{screen}");
        // mission (C1) and bulk (C3) both IDLE.
        let c1 = col_of("IDLE", "mission").expect("C1 row IDLE");
        let c3 = col_of("IDLE", "bulk").expect("C3 row IDLE");
        assert_eq!(c1, c3, "status column drifts between mission and bulk:\n{screen}");
    }

    #[test]
    fn selection_rides_peer_anchor_across_resort() {
        let mut st = TopState::new(None, TopSort::Msgs, 2);
        let now = Instant::now();
        st.apply_footprint(fp("a", 30.0, 1.0, false), now);
        st.apply_footprint(fp("b", 10.0, 99.0, false), now);
        st.sorted_cache = st.sorted_peers(&AliasRegistry::empty()); // [a, b] by msgs
        st.selected_peer = Some("b".to_string());
        assert_eq!(st.current_index(), Some(1));
        // Re-sort by bytes → b first; selection still points at b (index 0).
        st.sort = TopSort::Bytes;
        st.sorted_cache = st.sorted_peers(&AliasRegistry::empty());
        assert_eq!(st.current_index(), Some(0));
    }

    #[test]
    fn plain_snapshot_renders_panes_with_honesty() {
        // A synthetic state: a green fabric beat with one class, one producer,
        // and NO mesh/time/presence beats (so those chips read unmetered).
        let mut st = TopState::new(None, TopSort::Msgs, 2);
        let now = Instant::now();
        st.apply_fabric(
            fabric(
                "zagros",
                5,
                vec![ClassHealthV1 {
                    class: "C2".to_string(),
                    sample_rate: 40.0,
                    byte_rate: 1000.0,
                    target_sample_rate: 100.0,
                    target_byte_rate: 5000.0,
                    headroom_pct: 0.6,
                    color: "green".to_string(),
                    sample_count: 200,
                }],
            ),
            now,
        );
        let mut f = fp("maintainer-operator", 40.0, 1000.0, false);
        f.shares = vec![ClassShareV1 { class: "C2".to_string(), sample_rate: 40.0, byte_rate: 1000.0 }];
        st.apply_footprint(f, now);
        st.sorted_cache = st.sorted_peers(&AliasRegistry::empty());

        let vm = st.view_model(&AliasRegistry::empty(), &EnvelopeRegistry::empty(), now);
        let out = render_plain(&vm, TopSort::Msgs);

        // Weather strip: fabric grounded green; the other three honestly unmetered.
        assert!(out.contains("FABRIC GREEN n=200"), "fabric chip grounded:\n{out}");
        assert!(out.contains("MESH UNMETERED n=0"), "absent mesh reads unmetered, never green:\n{out}");
        assert!(out.contains("TIME UNMETERED n=0"));
        assert!(out.contains("PRESENCE UNMETERED n=0"));
        // No peer-health beat: the peer chip reads unmetered, never green.
        assert!(out.contains("PEER UNMETERED n=0"));
        // Class-load row with the of-target form.
        assert!(out.contains("C2"));
        assert!(out.contains("40 /") && out.contains("100 msgs/s"), "class of-target shown:\n{out}");
        // Producer row: alias degrades to the raw handle; mesh absent → PATH "—".
        assert!(out.contains("maintainer-operator"));
        assert!(out.contains("—"), "no mesh beat → honest dash path:\n{out}");
        // Totals line.
        assert!(out.contains("TOTALS  peers 1"), "totals present:\n{out}");
        // Never a faked green for an ungrounded domain.
        assert!(!out.contains("MESH GREEN"));
    }

    #[test]
    fn producer_rows_numbered_by_sort_position_not_meter_rank() {
        // Meter ranks are out of msgs/s order; the rendered `#` must follow the
        // sort (1,2,3 down the rows), so a top user reads it in order.
        let mut st = TopState::new(None, TopSort::Msgs, 2);
        let now = Instant::now();
        let mut a = fp("a-hi", 30.0, 1.0, false);
        a.rank = 5; // meter says rank 5, but it's top by msgs/s → #1
        let mut b = fp("b-mid", 20.0, 1.0, false);
        b.rank = 1; // meter rank 1, but middle msgs/s → #2
        let mut c = fp("c-lo", 10.0, 1.0, false);
        c.rank = 8; // meter rank 8, lowest msgs/s → #3
        st.apply_footprint(a, now);
        st.apply_footprint(b, now);
        st.apply_footprint(c, now);
        st.sorted_cache = st.sorted_peers(&AliasRegistry::empty());

        let vm = st.view_model(&AliasRegistry::empty(), &EnvelopeRegistry::empty(), now);
        // The view model assigns display_pos 1..N in sort order.
        assert_eq!(vm.producers[0].peer_display, "a-hi");
        assert_eq!(vm.producers[0].display_pos, 1);
        assert_eq!(vm.producers[1].display_pos, 2);
        assert_eq!(vm.producers[2].display_pos, 3);

        let out = render_plain(&vm, TopSort::Msgs);
        let num = |peer: &str| -> String {
            out.lines()
                .find(|l| l.contains(peer))
                .unwrap_or("")
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string()
        };
        assert_eq!(num("a-hi"), "1", "top row #1 not meter rank 5:\n{out}");
        assert_eq!(num("b-mid"), "2");
        assert_eq!(num("c-lo"), "3", "lowest row #3 not meter rank 8:\n{out}");

        // After cycling to bytes sort, numbering still reads 1..N in the NEW
        // order (all equal bytes here → peer-name tiebreak: a-hi,b-mid,c-lo).
        st.sort = TopSort::Bytes;
        st.sorted_cache = st.sorted_peers(&AliasRegistry::empty());
        let vm2 = st.view_model(&AliasRegistry::empty(), &EnvelopeRegistry::empty(), now);
        assert_eq!(vm2.producers[0].display_pos, 1);
        assert_eq!(vm2.producers[1].display_pos, 2);
        assert_eq!(vm2.producers[2].display_pos, 3);
    }

    #[test]
    fn envelope_bands_and_incremental_filter() {
        use panoscope_core::envelope::EnvelopeRegistry;
        use panoscope_core::portal::EnvelopeRow;
        // "acme" has a declared envelope (msgs ceiling 120); "ghost" has none.
        // bytes edges are 0 → that axis is disabled, so msgs/s governs.
        let env = EnvelopeRegistry::from_rows([EnvelopeRow {
            anchor: "acme".into(),
            min_msgs_per_s: 1.0,
            mean_msgs_per_s: 5.0,
            high_msgs_per_s: 30.0,
            max_msgs_per_s: 120.0,
            min_bytes_per_s: 0.0,
            mean_bytes_per_s: 0.0,
            high_bytes_per_s: 0.0,
            max_bytes_per_s: 0.0,
            source: "default".into(),
        }]);
        let mut st = TopState::new(None, TopSort::Msgs, 2);
        let now = Instant::now();
        st.apply_footprint(fp("acme", 200.0, 0.0, false), now); // > max → OVER
        st.apply_footprint(fp("ghost", 5.0, 0.0, false), now); // no envelope
        st.sorted_cache = st.sorted_peers(&AliasRegistry::empty());

        let vm = st.view_model(&AliasRegistry::empty(), &env, now);
        let band = |peer: &str| {
            vm.producers
                .iter()
                .find(|p| p.peer_display == peer)
                .map(|p| p.band_label.clone())
                .unwrap_or_default()
        };
        assert_eq!(band("acme"), "OVER", "flood past declared max reads OVER");
        // No declared envelope must render an honest dash, never a faked green.
        assert_eq!(band("ghost"), "—", "no envelope → honest dash");

        // The htop-style incremental filter narrows the producer set to
        // alias/anchor matches; totals stay honest (active unfiltered, shown=1).
        st.filter = "acme".into();
        let vm2 = st.view_model(&AliasRegistry::empty(), &env, now);
        assert_eq!(vm2.producers.len(), 1, "filter narrows to the match");
        assert_eq!(vm2.producers[0].peer_display, "acme");
        assert_eq!(vm2.filter.as_deref(), Some("acme"));
        assert_eq!(vm2.totals.active, 2, "unfiltered active count stays honest");
        assert_eq!(vm2.totals.shown, 1);
    }

    #[test]
    fn subscriber_fanout_projects_count_or_honest_dash_when_census_empty() {
        use panoscope_core::census::{Census, ResourceRole};
        let mut st = TopState::new(None, TopSort::Msgs, 2);
        let now = Instant::now();
        st.apply_footprint(fp("acme", 10.0, 100.0, false), now);
        st.sorted_cache = st.sorted_peers(&AliasRegistry::empty());

        // Default (empty) census → honest None, which renders `—`. NEVER a
        // fabricated 0 — an unknown consumer set must read unknown.
        let vm_empty = st.view_model(&AliasRegistry::empty(), &EnvelopeRegistry::empty(), now);
        assert_eq!(vm_empty.producers[0].subscribers, None, "empty census → honest None");
        assert_eq!(subs_label(vm_empty.producers[0].subscribers), "—", "None renders a dash, not 0");
        let out_empty = render_plain(&vm_empty, TopSort::Msgs);
        assert!(out_empty.contains("SUBS"), "SUBS column header present:\n{out_empty}");

        // Populate the census: acme publishes under its anchor; two DISTINCT
        // subscriber ZIDs consume topics that intersect it → fan-out of 2.
        let mut census = Census::default();
        census.insert(ResourceRole::Publisher, "acme/telemetry".into(), "ACMEZID".into());
        census.insert(ResourceRole::Subscriber, "acme/**".into(), "SUB1".into());
        census.insert(ResourceRole::Subscriber, "acme/telemetry".into(), "SUB2".into());
        st.apply_census(census);

        let vm = st.view_model(&AliasRegistry::empty(), &EnvelopeRegistry::empty(), now);
        assert_eq!(
            vm.producers[0].subscribers,
            Some(2),
            "census fan-out N → Some(N) distinct subscribers"
        );
        assert_eq!(subs_label(vm.producers[0].subscribers), "2");
    }

    #[test]
    fn plain_snapshot_stale_fabric_hides_bars() {
        let mut st = TopState::new(None, TopSort::Msgs, 2);
        let start = Instant::now();
        // window 5s → stale at 5×3=15s.
        st.apply_fabric(fabric("zagros", 5, vec![]), start);
        let later = start + Duration::from_secs(20);
        let vm = st.view_model(&AliasRegistry::empty(), &EnvelopeRegistry::empty(), later);
        let out = render_plain(&vm, TopSort::Msgs);
        assert!(out.contains("stale"), "stale fabric flagged, bars hidden:\n{out}");
        assert!(out.contains("not faked"));
        // The fabric chip itself collapses to IDLE, never re-asserts GREEN.
        assert!(out.contains("FABRIC IDLE"), "stale chip reads idle:\n{out}");
        assert!(!out.contains("FABRIC GREEN"));
    }

    /// Flatten a TestBackend buffer into a newline-joined string of cell
    /// symbols, so we can assert on what the interactive UI actually rendered.
    fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
        let area = buf.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn env_beat(domain: &str, color: &str, n: u64, detail: EnvDetail) -> EnvHealthV1 {
        EnvHealthV1 {
            domain: domain.to_string(),
            scope: "zagros".to_string(),
            ts_unix_ms: 1,
            overall_color: color.to_string(),
            sample_count: n,
            window_secs: 30,
            detail: Some(detail),
        }
    }

    #[test]
    fn a_peer_beat_colors_the_peer_chip() {
        use crate::health_wire::PeerFaultDetailV1;
        let mut st = TopState::new(None, TopSort::Msgs, 2);
        let now = Instant::now();
        st.apply_env(
            env_beat(
                "peer",
                "red",
                1,
                EnvDetail::Peer(PeerFaultDetailV1 {
                    verdict: "peer_at_fault".to_string(),
                    ..Default::default()
                }),
            ),
            now,
        );
        let vm = st.view_model(&AliasRegistry::empty(), &EnvelopeRegistry::empty(), now);
        let peer = vm
            .chips
            .iter()
            .find(|c| c.domain == "peer")
            .expect("peer chip");
        assert_eq!((peer.state.as_str(), peer.sample_count), ("red", 1));
        assert_eq!(
            vm.chips.last().map(|c| c.domain.as_str()),
            Some("peer"),
            "last in the strip"
        );
        assert!(render_plain(&vm, TopSort::Msgs).contains("PEER RED n=1"));
    }

    #[test]
    fn a_trust_reading_colors_the_peer_chip_amber_or_unmetered_never_green() {
        use crate::health_wire::PeerFaultDetailV1;
        for (verdict, color, want) in [
            ("trust_refused_candidates", "amber", "PEER AMBER n=1"),
            ("trust_refused_unattributed", "amber", "PEER AMBER n=1"),
            ("trust_unmeasured", "unmetered", "PEER UNMETERED n=1"),
        ] {
            let mut st = TopState::new(None, TopSort::Msgs, 2);
            let now = Instant::now();
            st.apply_env(
                env_beat(
                    "peer",
                    color,
                    1,
                    EnvDetail::Peer(PeerFaultDetailV1 {
                        verdict: verdict.to_string(),
                        trust_verdict: "candidates".to_string(),
                        ..Default::default()
                    }),
                ),
                now,
            );
            let vm = st.view_model(&AliasRegistry::empty(), &EnvelopeRegistry::empty(), now);
            let out = render_plain(&vm, TopSort::Msgs);
            assert!(out.contains(want), "{verdict}:\n{out}");
            assert!(!out.contains("PEER GREEN"), "{verdict}:\n{out}");
        }
    }

    #[test]
    fn the_legend_names_the_trust_reading() {
        let text: String = legend_lines()
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        assert!(
            text.contains(
                "amber on trust: a client refuses our certificate; candidates, not faults"
            ),
            "{text}"
        );
    }

    #[test]
    fn testbackend_interactive_draw_renders_panes_and_honesty() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        // --- a rich synthetic state ------------------------------------------
        let t0 = Instant::now();
        let mut st = TopState::new(None, TopSort::Msgs, 2);

        // FABRIC: grounded green, ≥2 classes — applied fresh.
        st.apply_fabric(
            fabric(
                "zagros",
                5,
                vec![
                    ClassHealthV1 {
                        class: "C2".to_string(),
                        sample_rate: 40.0,
                        byte_rate: 1200.0,
                        target_sample_rate: 100.0,
                        target_byte_rate: 5000.0,
                        headroom_pct: 0.6,
                        color: "green".to_string(),
                        sample_count: 200,
                    },
                    ClassHealthV1 {
                        class: "C0".to_string(),
                        sample_rate: 3.0,
                        byte_rate: 90.0,
                        target_sample_rate: 50.0,
                        target_byte_rate: 2000.0,
                        headroom_pct: 0.9,
                        color: "green".to_string(),
                        sample_count: 30,
                    },
                ],
            ),
            t0,
        );

        // MESH: present + grounded (amber), with a link whose short-name
        // matches a producer anchor so PATH renders a real path.
        st.apply_env(
            env_beat(
                "mesh",
                "amber",
                3,
                EnvDetail::Mesh(MeshDetailV1 {
                    links: vec![MeshLinkHealthV1 {
                        peer_fqdn: "synth-01.zagros.example.net".to_string(),
                        peer_ip: "100.127.0.5".to_string(),
                        color: "green".to_string(),
                        connected: true,
                        direct: true,
                        conn_type: "P2P".to_string(),
                        latency_ms: 12.0,
                        relay_address: String::new(),
                        handshake_age_ms: 1000,
                        transition_count: 0,
                    }],
                    vantage_host: "thor".to_string(),
                    peers_total: 3,
                    peers_connected: 1,
                }),
            ),
            t0,
        );

        // TIME: present but STALE — applied far in the past relative to render.
        st.apply_env(
            env_beat("time", "green", 4, EnvDetail::Time(TimeDetailV1 { hosts: vec![] })),
            t0,
        );
        // (PRESENCE: never applied → absent → unmetered.)

        // Producers: a near-limit one, one matching the mesh link (real path),
        // and one with no mesh match (honest "—"). The meter ranks are set
        // DELIBERATELY OUT of msgs/s-sort order (7/3/9) so the test proves the
        // `#` column follows DISPLAY POSITION (1,2,3 down the sort), not the
        // meter's fixed rank.
        let mut hog = fp("synth-01", 40.0, 1200.0, true);
        hog.rank = 7;
        hog.shares = vec![ClassShareV1 { class: "C2".to_string(), sample_rate: 40.0, byte_rate: 1200.0 }];
        st.apply_footprint(hog, t0);
        let mut p2 = fp("e7a9c75b1234567890abcdef01234567", 5.0, 200.0, false);
        p2.rank = 3;
        st.apply_footprint(p2, t0);
        let mut p3 = fp("octopus-alpha", 1.0, 60.0, false);
        p3.rank = 9;
        st.apply_footprint(p3, t0);

        // A held producer/decode error must reach the screen.
        st.status = Some("subscriber recv: connection reset".to_string());

        st.sorted_cache = st.sorted_peers(&AliasRegistry::empty());

        // Render at a `now` past the time-domain staleness window (30×3=90s) so
        // TIME reads stale; FABRIC + MESH applied at the same t0 are also "old"
        // relative to their windows, so to keep them live we render only a hair
        // past — instead, push TIME's beat into the past explicitly by rendering
        // 100s out and re-applying the live domains at that same instant.
        let now = t0 + Duration::from_secs(100);
        // Re-apply the domains we want LIVE at the render instant; leave TIME at
        // t0 so only it is stale.
        st.apply_fabric(
            fabric(
                "zagros",
                5,
                vec![ClassHealthV1 {
                    class: "C2".to_string(),
                    sample_rate: 40.0,
                    byte_rate: 1200.0,
                    target_sample_rate: 100.0,
                    target_byte_rate: 5000.0,
                    headroom_pct: 0.6,
                    color: "green".to_string(),
                    sample_count: 200,
                }],
            ),
            now,
        );
        st.apply_env(
            env_beat(
                "mesh",
                "amber",
                3,
                EnvDetail::Mesh(MeshDetailV1 {
                    links: vec![MeshLinkHealthV1 {
                        peer_fqdn: "synth-01.zagros.example.net".to_string(),
                        peer_ip: "100.127.0.5".to_string(),
                        color: "green".to_string(),
                        connected: true,
                        direct: true,
                        conn_type: "P2P".to_string(),
                        latency_ms: 12.0,
                        relay_address: String::new(),
                        handshake_age_ms: 1000,
                        transition_count: 0,
                    }],
                    vantage_host: "thor".to_string(),
                    peers_total: 3,
                    peers_connected: 1,
                }),
            ),
            now,
        );

        let vm = st.view_model(&AliasRegistry::empty(), &EnvelopeRegistry::empty(), now);

        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal.draw(|f| draw(f, &st, &vm)).expect("draw");
        let screen = buffer_to_string(terminal.backend().buffer());
        if std::env::var("GOAT_TOP_SNAPSHOT").is_ok() {
            eprintln!("\n==== goat top TestBackend render ====\n{screen}====");
        }

        // --- weather strip: all four domains, with honest states -------------
        assert!(screen.contains("MESH"), "mesh chip:\n{screen}");
        assert!(screen.contains("FABRIC"), "fabric chip");
        assert!(screen.contains("TIME"), "time chip");
        assert!(screen.contains("PRESENCE"), "presence chip");
        assert!(screen.contains("GREEN"), "fabric grounded green present");
        // Absent presence domain → UNMETERED (dim), never green.
        assert!(screen.contains("UNMETERED"), "absent presence reads unmetered:\n{screen}");
        // Stale time domain → flagged stale, collapsed to IDLE — not green.
        assert!(screen.contains("stale"), "stale time domain flagged:\n{screen}");

        // --- fabric class load: a gauge label with the class code + canonical
        // name + of-target. The name is a display legend; wire stays C2.
        assert!(screen.contains("C2"), "class gauge code");
        assert!(screen.contains("situational"), "C2 canonical name shown beside the code:\n{screen}");
        assert!(screen.contains("40 /") && screen.contains("100 msgs/s"), "of-target gauge label:\n{screen}");

        // --- producer table: header cells + ≥1 row ---------------------------
        assert!(screen.contains("PEER"), "producer header PEER");
        assert!(screen.contains("MSG/S"), "producer header MSG/S");
        assert!(screen.contains("PATH"), "producer header PATH");
        assert!(screen.contains("LATENCY"), "producer header LATENCY");
        assert!(screen.contains("synth-01"), "the near-limit producer row:\n{screen}");
        // The `#` column is the DISPLAY POSITION in the current (msgs/s) sort —
        // 1,2,3 down the rows — NOT the meter's fixed rank (which was 7/3/9
        // here). synth-01 (top by msgs/s) is #1, octopus-alpha (lowest) is #3.
        let row_num = |peer: &str| -> String {
            let line = screen.lines().find(|l| l.contains(peer)).unwrap_or("");
            // strip the table border + leading pad, take the first token (the #).
            line.trim_start_matches(['│', ' '])
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string()
        };
        assert_eq!(row_num("synth-01"), "1", "top msgs/s row is #1 (sort pos), not meter rank 7:\n{screen}");
        assert_eq!(row_num("e7a9c75b"), "2", "second row is #2:\n{screen}");
        assert_eq!(row_num("octopus-alpha"), "3", "lowest row is #3, not meter rank 9:\n{screen}");
        // The matching producer joins the mesh link → a real path + latency.
        assert!(screen.contains("direct P2P"), "matched mesh path rendered:\n{screen}");
        assert!(screen.contains("12 ms"), "matched mesh latency rendered:\n{screen}");
        // The near-limit flag glyph shows.
        assert!(screen.contains('⚠'), "near-limit + error glyph present:\n{screen}");
        // The unmatched UUID producer renders an honest dash path (no fake join).
        assert!(screen.contains('—'), "unmatched producer shows honest dash:\n{screen}");

        // --- totals line -----------------------------------------------------
        assert!(screen.contains("peers 3"), "totals peers count:\n{screen}");
        assert!(screen.contains("msgs/s"), "totals throughput");
        assert!(screen.contains("direct"), "totals links direct/relayed");
        assert!(screen.contains("alive"), "totals presence alive/dropped");

        // --- held error surfaced (the honesty guard at render level) ---------
        assert!(
            screen.contains("connection reset"),
            "held subscriber error is RENDERED, not swallowed:\n{screen}"
        );

        // --- honesty guard: absent/stale domains never render their own GREEN.
        // (Find the weather line and check PRESENCE/TIME aren't tagged GREEN.)
        let weather_line = screen.lines().find(|l| l.contains("PRESENCE")).unwrap_or("");
        assert!(
            !weather_line.contains("PRESENCE GREEN") && !weather_line.contains("TIME GREEN"),
            "absent/stale domains must not show GREEN:\n{weather_line}"
        );
    }

    /// The `?` legend and `↵` producer drill-down draw on top of the live table
    /// and carry their reference/consumer text — a read-only lens, census-honest.
    #[test]
    fn testbackend_overlays_render_legend_and_producer_detail() {
        use panoscope_core::census::{Census, ResourceRole};
        use panoscope_core::portal::EnvelopeRow;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let t0 = Instant::now();
        let mut st = TopState::new(None, TopSort::Msgs, 2);
        // One producer under a 32-hex anchor.
        let anchor = "e7a9c75b1234567890abcdef01234567";
        st.apply_footprint(fp(anchor, 40.0, 1200.0, false), t0);
        // Census: this producer publishes a topic under its anchor, and one
        // consumer subscribes to a key-expression that intersects it → fan-out 1.
        let mut c = Census::default();
        c.insert(ResourceRole::Publisher, format!("{anchor}/telemetry/pos"), "pub_zid".into());
        c.insert(ResourceRole::Subscriber, format!("{anchor}/**"), "consumer_zid".into());
        st.apply_census(c);
        st.sorted_cache = st.sorted_peers(&AliasRegistry::empty());
        st.selected_peer = Some(anchor.to_string());
        // Envelope with a msgs/s ceiling of 20 — the producer's 40 msgs/s is OVER,
        // so the drill-down must show the allotment + overage. Bytes stay in band.
        let envs = EnvelopeRegistry::from_rows([EnvelopeRow {
            anchor: anchor.to_string(),
            min_msgs_per_s: 1.0,
            mean_msgs_per_s: 5.0,
            high_msgs_per_s: 10.0,
            max_msgs_per_s: 20.0,
            min_bytes_per_s: 0.0,
            mean_bytes_per_s: 2000.0,
            high_bytes_per_s: 8000.0,
            max_bytes_per_s: 32000.0,
            source: "edited".into(),
        }]);
        let vm = st.view_model(&AliasRegistry::empty(), &envs, t0);

        let render = |st: &TopState| -> String {
            let backend = TestBackend::new(120, 34);
            let mut terminal = Terminal::new(backend).expect("test terminal");
            terminal.draw(|f| draw(f, st, &vm)).expect("draw");
            buffer_to_string(terminal.backend().buffer())
        };

        // --- legend overlay --------------------------------------------------
        st.overlay = Overlay::Help;
        let help = render(&st);
        if std::env::var("GOAT_TOP_SNAPSHOT").is_ok() {
            eprintln!("\n==== LEGEND ====\n{help}");
        }
        assert!(help.contains("legend"), "legend title present:\n{help}");
        assert!(help.contains("fabric weather"), "legend explains weather:\n{help}");
        assert!(help.contains("absence is not illness"), "legend states the honesty rule:\n{help}");
        assert!(help.contains("SUBS"), "legend documents the consumer column:\n{help}");

        // --- producer drill-down --------------------------------------------
        st.overlay = Overlay::Detail;
        let detail = render(&st);
        if std::env::var("GOAT_TOP_SNAPSHOT").is_ok() {
            eprintln!("\n==== DETAIL ====\n{detail}");
        }
        assert!(detail.contains("producer detail"), "detail title present:\n{detail}");
        assert!(detail.contains(anchor), "detail shows the anchor:\n{detail}");
        // The census fan-out is real: exactly one consumer reached.
        assert!(detail.contains("consumers reached: 1"), "detail shows census fan-out:\n{detail}");
        assert!(detail.contains("consumer_zid"), "detail lists the reached consumer:\n{detail}");
        assert!(detail.contains("published topics: 1"), "detail lists published topics:\n{detail}");
        // Band breach: the msgs/s axis is OVER its allotment of 20 by +20 (200%).
        assert!(detail.contains("OVER"), "detail shows the OVER verdict:\n{detail}");
        assert!(detail.contains("max 20"), "detail shows the breached allotment:\n{detail}");
        assert!(detail.contains("+20") && detail.contains("200%"), "detail shows the overage:\n{detail}");
    }
}
