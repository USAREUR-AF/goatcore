//! Per-peering values, allocated and never hand-picked.
//!
//! A peering needs values that no agreement carries and that two peerings on
//! one edge host or one core host must not share: the WireGuard interface and
//! its listen port, the link subnet we propose, the SPIRE outbound forward
//! port, the republisher's metrics port, the certificate directory, the
//! inventory names and groups, and (behind a forwarder) the carrier's interface,
//! ports and subnet and the forwarder's public port. This module allocates them.
//!
//! WHERE A VALUE COMES FROM, in order, one owner per field:
//!
//! 1. a pin: the peering's `values:` in goatbuilder.yaml, written by a person;
//! 2. the allocation record: `peering-values.yaml` beside goatbuilder.yaml,
//!    written by `goatbuilder federate allocate` and committed to the
//!    deployment's repository like any reviewed change;
//! 3. a fresh allocation: the first candidate in the field's sequence that no
//!    other peering of the same environment holds.
//!
//! WHY A RECORD, AND NOT ONLY A DETERMINISTIC ORDER. Deriving values from a
//! peering's position in the list moves every later peering's values when an
//! earlier one is removed or the list is reordered, and hashing a name into a
//! slot collides. The record makes allocation stable by construction: a value
//! once recorded is kept, and a new peering takes only what nobody holds. It is
//! also the reviewable proposal: the change the tool makes to the deployment's
//! own configuration is a diff of this file, read before it is applied.
//!
//! Defaults belong to no deployment: port numbers, link-local ranges, and the
//! working-directory root the driver already uses.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::{Config, Peering};

/// The allocation record's file name, beside goatbuilder.yaml.
pub const RECORD_FILE: &str = "peering-values.yaml";
pub const RECORD_SCHEMA: &str = "goatbuilder-peering-values/v1";

/// The root of every peering's working directory (the driver's default
/// `/var/lib/goat-federation/<peering>`).
pub const DEFAULT_WORK_ROOT: &str = "/var/lib/goat-federation";
/// UDP ports an edge host already holds: NetBird's WireGuard port.
pub const DEFAULT_RESERVED_EDGE_UDP: &[u16] = &[51820];

// The candidate sequences. Index 0 is what the first peering gets.
const WG_LISTEN_BASE: u32 = 51823; // odd ports from here: 51823, 51825, ...
const CARRIER_LISTEN_BASE: u32 = 51822; // even ports from here: 51822, 51824, ...
const PUBLIC_PORT_BASE: u32 = 51820; // on the forwarder: 51820, 51822, ...
const CARRIER_PORT_BASE: u32 = 51821; // on the forwarder: 51821, 51823, ...
const METRICS_BASE: u32 = 9310;
const SPIRE_OUT_BASE: u32 = 18443;
/// The edge router's partner-facing port. The SAME for every peering: `goat
/// federate apply` binds each edge router to its own link address, so two
/// edges on one host do not collide on it, and the partner dials this port
/// number on our link address. A different port per peering would break that
/// dial for a partner that kept the role default.
pub const EDGE_LISTEN_PORT: u16 = 7448;
const LINK_NET: [u8; 3] = [169, 254, 10];
const CARRIER_NET: [u8; 3] = [169, 254, 30];
const MAX_INDEX: u32 = 128;

/// The allocated values of one peering. Every workstream that needs a
/// per-peering value reads it from here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PeeringValues {
    /// The `with` id.
    pub peering: String,
    /// The environment the peering is declared on.
    pub environment: String,
    /// wg-fed0, wg-fed1, ...
    pub wg_interface: String,
    /// The edge's inner WireGuard listen port.
    pub wg_listen_port: u16,
    /// The forwarder's public port the partner dials, when a forwarder relays.
    pub public_port: Option<u16>,
    /// The /31 we propose for the link (`proposed_link_subnet`).
    pub link_subnet: String,
    /// The edge zenoh router's partner-facing port.
    pub edge_listen_port: u16,
    /// The outbound republisher's metrics port on the core host.
    pub republisher_metrics_port: u16,
    /// The edge's SPIRE bundle forward port (peering_spire_outbound_port).
    pub spire_outbound_port: u16,
    /// The peering's certificate directory: phase 8 writes it on the machine
    /// running the driver, and the edge reads its certificates from the same path.
    pub cert_dir: String,
    /// The inventory group of the edge.
    pub edge_group: String,
    /// The inventory group of the core.
    pub core_group: String,
    /// The edge machine's inventory name for this peering: one name per peering
    /// per machine, so two peerings' group_vars never merge on one inventory host.
    pub edge_host_name: String,
    /// The core machine's inventory name for this peering.
    pub core_host_name: String,
    /// Behind a forwarder: the edge's carrier interface (wg-carry0, ...).
    pub carrier_interface: Option<String>,
    /// Behind a forwarder: the edge's carrier listen port.
    pub carrier_listen_port: Option<u16>,
    /// Behind a forwarder: the forwarder's carrier port the edge dials.
    pub carrier_port: Option<u16>,
    /// Behind a forwarder: the carrier's /31 (forwarder the first address, edge the second).
    pub carrier_subnet: Option<String>,
    /// The fields a pin set, for display.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pinned: Vec<String>,
}

/// Pins (goatbuilder.yaml `values:`) and the record's entries: the same keys as
/// PeeringValues, each optional (declared in federate/declaration.rs).
pub use super::declaration::PeeringPins;

/// The allocation record, `peering-values.yaml`: one entry per peering, keyed
/// `<environment>/<with>`, holding only the values the tool allocated (a pinned
/// field is owned by the pin and is not recorded).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema: String,
    #[serde(default)]
    pub peerings: BTreeMap<String, PeeringPins>,
}

/// What allocation reads besides the config.
#[derive(Debug, Clone)]
pub struct AllocContext {
    /// The committed allocation record (empty when there is none yet).
    pub record: Record,
    /// The root of the peerings' working directories.
    pub work_root: String,
    /// UDP ports the edge host already holds.
    pub reserved_edge_udp: Vec<u16>,
}

impl Default for AllocContext {
    fn default() -> Self {
        AllocContext {
            record: Record {
                schema: RECORD_SCHEMA.into(),
                peerings: BTreeMap::new(),
            },
            work_root: DEFAULT_WORK_ROOT.into(),
            reserved_edge_udp: DEFAULT_RESERVED_EDGE_UDP.to_vec(),
        }
    }
}

impl AllocContext {
    /// The context for a config directory: its record, when there is one.
    pub fn load(config_dir: &Path) -> Result<Self> {
        let path = config_dir.join(RECORD_FILE);
        let mut ctx = AllocContext::default();
        if path.is_file() {
            ctx.record = read_record(&path)?;
        }
        Ok(ctx)
    }
}

/// Read and check an allocation record.
pub fn read_record(path: &Path) -> Result<Record> {
    let bad = |why: String| {
        goat_errors::coded(
            "GB-124",
            format!(
                "{} cannot be used as the peering allocation record (GB-124): {why}. Fix it from \
                 its history (git log -p {}), or remove the entry and run `goatbuilder federate \
                 allocate` again to allocate it afresh",
                path.display(),
                path.display()
            ),
        )
    };
    let text = std::fs::read_to_string(path).map_err(|e| bad(e.to_string()))?;
    let rec: Record = serde_yaml::from_str(&text).map_err(|e| bad(e.to_string()))?;
    if rec.schema != RECORD_SCHEMA {
        return Err(bad(format!(
            "schema is '{}', expected '{RECORD_SCHEMA}'",
            rec.schema
        ))
        .into());
    }
    Ok(rec)
}

