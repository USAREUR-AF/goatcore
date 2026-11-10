// `goat who` — the identity common operating picture screen (ADR 1047).
//
// FOLLOWS tui/top.rs's structure: a producer async task owns the Zenoh Session
// + the subscribers and feeds an unbounded mpsc channel; a synchronous ratatui
// event loop on `spawn_blocking` drains the channel each tick and redraws;
// terminal teardown is guaranteed even on error/panic. A SECOND async task —
// the access-who fetcher — turns UI selection into portal queries: the loop
// sends an anchor over a request channel, the fetcher calls
// `/api/access/who`, and the result returns on the SAME UI channel, so the
// sync loop never blocks on the portal.
//
// DIFFERENCE FROM top.rs: top is a usage leaderboard (producers, rate-ranked).
// `goat who` is a searchable identity ROSTER browser — the spine is the
// declared ⊕ live identity set, default-sorted by NAME, with a per-identity
// capability+activity detail pane. Honesty discipline is identical: never a
// state we can't ground (live `—`/dim when no presence; usage `—` when no
// footprint; permissions `·` until fetched, "portal unreachable" when down).

use crate::cmd::who::{DeclaredActor, WhoSort};
use crate::health_wire::{
    fmt_age, EnvDetail, EnvHealthV1, PresenceDetailV1, ProducerFootprintV1, FOOTPRINT_PREFIX,
    HEALTH_KEY, PRESENCE_KEY,
};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use panoscope_core::alias::AliasRegistry;
use panoscope_core::census::{query_census_mirror_first, Census};
use panoscope_core::envelope::EnvelopeRegistry;
use panoscope_core::portal::{AccessRosterEntry, AccessWho, MachineListItem, PortalClient};
use prost::Message as _;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Margin},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
    Terminal,
};
use std::collections::{BTreeSet, HashMap};
use std::io::stdout;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

/// Input-poll cadence — also the redraw interval when a beat arrives with no
/// key event.
const TICK: Duration = Duration::from_millis(120);
/// A presence/footprint beat is stale once older than this multiple of its
/// publish window — past it we stop asserting a live state.
const STALE_FACTOR: u32 = 3;
/// Fallback staleness window (secs) when a beat carries no `window_secs`.
const DEFAULT_WINDOW_SECS: u64 = 5;
/// Collection window floor for `--once` — long enough to catch a presence beat
/// (~30s) is too long for a snapshot; a footprint beat (~5s) is the floor.
const ONCE_MIN_WINDOW_SECS: u64 = 6;
/// How long each pub/sub census query waits for router adminspace replies. The
/// census is a `session.get` fan-out (panoscope_core::census::query_census); 3s
/// matches the shared primitive's tuned edge timeout.
const CENSUS_TIMEOUT: Duration = Duration::from_secs(3);
/// Idle gap between census refreshes (the query itself is up to CENSUS_TIMEOUT),
/// so the live-subscription dimension re-reads roughly every 5s — slow enough to
/// stay cheap on the lossy mesh, fresh enough to track a subscriber coming/going.
const CENSUS_REFRESH: Duration = Duration::from_secs(2);

// --- messages from the async tasks to the UI loop ----------------------------

enum WhoMsg {
    /// A per-producer footprint (`ops/fabric/footprint/<peer>`).
    Footprint(Box<ProducerFootprintV1>),
    /// The presence roll-up (`ops/presence/health/<scope>`).
    Presence(Box<EnvHealthV1>),
    /// An `access who` result for a previously-requested anchor.
    Access { anchor: String, result: AccessOutcome },
    /// The bulk `access roster` summary — every identity's W/R + state +
    /// last-activity in one shot, seeding the roster so it renders instantly
    /// instead of N lazy per-row fetches over the lossy mesh.
    Roster(Vec<AccessRosterEntry>),
    /// A fresh pub/sub CENSUS (the shared panoscope_core primitive): who is
    /// live-subscribed to what, read from router adminspaces. Latest-wins —
    /// the observed "is-reading" layer, distinct from the ACL read GRANTS.
    /// Empty when the census ACL grant is absent (honest: nothing observed).
    Census(Box<Census>),
    /// The machine fleet (`GET /api/machines`) in one shot — every enrolled
    /// machine + its custody edge, so the detail pane can show an identity's
    /// custodied machines (ADR 1096/1098). Empty when the portal is unreachable
    /// or the caller custodies nothing — rendered honestly, never faked.
    Machines(Vec<MachineListItem>),
    Error(String),
}

enum AccessOutcome {
    Ok(Box<AccessWho>),
    Err(AccessErr),
}

/// Why an `access who` fetch failed, classified so the UI can be honest about
/// the difference between "the portal is slow/unreachable" (degraded, common
/// at the lossy edge, retryable) and "the portal said no" (auth/real error).
/// goat-who 0.4.7 collapsed all of these into one scary red "network or parse
/// error" string, which read as a code bug when it was usually just the mesh.
#[derive(Clone, Debug, PartialEq)]
enum AccessErr {
    /// Transport/timeout — the portal didn't answer in time. On a tactical
    /// mesh this is overwhelmingly slow-link, not a real fault.
    Unreachable(String),
    /// 401/403 — reached the portal, but the operator token lacks scope.
    Unauthorized,
    /// Any other portal response (other HTTP status, decode failure).
    Other(String),
}

impl AccessErr {
    fn from_portal(e: panoscope_core::portal::PortalError) -> Self {
        use panoscope_core::portal::PortalError as PE;
        match e {
            PE::Transient(s) => AccessErr::Unreachable(s),
            // A 2xx body the CLI can't parse is a contract/version mismatch,
            // not an unreachable portal — surface it as Other so it doesn't
            // read as "portal down".
            PE::Decode(s) => AccessErr::Other(format!("portal response could not be decoded ({s}) — update goat")),
            PE::Http { status, .. } if status.as_u16() == 401 || status.as_u16() == 403 => {
                AccessErr::Unauthorized
            }
            PE::Http { status, path, .. } => {
                AccessErr::Other(format!("portal returned {status} for {path}"))
            }
            PE::HttpWithBody { status, .. }
                if status.as_u16() == 401 || status.as_u16() == 403 =>
            {
                AccessErr::Unauthorized
            }
            PE::HttpWithBody {
                status, message, ..
            } => {
                AccessErr::Other(format!("portal returned {status}: {message}"))
            }
            // access/who 404 means the portal has no record for this id.
            PE::NotFound { .. } => AccessErr::Other("no access record for this identity".to_string()),
            PE::Disabled => AccessErr::Unreachable("portal client disabled".to_string()),
        }
    }

    fn is_unreachable(&self) -> bool {
        matches!(self, AccessErr::Unreachable(_))
    }
}

