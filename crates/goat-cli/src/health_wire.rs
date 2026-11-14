// Shared environment-health wire types + small pure display helpers.
//
// These hand-written prost-derive structs are the decode side of the
// on-bus health signal (ADR 1034/1036/1042/1040 gap 3). They were first
// written in `cmd/health.rs`; `goat top` (the live dashboard, ADR 1044)
// consumes the same shapes, so the structs + the two clock-free display
// helpers (`fmt_age`, `short_name`) live here as a single `pub(crate)`
// source of truth rather than a second hand-copy. Tags MUST stay in
// lock-step with:
//   ops/zenoh-trial-peers/fabric-health/proto/fabric_health_v1.proto
//   ops/zenoh-trial-peers/time-health/proto/env_health_v1.proto
//
// Line-oriented color rendering (`colorize`, `pad_state`) stays in
// `cmd/health.rs` — it emits ANSI escapes for stdout, the wrong layer
// for the ratatui dashboard (which styles via `Style::fg`).

/// Health + footprint topics (`goat health` fabric weather + footprint).
pub(crate) const HEALTH_KEY: &str = "ops/fabric/**";
pub(crate) const HEALTH_PREFIX: &str = "ops/fabric/health/";
pub(crate) const FOOTPRINT_PREFIX: &str = "ops/fabric/footprint/";
/// `EnvHealthV1` board domains.
pub(crate) const MESH_KEY: &str = "ops/mesh/health/**";
pub(crate) const PRESENCE_KEY: &str = "ops/presence/health/**";
pub(crate) const TIME_KEY: &str = "ops/time/health/**";
pub(crate) const NET_KEY: &str = "ops/net/health/**";
pub(crate) const CONSISTENCY_KEY: &str = "ops/consistency/health/**";
/// Faults on peers we do not own, measured at the router we own (ADR 1232):
/// the per-site reading, and the per-peer verdict documents (JSON) the reader
/// publishes on `ops/peer/verdict/<zid>` and `ops/peer/verdict/peering/<name>`.
pub(crate) const PEER_KEY: &str = "ops/peer/health/**";
pub(crate) const PEER_VERDICT_PREFIX: &str = "ops/peer/verdict/";
/// Crypto posture — evidence-based (ADR 1079); see
/// ops/zenoh-trial-peers/crypto-posture/proto/crypto_posture_v1.proto.
pub(crate) const CRYPTO_KEY: &str = "ops/fabric/crypto/**";

// --- crypto domain (ADR 1079) — every field is observed evidence ---

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct CryptoPostureV1 {
    #[prost(string, tag = "1")]
    pub site: String,
    #[prost(int64, tag = "2")]
    pub ts_unix_ms: i64,
    #[prost(bool, tag = "3")]
    pub kx_ok: bool,
    #[prost(string, tag = "4")]
    pub kx_group: String,
    #[prost(bool, tag = "5")]
    pub kx_group_fips: bool,
    #[prost(bool, tag = "6")]
    pub kx_pq: bool,
    #[prost(int64, tag = "7")]
    pub kx_probed_ms: i64,
    #[prost(string, tag = "8")]
    pub kx_error: String,
    #[prost(string, tag = "9")]
    pub router_version: String,
    #[prost(bool, tag = "10")]
    pub attest_present: bool,
    #[prost(bool, tag = "11")]
    pub attest_fips_marker: bool,
    #[prost(string, tag = "12")]
    pub attest_digest: String,
    #[prost(int64, tag = "13")]
    pub attest_at_ms: i64,
    #[prost(string, tag = "14")]
    pub attest_module: String,
    #[prost(string, tag = "15")]
    pub color: String,
    #[prost(string, tag = "16")]
    pub headline: String,
}