/// One problem with the declaration or its pins: the catalog code and the
/// message, which names the fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub code: &'static str,
    pub message: String,
}

impl Problem {
    fn new(code: &'static str, message: String) -> Self {
        Problem { code, message }
    }
}

fn key(p: &Peering) -> String {
    format!("{}/{}", p.environment, p.with)
}

/// The UDP ports an environment's edge hosts hold: its
/// `peering_edge_reserved_udp`, else the context's default (NetBird's 51820).
fn reserved_udp(cfg: &Config, ctx: &AllocContext, env: &str) -> Vec<u16> {
    cfg.environments
        .iter()
        .find(|e| e.name == env)
        .and_then(|e| e.peering_edge_reserved_udp.clone())
        .unwrap_or_else(|| ctx.reserved_edge_udp.clone())
}

/// The values of one peering, from a full allocation of every peering.
#[allow(dead_code)] // the one-peering form of allocate_all, for the driver's later stages
pub fn allocate(cfg: &Config, with: &str, ctx: &AllocContext) -> Result<PeeringValues> {
    allocate_in(cfg, None, with, ctx)
}

/// `allocate`, narrowed to one environment when the same partner is declared on two.
pub fn allocate_in(
    cfg: &Config,
    env: Option<&str>,
    with: &str,
    ctx: &AllocContext,
) -> Result<PeeringValues> {
    let all = allocate_all(cfg, ctx)?;
    let mut hits: Vec<PeeringValues> = all
        .into_iter()
        .filter(|v| v.peering == with && env.map(|e| v.environment == e).unwrap_or(true))
        .collect();
    match hits.len() {
        1 => Ok(hits.remove(0)),
        0 => Err(anyhow::anyhow!("peering '{with}' is not declared in goatbuilder.yaml peerings: — declare it first (C1)")),
        _ => Err(anyhow::anyhow!("peering '{with}' is declared on several environments; pass --env")),
    }
}

/// Every declared peering's values. No two peerings of one environment share an
/// interface, a UDP port on the edge, a subnet, a TCP port on the edge or the
/// core, or a forwarder port; no two peerings at all share a group, an inventory
/// name or a certificate directory. Refuses with the first problem's catalog
/// code, every problem listed.
pub fn allocate_all(cfg: &Config, ctx: &AllocContext) -> Result<Vec<PeeringValues>> {
    let (values, problems) = run(cfg, ctx);
    if let Some(first) = problems.first() {
        let text = problems
            .iter()
            .map(|p| p.message.as_str())
            .collect::<Vec<_>>()
            .join("\n  ");
        return Err(goat_errors::coded(first.code, text).into());
    }
    Ok(values)
}

/// The problems a config's peerings have on their own, with no record: what
/// `goatbuilder validate` reports. Every line names its catalog code.
pub fn declaration_problems_in(cfg: &Config, record_dir: &Path) -> Vec<String> {
    match AllocContext::load(record_dir) {
        Ok(ctx) => run(cfg, &ctx).1.into_iter().map(|p| p.message).collect(),
        Err(e) => vec![e.to_string()],
    }
}

/// `declaration_problems_in` with no record: the declaration on its own.
pub fn declaration_problems(cfg: &Config) -> Vec<String> {
    run(cfg, &AllocContext::default())
        .1
        .into_iter()
        .map(|p| p.message)
        .collect()
}

// ------------------------------------------------------------------ the core

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
enum Field {
    WgInterface,
    WgListenPort,
    PublicPort,
    LinkSubnet,
    EdgeListenPort,
    RepublisherMetricsPort,
    SpireOutboundPort,
    CertDir,
    EdgeGroup,
    CoreGroup,
    EdgeHostName,
    CoreHostName,
    CarrierInterface,
    CarrierListenPort,
    CarrierPort,
    CarrierSubnet,
}

const FIELDS: &[Field] = &[
    Field::WgInterface,
    Field::WgListenPort,
    Field::PublicPort,
    Field::LinkSubnet,
    Field::EdgeListenPort,
    Field::RepublisherMetricsPort,
    Field::SpireOutboundPort,
    Field::CertDir,
    Field::EdgeGroup,
    Field::CoreGroup,
    Field::EdgeHostName,
    Field::CoreHostName,
    Field::CarrierInterface,
    Field::CarrierListenPort,
    Field::CarrierPort,
    Field::CarrierSubnet,
];

impl Field {
    fn name(self) -> &'static str {
        match self {
            Field::WgInterface => "wg_interface",
            Field::WgListenPort => "wg_listen_port",
            Field::PublicPort => "public_port",
            Field::LinkSubnet => "link_subnet",
            Field::EdgeListenPort => "edge_listen_port",
            Field::RepublisherMetricsPort => "republisher_metrics_port",
            Field::SpireOutboundPort => "spire_outbound_port",
            Field::CertDir => "cert_dir",
            Field::EdgeGroup => "edge_group",
            Field::CoreGroup => "core_group",
            Field::EdgeHostName => "edge_host_name",
            Field::CoreHostName => "core_host_name",
            Field::CarrierInterface => "carrier_interface",
            Field::CarrierListenPort => "carrier_listen_port",
            Field::CarrierPort => "carrier_port",
            Field::CarrierSubnet => "carrier_subnet",
        }
    }

    /// Only a peering a forwarder relays has this value.
    fn forwarder_only(self) -> bool {
        matches!(
            self,
            Field::PublicPort
                | Field::CarrierInterface
                | Field::CarrierListenPort
                | Field::CarrierPort
                | Field::CarrierSubnet
        )
    }

    /// The pool the value must be unique in. `None`: shared on purpose.
    fn pool(self) -> Option<Pool> {
        Some(match self {
            Field::WgInterface | Field::CarrierInterface => Pool::Interface,
            Field::WgListenPort | Field::CarrierListenPort => Pool::EdgeUdp,
            Field::LinkSubnet | Field::CarrierSubnet => Pool::Subnet,
            Field::SpireOutboundPort => Pool::EdgeTcp,
            Field::RepublisherMetricsPort => Pool::CoreTcp,
            Field::PublicPort | Field::CarrierPort => Pool::ForwarderUdp,
            Field::CertDir => Pool::CertDir,
            Field::EdgeGroup | Field::CoreGroup | Field::EdgeHostName | Field::CoreHostName => {
                Pool::InventoryName
            }
            Field::EdgeListenPort => return None,
        })
    }
}

