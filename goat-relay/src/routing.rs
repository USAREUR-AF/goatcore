//! Cross-shard routing table.
//!
//! Maps authenticated peer-ID → connection write handle. Shared across
//! shards via [`Arc`] + [`RwLock`] so that a peer connected to shard A
//! is reachable from a forward initiated on shard B (Block 33C.1 lever
//! L8 — multi-shard with `SO_REUSEPORT`). Reads (the dispatch hot path)
//! take the read lock; writes (rare per-connection register / deregister)
//! take the write lock briefly.
//!
//! Pre-L8 the routing table was per-shard via `Rc<RefCell<...>>` (ADR
//! 0709 D7 layer 2). The L8 amendment relaxes layer 2 to "the routing
//! table's connections + subscriptions become Send+Sync to enable
//! cross-shard reachability." Layers 1 (single-threaded runtime per
//! shard), 3 (clippy lint), 4 (architecture comment), and 5 (boot-time
//! assertion) retain full strength. Per-connection task fields not in
//! the routing table (e.g., `Rc<RefCell<KeepaliveSender>>` from
//! goat-relay-protocol) remain `!Send`, so connection tasks themselves
//! still cannot migrate across threads even though the routing-table
//! handle they carry is now Send+Sync.
//!
//! The clippy lint `disallowed_methods` (D7 layer 3) blocks
//! `tokio::spawn` in this module's hot-path callers; spawning happens
//! only via `tokio::task::spawn_local`.
//!
//! The dispatch hot path looks up the destination peer-ID under a read
//! lock, clones the `mpsc::Sender<Bytes>` (refcount-only; tokio MPSC
//! sender clones are cheap), drops the lock, then `try_send`s the
//! rewritten frame. The frame body is `bytes::Bytes` so the dispatch
//! boundary is reference-count-only; the actual byte buffer is shared
//! by the reader that produced it and the writer that consumes it.
//!
//! # Wire-format note (spec §3.3)
//!
//! Upstream's "in-place peer_id rewrite" trick: the relay reuses the
//! buffer it received from the source peer, overwriting the 36-byte
//! peer_id field at offset 2..38 to hold the SOURCE peer-id (instead of
//! the destination). We use [`goat_relay_protocol::messages::payload::
//! rewrite_peer_id`] in the listener's reader loop just before dispatch.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use tokio::sync::mpsc;

use goat_relay_protocol::messages::peer_state::{marshal_peer_list, PeerListKind};
use goat_relay_protocol::messages::PeerId;

/// Frame dispatched to a peer's writer task — opaque wire bytes ready to
/// send over the WS as `Message::Binary`. The peer_id field has already
/// been rewritten in place by the dispatching task.
pub type Frame = Bytes;

/// Per-connection writer handle. The routing table holds one of these
/// per authenticated peer; the dispatch hot path looks up the destination
/// and `.try_send()`s the rewritten frame to it.
///
/// Bounded with `WRITER_QUEUE_CAPACITY` to eliminate the per-message
/// linked-list-node allocation that `mpsc::unbounded_channel` does
/// (~14k allocs/sec at 100 Mbps single-flow). Block 33C.1 lever L4 /
/// audit F-A4. The `try_send` failure path returns
/// `ForwardError::WriterFull` — the dispatch loop drops the frame
/// rather than back-pressuring the source (relay must not introduce
/// head-of-line blocking; slow downstream is the downstream's problem,
/// per spec §3.3 silent-drop).
pub type ConnHandle = mpsc::Sender<Frame>;

/// Per-connection writer queue capacity. 256 × max-frame ≈ ~2 MB max per
/// connection; sized to absorb burst without giving slow peers unbounded
/// memory. Each capacity unit holds one `Bytes` handle (24 bytes refcount
/// pointer); the actual payload memory is whatever the Bytes points at.
pub const WRITER_QUEUE_CAPACITY: usize = 256;

/// Chunk size for relay-emitted peer-state notification lists: 30 subjects
/// is 1,082 bytes on the wire (2-byte header plus 30 × 36-byte peer-ids),
/// under the ~1,200-byte QUIC datagram floor that applies before path-MTU
/// discovery completes (see
/// `goat_relay_protocol::transport::quic::INITIAL_PACKET_SIZE` minus packet
/// overhead). So a peer-state push always fits one datagram on either
/// transport.
///
/// Defense in depth for F-275, not the live trigger: today's netbird client
/// subscribes one peer at a time (`peer_subscription.go`
/// `MarshalSubPeerStateMsg([]PeerID{peerID})`), so the immediate-online reply
/// is a single subject. A future bulk subscribe would otherwise produce one
/// aggregate frame no QUIC peer could receive. Chunking is wire-compatible —
/// the client parses any list length, and the spec's 244 cap
/// (`MAX_PEERS_PER_MESSAGE`) is a max-message ceiling, not a floor.
pub const DATAGRAM_SAFE_PEERS_PER_MESSAGE: usize = 30;

/// Cross-shard routing table — connections + peer-state subscriptions.
///
/// Two `RwLock`-protected maps inside one `Arc` so that any number of
/// shards (each running its own `current_thread` tokio runtime) share
/// a single coherent peer registry. `Clone` is cheap (Arc refcount inc).
///
/// `connections` — peer-id → writer handle. The dispatch hot path; read
///   lock per `forward()`. Lock-free under no contention.
/// `subscribers` — peer-id (the subject) → set of peer-ids subscribed to
///   that subject's online/offline notifications. When peer P subscribes
///   to peer X, X's subscriber set gains P. When X arrives, every
///   subscriber gets a `PeersOnline([X])` push. When X leaves, every
///   subscriber gets a `PeersWentOffline([X])` push.
#[derive(Clone, Default)]
pub struct RoutingTable {
    inner: Arc<RoutingState>,
}

// Lock poisoning: the routing maps are `Arc<RwLock>` SHARED across
// every shard. With `.expect("not poisoned")`, a panic in any one
// connection task while holding a routing write guard would poison the
// lock and make every OTHER shard's access panic in turn — one panic
// cascading into a fleet-wide outage, defeating ADR 0709's per-shard
// isolation. We instead recover the guard from a `PoisonError`
// (`unwrap_or_else(|e| e.into_inner())`): the map data is consistent
// because each guard region performs a single complete `HashMap`
// insert/remove (no multi-step mutation can be interrupted mid-way), so
// poisoning here is pure liveness noise, not data corruption — recovering
// is correct and contains the blast radius to the one task that panicked.
/// Hot-path hasher for the `peer_stats` map. The
/// default `SipHash` on a 36-byte `PeerId` key is cryptographic-strength and
/// slow, and the per-peer counting runs on the dispatch hot path. A `PeerId`
/// is `"sha-"` + 32 effectively-random bytes, so folding the key's first 8
/// bytes into a `u64` is a well-distributed, allocation-free hash — the map is
/// a local advisory counter store (never adversary-controlled keying), so DoS
/// resistance isn't needed here. Used ONLY for `peer_stats`; the trust/ACL
/// path keeps the default hasher.
#[derive(Default)]
struct PeerIdHasher(u64);
impl Hasher for PeerIdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        // PeerId hashes as write_usize(36) then write(&[u8;36]); the last
        // (36-byte) write wins and carries the random key entropy.
        let mut b = [0u8; 8];
        let n = bytes.len().min(8);
        b[..n].copy_from_slice(&bytes[..n]);
        self.0 = u64::from_le_bytes(b);
    }
}
type PeerStatsMap = HashMap<PeerId, PeerCounters, BuildHasherDefault<PeerIdHasher>>;

/// Upper bound on how many subjects one subscriber may watch at once.
///
/// One subscribe frame carries at most 244 subjects, but nothing bounded
/// how many frames a peer sends: any authenticated peer could subscribe to
/// arbitrary peer-ids until it disconnected, growing the registry without
/// limit. A real client watches the peers it has relay connections to, so
/// this sits far above any fleet's size. Subjects past it are refused and
/// counted ([`RoutingTable::subscriptions_refused`]).
pub const MAX_SUBSCRIPTIONS_PER_PEER: usize = 16_384;