/// Producer: own the session, subscribe footprints + presence, forward every
/// decoded reading. Mirrors top.rs::produce. We subscribe the whole
/// `ops/fabric/**` (as top does) but keep only footprints — the weather is
/// `goat top`'s concern, not the roster's.
async fn produce(session: zenoh::Session, router: String, tx: UnboundedSender<WhoMsg>) {
    let fabric_sub = match session.declare_subscriber(HEALTH_KEY).await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(WhoMsg::Error(crate::classify::classify_zenoh_err_line(
                e,
                &router,
                "declare a subscriber on the fabric-health key",
            )));
            return;
        }
    };
    let presence_sub = match session.declare_subscriber(PRESENCE_KEY).await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(WhoMsg::Error(crate::classify::classify_zenoh_err_line(
                e,
                &router,
                "declare a subscriber on the presence key",
            )));
            return;
        }
    };

    loop {
        let msg = tokio::select! {
            r = fabric_sub.recv_async() => match r {
                Ok(sample) => decode_footprint(&sample),
                Err(e) => Some(WhoMsg::Error(
                    crate::classify::classify_zenoh_err_line(e, &router, "receive a fabric-health sample"),
                )),
            },
            r = presence_sub.recv_async() => match r {
                Ok(sample) => decode_presence(&sample),
                Err(e) => Some(WhoMsg::Error(
                    crate::classify::classify_zenoh_err_line(e, &router, "receive a presence sample"),
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

/// Keep only footprints off `ops/fabric/**` (ignore the weather, which the
/// roster doesn't render). Returns None for the weather / an unknown key.
fn decode_footprint(sample: &zenoh::sample::Sample) -> Option<WhoMsg> {
    let key = sample.key_expr().as_str();
    if !key.starts_with(FOOTPRINT_PREFIX) {
        return None;
    }
    match ProducerFootprintV1::decode(sample.payload().to_bytes().as_ref()) {
        Ok(f) => Some(WhoMsg::Footprint(Box::new(f))),
        Err(e) => Some(WhoMsg::Error(format!("decode ProducerFootprintV1 from `{key}`: {e}"))),
    }
}

fn decode_presence(sample: &zenoh::sample::Sample) -> Option<WhoMsg> {
    match EnvHealthV1::decode(sample.payload().to_bytes().as_ref()) {
        Ok(env) if env.domain == "presence" => Some(WhoMsg::Presence(Box::new(env))),
        Ok(_) => None,
        Err(e) => Some(WhoMsg::Error(format!(
            "decode EnvHealthV1 from `{}`: {e}",
            sample.key_expr().as_str()
        ))),
    }
}

/// The access-who fetcher: turn each requested anchor into a portal query and
/// return the result on the UI channel. Serial by construction (one `recv` at a
/// time) — that's the natural throttle for the `a` "fetch all" drip.
async fn fetch_access(
    portal: PortalClient,
    mut req_rx: UnboundedReceiver<String>,
    tx: UnboundedSender<WhoMsg>,
) {
    while let Some(anchor) = req_rx.recv().await {
        let result = match portal.access_who(&anchor).await {
            Ok(w) => AccessOutcome::Ok(Box::new(w)),
            Err(e) => AccessOutcome::Err(AccessErr::from_portal(e)),
        };
        if tx.send(WhoMsg::Access { anchor, result }).is_err() {
            return;
        }
    }
}

/// The census refresher: reuse the already-open read-only session to poll the
/// shared pub/sub census on a loop, forwarding each fresh reading to the UI. This
/// is the LIVE-SUBSCRIPTION dimension (who is *reading* what right now) — ground
/// truth from router adminspaces, orthogonal to the portal's ACL read GRANTS
/// (who *may* read). `query_census` is fail-soft: on no data / no ACL grant it
/// returns an empty census, which the UI renders as "no live subscription
/// observed" — never a fabricated value.
async fn refresh_census(session: zenoh::Session, tx: UnboundedSender<WhoMsg>) {
    let own_zid = session.zid().to_string();
    loop {
        // Mirror-first (the one-crawler contract): the crawler's published
        // census when fresh+populated (single storage-backed GET), else the
        // direct adminspace walk. max_age = 3x the crawler's 60s interval.
        let census =
            query_census_mirror_first(&session, &own_zid, CENSUS_TIMEOUT, Duration::from_secs(180))
                .await;
        if tx.send(WhoMsg::Census(Box::new(census))).is_err() {
            return; // UI gone — stop.
        }
        tokio::time::sleep(CENSUS_REFRESH).await;
    }
}

// --- entry points ------------------------------------------------------------

#[allow(clippy::disallowed_methods)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    session: zenoh::Session,
    router: String,
    registry: AliasRegistry,
    envelopes: EnvelopeRegistry,
    declared: Vec<DeclaredActor>,
    portal: Option<PortalClient>,
    site: Option<String>,
    sort: WhoSort,
    filter: Option<String>,
) -> anyhow::Result<()> {
    let (ui_tx, ui_rx) = mpsc::unbounded_channel::<WhoMsg>();
    // Share the session with the census refresher before it moves into the
    // producer (zenoh Session is a cheap Arc handle). One session backs both
    // subscribers AND the census polls, so we don't burn a second read-only ZID
    //.
    let census_session = session.clone();
    let producer = tokio::spawn(produce(session, router, ui_tx.clone()));
    let census_task = tokio::spawn(refresh_census(census_session, ui_tx.clone()));

    // The fetcher consumes anchors from the loop; if there's no portal we drop
    // the receiver so the loop's sends no-op (and it renders "portal
    // unreachable" instead of trying to fetch).
    let (req_tx, req_rx) = mpsc::unbounded_channel::<String>();
    let portal_available = portal.is_some();
    // Bulk-roster one-shot: seed the whole roster's W/R + state + last-activity
    // from ONE call so scrolling is instant (the lazy per-row fetch was the
    // ~9s-per-selection bottleneck). The per-selection `access who` fetcher
    // still runs for the detail pane's full per-layer reason lists.
    if let Some(p) = portal.clone() {
        let roster_tx = ui_tx.clone();
        tokio::spawn(async move {
            // access_roster is the tool's core security signal (cut-off /
            // no-grants / stray / online) — a failure here used to be
            // dropped identically to "nothing changed yet", so a row's
            // fallback "not fetched yet" marker was indistinguishable from
            // a real failure to reach the portal.
            match p.access_roster().await {
                Ok(r) => {
                    let _ = roster_tx.send(WhoMsg::Roster(r.rows));
                }
                Err(e) => {
                    let _ = roster_tx.send(WhoMsg::Error(format!("access roster fetch: {e}")));
                }
            }
        });
    }
    // Machine-fleet one-shot (ADR 1096/1098): the custody edges + node-class /
    // autonomy / workloads for the detail pane's "custody" section. Operator sees
    // the whole fleet; a vendor-scoped caller its own — the server scopes it. A
    // failure (no portal, no grant) just leaves the section empty, never an error.
    if let Some(p) = portal.clone() {
        let mach_tx = ui_tx.clone();
        tokio::spawn(async move {
            if let Ok(r) = p.list_machines(None).await {
                let _ = mach_tx.send(WhoMsg::Machines(r.machines));
            }
        });
    }
    let fetcher = match portal {
        Some(p) => Some(tokio::spawn(fetch_access(p, req_rx, ui_tx.clone()))),
        None => {
            drop(req_rx);
            None
        }
    };
    drop(ui_tx);

    let handle = tokio::task::spawn_blocking(move || {
        run_loop(
            ui_rx,
            req_tx,
            registry,
            envelopes,
            declared,
            portal_available,
            site,
            sort,
            filter,
        )
    });
    let result = handle
        .await
        .map_err(|e| anyhow::anyhow!("who TUI thread panicked: {e}"))?;

    producer.abort();
    census_task.abort();
    if let Some(f) = fetcher {
        f.abort();
    }
    result
}

/// Batch snapshot (`goat who --once`): collect live state for one bounded
/// window, render ONE plain-text roster frame, exit. No terminal, no TTY gate,
/// no permission fetch (an N-call-per-identity cost; `goat access who <id>` is
/// the scriptable capability path). Same producer + view model as the screen.
#[allow(clippy::disallowed_methods)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_once(
    session: zenoh::Session,
    router: String,
    registry: AliasRegistry,
    envelopes: EnvelopeRegistry,
    declared: Vec<DeclaredActor>,
    site: Option<String>,
    sort: WhoSort,
    filter: Option<String>,
) -> anyhow::Result<()> {
    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<WhoMsg>();
    let producer = tokio::spawn(produce(session, router, ui_tx.clone()));
    drop(ui_tx);

    // Permissions never fetch in --once, so portal_available is irrelevant here.
    let mut state = WhoState::new(declared, site, sort, false);
    if let Some(f) = filter {
        state.filter = f;
    }

    let window = Duration::from_secs(ONCE_MIN_WINDOW_SECS);
    let deadline = tokio::time::Instant::now() + window;
    // See tui/top.rs's identical fix: distinct from `state.status`, which a
    // later/transient recv error also sets after good data already arrived.
    // Only "zero readings, ever" (both declare_subscriber calls failed at
    // startup) is the hard failure this tracks.
    let mut received_any = false;
    loop {
        let now = Instant::now();
        match tokio::time::timeout_at(deadline, ui_rx.recv()).await {
            Ok(Some(msg)) => match msg {
                WhoMsg::Footprint(f) => {
                    state.apply_footprint(*f, now);
                    received_any = true;
                }
                WhoMsg::Presence(e) => {
                    state.apply_presence(*e, now);
                    received_any = true;
                }
                WhoMsg::Access { .. } => {} // never requested in --once
                WhoMsg::Roster(_) => {}     // never requested in --once
                WhoMsg::Machines(_) => {}   // custody detail is screen-only
                WhoMsg::Census(_) => {}     // no census refresher spawned in --once
                WhoMsg::Error(e) => state.status = Some(e),
            },
            Ok(None) | Err(_) => break,
        }
    }
    producer.abort();

    let now = Instant::now();
    state.expire(now);
    let vm = state.view_model(&registry, &envelopes, now);
    print!("{}", render_plain(&vm, sort));
    if let Some(e) = &state.status {
        eprintln!("⚠ {e}");
    }
    // Before this check, both subscriptions failing to declare at startup
    // still rendered a plausible-looking, mostly-empty roster frame and
    // returned Ok(()) — indistinguishable from a genuinely quiet fabric.
    if !received_any {
        if let Some(e) = &state.status {
            anyhow::bail!("goat who --once collected zero readings — {e}");
        }
    }
    Ok(())
}

// --- live state --------------------------------------------------------------

struct FootprintEntry {
    fp: ProducerFootprintV1,
    seen: Instant,
}

/// The latest presence roll-up, with arrival instant + its declared publish
/// window (for staleness) + the beat's own clock (for "alive since" ages).
struct PresenceBeat {
    detail: PresenceDetailV1,
    ts_unix_ms: i64,
    seen: Instant,
    window_secs: u32,
}

/// Per-identity permission state — lazily fetched + cached. Absence from the
/// `access` map IS the not-fetched state, so there's no `NotFetched` variant.
enum AccessState {
    InFlight,
    Ready(Box<AccessWho>),
    Failed(AccessErr),
}

/// One identity's bulk-roster summary — the cross-layer posture seeded from the
/// single `access roster` call (W/R counts + reconcile state + on-mesh /
/// last-activity). The roster reads W/R + STATUS from here instantly; the
/// detail pane still lazily fetches the full per-layer `access who`.
#[derive(Clone, Debug)]
struct RosterSummary {
    kind: String,
    state: String,
    /// Identity-grammar affiliation (ADR 1043): org kind. Empty = none declared.
    affiliation: String,
    nation: String,
    /// Why a stray didn't reconcile (untracked / unregistered-key /
    /// no-declared-member). Empty for non-strays.
    recon_reason: String,
    on_mesh: bool,
    connected: bool,
    writes: usize,
    reads: usize,
    label: String,
}

struct WhoState {
    site_filter: Option<String>,
    sort: WhoSort,
    /// Declared roster: anchor → optional display name (from the registry).
    declared: HashMap<String, Option<String>>,
    /// Live producer footprints, latest-wins per anchor.
    footprints: HashMap<String, FootprintEntry>,
    /// Latest presence roll-up (covers all live peers).
    presence: Option<PresenceBeat>,
    /// Lazily-fetched permission state per anchor (the full per-layer detail).
    access: HashMap<String, AccessState>,
    /// Bulk-roster summaries, keyed by id/anchor — the instant W/R + cross-layer
    /// status source seeded from one `access roster` call.
    summaries: HashMap<String, RosterSummary>,
    /// Latest pub/sub census (the shared panoscope_core primitive): the observed
    /// LIVE-SUBSCRIPTION layer — who is *reading* what right now, off router
    /// adminspaces. Distinct from the portal's ACL read GRANTS (who *may* read).
    /// Empty until the census refresher lands / when the census ACL grant is
    /// absent — rendered honestly ("no live subscription observed"), never faked.
    census: Census,
    /// Machines each identity custodies (ADR 1096/1098), keyed by custodian
    /// anchor/handle → its enrolled machines. Seeded from one `GET /api/machines`;
    /// empty until it lands / when the caller custodies nothing.
    custody: HashMap<String, Vec<CustodyRow>>,
    /// True when a portal client backs the fetcher; false → "portal unreachable".
    portal_available: bool,
    selected: Option<String>,
    paused: bool,
    status: Option<String>,
    filter: String,
    editing_filter: bool,
    sorted_cache: Vec<String>,
    /// Drill-down mode: Enter on a row expands its detail to full screen
    /// (the roster hides); Esc/q pops back. Pure view state — no fabric effect.
    focused: bool,
}

impl WhoState {
    fn new(
        declared: Vec<DeclaredActor>,
        site_filter: Option<String>,
        sort: WhoSort,
        portal_available: bool,
    ) -> Self {
        Self {
            site_filter,
            sort,
            declared: declared
                .into_iter()
                .map(|d| (d.anchor, d.display_name))
                .collect(),
            footprints: HashMap::new(),
            presence: None,
            access: HashMap::new(),
            summaries: HashMap::new(),
            census: Census::default(),
            custody: HashMap::new(),
            portal_available,
            selected: None,
            paused: false,
            status: None,
            filter: String::new(),
            editing_filter: false,
            sorted_cache: Vec::new(),
            focused: false,
        }
    }

    fn scope_ok(&self, scope: &str) -> bool {
        match &self.site_filter {
            Some(s) => scope == s,
            None => true,
        }
    }

    fn apply_footprint(&mut self, fp: ProducerFootprintV1, now: Instant) {
        self.footprints
            .insert(fp.peer.clone(), FootprintEntry { fp, seen: now });
    }

    fn apply_presence(&mut self, env: EnvHealthV1, now: Instant) {
        if !self.scope_ok(&env.scope) {
            return;
        }
        let detail = match env.detail {
            Some(EnvDetail::Presence(d)) => d,
            _ => PresenceDetailV1::default(),
        };
        self.presence = Some(PresenceBeat {
            detail,
            ts_unix_ms: env.ts_unix_ms,
            seen: now,
            window_secs: env.window_secs,
        });
    }

    fn apply_access(&mut self, anchor: String, outcome: AccessOutcome) {
        let st = match outcome {
            AccessOutcome::Ok(w) => AccessState::Ready(w),
            AccessOutcome::Err(e) => AccessState::Failed(e),
        };
        self.access.insert(anchor, st);
    }

    /// Group the machine fleet by custodian (ADR 1096/1098) so the detail pane
    /// can show an identity's custodied machines. Latest-wins (a fresh fetch
    /// replaces the map wholesale).
    fn apply_machines(&mut self, machines: Vec<MachineListItem>) {
        let mut by_custodian: HashMap<String, Vec<CustodyRow>> = HashMap::new();
        for m in machines {
            if m.custodian.is_empty() {
                continue;
            }
            by_custodian.entry(m.custodian.clone()).or_default().push(CustodyRow {
                handle: m.system_handle,
                node_class: m.node_class,
                autonomy: m.autonomy,
                workloads: m.workloads,
            });
        }
        for rows in by_custodian.values_mut() {
            rows.sort_by(|a, b| a.handle.cmp(&b.handle));
        }
        self.custody = by_custodian;
    }

    /// Seed the bulk-roster summaries from one `access roster` call. Keyed by
    /// id (the slot anchor / vendor prefix / person id). These make W/R + the
    /// cross-layer STATUS render instantly for the whole roster.
    fn apply_roster(&mut self, rows: Vec<AccessRosterEntry>) {
        for r in rows {
            self.summaries.insert(
                r.id.clone(),
                RosterSummary {
                    kind: r.kind,
                    state: r.state,
                    affiliation: r.affiliation,
                    nation: r.nation,
                    recon_reason: r.recon_reason,
                    on_mesh: r.on_mesh,
                    connected: r.connected,
                    writes: r.writes,
                    reads: r.reads,
                    label: r.label,
                },
            );
        }
    }

    /// Replace the pub/sub census with a fresh reading (latest-wins). Feeds the
    /// per-identity LIVE-SUBSCRIPTION dimension in the detail pane.
    fn apply_census(&mut self, census: Census) {
        self.census = census;
    }

    /// The live subscriptions observed for an identity ANCHOR (key-expressions
    /// its correlated ZID is currently subscribed to, from the census). Empty for
    /// a pure consumer whose ZID can't be anchor-correlated, or before the census
    /// lands / when its ACL grant is absent — the caller renders that honestly
    /// ("no live subscription observed"), never a fabricated grant.
    fn live_subs_of(&self, anchor: &str) -> Vec<String> {
        self.census.subscriptions_for_anchor(anchor)
    }

    /// Drop footprints not refreshed within their staleness window (a producer
    /// gone quiet ages out rather than lingering as false-live). The presence
    /// beat is NOT dropped — it's stale-collapsed at render so its peers read
    /// "unmetered", not removed.
    fn expire(&mut self, now: Instant) {
        let window =
            Duration::from_secs(DEFAULT_WINDOW_SECS.max(1) * STALE_FACTOR as u64).max(TICK);
        self.footprints
            .retain(|_, e| now.duration_since(e.seen) < window);
    }

    /// Is the presence beat stale (older than STALE_FACTOR × its window)?
    fn presence_stale(&self, now: Instant) -> bool {
        match &self.presence {
            None => true,
            Some(b) => {
                let w = (b.window_secs as u64).max(1);
                now.duration_since(b.seen) >= Duration::from_secs(w * STALE_FACTOR as u64)
            }
        }
    }

    /// The presence peer for an anchor, only if the beat is fresh (else None —
    /// a stale beat must read "unmetered", never a re-asserted live state).
    fn presence_peer(&self, anchor: &str, now: Instant) -> Option<&crate::health_wire::PresencePeerV1> {
        if self.presence_stale(now) {
            return None;
        }
        self.presence
            .as_ref()?
            .detail
            .peers
            .iter()
            .find(|p| p.write_root == anchor)
    }

    /// The honest live state for an anchor (alive / dropped / unmetered).
    fn live_of(&self, anchor: &str, now: Instant) -> LiveVM {
        match self.presence_peer(anchor, now) {
            None => LiveVM::Unmetered,
            Some(p) => {
                let beat_ts = self.presence.as_ref().map(|b| b.ts_unix_ms).unwrap_or(0);
                let age = fmt_age(beat_ts.saturating_sub(p.since_unix_ms));
                if p.alive {
                    LiveVM::Alive(age)
                } else {
                    LiveVM::Dropped(age, p.drop_count)
                }
            }
        }
    }

    /// The human label for an anchor, in priority order:
    /// 1. the slot alias (`AliasRegistry`, e.g. `jane-doe`) — ADR 1039;
    /// 2. the registry display name (`/api/vendors`, e.g. `acme`) —
    ///    vendors carry a display name but no slot alias, so without this they
    ///    rendered as a bare UUID even when the portal answered (a real
    ///    goat-who 0.4.7 bug, distinct from the portal-timeout one);
    /// 3. the truncated UUID — the fail-closed floor (`AliasRegistry::render`
    ///    never invents a name). We never show ONLY a UUID when any
    ///    human-meaningful string exists.
    fn resolved_name(&self, registry: &AliasRegistry, anchor: &str) -> String {
        if let Some(alias) = registry.alias_of(anchor) {
            return alias;
        }
        if let Some(Some(name)) = self.declared.get(anchor) {
            if !name.is_empty() {
                return name.clone();
            }
        }
        // The bulk roster's label (Ory display name / vendor display name) names
        // a cross-layer identity — e.g. a person — that isn't in the alias or
        // declared sets.
        if let Some(s) = self.summaries.get(anchor) {
            if !s.label.is_empty() {
                return s.label.clone();
            }
        }
        registry.render(anchor) // truncated UUID, fail-closed
    }

    /// Case-folded sort key for an anchor: its resolved name, then the anchor
    /// (so two identities that render the same — e.g. both truncated UUIDs —
    /// stay deterministically ordered).
    fn name_key(&self, registry: &AliasRegistry, anchor: &str) -> (String, String) {
        (
            self.resolved_name(registry, anchor).to_lowercase(),
            anchor.to_string(),
        )
    }

    /// The full identity universe: declared ⊕ every anchor observed live this
    /// session (footprint peer ⊕ fresh presence peer write_root).
    fn identity_anchors(&self, now: Instant) -> BTreeSet<String> {
        let mut set: BTreeSet<String> = self.declared.keys().cloned().collect();
        set.extend(self.footprints.keys().cloned());
        // The bulk roster is a cross-layer set: it carries persons (Ory) +
        // peers the declared registry alone doesn't, so fold its ids in too.
        set.extend(self.summaries.keys().cloned());
        if !self.presence_stale(now) {
            if let Some(b) = &self.presence {
                set.extend(b.detail.peers.iter().map(|p| p.write_root.clone()));
            }
        }
        set
    }

    /// Does an anchor pass the active filter? Space-separated terms are AND-ed:
    /// a bare term is a case-insensitive substring of the rendered name / anchor
    /// / display name; `live:on|off` and `use:on|off` narrow by live/producing
    /// state. Pure over (self, registry, now) → unit-testable.
    fn matches_filter(&self, registry: &AliasRegistry, anchor: &str, now: Instant) -> bool {
        let filter = self.filter.trim();
        if filter.is_empty() {
            return true;
        }
        filter
            .split_whitespace()
            .all(|term| self.term_matches(registry, anchor, term, now))
    }

    fn term_matches(&self, registry: &AliasRegistry, anchor: &str, term: &str, now: Instant) -> bool {
        let t = term.to_lowercase();
        if let Some(v) = t.strip_prefix("live:") {
            let alive = matches!(self.live_of(anchor, now), LiveVM::Alive(_));
            return parse_bool(v) == Some(alive);
        }
        if let Some(v) = t.strip_prefix("use:") {
            let producing = self.footprints.contains_key(anchor);
            return parse_bool(v) == Some(producing);
        }
        // Bare substring: the resolved human name (alias OR display name) or
        // the raw anchor. resolved_name already folds in the display name, so
        // a search for "org89x" matches a vendor that has no slot alias.
        self.resolved_name(registry, anchor)
            .to_lowercase()
            .contains(&t)
            || anchor.to_lowercase().contains(&t)
    }

    /// Filtered + sorted anchors. THE single source of roster order — both the
    /// rendered table and the selection cache read it, so a filtered-out
    /// identity can never be selected. Default sort is NAME (searchable-first).
    fn sorted_anchors(&self, registry: &AliasRegistry, now: Instant) -> Vec<String> {
        let mut anchors: Vec<String> = self
            .identity_anchors(now)
            .into_iter()
            .filter(|a| self.matches_filter(registry, a, now))
            .collect();
        match self.sort {
            WhoSort::Name => {
                anchors.sort_by_key(|a| self.name_key(registry, a));
            }
            WhoSort::Live => anchors.sort_by(|a, b| {
                let la = matches!(self.live_of(a, now), LiveVM::Alive(_));
                let lb = matches!(self.live_of(b, now), LiveVM::Alive(_));
                // alive first (true before false), then name.
                lb.cmp(&la)
                    .then_with(|| self.name_key(registry, a).cmp(&self.name_key(registry, b)))
            }),
            WhoSort::Use => anchors.sort_by(|a, b| {
                let ra = self.footprints.get(a).map(|e| e.fp.sample_rate).unwrap_or(0.0);
                let rb = self.footprints.get(b).map(|e| e.fp.sample_rate).unwrap_or(0.0);
                rb.partial_cmp(&ra)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| self.name_key(registry, a).cmp(&self.name_key(registry, b)))
            }),
        }
        anchors
    }

    fn cycle_sort(&mut self) {
        self.sort = match self.sort {
            WhoSort::Name => WhoSort::Live,
            WhoSort::Live => WhoSort::Use,
            WhoSort::Use => WhoSort::Name,
        };
    }

    fn current_index(&self) -> Option<usize> {
        let sel = self.selected.as_ref()?;
        self.sorted_cache.iter().position(|a| a == sel)
    }

    fn select_next(&mut self) {
        if self.sorted_cache.is_empty() {
            return;
        }
        let i = self.current_index().map(|i| i + 1).unwrap_or(0);
        let i = i.min(self.sorted_cache.len() - 1);
        self.selected = self.sorted_cache.get(i).cloned();
    }

    fn select_prev(&mut self) {
        if self.sorted_cache.is_empty() {
            return;
        }
        let i = self.current_index().map(|i| i.saturating_sub(1)).unwrap_or(0);
        self.selected = self.sorted_cache.get(i).cloned();
    }

    /// Permission summary for the roster row (counts + state). The bulk-roster
    /// summary is the instant path (seeded from one call); the lazy per-row
    /// `access who` is the fallback before the roster lands / for a row the
    /// roster didn't carry.
    fn perms_of(&self, anchor: &str) -> PermsVM {
        if let Some(s) = self.summaries.get(anchor) {
            return PermsVM::Ready {
                writes: s.writes,
                reads: s.reads,
                state: s.state.clone(),
            };
        }
        if !self.portal_available {
            return PermsVM::PortalDown;
        }
        match self.access.get(anchor) {
            None => PermsVM::NotFetched,
            Some(AccessState::InFlight) => PermsVM::InFlight,
            Some(AccessState::Failed(e)) => PermsVM::Failed {
                unreachable: e.is_unreachable(),
            },
            Some(AccessState::Ready(w)) => PermsVM::Ready {
                writes: w.writes.len(),
                reads: w.reads.len(),
                state: w.state.clone(),
            },
        }
    }

    /// The declared affiliation (org kind, nation) for a row, from the bulk
    /// summary. Both empty when none is declared — the caller renders `·`, never
    /// a guessed default. Display-only attribution (ADR 1043), never access.
    fn aff_of(&self, anchor: &str) -> (String, String) {
        match self.summaries.get(anchor) {
            Some(s) => (s.affiliation.clone(), s.nation.clone()),
            None => (String::new(), String::new()),
        }
    }

    /// A stray's reconcile reason (why it didn't match the declared roster).
    /// Empty for non-strays / rows the roster didn't carry.
    fn recon_reason_of(&self, anchor: &str) -> String {
        self.summaries
            .get(anchor)
            .map(|s| s.recon_reason.clone())
            .unwrap_or_default()
    }

    /// The cross-layer STATUS for the roster row — what the uniform "active"
    /// STATE column could never convey. Composed from the bulk summary so it
    /// VARIES and each value is actionable:
    ///   cut-off   the identity is revoked / disabled (negative enforcement)
    ///   no-grants onboarded but can write+read nothing (the F-177 silent-denial
    ///             surface — present on the mesh, zero ACL grants)
    ///   online    a matched NetBird peer is currently connected
    ///   on-mesh   enrolled on the overlay (peer matched), not currently connected
    ///   stray     live on a layer with no declared/registry record (out-of-band)
    ///   active    has grants, no richer signal
    ///   ·         no cross-layer summary yet (roster not landed / not carried)
    fn status_of(&self, anchor: &str) -> StatusVM {
        let Some(s) = self.summaries.get(anchor) else {
            return StatusVM::Unknown;
        };
        match s.state.as_str() {
            "cut_off" | "cut-off" | "revoked" => return StatusVM::CutOff,
            "out_of_band" | "out-of-band" => return StatusVM::Stray,
            _ => {}
        }
        if s.writes == 0 && s.reads == 0 {
            return StatusVM::NoGrants;
        }
        if s.on_mesh && s.connected {
            return StatusVM::Online;
        }
        if s.on_mesh {
            return StatusVM::OnMesh;
        }
        StatusVM::Active
    }

    /// Project to the renderer-agnostic view model, applying honesty discipline
    /// ONCE. Pure over (self, registry, envelopes, now).
    fn view_model(
        &self,
        registry: &AliasRegistry,
        envelopes: &EnvelopeRegistry,
        now: Instant,
    ) -> WhoViewModel {
        let anchors = self.sorted_anchors(registry, now);
        let rows: Vec<RowVM> = anchors
            .iter()
            .map(|anchor| self.row_vm(registry, envelopes, anchor, now))
            .collect();

        let total = self.identity_anchors(now).len();
        let live = self
            .identity_anchors(now)
            .iter()
            .filter(|a| matches!(self.live_of(a, now), LiveVM::Alive(_)))
            .count();
        let producing = self.footprints.len();
        let declared = self.declared.len();
        let totals = TotalsVM {
            total,
            live,
            producing,
            declared,
            shown: rows.len(),
        };

        let detail = self
            .selected
            .as_ref()
            .filter(|a| anchors.iter().any(|x| x == *a))
            .map(|anchor| self.detail_vm(registry, envelopes, anchor, now));

        WhoViewModel {
            rows,
            detail,
            totals,
            filter: (!self.filter.trim().is_empty()).then(|| self.filter.clone()),
        }
    }

    fn row_vm(
        &self,
        registry: &AliasRegistry,
        envelopes: &EnvelopeRegistry,
        anchor: &str,
        now: Instant,
    ) -> RowVM {
        let fp = self.footprints.get(anchor).map(|e| &e.fp);
        let (band_label, band_color) = band_of(envelopes, anchor, fp);
        let (aff_kind, aff_nation) = self.aff_of(anchor);
        RowVM {
            anchor: anchor.to_string(),
            name: self.resolved_name(registry, anchor),
            declared: self.declared.contains_key(anchor),
            aff_kind,
            aff_nation,
            recon_reason: self.recon_reason_of(anchor),
            live: self.live_of(anchor, now),
            msgs: fp.map(|f| f.sample_rate),
            bytes: fp.map(|f| f.byte_rate),
            band_label,
            band_color,
            shares: fp.map(shares_label).unwrap_or_else(|| "—".to_string()),
            near_limit: fp.map(|f| f.near_limit).unwrap_or(false),
            perms: self.perms_of(anchor),
            status: self.status_of(anchor),
        }
    }

    fn detail_vm(
        &self,
        registry: &AliasRegistry,
        envelopes: &EnvelopeRegistry,
        anchor: &str,
        now: Instant,
    ) -> DetailVM {
        let row = self.row_vm(registry, envelopes, anchor, now);
        let access = if !self.portal_available {
            AccessDetailVM::PortalDown
        } else {
            match self.access.get(anchor) {
                None => AccessDetailVM::NotFetched,
                Some(AccessState::InFlight) => AccessDetailVM::InFlight,
                Some(AccessState::Failed(e)) => AccessDetailVM::Failed(e.clone()),
                Some(AccessState::Ready(w)) => AccessDetailVM::Ready {
                    on_mesh: w.on_mesh,
                    writes: w
                        .writes
                        .iter()
                        .map(|e| (e.topic.clone(), e.layer.clone(), e.reason.clone()))
                        .collect(),
                    reads: w
                        .reads
                        .iter()
                        .map(|e| (e.topic.clone(), e.layer.clone(), e.reason.clone()))
                        .collect(),
                    live_subs: self.live_subs_of(anchor),
                    notes: w.notes.clone(),
                },
            }
        };
        let xlayer = self.summaries.get(anchor).map(|s| XLayerVM {
            kind: s.kind.clone(),
            status: self.status_of(anchor),
            writes: s.writes,
            reads: s.reads,
            on_mesh: s.on_mesh,
        });
        let custody = self.custody.get(anchor).cloned().unwrap_or_default();
        DetailVM { row, access, xlayer, custody }
    }
}

// --- view model --------------------------------------------------------------

/// Honest live state for an identity.
#[derive(Clone, Debug, PartialEq)]
enum LiveVM {
    /// Presence token alive — carries "alive since" age.
    Alive(String),
    /// Presence token dropped — carries last-change age + drop count.
    Dropped(String, u64),
    /// No fresh presence beat covers this anchor — UNCONFIRMED, not "offline".
    Unmetered,
}

/// Roster-row permission summary across the fetch lifecycle.
#[derive(Clone, Debug, PartialEq)]
enum PermsVM {
    NotFetched,
    InFlight,
    Ready { writes: usize, reads: usize, state: String },
    /// Fetch failed. `unreachable` distinguishes a slow/down portal (amber,
    /// retryable) from a real denial/error (red).
    Failed { unreachable: bool },
    PortalDown,
}

/// Cross-layer status — the meaningful replacement for the uniform "active"
/// STATE column. Derived once in [`WhoState::status_of`].
#[derive(Clone, Copy, Debug, PartialEq)]
enum StatusVM {
    CutOff,
    NoGrants,
    Online,
    OnMesh,
    Stray,
    Active,
    Unknown,
}

impl StatusVM {
    fn label(&self) -> &'static str {
        match self {
            StatusVM::CutOff => "cut-off",
            StatusVM::NoGrants => "no-grants",
            StatusVM::Online => "online",
            StatusVM::OnMesh => "on-mesh",
            StatusVM::Stray => "stray",
            StatusVM::Active => "active",
            StatusVM::Unknown => "·",
        }
    }
    fn color(&self) -> &'static str {
        match self {
            StatusVM::CutOff => "red",
            StatusVM::NoGrants | StatusVM::Stray => "amber",
            StatusVM::Online => "green",
            StatusVM::OnMesh => "cyan",
            // Registry-active with no live signal behind it: dim, not green.
            // Green is for something that was actually seen (mirrors
            // panoscope who_view::status_of).
            StatusVM::Active | StatusVM::Unknown => "",
        }
    }
}

