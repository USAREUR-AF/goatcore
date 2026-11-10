// Name fallback resolver — docs/design/name-fallback-resolver.md, ADR 1082.
//
// Failure class: node network up, mesh name service down or unreachable
// (a silently empty zone, a declared-but-unresolvable name, an uplink
// outage where surfaces die by name while IPs work). This module
// resolves mesh hostnames in a strict order:
//
//   1. live DNS (the OS resolver, which includes the NetBird-pushed mesh
//      nameserver) — NEVER shadowed; if it answers, we use it;
//   2. the newest validly-signed cached name snapshot, subject to graded
//      staleness (fresh / aging / expired). Bad signature or past-TTL →
//      refuse, never silently serve.
//
// The snapshot is generated + signed repo-side (ops/dns/<site>/
// name-snapshot.json, ECDSA P-256 over the file bytes by the same
// per-goatnet offline-CA that signs enrollment bundles) and served at
// get.<site>.<zone> by the goat-get role. We refresh the per-context cache
// opportunistically while names work (goat status / goat doctor) and verify
// the signature at every USE, not just at fetch — a tampered cache file is
// inert. A lower serial never replaces a higher cached one (replay bound).

use crate::profile::Profile;
use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
#[cfg(test)]
use p256::elliptic_curve::Generate;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

pub const SNAPSHOT_FILE: &str = "name-snapshot.json";
pub const SIG_FILE: &str = "name-snapshot.json.sig";
/// Noncanonical observed-records tier (design §4.2): successful LIVE
/// resolutions persist here so live-only records (peer names via
/// sync-peers.sh; operator hotfixes the repo never captured) survive a
/// later DNS outage. Unsigned by construction — every answer from this
/// tier is labeled noncanonical (ad hoc) and logged as codification debt.
pub const OBSERVED_FILE: &str = "observed-names.json";
const SNAPSHOT_FORMAT: &str = "goat-name-snapshot-v1";
/// Observed records older than this are pruned — an observation this stale
/// is no better than a guess.
const OBSERVED_TTL_SECS: i64 = 30 * 24 * 3600;
/// Bounded store: newest-first cap so the file can't grow unbounded.
const OBSERVED_MAX: usize = 512;

/// Live-DNS budget before consulting the snapshot. Generous enough for a
/// slow-but-up resolver (never shadow live DNS), short enough that a dead
/// resolver doesn't stall every probe.
const LIVE_LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);
/// Fresh/aging boundary: under a week old reads as routine; older warns.
const FRESH_BOUND_SECS: i64 = 7 * 24 * 3600;
/// Best-effort refresh budget (both GETs + verify).
const REFRESH_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Deserialize)]
pub struct NameSnapshot {
    pub format: String,
    pub site_id: String,
    pub zone: String,
    pub serial: u64,
    /// True when this snapshot was read from the goat-clientd shared
    /// store rather than the per-context cache (§4.1: daemon present →
    /// daemon is the sole refresher, goat-cli degrades to a reader).
    /// Not part of the signed artifact — stamped at load time.
    #[serde(skip)]
    pub from_daemon: bool,
    /// RFC3339 twin of `generated_at_unix` — for humans reading the cached
    /// file; the client computes age from the unix field only.
    #[serde(default)]
    #[allow(dead_code)]
    pub generated_at: String,
    pub generated_at_unix: i64,
    pub ttl_seconds: u64,
    #[serde(default)]
    pub ca_id: String,
    pub records: Vec<NameRecord>,
    /// Name→anchor bindings for self-claim verification (§7.1). Additive:
    /// pre-Amendment-2 snapshots deserialize with an empty list.
    #[serde(default)]
    pub peer_bindings: Vec<PeerBinding>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NameRecord {
    pub name: String,
    pub ip: String,
}

/// Registry-derived name→anchor ownership (ADR 1082 Amendment 2): which
/// identity may self-claim which peer name. Rides the signed snapshot —
/// absent on pre-Amendment-2 snapshots (no claims verifiable, fail closed).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PeerBinding {
    pub name: String,
    pub anchor: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    Fresh,
    Aging,
    Expired,
}

/// The honesty label every fallback answer carries: source serial + age.
#[derive(Debug, Clone)]
pub struct SnapshotMeta {
    pub serial: u64,
    pub age_secs: i64,
    pub grade: Grade,
    /// Answer came from the daemon's shared store (see
    /// [`NameSnapshot::from_daemon`]) — surfaced in `goat status` so the
    /// one-store-per-device contract is visible.
    pub from_daemon: bool,
}

impl SnapshotMeta {
    /// Compact human age: "42m", "7h", "3d".
    pub fn age_human(&self) -> String {
        age_human(self.age_secs)
    }
}

/// Where an answer came from. Surfaced verbatim in status/doctor — an
/// operator on fallback names always sees it.
#[derive(Debug, Clone)]
pub enum NameSource {
    /// The target was an IP literal — no name resolution involved.
    IpLiteral,
    /// Live DNS answered.
    Live,
    /// Resolved from the signed snapshot (mesh DNS did not answer).
    Snapshot(SnapshotMeta),
    /// Resolved from a verified SELF-CLAIM (ADR 1082 Amendment 2): the
    /// name's own anchor signed this binding, and the cached snapshot's
    /// peer_bindings authorize it. Signed + labeled, slots between live
    /// DNS and the snapshot, peer names only.
    SelfClaim { age_secs: i64 },
    /// Resolved from the NONCANONICAL observed tier (mesh DNS did not
    /// answer AND the signed snapshot could not answer this name, or a
    /// fresher observation superseded it). Always labeled ad hoc.
    Observed { age_secs: i64 },
}

/// One record of the observed tier: a name→IP binding learned from a
/// successful live resolution, with its provenance timestamp.
#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
pub struct ObservedRecord {
    pub name: String,
    pub ip: String,
    pub observed_at: i64,
}

/// Compact human age: "42m", "7h", "3d".
pub fn age_human(age_secs: i64) -> String {
    let s = age_secs.max(0);
    if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86400)
    }
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub addr: SocketAddr,
    pub source: NameSource,
}

/// Both legs failed: live DNS and the snapshot fallback. Carries the
/// per-leg reasons so the surface can be honest about *why*.
#[derive(Debug, Clone)]
pub struct ResolveFailure {
    pub dns_error: String,
    pub snapshot_reason: String,
}

impl std::fmt::Display for ResolveFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "live DNS failed ({}); snapshot fallback unavailable ({})",
            self.dns_error, self.snapshot_reason
        )
    }
}