// --- fabric domain (ADR 1034) ---

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct ClassHealthV1 {
    #[prost(string, tag = "1")]
    pub class: String,
    #[prost(double, tag = "2")]
    pub sample_rate: f64,
    #[prost(double, tag = "3")]
    pub byte_rate: f64,
    #[prost(double, tag = "4")]
    pub target_sample_rate: f64,
    #[prost(double, tag = "5")]
    pub target_byte_rate: f64,
    #[prost(double, tag = "6")]
    pub headroom_pct: f64,
    #[prost(string, tag = "7")]
    pub color: String,
    #[prost(uint64, tag = "8")]
    pub sample_count: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct FabricHealthV1 {
    #[prost(string, tag = "1")]
    pub site: String,
    #[prost(int64, tag = "2")]
    pub ts_unix_ms: i64,
    #[prost(message, repeated, tag = "3")]
    pub classes: Vec<ClassHealthV1>,
    #[prost(string, tag = "4")]
    pub overall_color: String,
    #[prost(uint32, tag = "5")]
    pub active_producers: u32,
    #[prost(uint32, tag = "6")]
    pub window_secs: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct ClassShareV1 {
    #[prost(string, tag = "1")]
    pub class: String,
    #[prost(double, tag = "2")]
    pub sample_rate: f64,
    #[prost(double, tag = "3")]
    pub byte_rate: f64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct ProducerFootprintV1 {
    #[prost(string, tag = "1")]
    pub peer: String,
    #[prost(int64, tag = "2")]
    pub ts_unix_ms: i64,
    #[prost(double, tag = "3")]
    pub sample_rate: f64,
    #[prost(double, tag = "4")]
    pub byte_rate: f64,
    #[prost(message, repeated, tag = "5")]
    pub shares: Vec<ClassShareV1>,
    #[prost(uint32, tag = "6")]
    pub rank: u32,
    #[prost(bool, tag = "7")]
    pub near_limit: bool,
}

// --- EnvHealthV1 envelope + per-domain detail (ADR 1036) ---

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct MeshLinkHealthV1 {
    #[prost(string, tag = "1")]
    pub peer_fqdn: String,
    #[prost(string, tag = "2")]
    pub peer_ip: String,
    #[prost(string, tag = "3")]
    pub color: String,
    #[prost(bool, tag = "4")]
    pub connected: bool,
    #[prost(bool, tag = "5")]
    pub direct: bool,
    #[prost(string, tag = "6")]
    pub conn_type: String,
    #[prost(double, tag = "7")]
    pub latency_ms: f64,
    #[prost(string, tag = "8")]
    pub relay_address: String,
    #[prost(int64, tag = "9")]
    pub handshake_age_ms: i64,
    #[prost(uint64, tag = "10")]
    pub transition_count: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct MeshDetailV1 {
    #[prost(message, repeated, tag = "1")]
    pub links: Vec<MeshLinkHealthV1>,
    #[prost(string, tag = "2")]
    pub vantage_host: String,
    #[prost(uint32, tag = "3")]
    pub peers_total: u32,
    #[prost(uint32, tag = "4")]
    pub peers_connected: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct FabricDetailStub {}

/// Per-host clock health (tag 11 detail). Mirrors HostClockHealthV1 in
/// env_health_v1.proto — color is the 5-state honesty signal, attested=false
/// reads idle (never a false green).
#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct HostClockHealthV1 {
    #[prost(string, tag = "1")]
    pub host_id: String,
    #[prost(string, tag = "2")]
    pub color: String,
    #[prost(string, tag = "3")]
    pub verdict: String,
    #[prost(string, repeated, tag = "4")]
    pub verdict_reasons: Vec<String>,
    #[prost(double, tag = "5")]
    pub rms_offset_ms: f64,
    #[prost(int32, tag = "6")]
    pub stratum: i32,
    #[prost(bool, tag = "7")]
    pub gnss_present: bool,
    #[prost(double, tag = "8")]
    pub peer_median_offset_ms: f64,
    #[prost(bool, tag = "9")]
    pub divergence_detected: bool,
    #[prost(int64, tag = "10")]
    pub attestation_age_ms: i64,
    #[prost(bool, tag = "11")]
    pub attested: bool,
    /// A reading taken by a neighbor, not reported by the machine itself
    /// (ADR 1232): our router refused this machine's timestamps as ahead of
    /// its own clock. Such a row is outside the site's grade and count.
    #[prost(bool, tag = "12")]
    pub observed_by_neighbor: bool,
    /// The machine's clock minus our router's, latest, in milliseconds.
    #[prost(double, tag = "13")]
    pub neighbor_offset_ms: f64,
    /// Our router's Zenoh ID; empty when not known.
    #[prost(string, tag = "14")]
    pub observed_by: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct TimeDetailV1 {
    #[prost(message, repeated, tag = "1")]
    pub hosts: Vec<HostClockHealthV1>,
}

/// One liveliness presence token as seen by the presence-health watcher
/// (tag 13 detail). Mirrors PresencePeerV1 in env_health_v1.proto.
#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct PresencePeerV1 {
    #[prost(string, tag = "1")]
    pub token_key: String,
    #[prost(string, tag = "2")]
    pub write_root: String,
    #[prost(string, tag = "3")]
    pub label: String,
    #[prost(bool, tag = "4")]
    pub alive: bool,
    #[prost(int64, tag = "5")]
    pub since_unix_ms: i64,
    #[prost(uint64, tag = "6")]
    pub drop_count: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct PresenceDetailV1 {
    #[prost(message, repeated, tag = "1")]
    pub peers: Vec<PresencePeerV1>,
    #[prost(uint32, tag = "2")]
    pub alive_total: u32,
    #[prost(uint32, tag = "3")]
    pub dropped_recent: u32,
}

/// One WireGuard mesh peer as seen from a node's own vantage (tag 14 detail).
/// Mirrors NodeMeshPeerV1 in env_health_v1.proto (node-net-health.md).
#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct NodeMeshPeerV1 {
    #[prost(string, tag = "1")]
    pub peer_fqdn: String,
    #[prost(string, tag = "2")]
    pub peer_ip: String,
    #[prost(bool, tag = "3")]
    pub active: bool,
    #[prost(int64, tag = "4")]
    pub handshake_age_ms: i64,
    #[prost(string, tag = "5")]
    pub conn_type: String,
    #[prost(uint64, tag = "6")]
    pub rx_bytes_delta: u64,
    #[prost(uint64, tag = "7")]
    pub tx_bytes_delta: u64,
    #[prost(double, tag = "8")]
    pub latency_ms: f64,
}

/// One classified socket from the node's 4-bucket inventory
/// (mesh|fabric|baseline|undeclared). Mirrors NodeConnV1.
#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct NodeConnV1 {
    #[prost(string, tag = "1")]
    pub bucket: String,
    #[prost(string, tag = "2")]
    pub proto: String,
    #[prost(string, tag = "3")]
    pub state: String,
    #[prost(string, tag = "4")]
    pub local_addr: String,
    #[prost(string, tag = "5")]
    pub remote_addr: String,
    #[prost(uint32, tag = "6")]
    pub pid: u32,
    #[prost(string, tag = "7")]
    pub comm: String,
    #[prost(string, tag = "8")]
    pub container: String,
    #[prost(string, tag = "9")]
    pub baseline_rule: String,
    #[prost(int64, tag = "10")]
    pub first_seen_unix_ms: i64,
    #[prost(int64, tag = "11")]
    pub last_seen_unix_ms: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct NodeFabricSessionV1 {
    #[prost(string, tag = "1")]
    pub zid: String,
    #[prost(string, tag = "2")]
    pub whatami: String,
    #[prost(uint32, tag = "3")]
    pub links: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct NetDetailV1 {
    #[prost(message, repeated, tag = "1")]
    pub mesh_peers: Vec<NodeMeshPeerV1>,
    #[prost(uint32, tag = "2")]
    pub mesh_active: u32,
    #[prost(uint32, tag = "3")]
    pub mesh_stale: u32,
    #[prost(message, repeated, tag = "4")]
    pub fabric_sessions: Vec<NodeFabricSessionV1>,
    #[prost(message, repeated, tag = "5")]
    pub conns: Vec<NodeConnV1>,
    #[prost(uint32, tag = "6")]
    pub undeclared_count: u32,
    #[prost(string, tag = "7")]
    pub node_class: String,
    #[prost(uint32, tag = "8")]
    pub snapshot_gap_secs: u32,
}

// --- consistency domain (consistency-foundation.md §5, ADR 1110) ---
// The convergence record, rolled: which generation of its goatnet's IntentV1
// each node converged against, graded at READ TIME by the shared library and
// stored nowhere. The remediation pointer rides as DATA on every non-matching
// row (§6.3) — this CLI prints it VERBATIM and never composes, paraphrases,
// shortens or executes it (§6.4). If a pointer is wrong, the bug is in
// ops/consistency/grammar.yaml and its corpus, where one fix reaches every
// surface.

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct ConsistencyRowV1 {
    #[prost(string, tag = "1")]
    pub subject: String,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub artifact_type: String,
    #[prost(string, tag = "4")]
    pub grade: String,
    #[prost(string, tag = "5")]
    pub color: String,
    #[prost(string, tag = "6")]
    pub why: String,
    #[prost(string, tag = "7")]
    pub observed: String,
    #[prost(string, tag = "8")]
    pub expected: String,
    #[prost(string, tag = "9")]
    pub remediation_kind: String,
    #[prost(string, tag = "10")]
    pub remediation: String,
    #[prost(bool, tag = "11")]
    pub remediation_refuses: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct NodeConsistencyV1 {
    #[prost(string, tag = "1")]
    pub host_id: String,
    #[prost(string, tag = "2")]
    pub member_group: String,
    #[prost(string, tag = "3")]
    pub color: String,
    #[prost(string, tag = "4")]
    pub grade: String,
    #[prost(uint64, tag = "5")]
    pub intent_serial: u64,
    #[prost(string, tag = "6")]
    pub intent_content_hash: String,
    #[prost(uint32, tag = "7")]
    pub behind_by: u32,
    #[prost(int64, tag = "8")]
    pub intent_age_seconds: i64,
    #[prost(int64, tag = "9")]
    pub report_age_ms: i64,
    #[prost(bool, tag = "10")]
    pub intent_verified: bool,
    #[prost(message, repeated, tag = "11")]
    pub rows: Vec<ConsistencyRowV1>,
    #[prost(uint32, tag = "12")]
    pub components_governed: u32,
    #[prost(uint32, tag = "13")]
    pub components_observed: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct ConsistencyDetailV1 {
    #[prost(message, repeated, tag = "1")]
    pub nodes: Vec<NodeConsistencyV1>,
    #[prost(string, tag = "2")]
    pub goatnet: String,
    #[prost(uint64, tag = "3")]
    pub current_serial: u64,
    #[prost(string, tag = "4")]
    pub current_content_hash: String,
    #[prost(uint32, tag = "5")]
    pub nodes_expected: u32,
    #[prost(uint32, tag = "6")]
    pub nodes_reporting: u32,
    #[prost(uint32, tag = "7")]
    pub skew_band_generations: u32,
    #[prost(int64, tag = "8")]
    pub skew_band_seconds: i64,
    #[prost(bool, tag = "9")]
    pub intent_readable: bool,
    #[prost(string, tag = "10")]
    pub intent_note: String,
}

// --- peer domain (ADR 1232) — goat-peer-health, arm 17 ---

/// What the router saw wrong with a peer: the clock detector and the
/// trust-refusal detector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub(crate) enum PeerFaultKindV1 {
    PeerFaultKindUnspecified = 0,
    /// The router rejected this peer's timestamps as ahead of its clock.
    ClockAhead = 1,
    /// Clients refuse the router's certificate and this declared peer holds no
    /// session. A candidate, not a confirmed cause: the router's log cannot
    /// name the refusing client.
    TrustRootRefusedCandidate = 2,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct PeerFaultV1 {
    /// The Zenoh ID that STAMPED the rejected samples.
    #[prost(string, tag = "1")]
    pub zid: String,
    #[prost(enumeration = "PeerFaultKindV1", tag = "2")]
    pub kind: i32,
    /// Latest: peer time minus our router's time, seconds.
    #[prost(double, tag = "3")]
    pub offset_seconds: f64,
    #[prost(double, tag = "4")]
    pub min_offset_seconds: f64,
    #[prost(double, tag = "5")]
    pub max_offset_seconds: f64,
    #[prost(double, tag = "6")]
    pub rejections_per_minute: f64,
    #[prost(uint64, tag = "7")]
    pub rejections_in_window: u64,
    #[prost(int64, tag = "8")]
    pub first_seen_unix_ms: i64,
    #[prost(int64, tag = "9")]
    pub last_seen_unix_ms: i64,
    /// Our router's Zenoh ID; "" when not known.
    #[prost(string, tag = "10")]
    pub observed_by: String,
    #[prost(string, optional, tag = "11")]
    pub attribution_name: Option<String>,
    #[prost(string, optional, tag = "12")]
    pub attribution_owner: Option<String>,
    /// peer_at_fault | local_clock_suspect | quiet | cleared | unmeasured | candidate
    #[prost(string, tag = "13")]
    pub grade: String,
    #[prost(string, tag = "14")]
    pub color: String,
    #[prost(double, tag = "15")]
    pub limit_seconds: f64,
    /// Whether this Zenoh ID holds a session with our router; absent = unknown.
    #[prost(bool, optional, tag = "16")]
    pub direct_session: Option<bool>,
    /// The peers-file key when it is a source address, not a Zenoh ID (a
    /// candidate row for such an entry has `zid` = "").
    #[prost(string, optional, tag = "17")]
    pub source_address: Option<String>,
    /// Candidate rows: the first reading at which the reader saw the declared
    /// peer hold no session. Since the reader started; not persisted.
    #[prost(int64, optional, tag = "18")]
    pub absent_since_unix_ms: Option<i64>,
}

/// One TLS alert name the router received in the window: a fact about the
/// router (some client ended a handshake), not about a named peer.
#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct TlsAlertV1 {
    /// As logged, e.g. UnknownCA, BadCertificate, CertificateExpired.
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(uint64, tag = "2")]
    pub count_in_window: u64,
    #[prost(double, tag = "3")]
    pub per_minute: f64,
    /// In the trust-refusal set: the client does not accept the chain the
    /// router presents.
    #[prost(bool, tag = "4")]
    pub trust_refusal: bool,
    #[prost(int64, tag = "5")]
    pub last_seen_unix_ms: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct PeerFaultDetailV1 {
    #[prost(message, repeated, tag = "1")]
    pub peers: Vec<PeerFaultV1>,
    /// peer_at_fault | local_clock_suspect | recovering |
    /// trust_refused_candidates | trust_refused_unattributed | trust_unmeasured |
    /// clean | unmeasured
    #[prost(string, tag = "2")]
    pub verdict: String,
    #[prost(string, tag = "3")]
    pub verdict_reason: String,
    /// synced | unsynced | unknown
    #[prost(string, tag = "4")]
    pub local_clock: String,
    #[prost(string, tag = "5")]
    pub observed_by: String,
    #[prost(uint32, tag = "6")]
    pub peers_known: u32,
    /// -1 = no line read yet.
    #[prost(int64, tag = "7")]
    pub last_line_age_ms: i64,
    #[prost(uint64, tag = "8")]
    pub unparsed_lines_window: u64,
    /// Set when the reader sits beside one per-partner edge router.
    #[prost(string, tag = "9")]
    pub peering: String,
    #[prost(double, tag = "10")]
    pub peering_rejections_per_minute: f64,
    #[prost(double, tag = "11")]
    pub peering_max_offset_seconds: f64,
    /// Trust detector, site level: every TLS alert name in the window, by name.
    #[prost(message, repeated, tag = "12")]
    pub tls_alerts: Vec<TlsAlertV1>,
    /// The trust detector alone: clean | unattributed | candidates | unmeasured;
    /// "" from a reader without the trust detector.
    #[prost(string, tag = "13")]
    pub trust_verdict: String,
    /// Declared peers holding no session; absent = cannot be judged.
    #[prost(uint32, optional, tag = "14")]
    pub declared_peers_absent: Option<u32>,
}

/// The JSON document on `ops/peer/verdict/<zid>` (schema goat.peer.verdict.v1):
/// what the router says to the peer it measured.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct PeerVerdictV1 {
    pub schema: String,
    #[serde(default)]
    pub fault: String,
    /// at_fault | cleared
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub offset_seconds: f64,
    #[serde(default)]
    pub limit_seconds: f64,
    #[serde(default)]
    pub rejections_per_minute: f64,
    #[serde(default)]
    pub observed_at: String,
    #[serde(default)]
    pub observed_by: String,
    #[serde(default)]
    pub sentence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peering: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zids: Vec<String>,
}

pub(crate) const PEER_VERDICT_SCHEMA: &str = "goat.peer.verdict.v1";

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct EnvHealthV1 {
    #[prost(string, tag = "1")]
    pub domain: String,
    #[prost(string, tag = "2")]
    pub scope: String,
    #[prost(int64, tag = "3")]
    pub ts_unix_ms: i64,
    #[prost(string, tag = "4")]
    pub overall_color: String,
    #[prost(uint64, tag = "5")]
    pub sample_count: u64,
    #[prost(uint32, tag = "6")]
    pub window_secs: u32,
    // Arm 16 (peering) is not mirrored here; a decoder skips an arm it does not know.
    #[prost(oneof = "EnvDetail", tags = "10, 11, 12, 13, 14, 15, 17")]
    pub detail: Option<EnvDetail>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
pub(crate) enum EnvDetail {
    #[prost(message, tag = "10")]
    Fabric(FabricDetailStub),
    #[prost(message, tag = "11")]
    Time(TimeDetailV1),
    #[prost(message, tag = "12")]
    Mesh(MeshDetailV1),
    #[prost(message, tag = "13")]
    Presence(PresenceDetailV1),
    #[prost(message, tag = "14")]
    Net(NetDetailV1),
    #[prost(message, tag = "15")]
    Consistency(ConsistencyDetailV1),
    #[prost(message, tag = "17")]
    Peer(PeerFaultDetailV1),
}

// --- small pure display helpers (clock-free, unit-testable) ---

/// Humanize an age in ms: "44s ago" / "2m ago" / "2h ago"; negative → "—".
pub(crate) fn fmt_age(ms: i64) -> String {
    if ms < 0 {
        return "—".to_string();
    }
    let s = ms / 1000;
    if s < 90 {
        format!("{s}s ago")
    } else if s < 5400 {
        format!("{}m ago", s / 60)
    } else {
        format!("{}h ago", s / 3600)
    }
}

/// Short display label: first DNS label of a peer fqdn.
pub(crate) fn short_name(fqdn: &str) -> &str {
    fqdn.split('.').next().unwrap_or(fqdn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message as _;

    #[test]
    fn age_formats_by_magnitude() {
        assert_eq!(fmt_age(-1), "—");
        assert_eq!(fmt_age(44_000), "44s ago");
        assert_eq!(fmt_age(120_000), "2m ago");
        assert_eq!(fmt_age(7_200_000), "2h ago");
    }

    #[test]
    fn short_name_takes_first_label() {
        assert_eq!(short_name("thor.zagros.example.net"), "thor");
        assert_eq!(short_name("nodot"), "nodot");
    }

    #[test]
    fn envelope_round_trips_mesh_oneof() {
        let env = EnvHealthV1 {
            domain: "mesh".to_string(),
            scope: "zagros-sandbox".to_string(),
            ts_unix_ms: 1,
            overall_color: "amber".to_string(),
            sample_count: 2,
            window_secs: 30,
            detail: Some(EnvDetail::Mesh(MeshDetailV1 {
                links: vec![MeshLinkHealthV1 {
                    peer_fqdn: "a.x".to_string(),
                    peer_ip: "100.127.0.9".to_string(),
                    color: "amber".to_string(),
                    connected: true,
                    direct: false,
                    conn_type: "Relayed".to_string(),
                    latency_ms: 0.0,
                    relay_address: "rels://relay-rla.x:33073".to_string(),
                    handshake_age_ms: 5000,
                    transition_count: 0,
                }],
                vantage_host: "thor.x".to_string(),
                peers_total: 1,
                peers_connected: 1,
            })),
        };
        let bytes = env.encode_to_vec();
        let back = EnvHealthV1::decode(bytes.as_slice()).expect("decode");
        assert_eq!(back.domain, "mesh");
        match back.detail {
            Some(EnvDetail::Mesh(d)) => assert_eq!(d.links[0].peer_fqdn, "a.x"),
            _ => panic!("expected mesh detail"),
        }
    }

    #[test]
    fn peer_trust_fields_decode_on_their_proto_tags() {
        // Hand-encoded: PeerFaultV1 kind=2 (tag 2), source_address (tag 17),
        // absent_since_unix_ms (tag 18); detail tls_alerts (tag 12),
        // trust_verdict (tag 13), declared_peers_absent (tag 14).
        let mut row = vec![0x10, 0x02];
        row.extend_from_slice(&[0x8a, 0x01, 0x08]);
        row.extend_from_slice(b"10.0.0.7");
        row.extend_from_slice(&[0x90, 0x01, 0x05]);
        let mut alert = vec![0x0a, 0x09];
        alert.extend_from_slice(b"UnknownCA");
        alert.extend_from_slice(&[0x10, 0x03, 0x20, 0x01]);
        let mut detail = vec![0x0a, row.len() as u8];
        detail.extend_from_slice(&row);
        detail.extend_from_slice(&[0x62, alert.len() as u8]);
        detail.extend_from_slice(&alert);
        detail.extend_from_slice(&[0x6a, 0x0a]);
        detail.extend_from_slice(b"candidates");
        detail.extend_from_slice(&[0x70, 0x01]);
        let d = PeerFaultDetailV1::decode(detail.as_slice()).expect("decode");
        assert_eq!(
            d.peers[0].kind,
            PeerFaultKindV1::TrustRootRefusedCandidate as i32
        );
        assert_eq!(d.peers[0].source_address.as_deref(), Some("10.0.0.7"));
        assert_eq!(d.peers[0].absent_since_unix_ms, Some(5));
        assert_eq!(d.tls_alerts[0].name, "UnknownCA");
        assert_eq!(d.tls_alerts[0].count_in_window, 3);
        assert!(d.tls_alerts[0].trust_refusal);
        assert_eq!(d.trust_verdict, "candidates");
        assert_eq!(d.declared_peers_absent, Some(1));
    }

    #[test]
    fn peer_detail_from_a_reader_without_trust_fields_decodes_empty() {
        // Tags 1..=11 only, as a clock-only reader sends them.
        let mut detail = vec![0x12, 0x05];
        detail.extend_from_slice(b"clean");
        let d = PeerFaultDetailV1::decode(detail.as_slice()).expect("decode");
        assert_eq!(d.verdict, "clean");
        assert!(d.tls_alerts.is_empty());
        assert_eq!(d.trust_verdict, "");
        assert_eq!(d.declared_peers_absent, None);
    }
}