struct RowVM {
    anchor: String,
    name: String,
    declared: bool,
    /// Identity-grammar affiliation (ADR 1043): org kind + nation, both empty
    /// when none is declared (rendered `·`, never guessed).
    aff_kind: String,
    aff_nation: String,
    /// A stray's reconcile reason (empty for non-strays) — drives the RECON row
    /// + the STATE-column verify marker.
    recon_reason: String,
    live: LiveVM,
    msgs: Option<f64>,
    bytes: Option<f64>,
    band_label: String,
    band_color: String,
    shares: String,
    near_limit: bool,
    perms: PermsVM,
    status: StatusVM,
}

#[allow(clippy::large_enum_variant)]
enum AccessDetailVM {
    NotFetched,
    InFlight,
    PortalDown,
    Failed(AccessErr),
    Ready {
        on_mesh: bool,
        writes: Vec<(String, String, String)>,
        reads: Vec<(String, String, String)>,
        /// Observed LIVE SUBSCRIPTIONS from the pub/sub census — the key-exprs the
        /// identity IS reading right now (ground truth), rendered in the DATA
        /// section beside the read GRANTS above (what it MAY read). Empty →
        /// "no live subscription observed", never faked (honest census gap).
        live_subs: Vec<String>,
        notes: Vec<String>,
    },
}