/// Resolve `host:port` — live DNS first (never shadowed), signed-snapshot
/// fallback second, graded and fail-closed per the table in
/// docs/design/name-fallback-resolver.md §5.
pub async fn resolve_host_port(
    profile: &Profile,
    host: &str,
    port: u16,
) -> Result<Resolved, ResolveFailure> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(Resolved {
            addr: SocketAddr::new(ip, port),
            source: NameSource::IpLiteral,
        });
    }

    let dns_error = match tokio::time::timeout(
        LIVE_LOOKUP_TIMEOUT,
        tokio::net::lookup_host((host, port)),
    )
    .await
    {
        Ok(Ok(mut addrs)) => match addrs.next() {
            Some(addr) => {
                // Feed the observed tier (§4.2): a successful live answer is
                // worth remembering for the next outage. Best-effort.
                record_observation(
                    &profile.context_dir(),
                    host,
                    &addr.ip().to_string(),
                    now_unix(),
                );
                return Ok(Resolved {
                    addr,
                    source: NameSource::Live,
                });
            }
            None => "resolver returned no addresses".to_string(),
        },
        Ok(Err(e)) => e.to_string(),
        Err(_) => format!("no answer in {}ms", LIVE_LOOKUP_TIMEOUT.as_millis()),
    };

    // Live DNS failed — verified self-claim first (§7: signed, peer names
    // only, authorized by the snapshot's bindings), then the signed
    // snapshot, then the noncanonical observed tier (§4.2: gap-fill +
    // fresher-wins-on-conflict; demoted where a valid claim exists by
    // claim precedence).
    let now = now_unix();
    let snap_outcome = load_verified(profile);
    // Try every CA root, not just the first — a claim chained to a
    // goatnet whose root is not the first TRUSTED one is still a valid claim.
    let claim = snap_outcome.as_ref().ok().and_then(|snap| {
        ca_roots()
            .iter()
            .find_map(|root| lookup_claim(profile, snap, root, host, now))
    });
    {
        if let Some(vc) = claim {
            tracing::info!(
                "names: {host} → {} via SELF-CLAIM (age {}, signed by the name's own anchor; mesh DNS unreachable)",
                vc.ip,
                age_human(vc.age_secs)
            );
            return Ok(Resolved {
                addr: SocketAddr::new(vc.ip, port),
                source: NameSource::SelfClaim {
                    age_secs: vc.age_secs,
                },
            });
        }
    }
    let observed = lookup_observed_merged(profile, host, now);
    match pick_fallback(snap_outcome, observed.as_ref(), host, now) {
        FallbackPick::Snapshot(meta, ip) => Ok(Resolved {
            addr: SocketAddr::new(ip, port),
            source: NameSource::Snapshot(meta),
        }),
        FallbackPick::Observed { age_secs, ip, why } => {
            // The codification-debt signal: this name exists in reality but
            // not in the signed registry snapshot — loud by design.
            tracing::warn!(
                "names: serving NONCANONICAL (ad hoc) observed record for {host} \
                 (age {}, {why}) — codify this record in the site registry",
                age_human(age_secs)
            );
            Ok(Resolved {
                addr: SocketAddr::new(ip, port),
                source: NameSource::Observed { age_secs },
            })
        }
        FallbackPick::None { reason } => Err(ResolveFailure {
            dns_error,
            snapshot_reason: reason,
        }),
    }
}

/// Which fallback tier answered — carried by doctor/pubsub so their labels
/// distinguish canonical (signed snapshot) from noncanonical (observed).
#[derive(Debug, Clone)]
pub enum FallbackNote {
    Snapshot(SnapshotMeta),
    SelfClaim { age_secs: i64 },
    Observed { age_secs: i64 },
}

impl NameSource {
    /// The note for fallback-sourced answers; None for live/IP-literal.
    pub fn fallback_note(&self) -> Option<FallbackNote> {
        match self {
            NameSource::Snapshot(meta) => Some(FallbackNote::Snapshot(meta.clone())),
            NameSource::SelfClaim { age_secs } => Some(FallbackNote::SelfClaim {
                age_secs: *age_secs,
            }),
            NameSource::Observed { age_secs } => Some(FallbackNote::Observed {
                age_secs: *age_secs,
            }),
            _ => None,
        }
    }
}

/// The fallback decision, pure for testability.
#[derive(Debug)]
pub(crate) enum FallbackPick {
    Snapshot(SnapshotMeta, IpAddr),
    Observed {
        age_secs: i64,
        ip: IpAddr,
        why: &'static str,
    },
    None {
        reason: String,
    },
}

pub(crate) fn pick_fallback(
    snap_outcome: Result<NameSnapshot, String>,
    observed: Option<&ObservedRecord>,
    host: &str,
    now: i64,
) -> FallbackPick {
    let observed_ip = observed.and_then(|o| o.ip.parse::<IpAddr>().ok());
    let observed_pick = |why: &'static str| match (observed, observed_ip) {
        (Some(o), Some(ip)) => FallbackPick::Observed {
            age_secs: now.saturating_sub(o.observed_at),
            ip,
            why,
        },
        _ => FallbackPick::None {
            reason: format!("no observed record for {host}"),
        },
    };
    let snap_reason: String;
    match snap_outcome {
        Ok(snap) => {
            let meta = grade_of(&snap, now);
            if meta.grade == Grade::Expired {
                snap_reason = format!(
                    "snapshot expired (serial {}, age {} ≥ ttl {}s) — refusing stale names",
                    meta.serial,
                    meta.age_human(),
                    snap.ttl_seconds
                );
            } else {
                let wanted = host.to_ascii_lowercase();
                let rec = snap
                    .records
                    .iter()
                    .find(|r| r.name.to_ascii_lowercase() == wanted);
                match rec {
                    Some(rec) => {
                        // Conflict rule: a fresher observation supersedes the
                        // snapshot record — the operator-hotfix case.
                        if let (Some(o), Some(_)) = (observed, observed_ip) {
                            if o.ip != rec.ip && o.observed_at > snap.generated_at_unix {
                                return observed_pick("fresher than snapshot record");
                            }
                        }
                        return match rec.ip.parse::<IpAddr>() {
                            Ok(ip) => FallbackPick::Snapshot(meta, ip),
                            Err(e) => FallbackPick::None {
                                reason: format!(
                                    "snapshot record for {host} has malformed ip {}: {e}",
                                    rec.ip
                                ),
                            },
                        };
                    }
                    None => {
                        // Gap-fill rule: live-only names (sync'd peers,
                        // never-codified records) come from observations.
                        if observed.is_some() {
                            return observed_pick("not in signed snapshot");
                        }
                        snap_reason =
                            format!("no record for {host} in snapshot serial {}", snap.serial);
                    }
                }
            }
        }
        Err(reason) => snap_reason = reason,
    }
    // Snapshot could not answer (missing/expired/unverifiable/no record) —
    // the observed tier may still, labeled.
    if observed.is_some() {
        return observed_pick("signed snapshot unavailable for this name");
    }
    FallbackPick::None {
        reason: format!("{snap_reason}; no observed record for {host}"),
    }
}

// --- self-claims tier (ADR 1082 Amendment 2, design §7) ---

/// Third store file: claim envelopes collected from the fabric while
/// healthy (get.<site>/claims.json → shared store), served while broken.
pub const CLAIMS_FILE: &str = "claims.json";
pub(crate) const CLAIM_FORMAT: &str = "goat-name-claim-v1";

/// The signed artifact — canonical bytes are what the leaf signs.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct ClaimArtifact {
    pub format: String,
    pub name: String,
    pub ip: String,
    pub anchor: String,
    pub issued_at_unix: i64,
    pub ttl_seconds: u64,
}

/// Wire/file envelope: exact artifact bytes (base64) + leaf ECDSA
/// signature over them + the leaf cert. The transport (fabric, collector,
/// get tier, store file) is untrusted — verification happens at read.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct ClaimEnvelope {
    pub artifact_b64: String,
    pub sig_b64: String,
    pub leaf_pem: String,
}

/// A claim that survived full verification, ready to serve.
#[derive(Debug, Clone)]
pub struct ValidClaim {
    pub name: String,
    pub ip: IpAddr,
    pub age_secs: i64,
}