/// Every (pool, place) a field's value must be unique in for one peering. A
/// place is the environment (its edge and core hosts) or, for what lives on a
/// peering forwarder, that forwarder's endpoint: one forwarder may relay
/// peerings of several environments. Carrier interfaces and subnets are on
/// both the edge and the forwarder, so they are unique in both.
fn places(f: Field, p: &Peering) -> Vec<(Pool, String)> {
    let fwd = p
        .forwarder
        .as_ref()
        .map(|x| format!("forwarder {}", x.endpoint));
    let mut out = Vec::new();
    match (f, fwd) {
        (Field::PublicPort | Field::CarrierPort, Some(w)) => out.push((Pool::ForwarderUdp, w)),
        (Field::CarrierInterface, Some(w)) => {
            out.push((Pool::Interface, p.environment.clone()));
            out.push((Pool::ForwarderInterface, w));
        }
        (Field::CarrierSubnet, Some(w)) => {
            out.push((Pool::Subnet, p.environment.clone()));
            out.push((Pool::ForwarderSubnet, w));
        }
        _ => {
            if let Some(pool) = f.pool() {
                out.push((pool, p.environment.clone()));
            }
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Pool {
    Interface,
    EdgeUdp,
    Subnet,
    EdgeTcp,
    CoreTcp,
    ForwarderUdp,
    ForwarderInterface,
    ForwarderSubnet,
    CertDir,
    InventoryName,
}

impl Pool {
    /// Pools scoped to one environment's hosts; the rest span the repository
    /// (one inventory, one set of group_vars directories).
    fn per_environment(self) -> bool {
        // Certificate directories are on the machine running the driver, which
        // may run the peerings of every environment.
        !matches!(self, Pool::InventoryName | Pool::CertDir)
    }

    fn what(self) -> &'static str {
        match self {
            Pool::Interface => "interface on the edge host",
            Pool::EdgeUdp => "UDP port on the edge host",
            Pool::Subnet => "link-local subnet on the edge host",
            Pool::EdgeTcp => "TCP port on the edge host",
            Pool::CoreTcp => "TCP port on the core host",
            Pool::ForwarderUdp => "UDP port on the forwarder",
            Pool::ForwarderInterface => "carrier interface on the forwarder",
            Pool::ForwarderSubnet => "carrier subnet on the forwarder",
            Pool::CertDir => "certificate directory",
            Pool::InventoryName => "inventory name",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Val {
    S(String),
    P(u16),
}

impl Val {
    fn show(&self) -> String {
        match self {
            Val::S(s) => s.clone(),
            Val::P(p) => p.to_string(),
        }
    }
}

fn pins_get(p: &PeeringPins, f: Field) -> Option<Val> {
    let s = |v: &Option<String>| v.clone().map(Val::S);
    let n = |v: &Option<u16>| v.map(Val::P);
    match f {
        Field::WgInterface => s(&p.wg_interface),
        Field::WgListenPort => n(&p.wg_listen_port),
        Field::PublicPort => n(&p.public_port),
        Field::LinkSubnet => s(&p.link_subnet),
        Field::EdgeListenPort => n(&p.edge_listen_port),
        Field::RepublisherMetricsPort => n(&p.republisher_metrics_port),
        Field::SpireOutboundPort => n(&p.spire_outbound_port),
        Field::CertDir => s(&p.cert_dir),
        Field::EdgeGroup => s(&p.edge_group),
        Field::CoreGroup => s(&p.core_group),
        Field::EdgeHostName => s(&p.edge_host_name),
        Field::CoreHostName => s(&p.core_host_name),
        Field::CarrierInterface => s(&p.carrier_interface),
        Field::CarrierListenPort => n(&p.carrier_listen_port),
        Field::CarrierPort => n(&p.carrier_port),
        Field::CarrierSubnet => s(&p.carrier_subnet),
    }
}

fn pins_set(p: &mut PeeringPins, f: Field, v: Val) {
    let s = |v: Val| match v {
        Val::S(s) => Some(s),
        Val::P(n) => Some(n.to_string()),
    };
    let n = |v: Val| match v {
        Val::P(n) => Some(n),
        Val::S(_) => None,
    };
    match f {
        Field::WgInterface => p.wg_interface = s(v),
        Field::WgListenPort => p.wg_listen_port = n(v),
        Field::PublicPort => p.public_port = n(v),
        Field::LinkSubnet => p.link_subnet = s(v),
        Field::EdgeListenPort => p.edge_listen_port = n(v),
        Field::RepublisherMetricsPort => p.republisher_metrics_port = n(v),
        Field::SpireOutboundPort => p.spire_outbound_port = n(v),
        Field::CertDir => p.cert_dir = s(v),
        Field::EdgeGroup => p.edge_group = s(v),
        Field::CoreGroup => p.core_group = s(v),
        Field::EdgeHostName => p.edge_host_name = s(v),
        Field::CoreHostName => p.core_host_name = s(v),
        Field::CarrierInterface => p.carrier_interface = s(v),
        Field::CarrierListenPort => p.carrier_listen_port = n(v),
        Field::CarrierPort => p.carrier_port = n(v),
        Field::CarrierSubnet => p.carrier_subnet = s(v),
    }
}

/// The field's candidate at index `i`, or `None` past the end of its sequence.
fn candidate(f: Field, i: u32, p: &Peering, ctx: &AllocContext) -> Option<Val> {
    let port = |base: u32, step: u32| u16::try_from(base + step * i).ok().map(Val::P);
    let net = |n: [u8; 3]| {
        (i < MAX_INDEX).then(|| Val::S(format!("{}.{}.{}.{}/31", n[0], n[1], n[2], 2 * i)))
    };
    let once = |s: String| (i == 0).then_some(Val::S(s));
    if i >= MAX_INDEX {
        return None;
    }
    match f {
        Field::WgInterface => Some(Val::S(format!("wg-fed{i}"))),
        Field::CarrierInterface => Some(Val::S(format!("wg-carry{i}"))),
        Field::WgListenPort => port(WG_LISTEN_BASE, 2),
        Field::CarrierListenPort => port(CARRIER_LISTEN_BASE, 2),
        Field::PublicPort => port(PUBLIC_PORT_BASE, 2),
        Field::CarrierPort => port(CARRIER_PORT_BASE, 2),
        Field::RepublisherMetricsPort => port(METRICS_BASE, 1),
        Field::SpireOutboundPort => port(SPIRE_OUT_BASE, 1),
        Field::LinkSubnet => net(LINK_NET),
        Field::CarrierSubnet => net(CARRIER_NET),
        Field::EdgeListenPort => (i == 0).then_some(Val::P(EDGE_LISTEN_PORT)),
        // The driver's own default `<work>/pki`, so phase 8 mints where the edge
        // reads; a partner declared on a second environment takes
        // `<root>/<environment>-<with>/pki` (run it with that `--work`).
        Field::CertDir => match i {
            0 => Some(Val::S(format!(
                "{}/{}/pki",
                ctx.work_root.trim_end_matches('/'),
                p.with
            ))),
            1 => Some(Val::S(format!(
                "{}/{}-{}/pki",
                ctx.work_root.trim_end_matches('/'),
                p.environment,
                p.with
            ))),
            _ => None,
        },
        // `<with>_edge` is the driver's default group; a partner declared on two
        // environments takes `<environment>_<with>_edge` on the second.
        Field::EdgeGroup => match i {
            0 => Some(Val::S(format!("{}_edge", p.with))),
            1 => Some(Val::S(format!("{}_{}_edge", p.environment, p.with))),
            _ => None,
        },
        Field::CoreGroup => match i {
            0 => Some(Val::S(format!("{}_core", p.with))),
            1 => Some(Val::S(format!("{}_{}_core", p.environment, p.with))),
            _ => None,
        },
        Field::EdgeHostName => once(format!("{}-edge-{}", p.environment, p.with)),
        Field::CoreHostName => once(format!("{}-core-{}", p.environment, p.with)),
    }
}

/// An IPv4 /31 or /30 on its network boundary: (first address as u32, size).
pub fn parse_subnet(s: &str) -> Option<(u32, u32)> {
    let (addr, len) = s.split_once('/')?;
    let len: u32 = len.parse().ok()?;
    if len != 31 && len != 30 {
        return None;
    }
    let octets: Vec<u8> = addr
        .split('.')
        .map(|o| o.parse::<u8>())
        .collect::<Result<_, _>>()
        .ok()?;
    if octets.len() != 4 {
        return None;
    }
    let ip = u32::from_be_bytes([octets[0], octets[1], octets[2], octets[3]]);
    let size = 1u32 << (32 - len);
    ip.is_multiple_of(size).then_some((ip, size))
}

fn overlaps(a: &str, b: &str) -> bool {
    match (parse_subnet(a), parse_subnet(b)) {
        (Some((a0, an)), Some((b0, bn))) => a0 < b0 + bn && b0 < a0 + an,
        _ => a == b,
    }
}

fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Whether a pinned value is well formed for its field; `Some(why)` when not.
fn malformed(f: Field, v: &Val) -> Option<String> {
    match (f, v) {
        (Field::WgInterface | Field::CarrierInterface, Val::S(s)) => (!(name_ok(s)
            && s.len() <= 15))
            .then(|| "an interface name is 1-15 characters of a-z A-Z 0-9 _ -".into()),
        (Field::LinkSubnet | Field::CarrierSubnet, Val::S(s)) => {
            parse_subnet(s).is_none().then(|| {
                "a link subnet is an IPv4 /31 or /30 on its boundary, e.g. 169.254.10.0/31".into()
            })
        }
        (Field::CertDir, Val::S(s)) => (!(s.starts_with('/')
            && s.len() > 1
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c))))
        .then(|| "a certificate directory is an absolute path of a-z A-Z 0-9 / . _ -".into()),
        (
            Field::EdgeGroup | Field::CoreGroup | Field::EdgeHostName | Field::CoreHostName,
            Val::S(s),
        ) => (!name_ok(s)).then(|| "an inventory name is a-z A-Z 0-9 _ -".into()),
        (_, Val::P(0)) => Some("port 0 is not a port".into()),
        _ => None,
    }
}

/// Who holds a value: (peering index, field, from a pin).
#[derive(Clone, Debug)]
struct Holder {
    /// Who holds it, as a message names it.
    who: String,
    /// The declared peering whose PIN holds it, if a pin does.
    pin_of: Option<usize>,
}

#[derive(Default)]
struct Taken {
    // (pool, environment or "" for repository-wide) -> held values
    held: HashMap<(Pool, String), Vec<(Val, Holder)>>,
}

impl Taken {
    fn scope(pool: Pool, env: &str) -> (Pool, String) {
        (
            pool,
            if pool.per_environment() {
                env.to_string()
            } else {
                String::new()
            },
        )
    }

    fn clash(&self, pool: Pool, env: &str, v: &Val) -> Option<&Holder> {
        let list = self.held.get(&Self::scope(pool, env))?;
        list.iter()
            .find(|(held, _)| match (held, v) {
                (Val::S(a), Val::S(b)) if matches!(pool, Pool::Subnet | Pool::ForwarderSubnet) => {
                    overlaps(a, b)
                }
                (a, b) => a == b,
            })
            .map(|(_, h)| h)
    }

    fn take(&mut self, pool: Pool, env: &str, v: Val, h: Holder) {
        self.held
            .entry(Self::scope(pool, env))
            .or_default()
            .push((v, h));
    }
}

/// The whole allocation: values for every peering it could allocate, and every
/// problem. Order: the declaration's own problems; then every pinned and
/// recorded value is registered, so a fresh allocation can never take one; then
/// each peering, in declaration order, takes the first free candidate for each
/// field it still lacks.
fn run(cfg: &Config, ctx: &AllocContext) -> (Vec<PeeringValues>, Vec<Problem>) {
    let mut problems = Vec::new();
    let peerings = &cfg.peerings;
    let at = |i: usize| format!("peerings/{i} (with '{}')", peerings[i].with);

    // The declaration on its own.
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (i, p) in peerings.iter().enumerate() {
        if let Some(first) = seen.insert(key(p), i) {
            problems.push(Problem::new("GB-121", format!(
                "{}: declared twice on environment '{}' (also peerings/{first}) (GB-121). Keep one entry per partner per environment",
                at(i), p.environment
            )));
        }
        for f in FIELDS {
            if let Some(v) = pins_get(&p.values, *f) {
                if let Some(why) = malformed(*f, &v) {
                    problems.push(Problem::new("GB-123", format!(
                        "{}: values.{} '{}' is not usable (GB-123): {why}. Correct the pin, or remove it to have the value allocated",
                        at(i), f.name(), v.show()
                    )));
                }
                if f.forwarder_only() && p.forwarder.is_none() {
                    problems.push(Problem::new("GB-123", format!(
                        "{}: values.{} is pinned but no forwarder relays this peering (GB-123). Declare `forwarder:` on the peering, or remove the pin",
                        at(i), f.name()
                    )));
                }
                if matches!(f, Field::WgListenPort | Field::CarrierListenPort) {
                    if let Val::P(port) = v {
                        let held = reserved_udp(cfg, ctx, &p.environment);
                        if held.contains(&port) {
                            problems.push(Problem::new("GB-122", format!(
                                "{}: values.{} {port} is a UDP port the edge hosts of environment '{}' already hold (GB-122): {}. Pin another port, remove the pin to have one allocated, or, when these edges run no mesh client on it, set environments/{} peering_edge_reserved_udp in goatbuilder.yaml",
                                at(i), f.name(), p.environment, held.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "), p.environment
                            )));
                        }
                    }
                }
            }
        }
    }
    if !problems.is_empty() {
        return (Vec::new(), problems);
    }

    // Register what is fixed, in this order: the ports the edge host itself
    // holds; the values of record entries no longer declared (kept until a person
    // removes the entry, because the peering may still be running); pins; then
    // the record. A pin that takes a value another peering recorded is reported
    // against the pin.
    let mut fixed: Vec<PeeringPins> = vec![PeeringPins::default(); peerings.len()];
    let mut pinned: Vec<Vec<String>> = vec![Vec::new(); peerings.len()];
    let mut taken = Taken::default();
    let wants = |p: &Peering, f: Field| !f.forwarder_only() || p.forwarder.is_some();
    let envs: std::collections::BTreeSet<&str> =
        peerings.iter().map(|p| p.environment.as_str()).collect();
    for env in &envs {
        for port in reserved_udp(cfg, ctx, env) {
            taken.take(
                Pool::EdgeUdp,
                env,
                Val::P(port),
                Holder {
                    who: format!("the edge host itself (environment '{env}' peering_edge_reserved_udp, default NetBird's 51820)"),
                    pin_of: None,
                },
            );
        }
    }
    // A retired entry does not say which forwarder relayed it, so its
    // forwarder values stay reserved on every forwarder declared.
    let forwarders: std::collections::BTreeSet<String> = peerings
        .iter()
        .filter_map(|p| {
            p.forwarder
                .as_ref()
                .map(|x| format!("forwarder {}", x.endpoint))
        })
        .collect();
    for (k, entry) in retired_entries(cfg, ctx) {
        let env = k.split('/').next().unwrap_or_default().to_string();
        for f in FIELDS.iter().copied() {
            let (Some(v), Some(pool)) = (pins_get(entry, f), f.pool()) else {
                continue;
            };
            let mut at_places = vec![(pool, env.clone())];
            let fpool = match f {
                Field::PublicPort | Field::CarrierPort => Some(Pool::ForwarderUdp),
                Field::CarrierInterface => Some(Pool::ForwarderInterface),
                Field::CarrierSubnet => Some(Pool::ForwarderSubnet),
                _ => None,
            };
            if let Some(fp) = fpool {
                at_places.extend(forwarders.iter().map(|w| (fp, w.clone())));
            }
            for (pool, place) in at_places {
                if taken.clash(pool, &place, &v).is_none() {
                    taken.take(
                        pool,
                        &place,
                        v.clone(),
                        Holder {
                            who: format!(
                                "the record's entry {k}, no longer declared ({})",
                                f.name()
                            ),
                            pin_of: None,
                        },
                    );
                }
            }
        }
    }
    for pass_pins in [true, false] {
        for (i, p) in peerings.iter().enumerate() {
            let recorded = ctx.record.peerings.get(&key(p));
            for f in FIELDS.iter().copied().filter(|f| wants(p, *f)) {
                if pins_get(&fixed[i], f).is_some() {
                    continue;
                }
                let v = if pass_pins {
                    pins_get(&p.values, f)
                } else {
                    recorded.and_then(|r| pins_get(r, f))
                };
                let Some(v) = v else { continue };
                if !pass_pins {
                    if let Some(why) = malformed(f, &v) {
                        problems.push(Problem::new("GB-124", format!(
                            "{}: the allocation record's {} '{}' is not usable (GB-124): {why}. Remove that line from {RECORD_FILE} and run `goatbuilder federate allocate` again",
                            at(i), f.name(), v.show()
                        )));
                        continue;
                    }
                }
                let mut clashed = false;
                for (pool, place) in places(f, p) {
                    if let Some(h) = taken.clash(pool, &place, &v) {
                        let problem = match (pass_pins, h.pin_of) {
                            // A pin took what this peering had recorded: the pin is the change.
                            (false, Some(j)) => Problem::new("GB-121", format!(
                                "{}: {} is a {} that {}.{} (recorded) '{}' already holds (GB-121). Pin a different value on {}, or remove the pin to have one allocated",
                                at(j), h.who, pool.what(), at(i), f.name(), v.show(), at(j)
                            )),
                            (true, _) => Problem::new("GB-121", format!(
                                "{}: values.{} '{}' is a {} that {} already holds (GB-121). Pin a different {} on one of them, or remove the pin to have one allocated",
                                at(i), f.name(), v.show(), pool.what(), h.who, f.name()
                            )),
                            (false, None) => Problem::new("GB-124", format!(
                                "{}: the record's {} '{}' is a {} that {} already holds (GB-124). Remove one of the two from {RECORD_FILE} and run `goatbuilder federate allocate` again",
                                at(i), f.name(), v.show(), pool.what(), h.who
                            )),
                        };
                        problems.push(problem);
                        clashed = true;
                        break;
                    }
                    let who = if pass_pins {
                        format!("values.{} of {}", f.name(), at(i))
                    } else {
                        format!("{}.{} (recorded)", at(i), f.name())
                    };
                    taken.take(
                        pool,
                        &place,
                        v.clone(),
                        Holder {
                            who,
                            pin_of: pass_pins.then_some(i),
                        },
                    );
                }
                if clashed {
                    continue;
                }
                if pass_pins {
                    pinned[i].push(f.name().to_string());
                }
                pins_set(&mut fixed[i], f, v);
            }
        }
    }
    if !problems.is_empty() {
        return (Vec::new(), problems);
    }

    // Fresh allocation, in declaration order.
    for (i, p) in peerings.iter().enumerate() {
        for f in FIELDS.iter().copied().filter(|f| wants(p, *f)) {
            if pins_get(&fixed[i], f).is_some() {
                continue;
            }
            let mut got = None;
            let mut idx = 0;
            while let Some(c) = candidate(f, idx, p, ctx) {
                let free = places(f, p)
                    .iter()
                    .all(|(pool, place)| taken.clash(*pool, place, &c).is_none());
                if free {
                    got = Some(c);
                    break;
                }
                idx += 1;
            }
            match got {
                Some(v) => {
                    for (pool, place) in places(f, p) {
                        taken.take(pool, &place, v.clone(), Holder { who: format!("{}.{}", at(i), f.name()), pin_of: None });
                    }
                    pins_set(&mut fixed[i], f, v);
                }
                None => problems.push(Problem::new("GB-127", format!(
                    "{}: no free {} is left to allocate (GB-127). Pin one under the peering's values:",
                    at(i), f.name()
                ))),
            }
        }
    }
    if !problems.is_empty() {
        return (Vec::new(), problems);
    }

    let values = peerings
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let v = &fixed[i];
            PeeringValues {
                peering: p.with.clone(),
                environment: p.environment.clone(),
                wg_interface: v.wg_interface.clone().unwrap_or_default(),
                wg_listen_port: v.wg_listen_port.unwrap_or_default(),
                public_port: v.public_port,
                link_subnet: v.link_subnet.clone().unwrap_or_default(),
                edge_listen_port: v.edge_listen_port.unwrap_or(EDGE_LISTEN_PORT),
                republisher_metrics_port: v.republisher_metrics_port.unwrap_or_default(),
                spire_outbound_port: v.spire_outbound_port.unwrap_or_default(),
                cert_dir: v.cert_dir.clone().unwrap_or_default(),
                edge_group: v.edge_group.clone().unwrap_or_default(),
                core_group: v.core_group.clone().unwrap_or_default(),
                edge_host_name: v.edge_host_name.clone().unwrap_or_default(),
                core_host_name: v.core_host_name.clone().unwrap_or_default(),
                carrier_interface: v.carrier_interface.clone(),
                carrier_listen_port: v.carrier_listen_port,
                carrier_port: v.carrier_port,
                carrier_subnet: v.carrier_subnet.clone(),
                pinned: pinned[i].clone(),
            }
        })
        .collect();
    (values, problems)
}

/// Record entries whose peering is no longer declared, keyed `<environment>/<with>`.
/// Their values stay reserved: the peering may still be running on the hosts.
pub fn retired_entries<'a>(
    cfg: &Config,
    ctx: &'a AllocContext,
) -> Vec<(&'a String, &'a PeeringPins)> {
    let declared: std::collections::BTreeSet<String> = cfg.peerings.iter().map(key).collect();
    ctx.record
        .peerings
        .iter()
        .filter(|(k, _)| !declared.contains(*k))
        .collect()
}