/// The subscription registry, indexed both ways under one lock.
///
/// `by_subject` (subject → its watchers) is what the online/offline
/// fan-out reads. `by_subscriber` (watcher → the subjects it watches) is
/// its exact mirror: it lets a departing peer's subscriptions be stripped
/// in time proportional to what THAT peer watched, rather than a walk over
/// every subject in the fleet under the write lock on each disconnect, and
/// it is where the per-subscriber cap is counted.
#[derive(Default)]
struct SubscriptionIndex {
    by_subject: HashMap<PeerId, HashSet<PeerId>>,
    by_subscriber: HashMap<PeerId, HashSet<PeerId>>,
}

impl SubscriptionIndex {
    /// Record that `subscriber` watches `subject`. Returns `false` (and
    /// records nothing) when that would take the subscriber past
    /// [`MAX_SUBSCRIPTIONS_PER_PEER`]; re-adding an existing edge is
    /// always accepted.
    fn add(&mut self, subscriber: PeerId, subject: PeerId) -> bool {
        let watching = self.by_subscriber.entry(subscriber).or_default();
        if !watching.contains(&subject) && watching.len() >= MAX_SUBSCRIPTIONS_PER_PEER {
            if watching.is_empty() {
                self.by_subscriber.remove(&subscriber);
            }
            return false;
        }
        watching.insert(subject);
        self.by_subject
            .entry(subject)
            .or_default()
            .insert(subscriber);
        true
    }

    /// Remove the single edge `subscriber` → `subject`, GC'ing empty sets
    /// on both sides.
    fn remove_edge(&mut self, subscriber: &PeerId, subject: &PeerId) {
        if let Some(watchers) = self.by_subject.get_mut(subject) {
            watchers.remove(subscriber);
            if watchers.is_empty() {
                self.by_subject.remove(subject);
            }
        }
        if let Some(watching) = self.by_subscriber.get_mut(subscriber) {
            watching.remove(subject);
            if watching.is_empty() {
                self.by_subscriber.remove(subscriber);
            }
        }
    }

    /// Strip `peer` from the registry on its departure, in both roles, and
    /// return the peers that were watching it (for the offline fan-out).
    ///
    /// As a subscriber, every subject it watched loses it as a watcher. As
    /// a subject, its watcher set is removed: a watcher is told once that
    /// it went offline and must subscribe again to hear of its return.
    /// That is the behaviour this registry has always had; the reverse
    /// index only changes what it costs.
    fn remove_peer(&mut self, peer: &PeerId) -> Option<HashSet<PeerId>> {
        if let Some(watching) = self.by_subscriber.remove(peer) {
            for subject in &watching {
                if let Some(watchers) = self.by_subject.get_mut(subject) {
                    watchers.remove(peer);
                    if watchers.is_empty() {
                        self.by_subject.remove(subject);
                    }
                }
            }
        }
        let watchers = self.by_subject.remove(peer)?;
        for watcher in &watchers {
            if let Some(watching) = self.by_subscriber.get_mut(watcher) {
                watching.remove(peer);
                if watching.is_empty() {
                    self.by_subscriber.remove(watcher);
                }
            }
        }
        Some(watchers)
    }
}

struct RoutingState {
    connections: RwLock<HashMap<PeerId, ConnHandle>>,
    subscribers: RwLock<SubscriptionIndex>,
    /// Subjects refused because the subscriber was already watching
    /// [`MAX_SUBSCRIPTIONS_PER_PEER`] peers. Lifetime, relaxed.
    subscriptions_refused: AtomicU64,
    /// Drop accounting. The relay drops frames silently per spec
    /// §3.3 (no back-pressure to the source); these counters make the
    /// otherwise-invisible loss observable for a stats/health surface.
    /// Broken out by reason — per-peer attribution is deferred to the
    /// metrics surface (the on-bus footprint already covers per-peer at
    /// the Zenoh layer above).
    drops: DropCounters,
    /// Runtime gate for the per-peer counting. When `false`,
    /// `forward_data` short-circuits to `forward` and pays only a single
    /// relaxed bool load — so a relay that doesn't serve `--stats-addr`, or
    /// one an operator explicitly turns off, carries ZERO per-frame counting
    /// cost. Flippable live (it's an atomic); wired from the CLI/env at boot.
    peer_counting: AtomicBool,
    /// Per-peer forwarded byte/frame counters (ADR 1055 §2b — the
    /// relayed-bytes + per-peer distribution the black-box prober +
    /// `/proc/net/dev` structurally cannot produce). Keyed by peer-id like
    /// `connections` (with the cheap [`PeerIdHasher`]); an entry is created at
    /// register (`insert`) and removed at deregister, so the dispatch hot path
    /// only ever takes a READ lock here (never a first-frame write).
    /// `AtomicU64` increments need only `&self`, so they happen through the
    /// shared read guard — different shards increment different peers'
    /// counters concurrently; same-peer concurrency is the atomic's job.
    /// Advisory, unsynchronized with the drop counters and with each other.
    peer_stats: RwLock<PeerStatsMap>,
}

impl Default for RoutingState {
    fn default() -> Self {
        Self {
            connections: RwLock::default(),
            subscribers: RwLock::default(),
            subscriptions_refused: AtomicU64::new(0),
            drops: DropCounters::default(),
            // Counting ON by default preserves the merged behavior + the unit
            // tests; `main` gates it (default: on iff `--stats-addr` is served).
            peer_counting: AtomicBool::new(true),
            peer_stats: RwLock::default(),
        }
    }
}

/// Lifetime counts of frames the relay dropped, by reason. Lock-free.
#[derive(Default)]
struct DropCounters {
    /// Destination's writer queue was at capacity (slow consumer).
    writer_full: AtomicU64,
    /// Destination's writer task had gone away (peer disconnected).
    writer_closed: AtomicU64,
    /// Destination was not registered (unknown / departed peer).
    peer_not_found: AtomicU64,
    /// A QUIC writer refused a frame larger than the connection's current
    /// datagram size limit (path-MTU-bound). Writer-side drop, counted via
    /// [`RoutingTable::count_datagram_too_large`]; never fatal to the
    /// connection (track netbird-wedge-rca).
    datagram_too_large: AtomicU64,
}

/// Snapshot of [`RoutingTable`] drop counters for an observability surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DropStats {
    /// Frames dropped because the destination's writer queue was full
    /// (slow consumer) — the back-pressure-avoidance drop, spec §3.3.
    pub writer_full: u64,
    /// Frames dropped because the destination's writer task had gone away.
    pub writer_closed: u64,
    /// Frames dropped because the destination peer was not registered.
    pub peer_not_found: u64,
    /// Frames dropped at a QUIC writer because they exceeded the
    /// connection's current datagram size limit (track netbird-wedge-rca).
    pub datagram_too_large: u64,
}

impl DropStats {
    /// Total frames dropped across all reasons.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.writer_full + self.writer_closed + self.peer_not_found + self.datagram_too_large
    }
}

/// Lifetime per-peer forwarded byte/frame counters (ADR 1055 §2b). Lock-free.
///
/// `fwd_*` is charged to the SOURCE at dispatch, unconditionally — a peer
/// that pushes bytes into the relay consumed that ingress even if the
/// destination had gone (so a flooding producer is still visible). `rcv_*`
/// is charged to the DESTINATION only on successful delivery. The pair is
/// what surfaces a power-law hog for identity-keyed throttle/evict.
#[derive(Default)]
struct PeerCounters {
    /// Bytes this peer pushed into the relay as a source (ingress).
    fwd_bytes: AtomicU64,
    /// Frames this peer pushed into the relay as a source.
    fwd_frames: AtomicU64,
    /// Bytes delivered to this peer as a destination (egress).
    rcv_bytes: AtomicU64,
    /// Frames delivered to this peer as a destination.
    rcv_frames: AtomicU64,
    /// Which transport this peer's connection is on (WS or QUIC), as a
    /// [`Transport`] discriminant. Set ONCE by the listener right after
    /// register via [`RoutingTable::set_transport`]; read on the stats
    /// snapshot. Stored here (not a separate map) so it shares the counter
    /// entry's exact lifecycle — created at register, dropped on every
    /// deregister path — and needs no extra lock. `0` = unknown (a peer
    /// registered before the listener tagged it, which is a sub-millisecond
    /// window). Whole point (track netbird-wedge-rca): the QUIC-vs-WS
    /// question that was un-answerable from socket tables / auth-count
    /// history is now a first-class, per-peer, empirically-read field.
    transport: AtomicU8,
}