/// Full offline verification (§7.1) against the CACHED VERIFIED snapshot:
///  1. leaf cert parses and is signed by the embedded CA root (the same
///     root that signs snapshots + enrollment bundles);
///  2. the artifact bytes verify under the leaf key;
///  3. artifact format + the leaf's own identity == artifact.anchor
///     (a stolen valid leaf still can't claim someone else's name);
///  4. the snapshot binds artifact.name to artifact.anchor — own-name-only
///     and never-override-service-records hold by construction;
///  5. fresh: now − issued_at < ttl (fail closed, like snapshot grading).
pub(crate) fn verify_claim(
    env: &ClaimEnvelope,
    snap: &NameSnapshot,
    root: &VerifyingKey,
    now: i64,
) -> Result<ValidClaim, String> {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD;

    // Leaf parses + chains to the embedded root.
    let blob = ::pem::parse(env.leaf_pem.as_bytes()).map_err(|e| format!("leaf not PEM: {e}"))?;
    if blob.tag() != "CERTIFICATE" {
        return Err("leaf PEM is not a CERTIFICATE".to_string());
    }
    let (_, leaf) = x509_parser::parse_x509_certificate(blob.contents())
        .map_err(|e| format!("leaf not X.509: {e}"))?;
    let tbs_sig = Signature::from_der(leaf.signature_value.data.as_ref())
        .map_err(|e| format!("leaf signature not DER: {e}"))?;
    root.verify(leaf.tbs_certificate.as_ref(), &tbs_sig)
        .map_err(|_| "leaf not signed by the embedded CA root".to_string())?;
    let validity = leaf.validity();
    let (nb, na) = (
        validity.not_before.timestamp(),
        validity.not_after.timestamp(),
    );
    if now < nb || now > na {
        return Err(format!("leaf outside its validity window ({nb}..{na})"));
    }

    // Artifact bytes verify under the leaf key. The fleet's zenoh leafs
    // carry Ed25519 keys (raw 64-byte signatures); P-256 leafs (DER
    // signatures) are accepted for parity with the CA's own curve.
    let artifact = b64
        .decode(env.artifact_b64.trim())
        .map_err(|e| format!("artifact not base64: {e}"))?;
    let sig_raw = b64
        .decode(env.sig_b64.trim())
        .map_err(|e| format!("claim sig not base64: {e}"))?;
    match leaf.public_key().parsed() {
        Ok(x509_parser::public_key::PublicKey::EC(ec)) => {
            let leaf_key = VerifyingKey::from_sec1_bytes(ec.data())
                .map_err(|e| format!("leaf key not P-256: {e}"))?;
            let sig =
                Signature::from_der(&sig_raw).map_err(|e| format!("claim sig not DER: {e}"))?;
            leaf_key
                .verify(&artifact, &sig)
                .map_err(|_| "claim signature invalid under the leaf key".to_string())?;
        }
        _ => {
            // Ed25519 SPKI parses as Unknown in x509-parser — take the
            // raw BIT STRING bytes.
            let raw: [u8; 32] = leaf
                .public_key()
                .subject_public_key
                .data
                .as_ref()
                .try_into()
                .map_err(|_| "leaf key is neither P-256 nor Ed25519".to_string())?;
            let vk = ed25519_dalek::VerifyingKey::from_bytes(&raw)
                .map_err(|e| format!("leaf key not Ed25519: {e}"))?;
            let sig = ed25519_dalek::Signature::from_slice(&sig_raw)
                .map_err(|e| format!("claim sig not an Ed25519 signature: {e}"))?;
            vk.verify_strict(&artifact, &sig)
                .map_err(|_| "claim signature invalid under the leaf key".to_string())?;
        }
    }

    // Artifact shape + leaf-identity == claimed anchor.
    let claim: ClaimArtifact =
        serde_json::from_slice(&artifact).map_err(|e| format!("claim malformed: {e}"))?;
    if claim.format != CLAIM_FORMAT {
        return Err(format!("unexpected claim format {:?}", claim.format));
    }
    let leaf_id = leaf_identity(&leaf).ok_or_else(|| {
        "leaf carries no goat identity (CN anchor / URI-SAN org=/vendor=)".to_string()
    })?;
    if leaf_id != claim.anchor {
        return Err(format!(
            "leaf identity {leaf_id:?} does not match claimed anchor {:?}",
            claim.anchor
        ));
    }

    // Snapshot binding: the registry says who owns this name.
    let wanted = claim.name.to_ascii_lowercase();
    let bound = snap
        .peer_bindings
        .iter()
        .find(|b| b.name.to_ascii_lowercase() == wanted)
        .ok_or_else(|| {
            format!(
                "snapshot serial {} has no binding for {}",
                snap.serial, claim.name
            )
        })?;
    if bound.anchor != claim.anchor {
        return Err(format!(
            "name {} is bound to a different anchor — refusing cross-claim",
            claim.name
        ));
    }

    // Freshness.
    let age = now.saturating_sub(claim.issued_at_unix);
    if claim.issued_at_unix <= 0 || age >= claim.ttl_seconds as i64 {
        return Err(format!(
            "claim expired (age {}s ≥ ttl {}s) — refusing stale claim",
            age, claim.ttl_seconds
        ));
    }
    let ip = claim
        .ip
        .parse::<IpAddr>()
        .map_err(|e| format!("claim ip {:?} malformed: {e}", claim.ip))?;
    Ok(ValidClaim {
        name: wanted,
        ip,
        age_secs: age,
    })
}

/// PEM convenience wrapper over [`leaf_identity`] — used by the claim
/// publisher to stamp its own anchor with the exact rule verifiers apply.
pub(crate) fn leaf_identity_from_pem(pem_bytes: &[u8]) -> Option<String> {
    let blob = ::pem::parse(pem_bytes).ok()?;
    if blob.tag() != "CERTIFICATE" {
        return None;
    }
    let (_, leaf) = x509_parser::parse_x509_certificate(blob.contents()).ok()?;
    leaf_identity(&leaf)
}

/// Zone of the newest verified snapshot — the suffix the claim publisher
/// appends to this device's peer name.
pub(crate) fn verified_zone(profile: &Profile) -> Option<String> {
    load_verified(profile).ok().map(|s| s.zone)
}