/// The record that keeps these values: every value a pin does not own, keyed
/// `<environment>/<with>`, and every entry of `previous` whose peering is no
/// longer declared (removed by a person, once that peering is cut).
pub fn record_of(values: &[PeeringValues], previous: &Record) -> Record {
    // Entries no longer declared are kept: their values stay reserved until a
    // person removes the entry, after cutting that peering.
    let declared: std::collections::BTreeSet<String> = values
        .iter()
        .map(|v| format!("{}/{}", v.environment, v.peering))
        .collect();
    let mut peerings: BTreeMap<String, PeeringPins> = previous
        .peerings
        .iter()
        .filter(|(k, _)| !declared.contains(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for v in values {
        let mut e = PeeringPins::default();
        let all: [(Field, Option<Val>); 16] = [
            (Field::WgInterface, Some(Val::S(v.wg_interface.clone()))),
            (Field::WgListenPort, Some(Val::P(v.wg_listen_port))),
            (Field::PublicPort, v.public_port.map(Val::P)),
            (Field::LinkSubnet, Some(Val::S(v.link_subnet.clone()))),
            (Field::EdgeListenPort, Some(Val::P(v.edge_listen_port))),
            (
                Field::RepublisherMetricsPort,
                Some(Val::P(v.republisher_metrics_port)),
            ),
            (
                Field::SpireOutboundPort,
                Some(Val::P(v.spire_outbound_port)),
            ),
            (Field::CertDir, Some(Val::S(v.cert_dir.clone()))),
            (Field::EdgeGroup, Some(Val::S(v.edge_group.clone()))),
            (Field::CoreGroup, Some(Val::S(v.core_group.clone()))),
            (Field::EdgeHostName, Some(Val::S(v.edge_host_name.clone()))),
            (Field::CoreHostName, Some(Val::S(v.core_host_name.clone()))),
            (
                Field::CarrierInterface,
                v.carrier_interface.clone().map(Val::S),
            ),
            (Field::CarrierListenPort, v.carrier_listen_port.map(Val::P)),
            (Field::CarrierPort, v.carrier_port.map(Val::P)),
            (Field::CarrierSubnet, v.carrier_subnet.clone().map(Val::S)),
        ];
        for (f, val) in all {
            if let Some(val) = val {
                if !v.pinned.iter().any(|p| p == f.name()) {
                    pins_set(&mut e, f, val);
                }
            }
        }
        peerings.insert(format!("{}/{}", v.environment, v.peering), e);
    }
    Record {
        schema: RECORD_SCHEMA.into(),
        peerings,
    }
}

/// The record as committed text.
pub fn record_text(rec: &Record) -> String {
    let body = serde_yaml::to_string(rec).unwrap_or_default();
    format!(
        "# GENERATED by `goatbuilder federate allocate`: the per-peering values it allocated,\n\
         # one entry per peering (<environment>/<with>). Review it like any change to this\n\
         # repository. A recorded value is never moved by a later run, so adding a peering\n\
         # never changes another's values. To choose a value yourself, pin it under the\n\
         # peering's `values:` in goatbuilder.yaml (a pinned field is not recorded here).\n\
         {body}"
    )
}

/// The values grouped by field, for a table: one row per peering.
pub fn summary(v: &PeeringValues) -> String {
    let mut s = format!(
        "{}/{}: {} port {} link {} edge-listen {} spire-out {} metrics {} certs {} groups {}/{}",
        v.environment,
        v.peering,
        v.wg_interface,
        v.wg_listen_port,
        v.link_subnet,
        v.edge_listen_port,
        v.spire_outbound_port,
        v.republisher_metrics_port,
        v.cert_dir,
        v.edge_group,
        v.core_group
    );
    if let (Some(pp), Some(ci), Some(cl), Some(cp), Some(cs)) = (
        v.public_port,
        &v.carrier_interface,
        v.carrier_listen_port,
        v.carrier_port,
        &v.carrier_subnet,
    ) {
        s.push_str(&format!(
            " forwarder public {pp} carrier {ci} port {cl} -> {cp} subnet {cs}"
        ));
    }
    if !v.pinned.is_empty() {
        s.push_str(&format!(" (pinned: {})", v.pinned.join(", ")));
    }
    s
}

/// The set of field names (the pins' keys and the schema's `values:` keys).
#[cfg(test)]
pub fn field_names() -> std::collections::BTreeSet<&'static str> {
    FIELDS.iter().map(|f| f.name()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeSet;

    fn cfg(peerings: serde_json::Value) -> Config {
        let v = json!({
            "schema": "goatbuilder/v1",
            "preset": "full",
            "repo": {"source": "example-org/example-repo", "image_namespace": "ghcr.io/example-org"},
            "components": {},
            "environments": [
                {"name": "dev", "converger": "push-ansible", "sites": [{"name": "alpha", "role": "founder", "classes": {"core": {}, "edge": {}}}]},
                {"name": "prod", "converger": "push-ansible", "sites": [{"name": "alpha", "role": "founder", "classes": {"core": {}, "edge": {}}}]}
            ],
            "promotion": {"path": [{"name": "dev", "kind": "rung", "environment": "dev"}]},
            "peerings": peerings,
        });
        serde_json::from_value(v).unwrap()
    }

    fn p(with: &str) -> serde_json::Value {
        json!({"with": with, "environment": "dev", "site": "alpha", "edge_class": "edge"})
    }

    fn by<'a>(vs: &'a [PeeringValues], with: &str) -> &'a PeeringValues {
        vs.iter().find(|v| v.peering == with).unwrap()
    }

    fn code(r: Result<Vec<PeeringValues>>) -> (String, String) {
        let e = r.unwrap_err();
        let c = e
            .downcast_ref::<goat_errors::Coded>()
            .map(|c| c.code.clone())
            .unwrap_or_default();
        (c, e.to_string())
    }

    /// The first peering gets the shape of the values the first real run chose
    /// by hand; a second and a third on the same edge and core take the next
    /// free ones and share nothing that must not be shared.
    #[test]
    fn two_and_three_peerings_on_one_edge_do_not_collide() {
        let c = cfg(json!([p("xab"), p("xac"), p("xad")]));
        let vs = allocate_all(&c, &AllocContext::default()).unwrap();
        let a = by(&vs, "xab");
        assert_eq!(
            (
                a.wg_interface.as_str(),
                a.wg_listen_port,
                a.link_subnet.as_str(),
                a.edge_listen_port,
                a.republisher_metrics_port,
                a.spire_outbound_port
            ),
            ("wg-fed0", 51823, "169.254.10.0/31", 7448, 9310, 18443)
        );
        assert_eq!(a.cert_dir, "/var/lib/goat-federation/xab/pki");
        assert_eq!(
            (a.edge_group.as_str(), a.core_group.as_str()),
            ("xab_edge", "xab_core")
        );
        assert_eq!(
            (a.edge_host_name.as_str(), a.core_host_name.as_str()),
            ("dev-edge-xab", "dev-core-xab")
        );
        assert_eq!(a.public_port, None, "no forwarder, no public port");
        let b = by(&vs, "xac");
        assert_eq!(
            (
                b.wg_interface.as_str(),
                b.wg_listen_port,
                b.link_subnet.as_str()
            ),
            ("wg-fed1", 51825, "169.254.10.2/31")
        );
        assert_eq!(
            (b.republisher_metrics_port, b.spire_outbound_port),
            (9311, 18444)
        );
        let d = by(&vs, "xad");
        assert_eq!(
            (
                d.wg_interface.as_str(),
                d.wg_listen_port,
                d.link_subnet.as_str()
            ),
            ("wg-fed2", 51827, "169.254.10.4/31")
        );
        // Distinct where it matters; the edge listen port is shared on purpose.
        for f in [
            |v: &PeeringValues| v.wg_interface.clone(),
            |v: &PeeringValues| v.wg_listen_port.to_string(),
            |v: &PeeringValues| v.link_subnet.clone(),
            |v: &PeeringValues| v.cert_dir.clone(),
            |v: &PeeringValues| v.republisher_metrics_port.to_string(),
            |v: &PeeringValues| v.spire_outbound_port.to_string(),
            |v: &PeeringValues| v.edge_host_name.clone(),
        ] {
            let set: BTreeSet<String> = vs.iter().map(f).collect();
            assert_eq!(set.len(), 3);
        }
        assert!(vs.iter().all(|v| v.edge_listen_port == EDGE_LISTEN_PORT));
        assert!(
            vs.iter().all(|v| v.wg_listen_port != 51820),
            "NetBird's port is never allocated"
        );
    }

    /// Re-running with the record gives the same values; adding a peering in
    /// FRONT of the others, or removing one, never moves a recorded peering.
    #[test]
    fn allocation_is_stable_across_reruns_reorders_and_additions() {
        let c1 = cfg(json!([p("xab"), p("xac")]));
        let first = allocate_all(&c1, &AllocContext::default()).unwrap();
        let ctx = AllocContext {
            record: record_of(&first, &Record::default()),
            ..AllocContext::default()
        };
        assert_eq!(
            allocate_all(&c1, &ctx).unwrap(),
            first,
            "a re-run moves nothing"
        );
        // A new peering declared first, and the old ones reordered.
        let c2 = cfg(json!([p("xad"), p("xac"), p("xab")]));
        let second = allocate_all(&c2, &ctx).unwrap();
        assert_eq!(by(&second, "xab"), by(&first, "xab"));
        assert_eq!(by(&second, "xac"), by(&first, "xac"));
        let d = by(&second, "xad");
        assert_eq!(
            (
                d.wg_interface.as_str(),
                d.wg_listen_port,
                d.link_subnet.as_str()
            ),
            ("wg-fed2", 51827, "169.254.10.4/31")
        );
        // Removing the first peering leaves the second where it was.
        let c3 = cfg(json!([p("xac")]));
        let ctx2 = AllocContext {
            record: record_of(&second, &Record::default()),
            ..AllocContext::default()
        };
        assert_eq!(
            by(&allocate_all(&c3, &ctx2).unwrap(), "xac"),
            by(&first, "xac")
        );
        // The record survives a write and a read.
        let text = record_text(&record_of(&second, &Record::default()));
        let back: Record = serde_yaml::from_str(&text).unwrap();
        assert_eq!(back, record_of(&second, &Record::default()));
    }

    /// Removing a peering's declaration keeps its record entry, so a peering
    /// added after it cannot take values the removed one may still be running on.
    #[test]
    fn a_removed_peerings_values_stay_reserved_until_its_entry_is_removed() {
        let first = allocate_all(&cfg(json!([p("xab")])), &AllocContext::default()).unwrap();
        let rec = record_of(&first, &Record::default());
        let ctx = AllocContext {
            record: rec.clone(),
            ..AllocContext::default()
        };
        let c = cfg(json!([p("xad")]));
        let d = allocate_all(&c, &ctx).unwrap();
        assert_eq!(
            (
                d[0].wg_interface.as_str(),
                d[0].wg_listen_port,
                d[0].link_subnet.as_str()
            ),
            ("wg-fed1", 51825, "169.254.10.2/31")
        );
        let kept = record_of(&d, &rec);
        assert!(
            kept.peerings.contains_key("dev/xab") && kept.peerings.contains_key("dev/xad"),
            "{kept:?}"
        );
        assert_eq!(retired_entries(&c, &ctx).len(), 1);
        // Once the entry is removed, the values are free again.
        let mut freed = kept.clone();
        freed.peerings.remove("dev/xab");
        freed.peerings.remove("dev/xad");
        let again = allocate_all(
            &c,
            &AllocContext {
                record: freed,
                ..AllocContext::default()
            },
        )
        .unwrap();
        assert_eq!(again[0].wg_interface, "wg-fed0");
    }

    /// A record entry on NetBird's port is refused like a pin on it.
    #[test]
    fn a_recorded_reserved_port_is_refused() {
        let mut rec = Record {
            schema: RECORD_SCHEMA.into(),
            ..Default::default()
        };
        rec.peerings.insert(
            "dev/xab".into(),
            PeeringPins {
                wg_listen_port: Some(51820),
                ..Default::default()
            },
        );
        let (c, msg) = code(allocate_all(
            &cfg(json!([p("xab")])),
            &AllocContext {
                record: rec,
                ..AllocContext::default()
            },
        ));
        assert_eq!(c, "GB-124", "{msg}");
        assert!(msg.contains("NetBird"), "{msg}");
    }

    /// A pin wins, is not recorded, and fresh allocation steps around it.
    #[test]
    fn a_pin_is_kept_and_allocation_steps_around_it() {
        let mut a = p("xab");
        a["values"] = json!({"wg_listen_port": 51825, "wg_interface": "wg-fed1"});
        let c = cfg(json!([a, p("xac")]));
        let vs = allocate_all(&c, &AllocContext::default()).unwrap();
        let x = by(&vs, "xab");
        assert_eq!(
            (x.wg_interface.as_str(), x.wg_listen_port),
            ("wg-fed1", 51825)
        );
        assert_eq!(x.pinned, vec!["wg_interface", "wg_listen_port"]);
        let y = by(&vs, "xac");
        assert_eq!(
            (y.wg_interface.as_str(), y.wg_listen_port),
            ("wg-fed0", 51823)
        );
        let rec = record_of(&vs, &Record::default());
        assert_eq!(
            rec.peerings["dev/xab"].wg_listen_port, None,
            "a pinned field is the pin's, not the record's"
        );
        assert_eq!(rec.peerings["dev/xac"].wg_listen_port, Some(51823));
    }

    #[test]
    fn two_pins_on_one_value_are_refused() {
        let mut a = p("xab");
        a["values"] = json!({"wg_listen_port": 51900});
        let mut b = p("xac");
        b["values"] = json!({"wg_listen_port": 51900});
        let (c, msg) = code(allocate_all(&cfg(json!([a, b])), &AllocContext::default()));
        assert_eq!(c, "GB-121", "{msg}");
        assert!(
            msg.contains("UDP port on the edge host")
                && msg.contains("Pin a different wg_listen_port"),
            "{msg}"
        );
    }

    /// A pin that takes what another peering recorded is refused against the pin.
    #[test]
    fn a_pin_on_a_recorded_value_is_refused() {
        let first = allocate_all(&cfg(json!([p("xab")])), &AllocContext::default()).unwrap();
        let ctx = AllocContext {
            record: record_of(&first, &Record::default()),
            ..AllocContext::default()
        };
        let mut b = p("xac");
        b["values"] = json!({"link_subnet": "169.254.10.0/30"});
        let (c, msg) = code(allocate_all(&cfg(json!([p("xab"), b])), &ctx));
        assert_eq!(c, "GB-121", "{msg}");
        assert!(
            msg.starts_with("peerings/1 (with 'xac'): values.link_subnet")
                && msg.contains("peerings/0 (with 'xab').link_subnet (recorded)"),
            "{msg}"
        );
    }

    #[test]
    fn a_pin_on_netbirds_port_is_refused() {
        let mut a = p("xab");
        a["values"] = json!({"wg_listen_port": 51820});
        let (c, msg) = code(allocate_all(&cfg(json!([a])), &AllocContext::default()));
        assert_eq!(c, "GB-122", "{msg}");
    }

    /// The edge's reserved UDP ports are the environment's configuration: an
    /// environment whose edges run no mesh client may pin 51820 (adopting a
    /// peering that runs on the role default), and one whose edges hold another
    /// port steps around it.
    #[test]
    fn the_reserved_udp_ports_are_per_environment() {
        let mut a = p("xab");
        a["values"] = json!({"wg_listen_port": 51820});
        let mut c = cfg(json!([a, p("xac")]));
        c.environments[0].peering_edge_reserved_udp = Some(vec![51823]);
        let vs = allocate_all(&c, &AllocContext::default()).unwrap();
        assert_eq!(by(&vs, "xab").wg_listen_port, 51820);
        assert_eq!(
            by(&vs, "xac").wg_listen_port,
            51825,
            "51823 is reserved here"
        );
        // prod keeps the default.
        let mut b = p("xab");
        b["environment"] = json!("prod");
        b["values"] = json!({"wg_listen_port": 51820});
        let mut c = cfg(json!([b]));
        c.environments[0].peering_edge_reserved_udp = Some(vec![]);
        assert_eq!(code(allocate_all(&c, &AllocContext::default())).0, "GB-122");
    }

    /// `goatbuilder validate` reads the record beside the config, so a pin on a
    /// value the record gives another peering is reported there too.
    #[test]
    fn validation_reads_the_record_beside_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let first = allocate_all(&cfg(json!([p("xab")])), &AllocContext::default()).unwrap();
        std::fs::write(
            dir.path().join(RECORD_FILE),
            record_text(&record_of(&first, &Record::default())),
        )
        .unwrap();
        let mut b = p("xac");
        b["values"] = json!({"wg_interface": "wg-fed0"});
        let c = cfg(json!([p("xab"), b]));
        assert!(
            declaration_problems(&c).is_empty(),
            "no record, no collision"
        );
        let out = declaration_problems_in(&c, dir.path());
        assert!(out.iter().any(|m| m.contains("GB-121")), "{out:?}");
    }

    #[test]
    fn malformed_pins_and_forwarder_pins_without_a_forwarder_are_refused() {
        for (vals, frag) in [
            (
                json!({"wg_interface": "wg-federation-zero"}),
                "1-15 characters",
            ),
            (json!({"link_subnet": "169.254.10.1/31"}), "on its boundary"),
            (json!({"cert_dir": "relative/pki"}), "absolute path"),
            (json!({"public_port": 51830}), "no forwarder relays"),
        ] {
            let mut a = p("xab");
            a["values"] = vals;
            let (c, msg) = code(allocate_all(&cfg(json!([a])), &AllocContext::default()));
            assert_eq!(c, "GB-123", "{msg}");
            assert!(msg.contains(frag), "{frag}: {msg}");
        }
    }

    /// Behind a forwarder, the carrier values come too, and the first peering's
    /// match the forwarder role's defaults.
    /// One forwarder relaying peerings of two environments: its ports, carrier
    /// interfaces and carrier subnets are the forwarder's, so the second
    /// environment's peering steps past the first's there, while the edge-only
    /// values (on each environment's own edge) may repeat.
    #[test]
    fn two_environments_behind_one_forwarder_share_nothing_on_it() {
        let mut a = p("xab");
        a["forwarder"] = json!({"endpoint": "203.0.113.7"});
        let mut b = p("xac");
        b["environment"] = json!("prod");
        b["forwarder"] = json!({"endpoint": "203.0.113.7"});
        let mut c = p("xad");
        c["environment"] = json!("prod");
        c["forwarder"] = json!({"endpoint": "203.0.113.8"});
        let vs = allocate_all(&cfg(json!([a, b, c])), &AllocContext::default()).unwrap();
        let (x, y, z) = (by(&vs, "xab"), by(&vs, "xac"), by(&vs, "xad"));
        assert_eq!(
            (
                x.public_port,
                x.carrier_port,
                x.carrier_interface.as_deref(),
                x.carrier_subnet.as_deref()
            ),
            (
                Some(51820),
                Some(51821),
                Some("wg-carry0"),
                Some("169.254.30.0/31")
            )
        );
        assert_eq!(
            (
                y.public_port,
                y.carrier_port,
                y.carrier_interface.as_deref(),
                y.carrier_subnet.as_deref()
            ),
            (
                Some(51822),
                Some(51823),
                Some("wg-carry1"),
                Some("169.254.30.2/31")
            ),
            "the same forwarder as xab"
        );
        assert_eq!(y.wg_interface, "wg-fed0", "prod's own edge");
        // xad is on prod's edge beside xac, behind another forwarder: its
        // forwarder ports start over; its edge carrier values step past xac's.
        assert_eq!((z.public_port, z.carrier_port), (Some(51820), Some(51821)));
        assert_eq!(
            (z.carrier_interface.as_deref(), z.carrier_subnet.as_deref()),
            (Some("wg-carry0"), Some("169.254.30.0/31"))
        );
        assert_ne!(z.carrier_interface, y.carrier_interface, "one edge host");
        assert_ne!(z.carrier_subnet, y.carrier_subnet, "one edge host");
    }

    #[test]
    fn a_forwarder_peering_gets_carrier_values_that_do_not_collide() {
        let mut a = p("xab");
        a["forwarder"] = json!({"endpoint": "203.0.113.7"});
        let mut b = p("xac");
        b["forwarder"] = json!({"endpoint": "203.0.113.7"});
        let vs = allocate_all(&cfg(json!([a, b])), &AllocContext::default()).unwrap();
        let x = by(&vs, "xab");
        assert_eq!(
            (
                x.public_port,
                x.carrier_interface.as_deref(),
                x.carrier_listen_port,
                x.carrier_port,
                x.carrier_subnet.as_deref()
            ),
            (
                Some(51820),
                Some("wg-carry0"),
                Some(51822),
                Some(51821),
                Some("169.254.30.0/31")
            )
        );
        let y = by(&vs, "xac");
        assert_eq!(
            (
                y.public_port,
                y.carrier_interface.as_deref(),
                y.carrier_listen_port,
                y.carrier_port,
                y.carrier_subnet.as_deref()
            ),
            (
                Some(51822),
                Some("wg-carry1"),
                Some(51824),
                Some(51823),
                Some("169.254.30.2/31")
            )
        );
        let edge_udp: BTreeSet<u16> = vs
            .iter()
            .flat_map(|v| [v.wg_listen_port, v.carrier_listen_port.unwrap()])
            .collect();
        assert_eq!(edge_udp.len(), 4, "{edge_udp:?}");
    }

    /// The same partner on two environments: per-host values may repeat (they
    /// are different goatnets' hosts), repository-wide names may not.
    #[test]
    fn one_partner_on_two_environments_gets_distinct_groups_and_cert_dirs() {
        let mut b = p("xab");
        b["environment"] = json!("prod");
        let vs = allocate_all(&cfg(json!([p("xab"), b])), &AllocContext::default()).unwrap();
        let dev = vs.iter().find(|v| v.environment == "dev").unwrap();
        let prod = vs.iter().find(|v| v.environment == "prod").unwrap();
        assert_eq!(dev.wg_interface, prod.wg_interface);
        assert_eq!(
            (dev.edge_group.as_str(), prod.edge_group.as_str()),
            ("xab_edge", "prod_xab_edge")
        );
        assert_eq!(
            (dev.cert_dir.as_str(), prod.cert_dir.as_str()),
            (
                "/var/lib/goat-federation/xab/pki",
                "/var/lib/goat-federation/prod-xab/pki"
            ),
            "one machine may run both: phase 8 must not overwrite the other's leaves"
        );
        assert!(allocate(&cfg(json!([p("xab"), {"with": "xab", "environment": "prod", "site": "alpha", "edge_class": "edge"}])), "xab", &AllocContext::default()).is_err());
    }

    /// The schema's `values:` keys are exactly the pins' keys.
    #[test]
    fn the_schema_values_keys_are_the_pins_keys() {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../ops/goatbuilder/schema/goatbuilder-v1.schema.json"
        ))
        .unwrap();
        let keys: BTreeSet<&str> = schema["definitions"]["peering"]["properties"]["values"]
            ["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, field_names());
        let pins: serde_json::Value = serde_json::to_value(PeeringPins {
            wg_interface: Some("x".into()),
            ..Default::default()
        })
        .unwrap();
        assert!(field_names().contains(pins.as_object().unwrap().keys().next().unwrap().as_str()));
    }

    #[test]
    fn a_record_with_another_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(RECORD_FILE),
            "schema: something/v9\npeerings: {}\n",
        )
        .unwrap();
        let e = AllocContext::load(dir.path()).unwrap_err();
        assert_eq!(
            e.downcast_ref::<goat_errors::Coded>().unwrap().code,
            "GB-124"
        );
    }
}