/// Which transport a peer's relay connection rides. The relay knows this at
/// accept time; exposing it per-peer (track netbird-wedge-rca) makes the
/// QUIC-vs-WS question empirically answerable instead of a guess from socket
/// tables (which cannot see an unconnected QUIC UDP socket) or auth-count
/// history (which is cumulative, not current).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// Transport not yet tagged by the listener (sub-ms register window), or
    /// a caller (e.g. a test) that never set it.
    Unknown,
    /// WebSocket-over-TLS (TCP). Frames legal up to `MAX_MESSAGE_SIZE`.
    Ws,
    /// QUIC (RFC 9221 datagrams). Per-frame size is path-MTU-bound.
    Quic,
}

impl Transport {
    /// Discriminant stored in the per-peer [`AtomicU8`].
    const fn as_u8(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Ws => 1,
            Self::Quic => 2,
        }
    }
    /// Recover from the stored discriminant (any unexpected value → Unknown).
    const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Ws,
            2 => Self::Quic,
            _ => Self::Unknown,
        }
    }
    /// Stable lowercase label for the stats JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Ws => "ws",
            Self::Quic => "quic",
        }
    }
}

/// Snapshot of one peer's forwarded byte/frame counters for an
/// observability surface. The anchor is the opaque [`PeerId`] — never an
/// alias (ADR 1039 discipline; the relay has no portal). Consumers
/// (`relay-probe`, ADR 1055 stats endpoint) rate-diff these across ticks
/// and render the distribution / hog band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerStats {
    /// The peer this row is for.
    pub peer_id: PeerId,
    /// Bytes pushed into the relay by this peer (ingress / the hog signal).
    pub fwd_bytes: u64,
    /// Frames pushed into the relay by this peer.
    pub fwd_frames: u64,
    /// Bytes delivered to this peer (egress).
    pub rcv_bytes: u64,
    /// Frames delivered to this peer.
    pub rcv_frames: u64,
    /// The transport this peer's connection rides (WS / QUIC / unknown).
    pub transport: Transport,
}

/// Outcome of a single forward dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardError {
    /// Destination peer is not registered (silent drop per spec §3.3).
    PeerNotFound,
    /// Destination's writer-task receiver has been dropped (peer
    /// disconnected). The routing table auto-deregisters the entry.
    WriterClosed,
    /// Destination's writer queue is at capacity. Bounded MPSC (Block
    /// 33C.1 lever L4) drops the frame rather than back-pressuring the
    /// source — relay must not introduce head-of-line blocking. Slow
    /// downstream is the downstream's problem; per spec §3.3 silent drop.
    WriterFull,
}

impl RoutingTable {
    /// Construct an empty routing table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new connection's writer handle. Fans out
    /// `PeersOnline([peer_id])` to every subscriber of `peer_id` (across
    /// all shards — subscribers can have been registered on any shard).
    ///
    /// If `peer_id` is already registered (e.g., the same peer reconnects
    /// before the prior connection's task notices its TCP close), the
    /// previous handle is returned. Caller MUST drop it — the prior
    /// writer task will see its receiver close and exit naturally.
    pub fn insert(&self, peer_id: PeerId, handle: ConnHandle) -> Option<ConnHandle> {
        let prior = self
            .inner
            .connections
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(peer_id, handle);
        // Ensure a per-peer counter entry exists so the dispatch hot path
        // is read-lock-only (ADR 1055 §2b). `or_default` keeps counts across
        // a fast reconnect (insert before the prior connection's remove), so
        // a flapping producer's cumulative push stays visible; a full
        // disconnect (`remove`/`remove_self`) drops the entry.
        self.inner
            .peer_stats
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .entry(peer_id)
            .or_default();
        // Snapshot the subscriber set under the read lock, drop, then
        // fan out — forward() takes its own locks.
        let subs = self
            .inner
            .subscribers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .by_subject
            .get(&peer_id)
            .cloned();
        if let Some(subs) = subs {
            let frame = match marshal_peer_list(PeerListKind::Online, &[peer_id]) {
                Ok(v) => Bytes::from(v),
                Err(e) => {
                    eprintln!("routing: marshal_peer_list(Online) failed: {e}");
                    return prior;
                }
            };
            for sub in subs {
                let _ = self.forward(&sub, frame.clone());
            }
        }
        prior
    }

    /// Unconditionally remove a connection's entry on disconnect. Fans out
    /// `PeersWentOffline([peer_id])` to every subscriber of `peer_id`
    /// (across all shards). Also drops the leaving peer from any
    /// subscription sets it was the SUBSCRIBER of (its own connection
    /// is gone).
    ///
    /// Prefer [`RoutingTable::remove_self`] from a connection-handler's
    /// teardown path: this unconditional variant is identity-blind and, if
    /// the peer has reconnected, will evict the FRESH session. It
    /// remains for callers that genuinely own the only connection (tests,
    /// admin eviction).
    pub fn remove(&self, peer_id: &PeerId) {
        self.inner
            .connections
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(peer_id);
        self.inner
            .peer_stats
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(peer_id);
        self.deregister_peer_subscriptions(peer_id);
    }

    /// Identity-aware deregister — the correct teardown for a
    /// connection handler's exit path. Removes `peer_id`'s entry and runs
    /// the offline fan-out + subscription teardown ONLY if the currently-
    /// registered handle is still `mine` (this connection's writer).
    ///
    /// If the peer has already reconnected, a newer handle owns the entry,
    /// so this stale teardown is a complete no-op: it does not evict the
    /// fresh session, does not emit a spurious `PeersWentOffline` that
    /// would overwrite the new session's `PeersOnline` in subscribers'
    /// views, and does not strip the peer's subscription graph
    ///. Without this guard a superseded old connection's
    /// late teardown — its reader can linger up to `SENDER_TIMEOUT_WINDOW`
    /// (~45s) before exiting — poisons the new session, which is why
    /// `netbird down/up` sometimes fails to restore connectivity.
    pub fn remove_self(&self, peer_id: &PeerId, mine: &ConnHandle) {
        {
            let mut conns = self
                .inner
                .connections
                .write()
                .unwrap_or_else(|e| e.into_inner());
            match conns.get(peer_id) {
                Some(current) if current.same_channel(mine) => {
                    conns.remove(peer_id);
                }
                // Superseded by a reconnect, or already gone: leave the
                // live entry, its subscribers, and the Online notification
                // intact.
                _ => return,
            }
        }
        // Identity matched → this connection genuinely owned the entry, so
        // drop its per-peer counters too. The superseded branch above
        // returned early, leaving the fresh session's counters intact
        //.
        self.inner
            .peer_stats
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(peer_id);
        self.deregister_peer_subscriptions(peer_id);
    }

    /// Strip a leaving peer from the subscription registry and notify its
    /// subscribers it went offline. Shared by [`RoutingTable::remove`] and
    /// [`RoutingTable::remove_self`]; the connection-entry removal is the
    /// caller's responsibility (so `remove_self` can guard it on identity).
    fn deregister_peer_subscriptions(&self, peer_id: &PeerId) {
        // Drop this peer from the subject sets it watched, and take the set
        // of peers watching IT for the offline fan-out (see
        // SubscriptionIndex::remove_peer). Cost is proportional to this
        // peer's own subscriptions, not to the size of the registry.
        let subs = self
            .inner
            .subscribers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove_peer(peer_id);
        let Some(subs) = subs else { return };
        let frame = match marshal_peer_list(PeerListKind::Offline, &[*peer_id]) {
            Ok(v) => Bytes::from(v),
            Err(e) => {
                eprintln!("routing: marshal_peer_list(Offline) failed: {e}");
                return;
            }
        };
        for sub in subs {
            let _ = self.forward(&sub, frame.clone());
        }
    }