/// The leaf's goat identity, matching the certmint vocabulary: slot-mode
/// CN (bare anchor) first, else the URI-SAN `org=`/`vendor=` value.
fn leaf_identity(leaf: &x509_parser::certificate::X509Certificate<'_>) -> Option<String> {
    if let Some(cn) = leaf
        .subject()
        .iter_common_name()
        .next()
        .and_then(|a| a.as_str().ok())
    {
        if panoscope_core::alias::is_anchor_shaped(cn) {
            return Some(cn.to_string());
        }
    }
    for ext in leaf.extensions() {
        if let x509_parser::extensions::ParsedExtension::SubjectAlternativeName(san) =
            ext.parsed_extension()
        {
            for gn in &san.general_names {
                if let x509_parser::extensions::GeneralName::URI(uri) = gn {
                    for key in ["org=", "vendor="] {
                        if let Some(pos) = uri.find(key) {
                            let v = &uri[pos + key.len()..];
                            if !v.is_empty() {
                                return Some(v.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Newest valid claim for `host` across the daemon store and the context
/// store, verified at read against the given snapshot + root. Best-effort
/// on I/O (missing file = no claims); every refusal is silent here — the
/// resolution path falls through to snapshot/observed which label
/// themselves.
pub(crate) fn lookup_claim(
    profile: &Profile,
    snap: &NameSnapshot,
    root: &VerifyingKey,
    host: &str,
    now: i64,
) -> Option<ValidClaim> {
    let wanted = host.to_ascii_lowercase();
    let mut best: Option<ValidClaim> = None;
    let mut dirs = vec![profile.context_dir()];
    dirs.extend(daemon_names_dirs());
    for dir in dirs {
        let Ok(bytes) = std::fs::read(dir.join(CLAIMS_FILE)) else {
            continue;
        };
        let Ok(envs) = serde_json::from_slice::<Vec<ClaimEnvelope>>(&bytes) else {
            continue;
        };
        for env in &envs {
            match verify_claim(env, snap, root, now) {
                Ok(vc) => {
                    if vc.name == wanted
                        && best
                            .as_ref()
                            .map(|b| vc.age_secs < b.age_secs)
                            .unwrap_or(true)
                    {
                        best = Some(vc);
                    }
                }
                // Every rejection reason (tampered, expired, wrong anchor)
                // used to be discarded with zero output — not even at debug
                // level — which made "no self-claim resolved" and "a
                // self-claim was rejected for a specific reason" impossible
                // to tell apart when diagnosing a name-fallback miss.
                Err(e) => {
                    tracing::debug!(reason = %e, "self-claim rejected");
                }
            }
        }
    }
    best
}

// --- observed tier persistence: one flat JSON file, bounded + pruned ---

fn observed_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join(OBSERVED_FILE)
}

fn load_observed(dir: &std::path::Path) -> Vec<ObservedRecord> {
    std::fs::read(observed_path(dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<ObservedRecord>>(&b).ok())
        .unwrap_or_default()
}

/// Newest valid observation for `host` across the daemon's shared store
/// (when present) and the per-context store — freshest record wins, same
/// noncanonical label either way. goat-cli only ever WRITES its own
/// context store; the daemon's observed file is read-only here.
pub(crate) fn lookup_observed_merged(
    profile: &Profile,
    host: &str,
    now: i64,
) -> Option<ObservedRecord> {
    let mut best = lookup_observed(&profile.context_dir(), host, now);
    for dir in daemon_names_dirs() {
        if let Some(o) = lookup_observed(&dir, host, now) {
            if best
                .as_ref()
                .map(|b| o.observed_at > b.observed_at)
                .unwrap_or(true)
            {
                best = Some(o);
            }
        }
    }
    best
}

/// Newest valid observation for `host`, TTL-pruned at read.
pub(crate) fn lookup_observed(
    dir: &std::path::Path,
    host: &str,
    now: i64,
) -> Option<ObservedRecord> {
    let wanted = host.to_ascii_lowercase();
    load_observed(dir)
        .into_iter()
        .filter(|o| now.saturating_sub(o.observed_at) < OBSERVED_TTL_SECS)
        .find(|o| o.name.to_ascii_lowercase() == wanted)
}

/// Upsert an observation; prune expired entries; cap newest-first. Silent
/// on any I/O failure — the observed tier is best-effort by design.
pub(crate) fn record_observation(dir: &std::path::Path, host: &str, ip: &str, now: i64) {
    let wanted = host.to_ascii_lowercase();
    let mut records = load_observed(dir);
    records.retain(|o| {
        o.name.to_ascii_lowercase() != wanted
            && now.saturating_sub(o.observed_at) < OBSERVED_TTL_SECS
    });
    records.push(ObservedRecord {
        name: wanted,
        ip: ip.to_string(),
        observed_at: now,
    });
    records.sort_by_key(|o| std::cmp::Reverse(o.observed_at));
    records.truncate(OBSERVED_MAX);
    if let Ok(body) = serde_json::to_vec_pretty(&records) {
        let _ = std::fs::write(observed_path(dir), body);
    }
}

/// Grade a snapshot's staleness against `now` (unix seconds). Grading
/// inherits the local clock's UTC confidence (ADR 1037 caveat, accepted —
/// same as cert validity).
pub fn grade_of(snap: &NameSnapshot, now: i64) -> SnapshotMeta {
    let age_secs = now.saturating_sub(snap.generated_at_unix);
    let grade = if snap.generated_at_unix <= 0 || age_secs >= snap.ttl_seconds as i64 {
        Grade::Expired
    } else if age_secs >= FRESH_BOUND_SECS {
        Grade::Aging
    } else {
        Grade::Fresh
    };
    SnapshotMeta {
        serial: snap.serial,
        age_secs,
        grade,
        from_daemon: snap.from_daemon,
    }
}

pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Daemon-store summary for `goat status`'s device verdict: the best
/// validly-signed daemon snapshot (meta + zone), if any daemon store
/// candidate holds one. None = no goat-clientd store on this device
/// (or nothing verifiable — same answer either way: no device-wide
/// protection to report).
pub(crate) fn daemon_store_summary(now: i64) -> Option<(SnapshotMeta, String)> {
    let keys = ca_roots();
    if keys.is_empty() {
        return None;
    }
    let mut best: Option<NameSnapshot> = None;
    for dir in daemon_names_dirs() {
        if let Ok(mut s) = load_pair_any(&dir, &keys) {
            s.from_daemon = true;
            if best.as_ref().map(|b| s.serial >= b.serial).unwrap_or(true) {
                best = Some(s);
            }
        }
    }
    let snap = best?;
    let zone = snap.zone.clone();
    Some((grade_of(&snap, now), zone))
}

fn snapshot_path(profile: &Profile) -> std::path::PathBuf {
    profile.context_dir().join(SNAPSHOT_FILE)
}

fn sig_path(profile: &Profile) -> std::path::PathBuf {
    profile.context_dir().join(SIG_FILE)
}

/// goat-clientd shared-store candidates (§4.1): when the daemon is
/// installed it is the sole refresher and owns the store; goat-cli reads
/// the same bytes with the same verify-at-read rule (no trust cost, no
/// IPC — files + signatures suffice). `GOAT_DAEMON_NAMES_DIR` overrides
/// discovery (set empty to opt out).
fn daemon_names_dirs() -> Vec<std::path::PathBuf> {
    if let Ok(v) = std::env::var("GOAT_DAEMON_NAMES_DIR") {
        if v.is_empty() {
            return Vec::new();
        }
        return vec![v.into()];
    }
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    {
        // System install (LaunchDaemon --bundle-dir), then a user-run daemon.
        dirs.push("/Library/Application Support/goat-client/names".into());
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(
                std::path::Path::new(&home).join("Library/Application Support/goat-client/names"),
            );
        }
    }
    #[cfg(target_os = "linux")]
    {
        // System install (systemd StateDirectory), then a user-run daemon
        // (os.UserConfigDir()/goat-client — the Go daemon's default).
        dirs.push("/var/lib/goat-client/names".into());
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config")));
        if let Some(b) = base {
            dirs.push(b.join("goat-client/names"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(a) = std::env::var_os("APPDATA") {
            dirs.push(std::path::Path::new(&a).join("goat-client/names"));
        }
    }
    dirs
}

/// Read + verify the snapshot pair in one directory.
/// Every CA root this binary will accept — the same set bundle verification
/// uses, including operator-supplied roots from the trust-roots
/// directory.
///
/// This path verified against the FIRST root only until F-410. That is the
/// same one-goatnet bug F-406 fixed for bundles, on a different surface: a
/// name snapshot signed by any goatnet other than the first one was refused,
/// and because the refusal is fail-closed and quiet ("snapshot NOT served"),
/// the symptom is a resolver that silently never has fallback names rather
/// than an error anyone would chase.
///
/// Roots are now operator-supplied and nothing is compiled in, so a machine
/// with no roots resolves nothing from a snapshot — correct, and the same
/// fail-closed answer as an unverifiable one.
fn ca_roots() -> Vec<VerifyingKey> {
    crate::bundle::ca_root_verifying_keys()
        .into_iter()
        .map(|(_, k)| k)
        .collect()
}

/// `load_pair` against every root, returning the first that verifies.
/// Reports the LAST failure when none do, so a genuinely malformed snapshot
/// still says what is wrong with it rather than "wrong CA".
fn load_pair_any(dir: &std::path::Path, keys: &[VerifyingKey]) -> Result<NameSnapshot, String> {
    let mut last = "no CA roots available".to_string();
    for k in keys {
        match load_pair(dir, k) {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// `load_verified_with` against every root. Same contract as `load_pair_any`.
fn load_verified_with_any(
    json: &[u8],
    sig_b64: &[u8],
    keys: &[VerifyingKey],
) -> Result<NameSnapshot, String> {
    let mut last = "no CA roots available".to_string();
    for k in keys {
        match load_verified_with(json, sig_b64, k) {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn load_pair(dir: &std::path::Path, key: &VerifyingKey) -> Result<NameSnapshot, String> {
    let json = std::fs::read(dir.join(SNAPSHOT_FILE)).map_err(|_| "no snapshot".to_string())?;
    let sig = std::fs::read(dir.join(SIG_FILE))
        .map_err(|_| format!("snapshot at {} has no .sig file", dir.display()))?;
    load_verified_with(&json, &sig, key)
}

/// The store-precedence decision, pure for testability: highest
/// validly-signed serial across whichever stores exist wins; the daemon
/// store wins ties (it is the sole refresher — same artifact, honest
/// provenance label).
pub(crate) fn pick_best_snapshot(
    context: Result<NameSnapshot, String>,
    daemon: Vec<NameSnapshot>,
) -> Result<NameSnapshot, String> {
    let mut best = context.ok();
    for snap in daemon {
        if best
            .as_ref()
            .map(|b| snap.serial >= b.serial)
            .unwrap_or(true)
        {
            best = Some(snap);
        }
    }
    best.ok_or_else(|| {
        "no cached snapshot (refreshes from get.<site> while mesh DNS works; an operator can also drop name-snapshot.json{,.sig} into the context dir)".to_string()
    })
}

/// Load the newest validly-signed snapshot across the daemon's shared
/// store (when present) and the per-context cache, verifying the
/// detached signature against the embedded CA root at every use. Any
/// failure returns a human reason and the snapshot is NOT served — fail
/// closed, never silently.
fn load_verified(profile: &Profile) -> Result<NameSnapshot, String> {
    let keys = ca_roots();
    if keys.is_empty() {
        return Err("no CA roots available to verify a name snapshot".to_string());
    }
    let context = load_pair_any(&profile.context_dir(), &keys);
    let daemon = daemon_names_dirs()
        .into_iter()
        .filter_map(|dir| match load_pair_any(&dir, &keys) {
            Ok(mut s) => {
                s.from_daemon = true;
                Some(s)
            }
            Err(reason) => {
                // A present-but-unverifiable daemon store is worth a loud
                // line; an absent one is the normal daemon-less case.
                if dir.join(SNAPSHOT_FILE).exists() {
                    tracing::warn!("names: daemon store at {} refused: {reason}", dir.display());
                }
                None
            }
        })
        .collect();
    pick_best_snapshot(context, daemon)
}

/// Signature + shape verification against an explicit key (unit-testable —
/// production path is [`load_verified`] with the embedded CA root).
pub(crate) fn load_verified_with(
    json: &[u8],
    sig_b64: &[u8],
    key: &VerifyingKey,
) -> Result<NameSnapshot, String> {
    use base64::Engine as _;
    let sig_txt = std::str::from_utf8(sig_b64)
        .map_err(|_| "signature file is not UTF-8".to_string())?
        .trim();
    let der = base64::engine::general_purpose::STANDARD
        .decode(sig_txt)
        .map_err(|e| format!("signature file is not base64: {e}"))?;
    let sig = Signature::from_der(&der).map_err(|e| format!("signature not ASN.1 DER: {e}"))?;
    key.verify(json, &sig)
        .map_err(|_| "snapshot signature invalid — wrong CA or tampered file".to_string())?;
    let snap: NameSnapshot =
        serde_json::from_slice(json).map_err(|e| format!("snapshot malformed: {e}"))?;
    if snap.format != SNAPSHOT_FORMAT {
        return Err(format!("unexpected snapshot format {:?}", snap.format));
    }
    Ok(snap)
}

/// Parse `<proto>/<host>:<port>` (Zenoh wire locator) — shared shape with
/// the status/doctor probes.
pub fn parse_zenoh_locator(s: &str) -> Option<(String, u16)> {
    let after_proto = s.split_once('/')?.1;
    let (host, port) = after_proto.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    Some((host.to_string(), port))
}

/// Substitute a snapshot-resolved address into a Zenoh locator
/// (`proto/host:port` → `proto/ip:port`). Safe on the fabric because TLS
/// trust is the mTLS chain, not the name (`verify_name_on_connect: false`).
pub fn substitute_locator_host(locator: &str, addr: &SocketAddr) -> Option<String> {
    let (proto, _) = locator.split_once('/')?;
    let ip = match addr.ip() {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };
    Some(format!("{proto}/{ip}:{}", addr.port()))
}

/// Monotonic-serial acceptance: a fetched snapshot replaces the cache only
/// when strictly newer (or the cache is absent/unverifiable). The replay
/// bound: a compromised courier can never roll a client back.
pub(crate) fn should_accept(new_serial: u64, cached_serial: Option<u64>) -> bool {
    match cached_serial {
        Some(c) => new_serial > c,
        None => true,
    }
}

/// Best-effort cache refresh from `get.<site>.<zone>` — called from the
/// natural connectivity moments (goat status / goat doctor). Silent on any
/// failure: refresh only matters while names (and the get tier) work.
///
/// One refresher per device (§4.1): when the daemon's shared store holds
/// a fresh, validly-signed snapshot, goat-cli skips its own fetch — the
/// daemon refreshes hourly and goat-cli reads its bytes.
pub async fn refresh(profile: &Profile) {
    if daemon_store_fresh() {
        tracing::debug!(
            "names: daemon store fresh — skipping CLI refresh (daemon is the sole refresher)"
        );
        return;
    }
    let _ = tokio::time::timeout(REFRESH_TIMEOUT, refresh_inner(profile)).await;
}

/// True when any daemon store candidate holds a validly-signed snapshot
/// graded Fresh — the "daemon is refreshing this device" signal.
fn daemon_store_fresh() -> bool {
    let keys = ca_roots();
    if keys.is_empty() {
        return false;
    }
    let now = now_unix();
    daemon_names_dirs()
        .iter()
        .any(|dir| match load_pair_any(dir, &keys) {
            Ok(s) => grade_of(&s, now).grade == Grade::Fresh,
            Err(_) => false,
        })
}

async fn refresh_inner(profile: &Profile) -> Option<()> {
    let base = crate::version_check::get_base_url(profile)?;
    let client = reqwest::Client::builder()
        .timeout(REFRESH_TIMEOUT)
        .build()
        .ok()?;
    let json = client
        .get(format!("{}/{}", base.trim_end_matches('/'), SNAPSHOT_FILE))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .bytes()
        .await
        .ok()?;
    let sig = client
        .get(format!("{}/{}", base.trim_end_matches('/'), SIG_FILE))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .bytes()
        .await
        .ok()?;
    let keys = ca_roots();
    let fetched = match load_verified_with_any(&json, &sig, &keys) {
        Ok(s) => s,
        Err(reason) => {
            tracing::debug!("name-snapshot refresh: fetched artifact refused: {reason}");
            return None;
        }
    };
    let cached_serial = std::fs::read(snapshot_path(profile))
        .ok()
        .zip(std::fs::read(sig_path(profile)).ok())
        .and_then(|(j, s)| load_verified_with_any(&j, &s, &keys).ok())
        .map(|s| s.serial);
    if !should_accept(fetched.serial, cached_serial) {
        tracing::debug!(
            "name-snapshot refresh: fetched serial {} not newer than cached {:?} — kept cache",
            fetched.serial,
            cached_serial
        );
        return None;
    }
    // Claims ride the same tier, best-effort, no serial semantics (each
    // claim is individually leaf-signed, verified at read).
    if let Ok(resp) = client
        .get(format!("{}/{}", base.trim_end_matches('/'), CLAIMS_FILE))
        .send()
        .await
    {
        if let Ok(body) = resp.error_for_status().map(|r| r.bytes()) {
            if let Ok(bytes) = body.await {
                if serde_json::from_slice::<Vec<ClaimEnvelope>>(&bytes).is_ok() {
                    let _ = std::fs::write(profile.context_dir().join(CLAIMS_FILE), &bytes);
                }
            }
        }
    }
    // Write sig first, snapshot last — a torn write leaves an unverifiable
    // pair, which load_verified refuses (fail closed), never a mismatched
    // accept.
    std::fs::write(sig_path(profile), &sig).ok()?;
    std::fs::write(snapshot_path(profile), &json).ok()?;
    tracing::debug!(
        "name-snapshot refresh: cached serial {} (site {}, zone {}, ca {}) for context {}",
        fetched.serial,
        fetched.site_id,
        fetched.zone,
        fetched.ca_id,
        profile.context
    );
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use p256::ecdsa::{signature::Signer, SigningKey};

    fn fixture_snapshot_json(serial: u64, generated_at_unix: i64, ttl: u64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "format": "goat-name-snapshot-v1",
            "site_id": "testsite",
            "zone": "netbird.example.net",
            "serial": serial,
            "generated_at": "2026-07-15T00:00:00Z",
            "generated_at_unix": generated_at_unix,
            "ttl_seconds": ttl,
            "ca_id": "test-ca",
            "records": [
                {"name": "portal.testsite.netbird.example.net", "ip": "100.127.0.10"}
            ]
        }))
        .unwrap()
    }

    fn sign_b64(key: &SigningKey, msg: &[u8]) -> Vec<u8> {
        let sig: Signature = key.sign(msg);
        base64::engine::general_purpose::STANDARD
            .encode(sig.to_der())
            .into_bytes()
    }

    #[test]
    fn verify_roundtrip_and_tamper_refused() {
        let sk = SigningKey::generate();
        let vk = VerifyingKey::from(&sk);
        let json = fixture_snapshot_json(3, 1_000, 100);
        let sig = sign_b64(&sk, &json);
        let snap = load_verified_with(&json, &sig, &vk).expect("valid pair verifies");
        assert_eq!(snap.serial, 3);
        assert_eq!(snap.records[0].ip, "100.127.0.10");

        let mut tampered = json.clone();
        // Flip the IP inside the signed bytes.
        let s = String::from_utf8(tampered.clone())
            .unwrap()
            .replace("100.127.0.10", "6.6.6.6");
        tampered = s.into_bytes();
        let err = load_verified_with(&tampered, &sig, &vk).unwrap_err();
        assert!(err.contains("signature invalid"), "{err}");
    }

    #[test]
    fn wrong_key_refused() {
        let sk = SigningKey::generate();
        let other = SigningKey::generate();
        let json = fixture_snapshot_json(1, 1_000, 100);
        let sig = sign_b64(&sk, &json);
        assert!(load_verified_with(&json, &sig, &VerifyingKey::from(&other)).is_err());
    }

    #[test]
    fn grading_boundaries() {
        let snap: NameSnapshot =
            serde_json::from_slice(&fixture_snapshot_json(7, 1_000_000, 2_592_000)).unwrap();
        // Fresh: 1 day old.
        let m = grade_of(&snap, 1_000_000 + 86_400);
        assert_eq!(m.grade, Grade::Fresh);
        assert_eq!(m.serial, 7);
        assert_eq!(m.age_human(), "1d");
        // Aging: exactly the 7-day boundary.
        assert_eq!(
            grade_of(&snap, 1_000_000 + FRESH_BOUND_SECS).grade,
            Grade::Aging
        );
        // Expired: exactly at TTL.
        assert_eq!(grade_of(&snap, 1_000_000 + 2_592_000).grade, Grade::Expired);
        // Zero/absent generated_at_unix is never servable.
        let mut zeroed = snap.clone();
        zeroed.generated_at_unix = 0;
        assert_eq!(grade_of(&zeroed, 1_000).grade, Grade::Expired);
    }

    fn snap_with(records: serde_json::Value, generated_at_unix: i64) -> NameSnapshot {
        serde_json::from_value(serde_json::json!({
            "format": "goat-name-snapshot-v1",
            "site_id": "testsite",
            "zone": "netbird.example.net",
            "serial": 5,
            "generated_at": "",
            "generated_at_unix": generated_at_unix,
            "ttl_seconds": 2_592_000,
            "ca_id": "test-ca",
            "records": records
        }))
        .unwrap()
    }

    fn obs(name: &str, ip: &str, observed_at: i64) -> ObservedRecord {
        ObservedRecord {
            name: name.into(),
            ip: ip.into(),
            observed_at,
        }
    }

    const HOST: &str = "portal.testsite.netbird.example.net";

    #[test]
    fn fallback_snapshot_wins_when_no_observation() {
        let snap = snap_with(
            serde_json::json!([{"name": HOST, "ip": "100.127.0.10"}]),
            1_000,
        );
        match pick_fallback(Ok(snap), None, HOST, 2_000) {
            FallbackPick::Snapshot(meta, ip) => {
                assert_eq!(meta.serial, 5);
                assert_eq!(ip.to_string(), "100.127.0.10");
            }
            other => panic!("expected snapshot pick, got {other:?}"),
        }
    }

    #[test]
    fn fallback_observed_gap_fills_names_absent_from_snapshot() {
        let snap = snap_with(serde_json::json!([]), 1_000);
        let o = obs(HOST, "100.127.0.44", 1_500);
        match pick_fallback(Ok(snap), Some(&o), HOST, 2_000) {
            FallbackPick::Observed { ip, age_secs, .. } => {
                assert_eq!(ip.to_string(), "100.127.0.44");
                assert_eq!(age_secs, 500);
            }
            other => panic!("expected observed gap-fill, got {other:?}"),
        }
    }

    #[test]
    fn fallback_conflict_fresher_observation_supersedes_snapshot() {
        // The operator-hotfix case: observation newer than generated_at wins.
        let snap = snap_with(
            serde_json::json!([{"name": HOST, "ip": "100.127.0.10"}]),
            1_000,
        );
        let o = obs(HOST, "100.127.0.42", 1_500);
        match pick_fallback(Ok(snap), Some(&o), HOST, 2_000) {
            FallbackPick::Observed { ip, .. } => assert_eq!(ip.to_string(), "100.127.0.42"),
            other => panic!("expected fresher observation to win, got {other:?}"),
        }
    }

    #[test]
    fn fallback_conflict_older_observation_loses_to_snapshot() {
        let snap = snap_with(
            serde_json::json!([{"name": HOST, "ip": "100.127.0.10"}]),
            1_000,
        );
        let o = obs(HOST, "100.127.0.42", 900); // pre-dates the snapshot
        match pick_fallback(Ok(snap), Some(&o), HOST, 2_000) {
            FallbackPick::Snapshot(_, ip) => assert_eq!(ip.to_string(), "100.127.0.10"),
            other => panic!("expected snapshot to win over stale observation, got {other:?}"),
        }
    }

    #[test]
    fn fallback_observed_serves_when_snapshot_unavailable_or_expired() {
        let o = obs(HOST, "100.127.0.27", 1_500);
        // Snapshot missing entirely.
        match pick_fallback(Err("no cached snapshot".into()), Some(&o), HOST, 2_000) {
            FallbackPick::Observed { .. } => {}
            other => panic!("expected observed on missing snapshot, got {other:?}"),
        }
        // Snapshot expired (ttl 100, age 10_000).
        let expired = {
            let mut s = snap_with(
                serde_json::json!([{"name": HOST, "ip": "100.127.0.10"}]),
                1_000,
            );
            s.ttl_seconds = 100;
            s
        };
        match pick_fallback(Ok(expired), Some(&o), HOST, 11_000) {
            FallbackPick::Observed { .. } => {}
            other => panic!("expected observed on expired snapshot, got {other:?}"),
        }
        // Neither tier answers → both reasons in the failure.
        match pick_fallback(Err("no cached snapshot".into()), None, HOST, 2_000) {
            FallbackPick::None { reason } => {
                assert!(reason.contains("no cached snapshot"), "{reason}");
                assert!(reason.contains("no observed record"), "{reason}");
            }
            other => panic!("expected none, got {other:?}"),
        }
    }

    #[test]
    fn observed_store_roundtrip_prune_and_cap() {
        let dir = tempfile::tempdir().unwrap();
        let now = 10_000_000;
        record_observation(dir.path(), "A.example", "100.127.0.1", now - 100);
        record_observation(dir.path(), "b.example", "100.127.0.2", now);
        // Upsert replaces, case-insensitively.
        record_observation(dir.path(), "a.EXAMPLE", "100.127.0.9", now);
        let hit = lookup_observed(dir.path(), "A.example", now).unwrap();
        assert_eq!(hit.ip, "100.127.0.9");
        // TTL prune at read: an entry past OBSERVED_TTL_SECS is invisible…
        record_observation(
            dir.path(),
            "old.example",
            "100.127.0.3",
            now - OBSERVED_TTL_SECS - 1,
        );
        assert!(lookup_observed(dir.path(), "old.example", now).is_none());
        // …and dropped from disk by the next write.
        record_observation(dir.path(), "c.example", "100.127.0.4", now);
        let on_disk = load_observed(dir.path());
        assert!(on_disk.iter().all(|o| o.name != "old.example"));
        assert!(on_disk.len() <= OBSERVED_MAX);
    }

    #[test]
    fn serial_acceptance_is_monotonic() {
        assert!(should_accept(2, Some(1)));
        assert!(!should_accept(1, Some(1)));
        assert!(!should_accept(1, Some(2)));
        assert!(should_accept(1, None));
    }

    fn snap_serial(serial: u64, from_daemon: bool) -> NameSnapshot {
        let mut s = snap_with(serde_json::json!([]), 1_000);
        s.serial = serial;
        s.from_daemon = from_daemon;
        s
    }

    #[test]
    fn best_snapshot_highest_valid_serial_wins_daemon_ties() {
        // Daemon serial higher → daemon wins.
        let picked =
            pick_best_snapshot(Ok(snap_serial(3, false)), vec![snap_serial(5, true)]).unwrap();
        assert_eq!((picked.serial, picked.from_daemon), (5, true));
        // Context serial higher → context cache wins (stale daemon store
        // never rolls a reader back).
        let picked =
            pick_best_snapshot(Ok(snap_serial(7, false)), vec![snap_serial(5, true)]).unwrap();
        assert_eq!((picked.serial, picked.from_daemon), (7, false));
        // Tie → daemon wins (sole refresher; same artifact, honest label).
        let picked =
            pick_best_snapshot(Ok(snap_serial(5, false)), vec![snap_serial(5, true)]).unwrap();
        assert!(picked.from_daemon);
        // No context cache → daemon store alone serves.
        let picked =
            pick_best_snapshot(Err("no cached snapshot".into()), vec![snap_serial(2, true)])
                .unwrap();
        assert_eq!((picked.serial, picked.from_daemon), (2, true));
        // Nothing anywhere → the human reason survives.
        let err = pick_best_snapshot(Err("nope".into()), vec![]).unwrap_err();
        assert!(err.contains("no cached snapshot"), "{err}");
    }

    #[test]
    fn grade_of_carries_store_provenance() {
        let snap = snap_serial(5, true);
        assert!(grade_of(&snap, 2_000).from_daemon);
        assert!(!grade_of(&snap_serial(5, false), 2_000).from_daemon);
    }

    // --- self-claims (ADR 1082 Amendment 2) ---

    const ANCHOR: &str = "0123456789abcdef0123456789abcdef"; // anchor-shaped (32 hex)
    const OTHER_ANCHOR: &str = "ffff6789abcdef0123456789abcdef00";
    const PEER: &str = "peer-tester.testsite.netbird.example.net";

    struct ClaimFixture {
        root_vk: VerifyingKey,
        leaf_pem: String,
        leaf_signer: SigningKey,
    }

    /// Mint a real test CA + a leaf (CN = anchor-shaped) signed by it.
    fn claim_fixture(cn: &str) -> ClaimFixture {
        use rcgen::{
            BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256,
        };
        let root_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let mut root_params = CertificateParams::new(vec![]).unwrap();
        root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let root_cert = root_params.self_signed(&root_kp).unwrap();

        let leaf_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let mut leaf_params = CertificateParams::new(vec![]).unwrap();
        leaf_params.distinguished_name.push(DnType::CommonName, cn);
        let leaf_cert = leaf_params
            .signed_by(&leaf_kp, &root_cert, &root_kp)
            .unwrap();

        use p256::pkcs8::DecodePublicKey as _;
        let root_vk =
            VerifyingKey::from_public_key_der(&root_kp.public_key_der()).expect("root vk");
        use p256::pkcs8::DecodePrivateKey as _;
        let leaf_signer =
            SigningKey::from_pkcs8_der(&leaf_kp.serialize_der()).expect("leaf signer");
        ClaimFixture {
            root_vk,
            leaf_pem: leaf_cert.pem(),
            leaf_signer,
        }
    }

    fn envelope(
        fix: &ClaimFixture,
        name: &str,
        anchor: &str,
        issued_at: i64,
        ttl: u64,
    ) -> ClaimEnvelope {
        use base64::Engine as _;
        let b64 = &base64::engine::general_purpose::STANDARD;
        let artifact = serde_json::to_vec(&ClaimArtifact {
            format: "goat-name-claim-v1".into(),
            name: name.into(),
            ip: "100.127.0.67".into(),
            anchor: anchor.into(),
            issued_at_unix: issued_at,
            ttl_seconds: ttl,
        })
        .unwrap();
        let sig: Signature = fix.leaf_signer.sign(&artifact);
        ClaimEnvelope {
            artifact_b64: b64.encode(&artifact),
            sig_b64: b64.encode(sig.to_der()),
            leaf_pem: fix.leaf_pem.clone(),
        }
    }

    fn snap_with_binding(name: &str, anchor: &str) -> NameSnapshot {
        let mut s = snap_with(
            serde_json::json!([{"name": "portal.testsite.netbird.example.net", "ip": "100.127.0.10"}]),
            1_000,
        );
        s.peer_bindings = vec![PeerBinding {
            name: name.into(),
            anchor: anchor.into(),
        }];
        s
    }

    #[test]
    fn claim_verifies_and_serves() {
        let fix = claim_fixture(ANCHOR);
        let now = 1_784_000_000; // 2026 — inside rcgen's default validity
        let env = envelope(&fix, PEER, ANCHOR, now - 60, 3600);
        let snap = snap_with_binding(PEER, ANCHOR);
        let vc = verify_claim(&env, &snap, &fix.root_vk, now).expect("valid claim verifies");
        assert_eq!(vc.name, PEER);
        assert_eq!(vc.ip.to_string(), "100.127.0.67");
        assert_eq!(vc.age_secs, 60);
    }

    #[test]
    fn claim_verifies_with_ed25519_leaf() {
        // The usual shape: Ed25519 leaf key, P-256 CA signature.
        use rcgen::{
            BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256,
            PKCS_ED25519,
        };
        let root_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let mut root_params = CertificateParams::new(vec![]).unwrap();
        root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let root_cert = root_params.self_signed(&root_kp).unwrap();
        let leaf_kp = KeyPair::generate_for(&PKCS_ED25519).unwrap();
        let mut leaf_params = CertificateParams::new(vec![]).unwrap();
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, ANCHOR);
        let leaf_cert = leaf_params
            .signed_by(&leaf_kp, &root_cert, &root_kp)
            .unwrap();
        use p256::pkcs8::DecodePublicKey as _;
        let root_vk = VerifyingKey::from_public_key_der(&root_kp.public_key_der()).unwrap();
        use ed25519_dalek::pkcs8::DecodePrivateKey as _;
        let signer = ed25519_dalek::SigningKey::from_pkcs8_der(&leaf_kp.serialize_der()).unwrap();

        use base64::Engine as _;
        let b64 = &base64::engine::general_purpose::STANDARD;
        let now = 1_784_000_000;
        let artifact = serde_json::to_vec(&ClaimArtifact {
            format: "goat-name-claim-v1".into(),
            name: PEER.into(),
            ip: "100.127.0.67".into(),
            anchor: ANCHOR.into(),
            issued_at_unix: now - 60,
            ttl_seconds: 3600,
        })
        .unwrap();
        use ed25519_dalek::Signer as _;
        let env = ClaimEnvelope {
            artifact_b64: b64.encode(&artifact),
            sig_b64: b64.encode(signer.sign(&artifact).to_bytes()),
            leaf_pem: leaf_cert.pem(),
        };
        let snap = snap_with_binding(PEER, ANCHOR);
        let vc = verify_claim(&env, &snap, &root_vk, now).expect("ed25519 claim verifies");
        assert_eq!(vc.ip.to_string(), "100.127.0.67");

        // Tampered artifact refused under ed25519 too.
        let mut bad = env.clone();
        let flipped = String::from_utf8(artifact.clone())
            .unwrap()
            .replace("100.127.0.67", "6.6.6.6");
        bad.artifact_b64 = b64.encode(flipped.as_bytes());
        assert!(verify_claim(&bad, &snap, &root_vk, now)
            .unwrap_err()
            .contains("signature invalid"));
    }

    #[test]
    fn claim_refusals_fail_closed() {
        let fix = claim_fixture(ANCHOR);
        let now = 1_784_000_000; // 2026 — inside rcgen's default validity
        let snap = snap_with_binding(PEER, ANCHOR);

        // Cross-claim: name bound to a different anchor.
        let cross = snap_with_binding(PEER, OTHER_ANCHOR);
        let env = envelope(&fix, PEER, ANCHOR, now - 60, 3600);
        let err = verify_claim(&env, &cross, &fix.root_vk, now).unwrap_err();
        assert!(err.contains("different anchor"), "{err}");

        // Service name (no binding) can never be claimed.
        let svc = envelope(
            &fix,
            "portal.testsite.netbird.example.net",
            ANCHOR,
            now - 60,
            3600,
        );
        let err = verify_claim(&svc, &snap, &fix.root_vk, now).unwrap_err();
        assert!(err.contains("no binding"), "{err}");

        // Expired claim refused.
        let old = envelope(&fix, PEER, ANCHOR, now - 4_000, 3600);
        let err = verify_claim(&old, &snap, &fix.root_vk, now).unwrap_err();
        assert!(err.contains("expired"), "{err}");

        // Leaf identity != claimed anchor (stolen-name attempt with own leaf).
        let liar = envelope(&fix, PEER, OTHER_ANCHOR, now - 60, 3600);
        let err = verify_claim(
            &liar,
            &snap_with_binding(PEER, OTHER_ANCHOR),
            &fix.root_vk,
            now,
        )
        .unwrap_err();
        assert!(err.contains("does not match claimed anchor"), "{err}");

        // Leaf from a different CA refused.
        let other_ca = claim_fixture(ANCHOR);
        let env2 = envelope(&other_ca, PEER, ANCHOR, now - 60, 3600);
        let err = verify_claim(&env2, &snap, &fix.root_vk, now).unwrap_err();
        assert!(err.contains("not signed by the embedded CA root"), "{err}");

        // Tampered artifact refused.
        let mut tampered = envelope(&fix, PEER, ANCHOR, now - 60, 3600);
        use base64::Engine as _;
        let b64 = &base64::engine::general_purpose::STANDARD;
        let bytes = b64.decode(&tampered.artifact_b64).unwrap();
        let flipped = String::from_utf8(bytes)
            .unwrap()
            .replace("100.127.0.67", "6.6.6.6");
        tampered.artifact_b64 = b64.encode(flipped.as_bytes());
        let err = verify_claim(&tampered, &snap, &fix.root_vk, now).unwrap_err();
        assert!(err.contains("signature invalid"), "{err}");
    }

    #[test]
    fn age_human_units() {
        let m = |age_secs| SnapshotMeta {
            serial: 1,
            age_secs,
            grade: Grade::Fresh,
            from_daemon: false,
        };
        assert_eq!(m(120).age_human(), "2m");
        assert_eq!(m(7_200).age_human(), "2h");
        assert_eq!(m(200_000).age_human(), "2d");
    }
}