/// The instant cross-layer rollup for the detail grid, sourced from the bulk
/// roster (not the lazy per-row fetch) so KIND / status / W/R counts / mesh
/// show the moment a row is selected — even if the full per-layer `access who`
/// is slow or times out on the lossy mesh.
#[derive(Clone, Debug)]
struct XLayerVM {
    kind: String,
    status: StatusVM,
    writes: usize,
    reads: usize,
    on_mesh: bool,
}

struct DetailVM {
    row: RowVM,
    access: AccessDetailVM,
    xlayer: Option<XLayerVM>,
    /// Machines this identity custodies (ADR 1096/1098). Empty for a non-custodian
    /// (or before the fleet lands) — the render omits the section then.
    custody: Vec<CustodyRow>,
}

/// One custodied machine as the detail pane shows it.
#[derive(Clone)]
struct CustodyRow {
    handle: String,
    node_class: String,
    autonomy: String,
    workloads: Vec<String>,
}

struct TotalsVM {
    total: usize,
    live: usize,
    producing: usize,
    declared: usize,
    shown: usize,
}

struct WhoViewModel {
    rows: Vec<RowVM>,
    detail: Option<DetailVM>,
    totals: TotalsVM,
    filter: Option<String>,
}

// --- pure helpers ------------------------------------------------------------

fn parse_bool(v: &str) -> Option<bool> {
    match v {
        "on" | "yes" | "true" | "1" | "y" => Some(true),
        "off" | "no" | "false" | "0" | "n" => Some(false),
        _ => None,
    }
}

/// The throughput-envelope band (ADR 1045) for an actor's LIVE footprint, or an
/// honest `—` (no envelope, or no footprint to judge). Never a faked verdict.
fn band_of(
    envelopes: &EnvelopeRegistry,
    anchor: &str,
    fp: Option<&ProducerFootprintV1>,
) -> (String, String) {
    match fp.and_then(|f| envelopes.band(anchor, f.sample_rate, f.byte_rate)) {
        Some(b) => (b.label().to_string(), b.color().to_string()),
        None => ("—".to_string(), String::new()),
    }
}

/// Compact per-class share string, e.g. "C2 80% · C0 20%" (top 3 by msgs/s).
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

/// Compact roster live-cell text (no color): "● 4m" / "○ 2m ✗3" / "—".
fn live_cell(live: &LiveVM) -> String {
    match live {
        LiveVM::Alive(age) => format!("● {}", strip_ago(age)),
        LiveVM::Dropped(age, drops) => format!("○ {} ✗{drops}", strip_ago(age)),
        LiveVM::Unmetered => "—".to_string(),
    }
}

fn live_color(live: &LiveVM) -> &'static str {
    match live {
        LiveVM::Alive(_) => "green",
        LiveVM::Dropped(_, _) => "amber",
        LiveVM::Unmetered => "",
    }
}

/// "4m ago" → "4m" for the tight roster cell (the detail pane keeps "ago").
fn strip_ago(age: &str) -> String {
    age.trim_end_matches(" ago").to_string()
}

/// Roster permission-cell text: counts once fetched, an honest marker before.
fn perms_cell(perms: &PermsVM) -> String {
    match perms {
        PermsVM::NotFetched => "·".to_string(),
        PermsVM::InFlight => "…".to_string(),
        PermsVM::Ready { writes, reads, .. } => format!("{writes}/{reads}"),
        // Slow/unreachable reads as a soft "?" (we just couldn't ask); a real
        // denial/error reads as "✗".
        PermsVM::Failed { unreachable: true } => "?".to_string(),
        PermsVM::Failed { unreachable: false } => "✗".to_string(),
        PermsVM::PortalDown => "⌧".to_string(),
    }
}

/// Color for the roster W/R cell, matching `perms_cell`'s honesty: dim for
/// not-yet-known, amber for slow/unreachable, red for a real failure.
fn perms_color(perms: &PermsVM) -> &'static str {
    match perms {
        PermsVM::Failed { unreachable: true } | PermsVM::PortalDown => "amber",
        PermsVM::Failed { unreachable: false } => "red",
        _ => "",
    }
}

fn perms_state(perms: &PermsVM) -> String {
    match perms {
        PermsVM::Ready { state, .. } if !state.is_empty() => state.clone(),
        PermsVM::Ready { .. } => "—".to_string(),
        PermsVM::PortalDown => "no portal".to_string(),
        PermsVM::Failed { unreachable: true } => "slow".to_string(),
        PermsVM::Failed { unreachable: false } => "err".to_string(),
        _ => "·".to_string(),
    }
}

/// State→color: active green, cut-off red, out-of-band amber, else dim.
fn state_color(state: &str) -> &'static str {
    match state {
        "active" => "green",
        "cut-off" | "revoked" => "red",
        "out-of-band" => "amber",
        _ => "",
    }
}

fn color_style(state: &str) -> Style {
    match state {
        "green" => Style::default().fg(Color::Green),
        "amber" => Style::default().fg(Color::Yellow),
        "red" => Style::default().fg(Color::Red),
        "cyan" => Style::default().fg(Color::Cyan),
        "magenta" => Style::default().fg(Color::Magenta),
        "" => Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
        _ => Style::default().fg(Color::Gray),
    }
}

fn sort_label(sort: WhoSort) -> &'static str {
    match sort {
        WhoSort::Name => "name",
        WhoSort::Live => "live",
        WhoSort::Use => "use",
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

// --- plain-text renderer (`--once`) ------------------------------------------

fn render_plain(vm: &WhoViewModel, sort: WhoSort) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let t = &vm.totals;
    let _ = writeln!(
        out,
        "IDENTITIES  {} total · {} declared · {} live · {} producing · sort {}",
        t.total,
        t.declared,
        t.live,
        t.producing,
        sort_label(sort)
    );
    if let Some(f) = &vm.filter {
        let _ = writeln!(out, "filter \"{}\" — {} of {} shown", f, t.shown, t.total);
    }
    out.push('\n');
    // No W/R column in --once: batch mode never fetches permissions (an
    // N-call-per-identity cost), so a permissions column could only show a
    // misleading placeholder. The scriptable capability path is
    // `goat access who <id>`.
    let _ = writeln!(
        out,
        "  {:<28} {:<11} {:<5} {:<10} {:>8} {:<5}",
        "NAME", "AFF", "SRC", "LIVE", "MSG/S", "BAND"
    );
    if vm.rows.is_empty() {
        let _ = writeln!(out, "  (no identities match)");
    }
    for r in &vm.rows {
        let msgs = r
            .msgs
            .map(|m| format!("{m:.0}"))
            .unwrap_or_else(|| "—".to_string());
        let _ = writeln!(
            out,
            "  {:<28} {:<11} {:<5} {:<10} {:>8} {:<5}",
            truncate(&r.name, 28),
            aff_label(&r.aff_kind, &r.aff_nation),
            if r.declared { "reg" } else { "live" },
            live_cell(&r.live),
            msgs,
            r.band_label.to_uppercase(),
        );
    }
    out
}

// --- the loop ----------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn run_loop(
    ui_rx: UnboundedReceiver<WhoMsg>,
    req_tx: UnboundedSender<String>,
    registry: AliasRegistry,
    envelopes: EnvelopeRegistry,
    declared: Vec<DeclaredActor>,
    portal_available: bool,
    site: Option<String>,
    sort: WhoSort,
    filter: Option<String>,
) -> anyhow::Result<()> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    // Silence stderr while the alternate screen is up — the zenoh transport
    // stack logs WARN lines (TLS name-verification, link drops) through the
    // global tracing subscriber to stderr, and on a live mesh they would
    // otherwise paint over the roster mid-render (the goat-who 0.4.7 defect).
    // Held until after LeaveAlternateScreen; restored even on panic via Drop.
    let _stderr_guard = crate::tui::StderrSilencer::engage();
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let outcome = event_loop(
        &mut terminal,
        ui_rx,
        req_tx,
        &registry,
        &envelopes,
        declared,
        portal_available,
        site,
        sort,
        filter,
    );

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    outcome.map_err(|e| anyhow::anyhow!("who TUI: {e}"))
}

#[allow(clippy::too_many_arguments)]
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    mut ui_rx: UnboundedReceiver<WhoMsg>,
    req_tx: UnboundedSender<String>,
    registry: &AliasRegistry,
    envelopes: &EnvelopeRegistry,
    declared: Vec<DeclaredActor>,
    portal_available: bool,
    site: Option<String>,
    sort: WhoSort,
    filter: Option<String>,
) -> std::io::Result<()> {
    let mut state = WhoState::new(declared, site, sort, portal_available);
    if let Some(f) = filter {
        state.filter = f;
    }
    // Persisted across frames so the roster viewport scrolls smoothly and
    // follows the selection (ratatui maintains the offset in here). Fresh
    // each frame would re-anchor the window and jump.
    let mut table_state = TableState::default();

    loop {
        let now = Instant::now();
        while let Ok(msg) = ui_rx.try_recv() {
            if state.paused {
                if let WhoMsg::Error(e) = msg {
                    state.status = Some(e);
                }
                continue;
            }
            match msg {
                WhoMsg::Footprint(f) => state.apply_footprint(*f, now),
                WhoMsg::Presence(e) => state.apply_presence(*e, now),
                WhoMsg::Access { anchor, result } => state.apply_access(anchor, result),
                WhoMsg::Roster(rows) => state.apply_roster(rows),
                WhoMsg::Machines(m) => state.apply_machines(m),
                WhoMsg::Census(c) => state.apply_census(*c),
                WhoMsg::Error(e) => state.status = Some(e),
            }
        }

        if !state.paused {
            state.expire(now);
        }
        state.sorted_cache = state.sorted_anchors(registry, now);
        if state.current_index().is_none() {
            state.selected = state.sorted_cache.first().cloned();
        }

        // Lazily fetch the SELECTED identity's full per-layer detail (once).
        // The roster W/R + status already render instantly from the bulk
        // summary; this fetch is only for the detail pane's reason lists, so it
        // keys off the full-detail `access` map — NOT perms_of, which the bulk
        // summary now satisfies for every row.
        if portal_available {
            if let Some(sel) = state.selected.clone() {
                if !state.access.contains_key(&sel) {
                    state.access.insert(sel.clone(), AccessState::InFlight);
                    let _ = req_tx.send(sel);
                }
            }
        }

        let vm = state.view_model(registry, envelopes, now);
        terminal.draw(|f| draw(f, &state, &vm, &mut table_state))?;

        if event::poll(TICK)? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(());
                }
                if state.editing_filter {
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
                } else if state.focused {
                    // Drill-down mode: a focused full-screen detail. Esc/q/Enter
                    // pop back to the roster; j/k still move the selection so an
                    // operator can page through identities without leaving the
                    // detail. Pure navigation — no fabric mutation (ADR 1047).
                    match (k.code, k.modifiers) {
                        (KeyCode::Esc, _) | (KeyCode::Char('q'), _) | (KeyCode::Enter, _) => {
                            state.focused = false;
                        }
                        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => state.select_next(),
                        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => state.select_prev(),
                        (KeyCode::Char('r'), _) => {
                            if let Some(sel) = state.selected.clone() {
                                if matches!(state.access.get(&sel), Some(AccessState::Failed(_))) {
                                    state.access.remove(&sel);
                                }
                            }
                        }
                        _ => {}
                    }
                } else {
                    match (k.code, k.modifiers) {
                        (KeyCode::Char('q'), _) => return Ok(()),
                        // Enter drills into the selected identity's full-screen
                        // detail (the roster hides; Esc pops back).
                        (KeyCode::Enter, _) if state.selected.is_some() => {
                            state.focused = true;
                        }
                        (KeyCode::Esc, _) => {
                            if state.filter.is_empty() {
                                return Ok(());
                            }
                            state.filter.clear();
                        }
                        (KeyCode::Char('/'), _) | (KeyCode::Char('\\'), _) => {
                            state.editing_filter = true
                        }
                        (KeyCode::Char('s'), _) => state.cycle_sort(),
                        (KeyCode::Char('p'), _) => state.paused = !state.paused,
                        // `a` — enqueue every shown identity for an access fetch
                        // (the serial fetcher is the natural throttle).
                        (KeyCode::Char('a'), _) if portal_available => {
                            for anchor in state.sorted_cache.clone() {
                                if matches!(state.perms_of(&anchor), PermsVM::NotFetched) {
                                    state.access.insert(anchor.clone(), AccessState::InFlight);
                                    let _ = req_tx.send(anchor);
                                }
                            }
                        }
                        // `r` — retry the selected identity's perms fetch:
                        // clear a cached failure so the lazy fetcher re-fires.
                        // The common case it fixes is a slow-mesh timeout.
                        (KeyCode::Char('r'), _) => {
                            if let Some(sel) = state.selected.clone() {
                                if matches!(state.access.get(&sel), Some(AccessState::Failed(_))) {
                                    state.access.remove(&sel);
                                }
                            }
                        }
                        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => state.select_next(),
                        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => state.select_prev(),
                        (KeyCode::PageDown, _) => {
                            for _ in 0..10 {
                                state.select_next();
                            }
                        }
                        (KeyCode::PageUp, _) => {
                            for _ in 0..10 {
                                state.select_prev();
                            }
                        }
                        (KeyCode::Home, _) | (KeyCode::Char('g'), _) => {
                            state.selected = state.sorted_cache.first().cloned();
                        }
                        (KeyCode::End, _) | (KeyCode::Char('G'), _) => {
                            state.selected = state.sorted_cache.last().cloned();
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

// --- rendering ---------------------------------------------------------------

fn draw(f: &mut ratatui::Frame, state: &WhoState, vm: &WhoViewModel, table_state: &mut TableState) {
    let footer_h = if state.status.is_some() { 3 } else { 2 };

    // Drill-down: Enter expanded the selected identity to full screen. The
    // roster hides; the detail gets the whole middle so it can show every
    // domain + the remediation surface in full.
    if state.focused {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),    // header
                Constraint::Min(5),       // full-screen detail
                Constraint::Length(footer_h),
            ])
            .split(f.area());
        draw_header(f, chunks[0], state, vm);
        draw_detail_full(f, chunks[1], vm);
        draw_footer(f, chunks[2], state);
        return;
    }

    // Detail pane shows the four identity domains (identity / mesh / data /
    // release) + live activity, so it needs ~13 rows; the roster flexes.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // header / search bar
            Constraint::Min(5),     // roster table (grows; floor keeps it usable)
            Constraint::Length(13), // detail pane
            Constraint::Length(footer_h),
        ])
        .split(f.area());

    draw_header(f, chunks[0], state, vm);
    draw_roster(f, chunks[1], state, vm, table_state);
    draw_detail(f, chunks[2], vm);
    draw_footer(f, chunks[3], state);
}