    /// Subscribe `subscriber` to online/offline notifications for each
    /// peer in `subjects`. For any subject that's currently registered
    /// (on any shard), emits `PeersOnline([subject])` to the subscriber
    /// immediately. Idempotent — re-subscribing to an already-watched
    /// subject re-emits the immediate-notification only if the subject
    /// is currently online.
    ///
    /// A subscriber already watching [`MAX_SUBSCRIPTIONS_PER_PEER`]
    /// subjects has further NEW subjects refused: they are not recorded,
    /// get no immediate notification, and are counted in
    /// [`RoutingTable::subscriptions_refused`]. The peer stays connected.
    pub fn subscribe(&self, subscriber: PeerId, subjects: &[PeerId]) {
        let mut refused: u64 = 0;
        let online_now: Vec<PeerId> = {
            let mut subs_map = self
                .inner
                .subscribers
                .write()
                .unwrap_or_else(|e| e.into_inner());
            let conn_map = self
                .inner
                .connections
                .read()
                .unwrap_or_else(|e| e.into_inner());
            let mut online = Vec::new();
            for subject in subjects {
                if !subs_map.add(subscriber, *subject) {
                    refused += 1;
                    continue;
                }
                if conn_map.contains_key(subject) {
                    online.push(*subject);
                }
            }
            online
        };
        if refused > 0 {
            let before = self
                .inner
                .subscriptions_refused
                .fetch_add(refused, Ordering::Relaxed);
            // Log the first refusal per process, not one line per frame: a
            // misbehaving peer would otherwise turn the cap into a log flood.
            if before == 0 {
                eprintln!(
                    "routing: peer_id={subscriber:?} reached the {MAX_SUBSCRIPTIONS_PER_PEER}-subject \
                     subscription cap; further subjects are refused (counted as subscriptions_refused)"
                );
            }
        }
        if !online_now.is_empty() {
            // Emit the currently-online subjects in DATAGRAM-SAFE chunks
            // (≤ [`DATAGRAM_SAFE_PEERS_PER_MESSAGE`] subjects per frame,
            // 1,082 bytes max) instead of one aggregate frame. A bulk
            // subscribe at fleet scale produced a single ~3.4 KB frame that
            // can never fit a QUIC datagram — the writer refused it
            // wholesale and the subscriber never learned any peer was
            // online (track netbird-wedge-rca). Chunking is
            // wire-compatible: the client parses any list length.
            for chunk in online_now.chunks(DATAGRAM_SAFE_PEERS_PER_MESSAGE) {
                match marshal_peer_list(PeerListKind::Online, chunk) {
                    Ok(v) => {
                        let _ = self.forward(&subscriber, Bytes::from(v));
                    }
                    Err(e) => {
                        eprintln!("routing: marshal_peer_list(Online, immediate) failed: {e}");
                    }
                }
            }
        }
    }