fn draw_header(f: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &WhoState, vm: &WhoViewModel) {
    let t = &vm.totals;
    let summary = Line::from(vec![
        Span::styled(format!("{} identities", t.total), Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("  ·  {} declared  ·  ", t.declared)),
        Span::styled(format!("{} live", t.live), color_style("green")),
        Span::raw(format!("  ·  {} producing", t.producing)),
    ]);
    let line = if state.editing_filter {
        Line::from(vec![Span::styled(
            format!("filter: {}▏ (Esc clears · Enter applies · live:on/off use:on/off + text)", state.filter),
            Style::default().fg(Color::Cyan),
        )])
    } else if let Some(fstr) = &vm.filter {
        Line::from(vec![
            summary.spans[0].clone(),
            Span::raw(format!("  ·  {} of {} shown  ·  filter \"{}\"", t.shown, t.total, fstr)),
        ])
    } else {
        summary
    };
    let p = Paragraph::new(line)
        .block(Block::default().borders(Borders::ALL).title("goat who — identities"));
    f.render_widget(p, area);
}

/// Roster column widths. NAME is wide enough for the human names the portal
/// returns (`octopus-alpha (synthetic)`, `2nd-sig-odt-moore`); numerics
/// right-align; every cell hard-truncates so nothing bleeds across a column.
const NAME_W: u16 = 28;

fn draw_roster(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &WhoState,
    vm: &WhoViewModel,
    table_state: &mut TableState,
) {
    let header = Row::new(vec![
        Cell::from("NAME"),
        Cell::from("AFF"),
        Cell::from("SRC"),
        Cell::from("LIVE"),
        rcell("MSG/S"),
        Cell::from("BAND"),
        Cell::from("W/R"),
        Cell::from("STATE"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = vm.rows.iter().map(roster_row).collect();

    let title = if let Some(fstr) = &vm.filter {
        format!(
            "roster ({} of {} · filter \"{}\" · / edit · sort: {})",
            vm.totals.shown,
            vm.totals.total,
            fstr,
            sort_label(state.sort)
        )
    } else {
        format!(
            "roster ({} identities · / filter · s sort: {} · a fetch-all-perms)",
            vm.totals.total,
            sort_label(state.sort)
        )
    };
    let widths = [
        Constraint::Length(NAME_W), // NAME
        Constraint::Length(11),     // AFF (fits "organic EST")
        Constraint::Length(5),      // SRC
        Constraint::Length(11),     // LIVE
        Constraint::Length(8),      // MSG/S
        Constraint::Length(5),      // BAND
        Constraint::Length(5),      // W/R
        Constraint::Length(9),      // STATE (fits "no-grants")
    ];

    // Drive the viewport off the selection so ratatui scrolls to keep the
    // selected row visible (the goat-who 0.4.7 no-scroll defect). The
    // persisted TableState carries the offset across frames.
    let selected_idx = state
        .selected
        .as_deref()
        .and_then(|sel| vm.rows.iter().position(|r| r.anchor == sel));
    table_state.select(selected_idx);

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(
            Style::default()
                .add_modifier(Modifier::REVERSED)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_stateful_widget(table, area, table_state);

    // Scrollbar on the inner right edge when the roster overflows the
    // viewport — the visible cue that there's more above/below.
    let viewport_rows = area.height.saturating_sub(3) as usize; // borders + header
    if vm.rows.len() > viewport_rows && !vm.rows.is_empty() {
        let mut sb = ScrollbarState::new(vm.rows.len()).position(selected_idx.unwrap_or(0));
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        f.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut sb,
        );
    }
}

fn rcell<'a>(s: impl Into<String>) -> Cell<'a> {
    Cell::from(Text::from(s.into()).alignment(Alignment::Right))
}

/// Compact affiliation label: `kind nation` (e.g. `organic EST`), `kind` alone
/// when no nation, `·` when nothing is declared — never a guessed default.
fn aff_label(kind: &str, nation: &str) -> String {
    if kind.is_empty() {
        "·".to_string()
    } else if nation.is_empty() {
        kind.to_string()
    } else {
        format!("{kind} {nation}")
    }
}

/// Per-kind affiliation tint so the three families read at a glance; absent → dim.
fn aff_color(kind: &str) -> &'static str {
    match kind {
        "vendor" => "cyan",
        "organic" => "green",
        "partner" => "magenta",
        _ => "",
    }
}

/// Human rendering of a stray's reconcile reason: (short label, verify-first?,
/// one-line guidance). `verify` = the false-positive class an operator must NOT
/// blind-cut (unregistered-key / anything unrecognized). Empty reason → None.
fn recon_reason_display(reason: &str) -> Option<(&'static str, bool, &'static str)> {
    match reason {
        "untracked" => Some((
            "untracked",
            false,
            "no declared org key — unmanaged peer; cut candidate",
        )),
        "no-declared-member" => Some((
            "residue",
            false,
            "org declared, this member isn't — offboarding residue; cut candidate",
        )),
        "unregistered-key" => Some((
            "unregistered",
            true,
            "key matches no declared org — VERIFY: naming drift (fix it) OR offboarded org",
        )),
        "" => None,
        _ => Some(("stray?", true, "unclassified — verify before any cut")),
    }
}

fn roster_row<'a>(r: &RowVM) -> Row<'a> {
    let msgs = r.msgs.map(|m| format!("{m:.0}")).unwrap_or_else(|| "—".to_string());
    // STATE column: the cross-layer status (varies + actionable), not the old
    // uniform recon "active". When the bulk roster hasn't landed it falls back
    // to the perms fetch-lifecycle marker so the column is never blank-blank.
    let (state_str, state_col) = if r.status == StatusVM::Unknown {
        let s = perms_state(&r.perms);
        let c = state_color(&s).to_string();
        (s, c)
    } else if r.status == StatusVM::Stray && matches!(recon_reason_display(&r.recon_reason), Some((_, true, _))) {
        // A verify-first stray (unregistered-key): flag it distinctly so it's
        // never swept as a clean cut candidate.
        ("stray?".to_string(), "amber".to_string())
    } else {
        (r.status.label().to_string(), r.status.color().to_string())
    };
    let cells = vec![
        Cell::from(truncate(&r.name, NAME_W as usize)),
        Cell::from(aff_label(&r.aff_kind, &r.aff_nation)).style(color_style(aff_color(&r.aff_kind))),
        // SRC: live-only identities are dim "live"; registry members are "reg".
        if r.declared {
            Cell::from("reg")
        } else {
            Cell::from("live").style(color_style(""))
        },
        Cell::from(live_cell(&r.live)).style(color_style(live_color(&r.live))),
        rcell(msgs),
        Cell::from(r.band_label.clone()).style(color_style(&r.band_color)),
        Cell::from(perms_cell(&r.perms)).style(color_style(perms_color(&r.perms))),
        Cell::from(state_str).style(color_style(&state_col)),
    ];
    // Selection highlight is the Table's row_highlight_style; here we only
    // flag a near-limit producer (amber) when it's NOT the selected row.
    let style = if r.near_limit {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    Row::new(cells).style(style)
}

fn draw_detail(f: &mut ratatui::Frame, area: ratatui::layout::Rect, vm: &WhoViewModel) {
    let block = Block::default().borders(Borders::ALL).title("identity");
    let Some(d) = &vm.detail else {
        f.render_widget(
            Paragraph::new(Span::styled(
                "no identity selected — ↑/↓ to pick one",
                color_style(""),
            ))
            .block(block),
            area,
        );
        return;
    };
    f.render_widget(Paragraph::new(build_detail_lines(d)).block(block), area);
}

/// The drill-down: Enter expanded the selected identity to full screen. Same
/// detail content as the side pane (so there's one source of truth), plus a
/// remediation surface for actionable states — the copy-ready `goat admin`
/// command. `goat who` stays a pure read-only consumer (ADR 1047): it reveals
/// and proposes the fix; the mutation itself runs through the audited
/// `goat admin` chokepoint, never from inside this surface.
fn draw_detail_full(f: &mut ratatui::Frame, area: ratatui::layout::Rect, vm: &WhoViewModel) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("identity — drill-down  ·  Esc/q back  ·  ↑/↓ next");
    let Some(d) = &vm.detail else {
        f.render_widget(
            Paragraph::new(Span::styled("no identity selected", color_style("")))
                .block(block),
            area,
        );
        return;
    };
    let mut lines = build_detail_lines(d);
    append_remediation_lines(&mut lines, d);
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Fixed left-label width so the detail pane reads as an aligned key/value
/// grid — the eye scans labels down one column and values across another —
/// rather than the ragged stack of mixed-width labels it used to be.
const DETAIL_LABEL_W: usize = 12;

/// One grid row: a dim fixed-width label, then the value spans aligned to a
/// common left edge.
fn kv_line<'a>(label: &str, value: Vec<Span<'a>>) -> Line<'a> {
    let mut spans = vec![Span::styled(
        format!("{:<w$}", label, w = DETAIL_LABEL_W),
        Style::default().fg(Color::DarkGray),
    )];
    spans.extend(value);
    Line::from(spans)
}

/// A sub-row under a section (a write/read grant under DATA): 2-space indent +
/// short label, padded so the value column still lines up with the grid.
fn kv_sub<'a>(label: &str, value: Vec<Span<'a>>) -> Line<'a> {
    let mut spans = vec![Span::styled(
        format!("  {:<w$}", label, w = DETAIL_LABEL_W - 2),
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
    )];
    spans.extend(value);
    Line::from(spans)
}

/// Build the per-identity detail lines shared by the side pane and the
/// full-screen drill-down (one source of truth). Rendered as an aligned
/// key/value grid (see [`kv_line`]) so it's scannable, not a ragged stack.
fn build_detail_lines<'a>(d: &'a DetailVM) -> Vec<Line<'a>> {
    let r = &d.row;
    let x = d.xlayer.as_ref();
    let mut lines: Vec<Line> = Vec::new();

    // IDENTITY — name + composed cross-layer status + registry membership.
    let mut ident = vec![Span::styled(
        r.name.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if let Some(x) = x {
        ident.push(Span::raw("   "));
        ident.push(Span::styled(
            x.status.label().to_string(),
            color_style(x.status.color()),
        ));
    }
    ident.push(Span::raw("   "));
    ident.push(if r.declared {
        Span::styled("in registry", color_style("green"))
    } else {
        Span::styled("not in registry", color_style("amber"))
    });
    lines.push(kv_line("IDENTITY", ident));

    // ANCHOR — the UUID, exactly once.
    lines.push(kv_line("ANCHOR", vec![Span::raw(truncate(&r.anchor, 48))]));

    // KIND — node class (vendor / person / slot), from the bulk roster.
    if let Some(k) = x.map(|x| x.kind.as_str()).filter(|k| !k.is_empty()) {
        lines.push(kv_line("KIND", vec![Span::raw(k.to_string())]));
    }

    // AFFILIATION — declared org attribution (ADR 1043), never access.
    let aff = if r.aff_kind.is_empty() {
        vec![Span::styled("— none declared", color_style(""))]
    } else {
        let txt = if r.aff_nation.is_empty() {
            r.aff_kind.clone()
        } else {
            format!("{} · {}", r.aff_kind, r.aff_nation)
        };
        vec![Span::styled(txt, color_style(aff_color(&r.aff_kind)))]
    };
    lines.push(kv_line("AFFILIATION", aff));

    // CUSTODY — machines this identity has enrolled (ADR 1096/1098): the
    // person→system edge, with each machine's node-class / autonomy / (PLATFORM)
    // workloads. Omitted when it custodies none, or before the fleet lands.
    if !d.custody.is_empty() {
        let n = d.custody.len();
        lines.push(kv_line(
            "CUSTODY",
            vec![Span::styled(
                format!("{n} machine{}", if n == 1 { "" } else { "s" }),
                color_style("green"),
            )],
        ));
        for m in &d.custody {
            let mut s = format!("  {}  {} / {}", m.handle, m.node_class, m.autonomy);
            if !m.workloads.is_empty() {
                s.push_str(&format!("  [{}]", m.workloads.join(",")));
            }
            lines.push(Line::from(vec![Span::raw(s)]));
        }
    }

    // RECON — for a stray, WHY it didn't reconcile + whether it's safe to cut.
    // The verify-first class (unregistered-key) is amber so it's never mistaken
    // for a clean cut candidate; the guidance line spells out the choice.
    if let Some((label, verify, guidance)) = recon_reason_display(&r.recon_reason) {
        let color = if verify { "amber" } else { "red" };
        lines.push(kv_line(
            "RECON",
            vec![
                Span::styled(label.to_string(), color_style(color)),
                Span::styled(format!("  {guidance}"), color_style("")),
            ],
        ));
    }

    // PRESENCE — liveness (honest: unconfirmed when no presence beat).
    let (ptxt, pcol) = match &r.live {
        LiveVM::Alive(age) => (format!("● alive {age}"), live_color(&r.live)),
        LiveVM::Dropped(age, drops) => {
            (format!("○ dropped {age} ({drops} drops)"), live_color(&r.live))
        }
        LiveVM::Unmetered => ("— no presence (unconfirmed)".to_string(), ""),
    };
    lines.push(kv_line("PRESENCE", vec![Span::styled(ptxt, color_style(pcol))]));

    // TRAFFIC — production rate + envelope band.
    let mut traffic = match (r.msgs, r.bytes) {
        (Some(m), Some(b)) => vec![Span::raw(format!(
            "{m:.0} msgs/s · {b:.0} B/s  [{}]",
            r.shares
        ))],
        _ => vec![Span::styled("— not producing".to_string(), color_style(""))],
    };
    if r.band_label != "—" {
        traffic.push(Span::raw("   band "));
        traffic.push(Span::styled(
            r.band_label.to_uppercase(),
            color_style(&r.band_color),
        ));
    }
    lines.push(kv_line("TRAFFIC", traffic));

    // ── access (mesh / data / release) ──────────────────────────────────────
    match &d.access {
        AccessDetailVM::PortalDown => lines.push(kv_line(
            "ACCESS",
            vec![Span::styled("portal unreachable", color_style("amber"))],
        )),
        AccessDetailVM::NotFetched | AccessDetailVM::InFlight => {
            // Until the full per-layer fetch lands, show the instant bulk-roster
            // rollup (W/R counts + mesh) so the grid isn't blank.
            if let Some(x) = x {
                let mesh = if x.on_mesh { "on" } else { "off" };
                lines.push(kv_line(
                    "ACCESS",
                    vec![Span::styled(
                        format!("fetching… · {}W/{}R · mesh {mesh}", x.writes, x.reads),
                        color_style(""),
                    )],
                ));
            } else {
                lines.push(kv_line("ACCESS", vec![Span::styled("fetching…", color_style(""))]));
            }
        }
        AccessDetailVM::Failed(e) => {
            let (msg, color) = match e {
                AccessErr::Unreachable(_) => (
                    "portal didn't answer in time (slow mesh) — press r to retry".to_string(),
                    "amber",
                ),
                AccessErr::Unauthorized => {
                    ("unauthorized — token can't query this identity".to_string(), "red")
                }
                AccessErr::Other(s) => (format!("error — {}", truncate(s, 52)), "red"),
            };
            lines.push(kv_line("ACCESS", vec![Span::styled(msg, color_style(color))]));
        }
        AccessDetailVM::Ready {
            on_mesh,
            writes,
            reads,
            live_subs,
            notes,
            ..
        } => {
            push_access_grid(&mut lines, on_mesh, writes, reads, live_subs, &r.name);
            for n in notes.iter().take(1) {
                lines.push(kv_line(
                    "NOTE",
                    vec![Span::styled(truncate(n, 64), color_style(""))],
                ));
            }
        }
    }

    lines
}

/// Append a remediation surface to the drill-down for actionable states. A
/// `stray` (live on the wire, no declared record) and a `cut-off` identity are
/// the two an operator wants to act on; both get the exact copy-ready
/// `goat admin` command. Nothing executes here — the COP proposes, the audited
/// `goat admin` path enforces (ADR 1047 / F-207). Healthy rows show no section.
fn append_remediation_lines(lines: &mut Vec<Line<'_>>, d: &DetailVM) {
    let r = &d.row;
    let status = d.xlayer.as_ref().map(|x| x.status);
    // A stray is out-of-band: present on a live layer, absent from the declared
    // roster. `goat who` surfaces it; the fix is to either claim it (onboard a
    // declared record) or cut it (mesh-first revoke). Both run via goat admin.
    let (heading, actions): (&str, Vec<(&str, String)>) = match status {
        Some(StatusVM::Stray) => (
            "REMEDIATION — stray (live, no declared record)",
            vec![
                (
                    "claim",
                    format!("goat admin onboard <handle> --env <goatnet>   # adopt; pick a handle for {}", truncate(&r.name, 24)),
                ),
                (
                    "cut",
                    format!("goat admin onboard --revoke {}   # mesh-first cut", truncate(&r.anchor, 32)),
                ),
            ],
        ),
        Some(StatusVM::CutOff) => (
            "REMEDIATION — cut off (declared, negatively enforced)",
            vec![
                (
                    "re-onboard",
                    "goat admin onboard <handle> --env <goatnet>   # restore access".to_string(),
                ),
                (
                    "rotate",
                    format!("goat admin rotate {}   # re-key the leaf", truncate(&r.anchor, 32)),
                ),
            ],
        ),
        _ => return,
    };
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        heading,
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )));
    // A verify-first stray (unregistered-key) may be reconcilable, not residue —
    // warn before the cut command is anywhere near the operator's fingers.
    if let Some((_, true, guidance)) = recon_reason_display(&r.recon_reason) {
        lines.push(Line::from(Span::styled(
            format!("  ⚠ verify first — {guidance}"),
            color_style("amber"),
        )));
    }
    for (verb, cmd) in actions {
        lines.push(Line::from(vec![
            Span::styled(format!("  {verb:<10} "), color_style("cyan")),
            Span::raw(cmd),
        ]));
    }
    lines.push(Line::from(Span::styled(
        "  (copy-ready — run in goat admin; goat who never mutates the fabric)",
        color_style(""),
    )));
}

/// Map a portal access `.layer` string to one of the four identity domains
/// the detail pane groups by. Unknown layers fall into DATA (the Zenoh ACL is
/// the default write-integrity layer). Keeps the grouping honest: a domain
/// shows only the entries the portal actually attributed to it.
fn perm_domain(layer: &str) -> &'static str {
    let l = layer.to_ascii_lowercase();
    if l.contains("mesh") || l.contains("netbird") {
        "mesh"
    } else if l.contains("ident") || l.contains("ory") || l.contains("group") {
        "identity"
    } else if l.contains("release") || l.contains("quarantine") {
        "release"
    } else {
        "data"
    }
}

/// Render the access domains (MESH / DATA / RELEASE) as aligned grid rows + a
/// few indented grant sub-rows — replacing the old ragged domain dump. The
/// IDENTITY / KIND lines now live in the grid above; this is the capability
/// half. Overflow points at `goat access why <name>` for the full trace.
fn push_access_grid(
    lines: &mut Vec<Line>,
    on_mesh: &bool,
    writes: &[(String, String, String)],
    reads: &[(String, String, String)],
    live_subs: &[String],
    name: &str,
) {
    lines.push(kv_line(
        "MESH",
        vec![
            Span::styled(
                if *on_mesh { "on" } else { "off" }.to_string(),
                color_style(if *on_mesh { "green" } else { "" }),
            ),
            Span::styled("  (NetBird overlay)", color_style("")),
        ],
    ));
    push_perm_section(lines, "DATA", "data", writes, reads, name);
    // The observed LIVE-SUBSCRIPTION rows sit inside DATA, directly under the
    // read GRANTS, so an operator reads the two side by side: a grant is
    // may-read (ACL allow-list); a live sub is is-reading (census ground truth).
    push_live_subs(lines, live_subs);
    push_perm_section(lines, "RELEASE", "release", writes, reads, name);
}

/// Render the DATA-layer LIVE SUBSCRIPTIONS observed from the pub/sub census —
/// what the identity IS reading right now — as a sub-block under the read GRANTS.
/// The distinction is the whole point: a GRANT is may-read (ACL allow-list); a
/// LIVE SUB is is-reading (fabric ground truth). Honest-degrade: an empty census
/// (the ACL grant is deploy-pending) or an anchor with no correlated subscriber
/// renders NOTHING — never a faked topic, and never a claim of zero (absence of
/// an observation is not proof of no subscription, so we stay silent).
fn push_live_subs(lines: &mut Vec<Line>, live_subs: &[String]) {
    if live_subs.is_empty() {
        return;
    }
    lines.push(kv_line(
        "LIVE SUBS",
        vec![Span::styled(
            format!("{} observed (is-reading)", live_subs.len()),
            color_style("green"),
        )],
    ));
    let shown = live_subs.len().min(3);
    for ke in live_subs.iter().take(shown) {
        lines.push(kv_sub(
            "live-sub",
            vec![
                Span::styled(format!("{:<34}", truncate(ke, 34)), color_style("green")),
                Span::styled("  is-reading (census)", color_style("")),
            ],
        ));
    }
    if live_subs.len() > shown {
        lines.push(kv_sub(
            "",
            vec![Span::styled(
                format!("+{} more", live_subs.len() - shown),
                color_style(""),
            )],
        ));
    }
}

/// One access domain as a grid row (`HEADING  nW / mR`) + up to 3 grant
/// sub-rows (writes first, aligned), overflow → `+N more — goat access why
/// <name>`. A domain with nothing stays silent, except DATA whose emptiness is
/// meaningful (the F-177 silent-denial surface).
fn push_perm_section(
    lines: &mut Vec<Line>,
    heading: &str,
    domain: &str,
    writes: &[(String, String, String)],
    reads: &[(String, String, String)],
    name: &str,
) {
    let w: Vec<&(String, String, String)> =
        writes.iter().filter(|(_, l, _)| perm_domain(l) == domain).collect();
    let r: Vec<&(String, String, String)> =
        reads.iter().filter(|(_, l, _)| perm_domain(l) == domain).collect();
    if w.is_empty() && r.is_empty() && domain != "data" {
        return; // a domain with nothing to say stays silent (honest)
    }
    lines.push(kv_line(
        heading,
        vec![Span::styled(
            format!("{}W / {}R", w.len(), r.len()),
            color_style(if w.is_empty() && r.is_empty() { "" } else { "cyan" }),
        )],
    ));
    let grant = |label: &'static str, topic: &str, reason: &str| -> Line<'static> {
        kv_sub(
            label,
            vec![
                Span::raw(format!("{:<34}", truncate(topic, 34))),
                Span::styled(format!("  {}", truncate(reason, 22)), color_style("")),
            ],
        )
    };
    let mut shown = 0usize;
    for (topic, _, reason) in w.iter().take(2) {
        lines.push(grant("write", topic, reason));
        shown += 1;
    }
    for (topic, _, reason) in r.iter().take(2) {
        if shown >= 3 {
            break;
        }
        lines.push(grant("read", topic, reason));
        shown += 1;
    }
    let total = w.len() + r.len();
    if total > shown {
        lines.push(kv_sub(
            "",
            vec![Span::styled(
                format!("+{} more — goat access why {name}", total - shown),
                color_style(""),
            )],
        ));
    }
}

fn draw_footer(f: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &WhoState) {
    let paused = if state.paused { "  [PAUSED]" } else { "" };
    let help_text = if state.editing_filter {
        " type to filter · Enter applies · Esc clears · live:on/off use:on/off + substring (AND-ed) ".to_string()
    } else if state.focused {
        " drill-down · Esc/q back · ↑/↓ jk next identity · r retry perms · read-only ".to_string()
    } else {
        format!(" q quit · Enter drill-down · / filter · s sort · a fetch-all-perms · r retry · p pause{paused} · ↑/↓ jk g/G · read-only ")
    };
    let mut lines = vec![Line::from(vec![Span::styled(
        help_text,
        Style::default().add_modifier(Modifier::DIM),
    )])];
    if let Some(e) = &state.status {
        lines.push(Line::from(Span::styled(
            format!(" ⚠ {}", truncate(e, 100)),
            Style::default().fg(Color::Red),
        )));
    }
    f.render_widget(Paragraph::new(lines), area);
}