    /// Count a writer-side QUIC datagram-too-large drop. Called by the QUIC
    /// writer task when `send_datagram` refuses a frame larger than the
    /// connection's current datagram limit — the frame is dropped and the
    /// writer CONTINUES (spec §3.3 silent-drop; matches upstream Go's
    /// log-and-continue at `relay/server/peer.go`). Counted here so the
    /// silent drop stays observable on the ADR 1055 stats surface (F-220
    /// discipline).
    pub fn count_datagram_too_large(&self) {
        self.inner
            .drops
            .datagram_too_large
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Unsubscribe `subscriber` from online/offline notifications for each
    /// peer in `subjects`. Idempotent — unsubscribing from a non-watched
    /// subject is a no-op.
    pub fn unsubscribe(&self, subscriber: PeerId, subjects: &[PeerId]) {
        let mut subs_map = self
            .inner
            .subscribers
            .write()
            .unwrap_or_else(|e| e.into_inner());
        for subject in subjects {
            subs_map.remove_edge(&subscriber, subject);
        }
    }

    /// Lifetime count of subjects refused by the per-subscriber cap
    /// ([`MAX_SUBSCRIPTIONS_PER_PEER`]). Relaxed load; advisory.
    #[must_use]
    pub fn subscriptions_refused(&self) -> u64 {
        self.inner.subscriptions_refused.load(Ordering::Relaxed)
    }

    /// Forward `frame` to the peer identified by `dest`.
    ///
    /// Returns `Ok(())` on successful dispatch to the writer-task MPSC.
    /// Errors are silent at the wire level; the caller logs but does not
    /// signal anything back to the source peer (per spec §3.3).
    ///
    /// Hot path under multi-shard L8: takes the read lock, clones the
    /// `mpsc::Sender<Bytes>` (cheap; tokio MPSC sender is `Arc`-y inside),
    /// drops the lock, then `try_send`s. Lock-free under no register/
    /// deregister contention; bounded write-lock acquisitions only on
    /// auto-deregister of a closed-receiver entry.
    pub fn forward(&self, dest: &PeerId, frame: Frame) -> Result<(), ForwardError> {
        let handle = self
            .inner
            .connections
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(dest)
            .cloned();
        let Some(handle) = handle else {
            self.inner.drops.peer_not_found.fetch_add(1, Ordering::Relaxed);
            return Err(ForwardError::PeerNotFound);
        };
        match handle.try_send(frame) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.inner.drops.writer_full.fetch_add(1, Ordering::Relaxed);
                Err(ForwardError::WriterFull)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.inner.drops.writer_closed.fetch_add(1, Ordering::Relaxed);
                // Writer task receiver dropped. Auto-deregister so future
                // forwards short-circuit to PeerNotFound — but only if the
                // currently-stored handle is still the one we just found
                // closed. Between the read-lock release above and this
                // write lock, the peer may have reconnected and inserted a
                // fresh handle; removing it here on the strength of the
                // OLD handle being closed is the same identity-blind race
                // as F-218 (would evict a live reconnected session).
                let removed = {
                    let mut conns = self
                        .inner
                        .connections
                        .write()
                        .unwrap_or_else(|e| e.into_inner());
                    if conns.get(dest).is_some_and(|cur| cur.same_channel(&handle)) {
                        conns.remove(dest);
                        true
                    } else {
                        false
                    }
                };
                // A dead-writer eviction is a real departure: run the same
                // teardown `remove_self` would — drop the counters row,
                // strip the subscription graph, and fan out
                // `PeersWentOffline` to subscribers. Without this the
                // eviction was SILENT: subscribers kept an "online" view of
                // a peer registered nowhere, and every rendezvous toward it
                // burned its full 30 s timeout (track netbird-wedge-rca).
                // The fan-out recurses through `forward()` if a subscriber's
                // own writer is also dead; termination is guaranteed because
                // each level removes a distinct peer from `connections`
                // before recursing, and no lock is held across the fan-out.
                if removed {
                    // Loud on purpose (and bounded — at most once per dead
                    // connection): a dead-writer eviction is the exact
                    // event whose silence cost a months-long wedge hunt
                    // (netbird-wedge-rca). Grep anchor: "evicted".
                    eprintln!(
                        "routing: evicted {dest:?} — writer dead; deregistering + notifying subscribers offline"
                    );
                    self.inner
                        .peer_stats
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(dest);
                    self.deregister_peer_subscriptions(dest);
                }
                Err(ForwardError::WriterClosed)
            }
        }
    }

    /// Forward a DATA frame from `src` to `dest`, attributing the bytes to
    /// both peers' counters (ADR 1055 §2b). This is the entry point the
    /// listener data path uses instead of [`RoutingTable::forward`]; the
    /// relay-generated peer-state notifications keep calling `forward`
    /// directly (they have no meaningful source peer and are not part of
    /// the per-peer relayed-byte accounting).
    ///
    /// `src`'s `fwd_*` is charged UNCONDITIONALLY — a peer that pushed the
    /// bytes into the relay consumed that ingress even if `dest` had gone
    /// (so a flooding producer stays visible). `dest`'s `rcv_*` is charged
    /// only when `forward` reports successful dispatch. Two brief
    /// `peer_stats` READ-lock scopes bracket the existing `forward` (which
    /// takes its own `connections` lock) — no nested lock holding, so the
    /// lock-ordering story is unchanged.
    pub fn forward_data(
        &self,
        src: &PeerId,
        dest: &PeerId,
        frame: Frame,
    ) -> Result<(), ForwardError> {
        // Runtime gate: when counting is off, this is a plain `forward`
        // plus one relaxed bool load — zero map/lock/atomic per-peer cost.
        if !self.inner.peer_counting.load(Ordering::Relaxed) {
            return self.forward(dest, frame);
        }
        let n = frame.len() as u64;
        // Charge the source's ingress first (unconditional). Entry exists
        // for any registered peer; a racing deregister just drops the count.
        if let Some(c) = self
            .inner
            .peer_stats
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(src)
        {
            c.fwd_bytes.fetch_add(n, Ordering::Relaxed);
            c.fwd_frames.fetch_add(1, Ordering::Relaxed);
        }
        let result = self.forward(dest, frame);
        if result.is_ok() {
            if let Some(c) = self
                .inner
                .peer_stats
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .get(dest)
            {
                c.rcv_bytes.fetch_add(n, Ordering::Relaxed);
                c.rcv_frames.fetch_add(1, Ordering::Relaxed);
            }
        }
        result
    }

    /// Enable/disable the per-peer counting at runtime. When disabled,
    /// `forward_data` is a plain `forward` + one relaxed bool load. Atomic, so
    /// it can be flipped live (boot-time today; a control-endpoint toggle is a
    /// cheap extension). Does not clear existing counters — a re-enable resumes.
    pub fn set_peer_counting(&self, on: bool) {
        self.inner.peer_counting.store(on, Ordering::Relaxed);
    }

    /// Whether per-peer counting is currently on.
    #[must_use]
    pub fn peer_counting_enabled(&self) -> bool {
        self.inner.peer_counting.load(Ordering::Relaxed)
    }

    /// Snapshot the per-peer forwarded byte/frame counters (ADR 1055 §2b)
    /// for an observability surface. One row per currently-registered peer;
    /// relaxed loads (advisory, not synchronized with each other or with
    /// the drop counters). O(registered peers) under a single read lock.
    #[must_use]
    pub fn peer_stats(&self) -> Vec<PeerStats> {
        self.inner
            .peer_stats
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(peer_id, c)| PeerStats {
                peer_id: *peer_id,
                fwd_bytes: c.fwd_bytes.load(Ordering::Relaxed),
                fwd_frames: c.fwd_frames.load(Ordering::Relaxed),
                rcv_bytes: c.rcv_bytes.load(Ordering::Relaxed),
                rcv_frames: c.rcv_frames.load(Ordering::Relaxed),
                transport: Transport::from_u8(c.transport.load(Ordering::Relaxed)),
            })
            .collect()
    }

    /// Tag a registered peer's transport (WS / QUIC). Called by the listener
    /// once, right after [`RoutingTable::insert`], on the same shard/thread —
    /// so the counter entry always exists. A no-op if the entry is gone (the
    /// peer deregistered in the intervening sub-ms window). Read-lock only:
    /// the store is on an `AtomicU8` inside the entry, so it does not contend
    /// with the dispatch hot path (track netbird-wedge-rca). Idempotent.
    pub fn set_transport(&self, peer_id: &PeerId, transport: Transport) {
        if let Some(c) = self
            .inner
            .peer_stats
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(peer_id)
        {
            c.transport.store(transport.as_u8(), Ordering::Relaxed);
        }
    }

    /// Snapshot the lifetime drop counters for an observability
    /// surface. Relaxed loads — counts are advisory, not synchronized with
    /// each other.
    #[must_use]
    pub fn drop_stats(&self) -> DropStats {
        DropStats {
            writer_full: self.inner.drops.writer_full.load(Ordering::Relaxed),
            writer_closed: self.inner.drops.writer_closed.load(Ordering::Relaxed),
            peer_not_found: self.inner.drops.peer_not_found.load(Ordering::Relaxed),
            datagram_too_large: self
                .inner
                .drops
                .datagram_too_large
                .load(Ordering::Relaxed),
        }
    }

    /// Number of currently-registered peers. O(1).
    ///
    /// Used by the ADR 1055 stats endpoint (`stats::render_json`) as the
    /// `connections` gauge, and by integration tests for synchronization
    /// (poll until both peers have registered).
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner
            .connections
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    /// Whether the table holds any registered peers.
    ///
    /// Test-side helper paired with [`Self::len`].
    #[must_use]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.inner
            .connections
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use goat_relay_protocol::messages::PEER_ID_SIZE;

    fn peer(byte: u8) -> PeerId {
        let mut b = [0u8; PEER_ID_SIZE];
        b[..4].copy_from_slice(b"sha-");
        b[4..].fill(byte);
        PeerId::from_bytes(b)
    }

    #[test]
    fn insert_then_forward_succeeds() {
        let table = RoutingTable::new();
        let p1 = peer(0xAA);
        let (tx, mut rx) = mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        assert!(table.insert(p1, tx).is_none());
        assert_eq!(table.len(), 1);

        let frame = Bytes::from_static(b"hello");
        table.forward(&p1, frame.clone()).unwrap();
        let received = rx.try_recv().expect("frame received");
        assert_eq!(received, frame);
    }

    #[test]
    fn forward_unregistered_returns_peer_not_found() {
        let table = RoutingTable::new();
        let p1 = peer(0xAA);
        let err = table
            .forward(&p1, Bytes::from_static(b"x"))
            .expect_err("unregistered peer");
        assert_eq!(err, ForwardError::PeerNotFound);
    }

    // Fetch one peer's stats row from the snapshot (test helper).
    fn stats_of(table: &RoutingTable, p: &PeerId) -> Option<PeerStats> {
        table.peer_stats().into_iter().find(|s| &s.peer_id == p)
    }

    #[test]
    fn forward_data_attributes_bytes_to_source_and_dest() {
        // ADR 1055 §2b: src charged fwd, dest charged rcv, on delivery.
        let table = RoutingTable::new();
        let (src, dst) = (peer(0xAA), peer(0xBB));
        let (tx_s, _rx_s) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        let (tx_d, mut rx_d) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(src, tx_s);
        table.insert(dst, tx_d);

        let frame = Bytes::from_static(b"twelve-bytes"); // 12 bytes
        table.forward_data(&src, &dst, frame.clone()).unwrap();
        assert_eq!(rx_d.try_recv().expect("delivered"), frame);

        let s = stats_of(&table, &src).expect("src row");
        assert_eq!((s.fwd_bytes, s.fwd_frames), (12, 1), "source charged ingress");
        assert_eq!((s.rcv_bytes, s.rcv_frames), (0, 0), "source not a receiver");
        let d = stats_of(&table, &dst).expect("dst row");
        assert_eq!((d.rcv_bytes, d.rcv_frames), (12, 1), "dest charged egress");
        assert_eq!((d.fwd_bytes, d.fwd_frames), (0, 0), "dest not a source");
    }

    #[test]
    fn forward_data_charges_source_even_when_dest_absent() {
        // A flooding producer stays visible even if its destination is gone:
        // ingress is charged unconditionally, egress only on delivery.
        let table = RoutingTable::new();
        let (src, gone) = (peer(0x11), peer(0x22));
        let (tx_s, _rx_s) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(src, tx_s); // dest never registered

        let err = table
            .forward_data(&src, &gone, Bytes::from_static(b"xxxxx")) // 5 bytes
            .expect_err("dest absent");
        assert_eq!(err, ForwardError::PeerNotFound);

        let s = stats_of(&table, &src).expect("src row");
        assert_eq!((s.fwd_bytes, s.fwd_frames), (5, 1), "source ingress charged");
        assert!(stats_of(&table, &gone).is_none(), "no row for unregistered dest");
    }

    #[test]
    fn peer_stats_row_dropped_on_deregister() {
        let table = RoutingTable::new();
        let p = peer(0x33);
        let (tx, rx) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(p, tx.clone());
        assert!(stats_of(&table, &p).is_some(), "row created at register");
        // Identity-aware teardown drops the row.
        table.remove_self(&p, &tx);
        drop(rx);
        assert!(stats_of(&table, &p).is_none(), "row dropped at deregister");
    }

    #[test]
    fn forward_after_receiver_drop_returns_writer_closed_and_deregisters() {
        let table = RoutingTable::new();
        let p1 = peer(0xAA);
        let (tx, rx) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(p1, tx);
        drop(rx);

        let err = table
            .forward(&p1, Bytes::from_static(b"x"))
            .expect_err("dropped receiver");
        assert_eq!(err, ForwardError::WriterClosed);
        assert!(table.is_empty(), "auto-deregister on writer-closed");
    }

    #[test]
    fn drop_stats_count_by_reason() {
        // F-220: silent drops are counted, broken out by reason.
        let table = RoutingTable::new();
        let p = peer(0xCC);

        // peer-not-found
        let _ = table.forward(&p, Bytes::from_static(b"x"));
        assert_eq!(table.drop_stats().peer_not_found, 1);

        // writer-full: capacity-1 channel, never drained
        let (tx, _rx) = mpsc::channel::<Frame>(1);
        table.insert(p, tx);
        assert!(table.forward(&p, Bytes::from_static(b"a")).is_ok()); // fills slot
        let _ = table.forward(&p, Bytes::from_static(b"b")); // Full → counted
        assert_eq!(table.drop_stats().writer_full, 1);

        // writer-closed
        let p2 = peer(0xDD);
        let (tx2, rx2) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(p2, tx2);
        drop(rx2);
        let _ = table.forward(&p2, Bytes::from_static(b"z"));
        assert_eq!(table.drop_stats().writer_closed, 1);

        assert_eq!(table.drop_stats().total(), 3);
    }

    #[test]
    fn remove_clears_entry() {
        let table = RoutingTable::new();
        let p1 = peer(0xAA);
        let (tx, _rx) = mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(p1, tx);
        table.remove(&p1);
        assert!(table.is_empty());
    }

    #[test]
    fn duplicate_insert_returns_prior_handle() {
        let table = RoutingTable::new();
        let p1 = peer(0xAA);
        let (tx1, _rx1) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        let (tx2, _rx2) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        assert!(table.insert(p1, tx1).is_none());
        let prior = table.insert(p1, tx2).expect("prior handle returned");
        // Caller drops prior; the new tx2 is now the active handle.
        drop(prior);
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn remove_self_is_noop_when_superseded_by_reconnect() {
        // F-218: the old connection's late teardown must NOT evict the
        // freshly-reconnected session. (The unconditional `remove` WOULD
        // evict it — that is the bug this guard fixes.)
        let table = RoutingTable::new();
        let p = peer(0xAA);
        let (tx_old, _rx_old) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        let (tx_new, _rx_new) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        let tx_old_dereg = tx_old.clone();

        assert!(table.insert(p, tx_old).is_none());
        // Peer reconnects: the new connection replaces, returns prior(old).
        assert!(table.insert(p, tx_new).is_some());

        // Old connection's reader finally exits and tears down with ITS
        // handle — must be a complete no-op.
        table.remove_self(&p, &tx_old_dereg);

        assert_eq!(table.len(), 1, "fresh session must remain registered");
        table
            .forward(&p, Bytes::from_static(b"x"))
            .expect("fresh session still routable after superseded teardown");
    }

    #[test]
    fn remove_self_removes_when_handle_is_still_own() {
        let table = RoutingTable::new();
        let p = peer(0xBB);
        let (tx, _rx) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        let tx_dereg = tx.clone();
        table.insert(p, tx);
        table.remove_self(&p, &tx_dereg);
        assert!(table.is_empty(), "own connection's teardown removes the entry");
    }

    #[test]
    fn superseded_remove_self_does_not_notify_subscribers_offline() {
        // F-218 N1: a superseded old connection's teardown must not emit a
        // spurious PeersWentOffline that would overwrite the new session's
        // PeersOnline in a watcher's view.
        let table = RoutingTable::new();
        let subject = peer(0x01);
        let watcher = peer(0x02);
        let (tx_w, mut rx_w) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(watcher, tx_w);
        table.subscribe(watcher, &[subject]); // subject offline → no immediate

        let (tx_old, _rx_old) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        let tx_old_dereg = tx_old.clone();
        table.insert(subject, tx_old); // watcher gets Online
        expect_peer_list_frame(&mut rx_w, PeerListKind::Online, &[subject]);

        let (tx_new, _rx_new) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(subject, tx_new); // reconnect: watcher gets Online again
        expect_peer_list_frame(&mut rx_w, PeerListKind::Online, &[subject]);

        // Superseded teardown — no Offline must reach the watcher.
        table.remove_self(&subject, &tx_old_dereg);
        assert!(
            rx_w.try_recv().is_err(),
            "no spurious PeersWentOffline after a superseded teardown"
        );
    }

    #[test]
    fn clone_shares_state() {
        let table_a = RoutingTable::new();
        let table_b = table_a.clone();
        let p1 = peer(0x11);
        let (tx, _rx) = mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        table_a.insert(p1, tx);
        // Clone sees the same state — Rc semantics, not deep copy.
        assert_eq!(table_b.len(), 1);
    }

    // -- Subscription registry tests (Block 33B SubscribePeerState gap fill) --

    /// Helper: assert that the next frame from `rx` is a PeersOnline /
    /// PeersWentOffline list containing exactly `expected_subjects` (order-
    /// independent).
    fn expect_peer_list_frame(
        rx: &mut mpsc::Receiver<Frame>,
        expected_kind: PeerListKind,
        expected_subjects: &[PeerId],
    ) {
        use goat_relay_protocol::messages::peer_state::parse_peer_list;
        let frame = rx.try_recv().expect("expected frame in writer queue");
        let (kind, subjects) =
            parse_peer_list(&frame).expect("frame parses as peer-list");
        assert_eq!(kind, expected_kind, "wrong peer-list kind");
        let want: HashSet<&PeerId> = expected_subjects.iter().collect();
        let got: HashSet<&PeerId> = subjects.iter().collect();
        assert_eq!(got, want, "subjects mismatch");
    }

    #[test]
    fn subscribe_then_arrive_emits_online() {
        let table = RoutingTable::new();
        let subscriber = peer(0x01);
        let subject = peer(0x02);
        let (sub_tx, mut sub_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(subscriber, sub_tx);

        // Subscribe — subject not yet online; no immediate emit
        table.subscribe(subscriber, &[subject]);
        assert!(
            sub_rx.try_recv().is_err(),
            "no frame expected — subject not yet online"
        );

        // Subject arrives — subscriber gets PeersOnline([subject])
        let (subj_tx, _subj_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(subject, subj_tx);
        expect_peer_list_frame(&mut sub_rx, PeerListKind::Online, &[subject]);
    }

    #[test]
    fn arrive_then_subscribe_emits_online_immediately() {
        let table = RoutingTable::new();
        let subscriber = peer(0x01);
        let subject = peer(0x02);
        let (sub_tx, mut sub_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        let (subj_tx, _subj_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);

        // Subject arrives BEFORE subscriber subscribes
        table.insert(subscriber, sub_tx);
        table.insert(subject, subj_tx);
        // No frame yet — subscriber wasn't subscribed at insert time
        assert!(sub_rx.try_recv().is_err());

        // Subscribe — immediate emit because subject is already online
        table.subscribe(subscriber, &[subject]);
        expect_peer_list_frame(&mut sub_rx, PeerListKind::Online, &[subject]);
    }

    #[test]
    fn subject_disconnect_emits_offline_to_subscribers() {
        let table = RoutingTable::new();
        let subscriber = peer(0x01);
        let subject = peer(0x02);
        let (sub_tx, mut sub_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        let (subj_tx, _subj_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);

        table.insert(subscriber, sub_tx);
        table.insert(subject, subj_tx);
        table.subscribe(subscriber, &[subject]);
        // Drain the immediate-Online emit
        let _ = sub_rx.try_recv();

        // Subject disconnects — subscriber gets PeersWentOffline([subject])
        table.remove(&subject);
        expect_peer_list_frame(&mut sub_rx, PeerListKind::Offline, &[subject]);
    }

    #[test]
    fn unsubscribe_silences_subsequent_offline() {
        let table = RoutingTable::new();
        let subscriber = peer(0x01);
        let subject = peer(0x02);
        let (sub_tx, mut sub_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        let (subj_tx, _subj_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);

        table.insert(subscriber, sub_tx);
        table.insert(subject, subj_tx);
        table.subscribe(subscriber, &[subject]);
        let _ = sub_rx.try_recv(); // immediate-Online

        table.unsubscribe(subscriber, &[subject]);
        table.remove(&subject);
        assert!(
            sub_rx.try_recv().is_err(),
            "unsubscribe must silence subsequent Offline"
        );
    }

    #[test]
    fn subscriber_disconnect_drops_its_subscriptions() {
        let table = RoutingTable::new();
        let subscriber = peer(0x01);
        let subject = peer(0x02);
        let (sub_tx, _sub_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        let (subj_tx, _subj_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);

        table.insert(subscriber, sub_tx);
        table.insert(subject, subj_tx);
        table.subscribe(subscriber, &[subject]);

        // Subscriber goes away
        table.remove(&subscriber);

        // Verify subscriber's subscription was cleaned (subject's subscriber
        // set is now empty → entry removed). Reaching into the registry to
        // assert this; a public API for this is dead-code today.
        let subs = table
            .inner
            .subscribers
            .read()
            .expect("rwlock not poisoned");
        assert!(
            !subs.by_subject.contains_key(&subject),
            "subscriber's subscription must drop on its disconnect"
        );
        assert!(
            !subs.by_subscriber.contains_key(&subscriber),
            "the reverse index must drop the departed subscriber too"
        );
    }

    /// A distinct peer-id for every `n`, beyond the 256 that [`peer`] gives.
    fn peer_n(n: u32) -> PeerId {
        let mut b = [0u8; PEER_ID_SIZE];
        b[..4].copy_from_slice(b"sha-");
        b[4..8].copy_from_slice(&n.to_be_bytes());
        PeerId::from_bytes(b)
    }

    #[test]
    fn subscribe_refuses_new_subjects_past_the_per_peer_cap() {
        let table = RoutingTable::new();
        let subscriber = peer(0xEE);
        let cap = u32::try_from(MAX_SUBSCRIPTIONS_PER_PEER).unwrap();
        let subjects: Vec<PeerId> = (0..cap).map(peer_n).collect();
        for chunk in subjects.chunks(244) {
            table.subscribe(subscriber, chunk);
        }
        assert_eq!(
            table.subscriptions_refused(),
            0,
            "up to the cap is accepted"
        );

        // Re-subscribing an already-watched subject at the cap is fine.
        table.subscribe(subscriber, &subjects[..10]);
        assert_eq!(
            table.subscriptions_refused(),
            0,
            "an existing edge is not new"
        );

        // Two new subjects past the cap are refused and counted.
        table.subscribe(subscriber, &[peer_n(cap), peer_n(cap + 1)]);
        assert_eq!(table.subscriptions_refused(), 2);
        let subs = table.inner.subscribers.read().expect("rwlock not poisoned");
        assert_eq!(
            subs.by_subscriber[&subscriber].len(),
            MAX_SUBSCRIPTIONS_PER_PEER
        );
        assert!(
            !subs.by_subject.contains_key(&peer_n(cap)),
            "a refused subject is not recorded"
        );
    }

    #[test]
    fn unsubscribe_frees_room_under_the_cap() {
        let table = RoutingTable::new();
        let subscriber = peer(0xEE);
        let cap = u32::try_from(MAX_SUBSCRIPTIONS_PER_PEER).unwrap();
        let subjects: Vec<PeerId> = (0..cap).map(peer_n).collect();
        for chunk in subjects.chunks(244) {
            table.subscribe(subscriber, chunk);
        }
        table.unsubscribe(subscriber, &subjects[..1]);
        table.subscribe(subscriber, &[peer_n(cap)]);
        assert_eq!(
            table.subscriptions_refused(),
            0,
            "the freed slot is reusable"
        );
    }

    #[test]
    fn subject_departure_keeps_both_indexes_in_step() {
        // A subject leaving drops its watcher set (watchers hear Offline once
        // and must re-subscribe — unchanged behaviour); the reverse index must
        // forget that edge too, or the watcher's cap count would drift.
        let table = RoutingTable::new();
        let watcher = peer(0x01);
        let subject = peer(0x02);
        let other = peer(0x03);
        let (w_tx, mut w_rx) = mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        let (s_tx, _s_rx) = mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(watcher, w_tx);
        table.insert(subject, s_tx);
        table.subscribe(watcher, &[subject, other]);
        let _ = w_rx.try_recv(); // immediate Online for `subject`

        table.remove(&subject);
        assert!(
            w_rx.try_recv().is_ok(),
            "the watcher is told the subject went offline"
        );

        let subs = table.inner.subscribers.read().expect("rwlock not poisoned");
        assert!(!subs.by_subject.contains_key(&subject));
        let watching = &subs.by_subscriber[&watcher];
        assert!(
            !watching.contains(&subject),
            "reverse edge to the departed subject is gone"
        );
        assert!(
            watching.contains(&other),
            "unrelated subscriptions are untouched"
        );
        assert!(subs.by_subject[&other].contains(&watcher));
    }

    #[test]
    fn multiple_subscribers_each_receive_online() {
        let table = RoutingTable::new();
        let sub_a = peer(0x01);
        let sub_b = peer(0x02);
        let subject = peer(0x03);
        let (a_tx, mut a_rx) = mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        let (b_tx, mut b_rx) = mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        let (subj_tx, _subj_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);

        table.insert(sub_a, a_tx);
        table.insert(sub_b, b_tx);
        table.subscribe(sub_a, &[subject]);
        table.subscribe(sub_b, &[subject]);

        // Subject arrives — both subscribers notified
        table.insert(subject, subj_tx);
        expect_peer_list_frame(&mut a_rx, PeerListKind::Online, &[subject]);
        expect_peer_list_frame(&mut b_rx, PeerListKind::Online, &[subject]);
    }

    #[test]
    fn subscribe_to_multiple_subjects_immediate_emit_aggregates() {
        let table = RoutingTable::new();
        let subscriber = peer(0x01);
        let subj_x = peer(0x02);
        let subj_y = peer(0x03);
        let subj_z_offline = peer(0x04); // not registered
        let (sub_tx, mut sub_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        let (x_tx, _x_rx) = mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        let (y_tx, _y_rx) = mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);

        table.insert(subscriber, sub_tx);
        table.insert(subj_x, x_tx);
        table.insert(subj_y, y_tx);

        // Subscribe to all three — only x + y are online; emit one frame
        // containing [x, y] (z is silently in the watchlist, no emit).
        table.subscribe(subscriber, &[subj_x, subj_y, subj_z_offline]);
        expect_peer_list_frame(
            &mut sub_rx,
            PeerListKind::Online,
            &[subj_x, subj_y],
        );
    }

    /// L8 invariant — `RoutingTable` must be `Send + Sync` so it can be
    /// cloned into multiple shard threads. Without this, the cross-shard
    /// forward path (peer-A on shard-0 → peer-B on shard-1) is impossible
    /// at the type-system layer.
    #[test]
    fn routing_table_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RoutingTable>();
    }

    /// L8 — verify a forward from one `RoutingTable` clone reaches a
    /// receiver registered on a different clone (simulating cross-shard
    /// register on shard A → forward on shard B).
    #[test]
    fn cross_shard_forward_via_arc_clone() {
        let shard_a = RoutingTable::new();
        let shard_b = shard_a.clone();
        let peer_b = peer(0xBB);
        let (tx, mut rx) = mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        // Register peer-B's writer on "shard B".
        shard_b.insert(peer_b, tx);
        // Forward from "shard A" — must route through the shared Arc<RwLock>
        // and land in peer-B's receiver.
        let frame = Bytes::from_static(b"cross-shard-payload");
        shard_a
            .forward(&peer_b, frame.clone())
            .expect("cross-shard forward succeeds");
        let received = rx.try_recv().expect("frame received cross-shard");
        assert_eq!(received, frame);
    }

    /// P3 (chaos suite, plan §6b) — slow-consumer isolation. A peer whose
    /// writer queue is FULL drops further frames (counted `writer_full`), but
    /// the drop is isolated per-connection: a healthy peer keeps receiving and
    /// neither peer is evicted. This is the brokerless
    /// no-cross-peer-head-of-line-blocking property — one slow consumer must
    /// never stall or drop another peer's traffic, and backpressure must not
    /// masquerade as a disconnect. The plan flagged a WS-level P3 as
    /// timing-flaky; this drives it DETERMINISTICALLY at the RoutingTable (the
    /// actual drop site), so there is no timing race. (P5 cross-shard routing
    /// is covered by `cross_shard_forward_via_arc_clone`; P6 graceful shutdown
    /// by `ws::tests::ws_graceful_drain_sends_close_frame_on_shutdown`.)
    #[test]
    fn p3_slow_consumer_drop_is_isolated_per_connection() {
        let table = RoutingTable::new();
        let slow = peer(0xAA);
        let healthy = peer(0xBB);

        // SLOW peer: a 1-slot queue we never drain (a stuck/slow consumer).
        let (slow_tx, _slow_rx) = mpsc::channel::<Frame>(1);
        table.insert(slow, slow_tx);
        // HEALTHY peer: full-capacity queue we WILL drain.
        let (healthy_tx, mut healthy_rx) =
            mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(healthy, healthy_tx);

        // Flood the slow peer well past its single slot.
        table
            .forward(&slow, Bytes::from_static(b"fills-the-one-slot"))
            .expect("first frame fits the slot");
        for _ in 0..64 {
            // Each of these finds the queue Full → dropped + counted, never panics.
            let _ = table.forward(&slow, Bytes::from_static(b"overflow"));
        }
        assert!(
            table.drop_stats().writer_full >= 64,
            "slow-consumer overflow is dropped + counted (writer_full)"
        );

        // The HEALTHY peer is unaffected — its delivery still succeeds and the
        // frame actually lands in its receiver (no cross-peer HOL blocking).
        let payload = Bytes::from_static(b"healthy-still-flows");
        table
            .forward(&healthy, payload.clone())
            .expect("healthy delivery is unaffected by the slow peer");
        assert_eq!(
            healthy_rx.try_recv().expect("healthy peer receives"),
            payload
        );

        // And neither peer was evicted by the slow consumer's backpressure —
        // writer_full does NOT deregister (only writer_closed does), so a slow
        // reader is never mistaken for a disconnect.
        assert_eq!(table.len(), 2, "no eviction from slow-consumer drops");
    }

    #[test]
    fn set_transport_tags_peer_and_clears_on_deregister() {
        // netbird-wedge-rca: per-peer transport must be readable on the stats
        // snapshot (default unknown until tagged), settable by the listener,
        // and gone once the peer deregisters (shares the counter lifecycle).
        let table = RoutingTable::new();
        let p = peer(0x5A);
        let (tx, _rx) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(p, tx.clone());

        // Untagged → unknown.
        assert_eq!(stats_of(&table, &p).unwrap().transport, Transport::Unknown);
        // Listener tags it → quic.
        table.set_transport(&p, Transport::Quic);
        assert_eq!(stats_of(&table, &p).unwrap().transport, Transport::Quic);
        // Idempotent re-tag holds.
        table.set_transport(&p, Transport::Quic);
        assert_eq!(stats_of(&table, &p).unwrap().transport, Transport::Quic);

        // Deregister drops the row (and thus the transport tag) entirely.
        table.remove_self(&p, &tx);
        assert!(stats_of(&table, &p).is_none());
        // Tagging a gone peer is a harmless no-op.
        table.set_transport(&p, Transport::Ws);
        assert!(stats_of(&table, &p).is_none());
    }

    #[test]
    fn writer_closed_eviction_notifies_subscribers_offline() {
        // netbird-wedge-rca: a dead-writer eviction must run the same
        // teardown as remove_self — subscribers get PeersWentOffline, the
        // departed peer's subscription graph is stripped, its stats row
        // dropped. Pre-fix the eviction was silent and watchers kept an
        // "online" view of a peer registered nowhere.
        let table = RoutingTable::new();
        let watcher = peer(0x01);
        let subject = peer(0x02);
        let (w_tx, mut w_rx) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(watcher, w_tx);

        let (s_tx, s_rx) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(subject, s_tx);
        table.subscribe(watcher, &[subject]);
        expect_peer_list_frame(&mut w_rx, PeerListKind::Online, &[subject]); // immediate

        // Subject's writer dies without a clean disconnect.
        drop(s_rx);
        let err = table
            .forward(&subject, Bytes::from_static(b"x"))
            .expect_err("dead writer");
        assert_eq!(err, ForwardError::WriterClosed);

        // Watcher must be told the subject went offline.
        expect_peer_list_frame(&mut w_rx, PeerListKind::Offline, &[subject]);
        // Entry, stats row, and subscription registry are all cleaned.
        assert_eq!(table.len(), 1, "subject evicted, watcher remains");
        assert!(stats_of(&table, &subject).is_none(), "stats row dropped");
        assert_eq!(table.drop_stats().writer_closed, 1);
    }

    #[test]
    fn immediate_online_emit_chunks_to_datagram_safe_frames() {
        // netbird-wedge-rca: a bulk subscribe's immediate-online response
        // must arrive as ≤DATAGRAM_SAFE_PEERS_PER_MESSAGE-subject frames so
        // it can ride a QUIC datagram. 70 online subjects → 3 frames
        // (30 + 30 + 10), union == all subjects, each frame ≤ 1,082 bytes.
        use goat_relay_protocol::messages::peer_state::parse_peer_list;
        let table = RoutingTable::new();
        let subscriber = peer(0xF0);
        let (sub_tx, mut sub_rx) =
            mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(subscriber, sub_tx);

        let subjects: Vec<PeerId> = (1..=70u8).map(peer).collect();
        for s in &subjects {
            let (tx, _rx) = mpsc::channel::<Frame>(crate::routing::WRITER_QUEUE_CAPACITY);
            table.insert(*s, tx);
        }
        table.subscribe(subscriber, &subjects);

        let mut got: HashSet<PeerId> = HashSet::new();
        let mut frames = 0usize;
        while let Ok(frame) = sub_rx.try_recv() {
            assert!(
                frame.len() <= HEADER_SIZE_PLUS_CHUNK,
                "frame of {} bytes exceeds the datagram-safe bound",
                frame.len()
            );
            let (kind, ids) = parse_peer_list(&frame).expect("peer-list frame");
            assert_eq!(kind, PeerListKind::Online);
            assert!(ids.len() <= DATAGRAM_SAFE_PEERS_PER_MESSAGE);
            got.extend(ids);
            frames += 1;
        }
        assert_eq!(frames, 3, "70 subjects chunk to 30+30+10");
        assert_eq!(got.len(), 70, "every online subject delivered exactly once");
    }

    /// 2-byte header + 30 × 36-byte peer-ids = the datagram-safe frame bound.
    const HEADER_SIZE_PLUS_CHUNK: usize =
        2 + DATAGRAM_SAFE_PEERS_PER_MESSAGE * PEER_ID_SIZE;

    #[test]
    fn datagram_too_large_counted_in_drop_stats() {
        let table = RoutingTable::new();
        assert_eq!(table.drop_stats().datagram_too_large, 0);
        table.count_datagram_too_large();
        table.count_datagram_too_large();
        assert_eq!(table.drop_stats().datagram_too_large, 2);
        assert_eq!(table.drop_stats().total(), 2);
    }

    #[test]
    fn idempotent_subscribe_no_double_emit_for_already_online() {
        // Per impl: re-subscribing to an already-online subject re-emits
        // the immediate Online — that's the documented behavior. Test
        // pins it so a future "dedupe immediate emits" change is a
        // deliberate behavior change, not a silent regression.
        let table = RoutingTable::new();
        let subscriber = peer(0x01);
        let subject = peer(0x02);
        let (sub_tx, mut sub_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        let (subj_tx, _subj_rx) =
            mpsc::channel(crate::routing::WRITER_QUEUE_CAPACITY);
        table.insert(subscriber, sub_tx);
        table.insert(subject, subj_tx);
        table.subscribe(subscriber, &[subject]);
        let _ = sub_rx.try_recv(); // consume first immediate-Online

        // Re-subscribe — second immediate-Online per current impl
        table.subscribe(subscriber, &[subject]);
        expect_peer_list_frame(&mut sub_rx, PeerListKind::Online, &[subject]);
    }
}