// --- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health_wire::{ClassShareV1, PresencePeerV1};
    use panoscope_core::portal::AliasRow;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn reg() -> AliasRegistry {
        AliasRegistry::from_rows([
            AliasRow { slot_id: A.to_string(), alias: "alpha-sensor".into() },
            AliasRow { slot_id: B.to_string(), alias: "bravo-effector".into() },
        ])
    }

    fn declared() -> Vec<DeclaredActor> {
        vec![
            DeclaredActor { anchor: A.to_string(), display_name: Some("Alpha Sensor".into()) },
            DeclaredActor { anchor: B.to_string(), display_name: None },
        ]
    }

    fn fp(peer: &str, rate: f64) -> ProducerFootprintV1 {
        ProducerFootprintV1 {
            peer: peer.to_string(),
            ts_unix_ms: 0,
            sample_rate: rate,
            byte_rate: rate * 100.0,
            shares: vec![ClassShareV1 { class: "C2".into(), sample_rate: rate, byte_rate: rate * 100.0 }],
            rank: 0,
            near_limit: false,
        }
    }

    fn presence_env(peers: Vec<PresencePeerV1>) -> EnvHealthV1 {
        EnvHealthV1 {
            domain: "presence".into(),
            scope: "test".into(),
            ts_unix_ms: 100_000,
            overall_color: "green".into(),
            sample_count: peers.len() as u64,
            window_secs: 30,
            detail: Some(EnvDetail::Presence(PresenceDetailV1 {
                alive_total: peers.iter().filter(|p| p.alive).count() as u32,
                dropped_recent: 0,
                peers,
            })),
        }
    }

    fn alive_peer(write_root: &str, alive: bool) -> PresencePeerV1 {
        PresencePeerV1 {
            token_key: format!("{write_root}/presence/x"),
            write_root: write_root.to_string(),
            label: "x".into(),
            alive,
            since_unix_ms: 40_000,
            drop_count: if alive { 0 } else { 2 },
        }
    }

    #[test]
    fn roster_is_union_of_declared_and_live() {
        let now = Instant::now();
        let mut s = WhoState::new(declared(), None, WhoSort::Name, true);
        // A live anchor C with no registry entry must still appear.
        let c = "cccccccccccccccccccccccccccccccc";
        s.apply_footprint(fp(c, 5.0), now);
        let anchors = s.identity_anchors(now);
        assert!(anchors.contains(A));
        assert!(anchors.contains(B));
        assert!(anchors.contains(c), "live-only anchor must be in the roster");
    }

    #[test]
    fn default_sort_is_name_not_usage() {
        let now = Instant::now();
        let mut s = WhoState::new(declared(), None, WhoSort::Name, true);
        // B produces far more than A, but NAME sort keeps alpha before bravo.
        s.apply_footprint(fp(B, 999.0), now);
        s.apply_footprint(fp(A, 1.0), now);
        let order = s.sorted_anchors(&reg(), now);
        assert_eq!(order, vec![A.to_string(), B.to_string()]);
    }

    #[test]
    fn use_sort_ranks_by_rate() {
        let now = Instant::now();
        let mut s = WhoState::new(declared(), None, WhoSort::Use, true);
        s.apply_footprint(fp(A, 1.0), now);
        s.apply_footprint(fp(B, 999.0), now);
        let order = s.sorted_anchors(&reg(), now);
        assert_eq!(order, vec![B.to_string(), A.to_string()]);
    }

    #[test]
    fn substring_filter_matches_alias() {
        let now = Instant::now();
        let mut s = WhoState::new(declared(), None, WhoSort::Name, true);
        s.filter = "bravo".into();
        let order = s.sorted_anchors(&reg(), now);
        assert_eq!(order, vec![B.to_string()]);
    }

    #[test]
    fn live_facet_filters_by_presence() {
        let now = Instant::now();
        let mut s = WhoState::new(declared(), None, WhoSort::Name, true);
        s.apply_presence(presence_env(vec![alive_peer(A, true), alive_peer(B, false)]), now);
        s.filter = "live:on".into();
        assert_eq!(s.sorted_anchors(&reg(), now), vec![A.to_string()]);
        s.filter = "live:off".into();
        assert_eq!(s.sorted_anchors(&reg(), now), vec![B.to_string()]);
    }

    #[test]
    fn anded_terms_must_all_match() {
        let now = Instant::now();
        let mut s = WhoState::new(declared(), None, WhoSort::Name, true);
        s.apply_presence(presence_env(vec![alive_peer(A, true), alive_peer(B, true)]), now);
        // live:on AND name-contains "alpha" → only A.
        s.filter = "live:on alpha".into();
        assert_eq!(s.sorted_anchors(&reg(), now), vec![A.to_string()]);
    }

    #[test]
    fn live_is_unmetered_without_presence_never_offline() {
        let now = Instant::now();
        let s = WhoState::new(declared(), None, WhoSort::Name, true);
        // No presence beat → unmetered (honest), not a fabricated alive/offline.
        assert_eq!(s.live_of(A, now), LiveVM::Unmetered);
    }

    #[test]
    fn perms_are_portal_down_when_no_portal() {
        let s = WhoState::new(declared(), None, WhoSort::Name, false);
        assert_eq!(s.perms_of(A), PermsVM::PortalDown);
    }

    #[test]
    fn perms_not_fetched_until_requested() {
        let s = WhoState::new(declared(), None, WhoSort::Name, true);
        assert_eq!(s.perms_of(A), PermsVM::NotFetched);
    }

    #[test]
    fn band_is_dash_without_footprint() {
        let env = EnvelopeRegistry::empty();
        let (label, color) = band_of(&env, A, None);
        assert_eq!(label, "—");
        assert!(color.is_empty());
    }

    #[test]
    fn view_model_totals_are_honest() {
        let now = Instant::now();
        let mut s = WhoState::new(declared(), None, WhoSort::Name, true);
        s.apply_footprint(fp(A, 10.0), now);
        s.apply_presence(presence_env(vec![alive_peer(A, true)]), now);
        let vm = s.view_model(&reg(), &EnvelopeRegistry::empty(), now);
        assert_eq!(vm.totals.declared, 2);
        assert_eq!(vm.totals.producing, 1);
        assert_eq!(vm.totals.live, 1);
        assert_eq!(vm.totals.total, 2); // A,B declared; A also live/producing.
    }

    const C: &str = "cccccccccccccccccccccccccccccccc";
    const D: &str = "dddddddddddddddddddddddddddddddd";

    #[test]
    fn name_priority_is_alias_then_display_then_uuid() {
        // The bug-4 regression: a vendor with a display name but NO slot alias
        // must render its display name (e.g. "org89x"), never a bare UUID.
        let s = WhoState::new(
            vec![
                DeclaredActor { anchor: A.to_string(), display_name: Some("Alpha Sensor".into()) },
                DeclaredActor { anchor: C.to_string(), display_name: Some("Lighthouse".into()) },
                DeclaredActor { anchor: D.to_string(), display_name: None },
            ],
            None,
            WhoSort::Name,
            true,
        );
        let r = reg(); // aliases for A + B only
        assert_eq!(s.resolved_name(&r, A), "alpha-sensor"); // alias beats display
        assert_eq!(s.resolved_name(&r, C), "Lighthouse"); // display when no alias
        assert_eq!(s.resolved_name(&r, D), r.render(D)); // truncated-UUID floor
        assert_ne!(s.resolved_name(&r, C), C); // never the bare UUID when a name exists
    }

    #[test]
    fn substring_filter_matches_display_name() {
        let now = Instant::now();
        let mut s = WhoState::new(
            vec![DeclaredActor { anchor: C.to_string(), display_name: Some("Lighthouse".into()) }],
            None,
            WhoSort::Name,
            true,
        );
        // No alias registry, but the search still finds it by display name.
        s.filter = "light".into();
        assert_eq!(
            s.sorted_anchors(&AliasRegistry::empty(), now),
            vec![C.to_string()]
        );
    }

    #[test]
    fn access_err_classifies_portal_errors() {
        use panoscope_core::portal::PortalError;
        use reqwest::StatusCode;
        assert!(AccessErr::from_portal(PortalError::Transient("timeout".into())).is_unreachable());
        assert_eq!(
            AccessErr::from_portal(PortalError::status(StatusCode::UNAUTHORIZED)),
            AccessErr::Unauthorized
        );
        assert_eq!(
            AccessErr::from_portal(PortalError::status(StatusCode::FORBIDDEN)),
            AccessErr::Unauthorized
        );
        // A 5xx is a real error, not "unreachable".
        assert!(!AccessErr::from_portal(PortalError::status(StatusCode::INTERNAL_SERVER_ERROR))
        .is_unreachable());
    }

    #[test]
    fn perms_failed_distinguishes_slow_from_denied() {
        let mut s = WhoState::new(declared(), None, WhoSort::Name, true);
        s.apply_access(A.to_string(), AccessOutcome::Err(AccessErr::Unreachable("t".into())));
        assert_eq!(s.perms_of(A), PermsVM::Failed { unreachable: true });
        s.apply_access(B.to_string(), AccessOutcome::Err(AccessErr::Unauthorized));
        assert_eq!(s.perms_of(B), PermsVM::Failed { unreachable: false });
    }

    fn roster_entry(id: &str, writes: usize, reads: usize) -> AccessRosterEntry {
        AccessRosterEntry {
            id: id.to_string(),
            label: format!("name-{id}"),
            kind: "vendor".into(),
            state: "active".into(),
            writes,
            reads,
            ..Default::default()
        }
    }

    #[test]
    fn bulk_roster_seeds_perms_instantly() {
        // The responsiveness fix: one apply_roster makes W/R render for the
        // whole roster with NO per-row lazy fetch.
        let mut s = WhoState::new(declared(), None, WhoSort::Name, true);
        assert_eq!(s.perms_of(A), PermsVM::NotFetched); // before the roster lands
        s.apply_roster(vec![roster_entry(A, 3, 1)]);
        assert_eq!(
            s.perms_of(A),
            PermsVM::Ready { writes: 3, reads: 1, state: "active".into() },
            "bulk summary must satisfy perms_of instantly, no lazy fetch"
        );
    }

    #[test]
    fn affiliation_seeds_from_roster_and_renders() {
        // Affiliation rides the bulk roster onto the row; absent stays "·".
        let mut s = WhoState::new(vec![], None, WhoSort::Name, true);
        s.apply_roster(vec![
            AccessRosterEntry {
                id: A.into(),
                label: "USAREUR-AF".into(),
                kind: "vendor".into(),
                affiliation: "organic".into(),
                nation: "USA".into(),
                ..Default::default()
            },
            AccessRosterEntry { id: B.into(), label: "no-aff".into(), ..Default::default() },
        ]);
        assert_eq!(s.aff_of(A), ("organic".into(), "USA".into()));
        assert_eq!(s.aff_of(B), (String::new(), String::new()));
        // The compact label joins kind+nation; honest "·" when none declared.
        assert_eq!(aff_label("organic", "USA"), "organic USA");
        assert_eq!(aff_label("vendor", ""), "vendor");
        assert_eq!(aff_label("", ""), "·");
    }

    #[test]
    fn status_varies_by_cross_layer_summary() {
        // The STATE-is-uniformly-active fix: status_of varies + is actionable.
        let mut s = WhoState::new(vec![], None, WhoSort::Name, true);
        s.apply_roster(vec![
            roster_entry(A, 1, 1),                                  // active (has grants)
            AccessRosterEntry { id: B.into(), state: "cut_off".into(), ..Default::default() },
            AccessRosterEntry { id: C.into(), writes: 0, reads: 0, ..Default::default() }, // no-grants
            AccessRosterEntry { id: D.into(), writes: 1, reads: 1, on_mesh: true, connected: true, ..Default::default() },
        ]);
        assert_eq!(s.status_of(A), StatusVM::Active);
        // Active carries no live signal and must not wear the live colour.
        assert_eq!(StatusVM::Active.color(), "");
        assert_eq!(StatusVM::Online.color(), "green");
        assert_eq!(s.status_of(B), StatusVM::CutOff);
        assert_eq!(s.status_of(C), StatusVM::NoGrants);
        assert_eq!(s.status_of(D), StatusVM::Online);
        // an anchor the roster never carried is honestly Unknown, never a guess.
        assert_eq!(s.status_of("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"), StatusVM::Unknown);
    }

    #[test]
    fn roster_label_names_cross_layer_identity() {
        // A person/entity carried only by the bulk roster (not declared, no
        // alias) renders by its roster label, never a bare UUID.
        let s = {
            let mut s = WhoState::new(vec![], None, WhoSort::Name, true);
            s.apply_roster(vec![AccessRosterEntry {
                id: C.into(),
                label: "person-85@example.org".into(),
                kind: "person".into(),
                ..Default::default()
            }]);
            s
        };
        assert_eq!(s.resolved_name(&AliasRegistry::empty(), C), "person-85@example.org");
        // and it joins the roster universe even though it's not declared/live.
        assert!(s.identity_anchors(Instant::now()).contains(C));
    }

    // --- headless render tests (ratatui TestBackend) -------------------------
    // Two behaviours are interactive-only: the roster scrolls to follow the
    // selection, and the detail pane is structured rather than a flat Zenoh dump.
    // These render the real `draw()` to an in-memory buffer (no TTY) and assert
    // the pixels, so the regressions can't silently come back.

    fn buffer_text(term: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        let buf = term.backend().buffer();
        let w = buf.area.width as usize;
        let mut s = String::new();
        for (i, cell) in buf.content.iter().enumerate() {
            if i > 0 && i % w == 0 {
                s.push('\n');
            }
            s.push_str(cell.symbol());
        }
        s
    }

    fn many_declared(n: usize) -> Vec<DeclaredActor> {
        (0..n)
            .map(|i| DeclaredActor {
                anchor: format!("{i:0>32}"),
                display_name: Some(format!("Org{i:02}")),
            })
            .collect()
    }

    #[test]
    fn drilldown_full_screen_surfaces_stray_remediation() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let now = Instant::now();
        let reg = AliasRegistry::empty();
        let env = EnvelopeRegistry::empty();
        let mut s = WhoState::new(vec![], None, WhoSort::Name, true);
        // A stray: live on the mesh, out-of-band from the declared roster.
        s.apply_roster(vec![AccessRosterEntry {
            id: C.into(),
            label: "ghost-peer".into(),
            kind: "mesh_peer".into(),
            state: "out_of_band".into(),
            on_mesh: true,
            ..Default::default()
        }]);
        s.sorted_cache = s.sorted_anchors(&reg, now);
        s.selected = Some(C.into());
        s.focused = true; // Enter drilled in
        let vm = s.view_model(&reg, &env, now);

        let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut ts = TableState::default();
        term.draw(|f| draw(f, &s, &vm, &mut ts)).unwrap();
        let text = buffer_text(&term);

        assert!(text.contains("drill-down"), "full-screen header missing:\n{text}");
        assert!(text.contains("REMEDIATION"), "stray remediation section missing:\n{text}");
        assert!(text.contains("onboard --revoke"), "copy-ready cut command missing:\n{text}");
        // The roster table is hidden in drill-down (its header column is gone).
        assert!(!text.contains("MSG/S"), "roster must hide in drill-down:\n{text}");
    }

    #[test]
    fn stray_recon_reason_surfaces_and_flags_verify_class() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let now = Instant::now();
        let reg = AliasRegistry::empty();
        let env = EnvelopeRegistry::empty();
        // A verify-first stray: carries an org-shaped key that matched nothing.
        let mut s = WhoState::new(vec![], None, WhoSort::Name, true);
        s.apply_roster(vec![AccessRosterEntry {
            id: C.into(),
            label: "peer-acme-1".into(),
            kind: "mesh_peer".into(),
            state: "out_of_band".into(),
            recon_reason: "unregistered-key".into(),
            on_mesh: true,
            ..Default::default()
        }]);
        s.sorted_cache = s.sorted_anchors(&reg, now);
        s.selected = Some(C.into());
        let vm = s.view_model(&reg, &env, now);

        let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
        let mut ts = TableState::default();
        term.draw(|f| draw(f, &s, &vm, &mut ts)).unwrap();
        let text = buffer_text(&term);

        // STATE column flags the verify class distinctly (never a clean "stray").
        assert!(text.contains("stray?"), "verify-class stray must flag stray?:\n{text}");
        // Detail RECON row explains why, and names the unregistered-key class.
        assert!(text.contains("RECON"), "RECON row missing:\n{text}");
        assert!(text.contains("unregistered"), "recon label missing:\n{text}");

        // Class discrimination: unregistered-key = verify; untracked/residue =
        // cut candidate; empty = not a stray.
        assert!(matches!(recon_reason_display("unregistered-key"), Some((_, true, _))));
        assert!(matches!(recon_reason_display("untracked"), Some((_, false, _))));
        assert!(matches!(recon_reason_display("no-declared-member"), Some((_, false, _))));
        assert!(recon_reason_display("").is_none());
    }

    #[test]
    fn roster_viewport_follows_selection_past_the_fold() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let now = Instant::now();
        let reg = AliasRegistry::empty(); // names come from display_name
        let env = EnvelopeRegistry::empty();
        let mut s = WhoState::new(many_declared(40), None, WhoSort::Name, true);
        s.sorted_cache = s.sorted_anchors(&reg, now);
        // Select the LAST identity (Org39) — far past one screenful.
        s.selected = s.sorted_cache.last().cloned();
        let vm = s.view_model(&reg, &env, now);

        let mut term = Terminal::new(TestBackend::new(80, 40)).unwrap();
        let mut ts = TableState::default();
        term.draw(|f| draw(f, &s, &vm, &mut ts)).unwrap();
        let text = buffer_text(&term);

        // The selected row scrolled into view; the top of the list scrolled off.
        assert!(text.contains("Org39"), "selected row must be visible after scroll:\n{text}");
        assert!(!text.contains("Org00"), "top of list must scroll off when selection is at the end");
        // The roster NAME column shows the display name, not the raw UUID.
        // (The detail pane legitimately shows the selected anchor, so we check
        // the roster line specifically, not the whole buffer.)
        assert!(
            text.lines().any(|l| l.contains("Org39") && !l.contains("0000000000")),
            "the Org39 roster row must show the name, not its UUID"
        );
    }

    #[test]
    fn detail_pane_renders_all_four_domains() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use panoscope_core::portal::{AccessEntry, AccessWho};
        let now = Instant::now();
        let reg = reg();
        let env = EnvelopeRegistry::empty();
        let mut s = WhoState::new(declared(), None, WhoSort::Name, true);
        s.sorted_cache = s.sorted_anchors(&reg, now);
        s.selected = Some(A.to_string());
        // Bulk roster seeds the grid's status/KIND (the IDENTITY row's badge).
        s.apply_roster(vec![AccessRosterEntry {
            id: A.into(),
            kind: "system".into(),
            state: "active".into(),
            writes: 1,
            reads: 1,
            ..Default::default()
        }]);
        // A Ready access with a Zenoh-ACL write and a release-layer write, so
        // the DATA + RELEASE domains both have content to group.
        let who = AccessWho {
            kind: "system".into(),
            state: "active".into(),
            on_mesh: true,
            vendor: "Acme".into(),
            writes: vec![
                AccessEntry { topic: format!("{A}/track/v1"), layer: "zenoh-acl".into(), reason: "slot anchor".into() },
                AccessEntry { topic: "release/coalition/**".into(), layer: "release".into(), reason: "releasable".into() },
            ],
            reads: vec![AccessEntry { topic: "**".into(), layer: "zenoh-acl".into(), reason: "community-read".into() }],
            ..Default::default()
        };
        s.apply_access(A.to_string(), AccessOutcome::Ok(Box::new(who)));
        let vm = s.view_model(&reg, &env, now);

        let mut term = Terminal::new(TestBackend::new(90, 40)).unwrap();
        let mut ts = TableState::default();
        term.draw(|f| draw(f, &s, &vm, &mut ts)).unwrap();
        let text = buffer_text(&term);

        for domain in ["IDENTITY", "MESH", "DATA", "RELEASE"] {
            assert!(text.contains(domain), "detail pane must show the {domain} domain:\n{text}");
        }
        assert!(text.contains("active"), "identity state must render");
    }

    #[test]
    fn custody_machines_render_in_detail_pane() {
        use ratatui::backend::TestBackend;
        use ratatui::widgets::TableState;
        use ratatui::Terminal;
        use panoscope_core::portal::MachineListItem;
        let now = Instant::now();
        let reg = reg();
        let env = EnvelopeRegistry::empty();
        let mut s = WhoState::new(declared(), None, WhoSort::Name, true);
        s.sorted_cache = s.sorted_anchors(&reg, now);
        s.selected = Some(A.to_string());

        // A custodies two machines — a plain SENSOR and a PLATFORM decomposed into
        // sensor+effector workloads (ADR 1096/1098). B custodies nothing.
        s.apply_machines(vec![
            MachineListItem {
                slot_id: "m1".into(),
                system_handle: "a-sensor-1".into(),
                custodian: A.into(),
                node_class: "SENSOR".into(),
                autonomy: "manual".into(),
                affiliation: "unit".into(),
                workloads: vec![],
                enrolled_at: 0,
            },
            MachineListItem {
                slot_id: "m2".into(),
                system_handle: "a-uas-1".into(),
                custodian: A.into(),
                node_class: "PLATFORM".into(),
                autonomy: "supervised".into(),
                affiliation: "unit".into(),
                workloads: vec!["SENSOR".into(), "EFFECTOR".into()],
                enrolled_at: 0,
            },
        ]);

        let vm = s.view_model(&reg, &env, now);
        let mut term = Terminal::new(TestBackend::new(90, 40)).unwrap();
        let mut ts = TableState::default();
        term.draw(|f| draw(f, &s, &vm, &mut ts)).unwrap();
        let text = buffer_text(&term);

        assert!(text.contains("CUSTODY"), "detail pane must show the CUSTODY section:\n{text}");
        assert!(text.contains("a-sensor-1"), "custodied machine handle must render:\n{text}");
        assert!(text.contains("a-uas-1"), "platform handle must render:\n{text}");
        assert!(text.contains("SENSOR,EFFECTOR"), "platform workloads must render:\n{text}");
    }

    #[test]
    fn live_subscriptions_render_distinct_from_read_grants() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use panoscope_core::census::{Census, ResourceRole};
        use panoscope_core::portal::{AccessEntry, AccessWho};
        let now = Instant::now();
        let reg = reg();
        let env = EnvelopeRegistry::empty();
        let mut s = WhoState::new(declared(), None, WhoSort::Name, true);

        // A has an ACL read GRANT (may-read) on `**` — the existing grant layer.
        s.apply_access(
            A.to_string(),
            AccessOutcome::Ok(Box::new(AccessWho {
                kind: "system".into(),
                state: "active".into(),
                on_mesh: true,
                reads: vec![AccessEntry {
                    topic: "**".into(),
                    layer: "zenoh-acl".into(),
                    reason: "community-read".into(),
                }],
                ..Default::default()
            })),
        );

        // Census: A's ZID publishes under its anchor (the only ZID->anchor
        // bridge), and that ZID is live-subscribed to a DATA topic — the observed
        // is-reading layer, orthogonal to the ACL grant above.
        let mut c = Census::default();
        let zid = "e3998a3d8a10e1943ba585c6b2eae979";
        c.insert(ResourceRole::Publisher, format!("{A}/track/v1"), zid.into());
        c.insert(ResourceRole::Subscriber, "live/telemetry/**".into(), zid.into());
        s.apply_census(c);

        // State level: the anchor's observed live subs == the census subscription.
        assert_eq!(s.live_subs_of(A), vec!["live/telemetry/**".to_string()]);

        s.sorted_cache = s.sorted_anchors(&reg, now);
        s.selected = Some(A.to_string());
        s.focused = true; // full-screen detail — no side-pane row clipping.
        let vm = s.view_model(&reg, &env, now);

        let mut term = Terminal::new(TestBackend::new(100, 44)).unwrap();
        let mut ts = TableState::default();
        term.draw(|f| draw(f, &s, &vm, &mut ts)).unwrap();
        let text = buffer_text(&term);

        // Both layers render, distinctly labeled: a GRANT is may-read (ACL); a
        // LIVE SUB is is-reading (census). An operator must tell them apart.
        assert!(text.contains("community-read"), "ACL read GRANT (may-read) must still render:\n{text}");
        assert!(text.contains("LIVE SUBS"), "observed live-subscription block must render:\n{text}");
        assert!(text.contains("is-reading"), "live subs must be labeled is-reading, distinct from the may-read grant:\n{text}");
        assert!(text.contains("live/telemetry"), "the observed subscribed key-expr must render:\n{text}");
    }

    #[test]
    fn empty_census_surfaces_no_live_subscriptions() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use panoscope_core::portal::{AccessEntry, AccessWho};
        let now = Instant::now();
        let reg = reg();
        let env = EnvelopeRegistry::empty();
        let mut s = WhoState::new(declared(), None, WhoSort::Name, true);

        // Same ACL read GRANT, but NO census applied — the census ACL grant is
        // deploy-pending, so query_census returns empty (the live state today).
        s.apply_access(
            A.to_string(),
            AccessOutcome::Ok(Box::new(AccessWho {
                kind: "system".into(),
                state: "active".into(),
                on_mesh: true,
                reads: vec![AccessEntry {
                    topic: "**".into(),
                    layer: "zenoh-acl".into(),
                    reason: "community-read".into(),
                }],
                ..Default::default()
            })),
        );

        // Honest-degrade at the state level: nothing observed, nothing fabricated.
        assert!(s.live_subs_of(A).is_empty(), "empty census must observe no live subs");

        s.sorted_cache = s.sorted_anchors(&reg, now);
        s.selected = Some(A.to_string());
        s.focused = true;
        let vm = s.view_model(&reg, &env, now);

        let mut term = Terminal::new(TestBackend::new(100, 44)).unwrap();
        let mut ts = TableState::default();
        term.draw(|f| draw(f, &s, &vm, &mut ts)).unwrap();
        let text = buffer_text(&term);

        // The read GRANT still renders; the LIVE-SUB block does NOT — an empty
        // census surfaces no rows and never invents an is-reading value.
        assert!(text.contains("community-read"), "the ACL read grant must still render:\n{text}");
        assert!(!text.contains("LIVE SUBS"), "empty census must not render a live-subscription block:\n{text}");
        assert!(!text.contains("is-reading"), "empty census must never fabricate an is-reading row:\n{text}");
    }

    #[test]
    fn loaded_detail_drops_redundant_rollup_line() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use panoscope_core::portal::{AccessEntry, AccessWho};
        let now = Instant::now();
        let reg = AliasRegistry::empty();
        let env = EnvelopeRegistry::empty();
        let mut s = WhoState::new(vec![], None, WhoSort::Name, true);
        // Bulk roster seeds the rollup (status + W/R + mesh) ...
        s.apply_roster(vec![AccessRosterEntry {
            id: A.into(),
            label: "acme-track".into(),
            kind: "system".into(),
            state: "active".into(),
            on_mesh: true,
            connected: true,
            writes: 1,
            reads: 1,
            ..Default::default()
        }]);
        // ... and the full per-layer detail then lands.
        s.apply_access(
            A.to_string(),
            AccessOutcome::Ok(Box::new(AccessWho {
                kind: "system".into(),
                state: "active".into(),
                on_mesh: true,
                vendor: A.into(), // org == anchor: must NOT print a redundant "org <uuid>"
                writes: vec![AccessEntry {
                    topic: format!("{A}/track/v1"),
                    layer: "zenoh-acl".into(),
                    reason: "slot anchor".into(),
                }],
                reads: vec![AccessEntry {
                    topic: "**".into(),
                    layer: "zenoh-acl".into(),
                    reason: "community-read".into(),
                }],
                ..Default::default()
            })),
        );
        s.sorted_cache = s.sorted_anchors(&reg, now);
        s.selected = Some(A.into());
        let vm = s.view_model(&reg, &env, now);

        let mut term = Terminal::new(TestBackend::new(90, 40)).unwrap();
        let mut ts = TableState::default();
        term.draw(|f| draw(f, &s, &vm, &mut ts)).unwrap();
        let text = buffer_text(&term);

        // Composed status badge rode up to the identity line.
        assert!(text.contains("online"), "status badge in identity line:\n{text}");
        // The pre-fetch rollup ("nW/mR grants") is gone once the full block loads.
        assert!(!text.contains("R grants"), "redundant rollup line must drop:\n{text}");
        // The four-domain block is the canonical detail.
        assert!(text.contains("IDENTITY") && text.contains("DATA"), "domains:\n{text}");
        // org == anchor is suppressed (anchor already shown once on line 1).
        assert!(!text.contains("org aaaa"), "redundant org==anchor must be suppressed:\n{text}");
    }

    #[test]
    fn detail_pane_slow_failure_is_amber_not_scary() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let now = Instant::now();
        let reg = reg();
        let env = EnvelopeRegistry::empty();
        let mut s = WhoState::new(declared(), None, WhoSort::Name, true);
        s.sorted_cache = s.sorted_anchors(&reg, now);
        s.selected = Some(A.to_string());
        s.apply_access(A.to_string(), AccessOutcome::Err(AccessErr::Unreachable("timeout".into())));
        let vm = s.view_model(&reg, &env, now);

        let mut term = Terminal::new(TestBackend::new(90, 40)).unwrap();
        let mut ts = TableState::default();
        term.draw(|f| draw(f, &s, &vm, &mut ts)).unwrap();
        let text = buffer_text(&term);
        // The honest slow-mesh message, not the old "network or parse error".
        assert!(text.contains("press r to retry"), "slow failure must offer retry:\n{text}");
        assert!(!text.contains("parse error"), "must not show the scary 0.4.7 string");
    }
}
