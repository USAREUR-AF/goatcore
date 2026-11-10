//! The mesh ACL as a resolved graph — shared by microscope and netvis.
//!
//! Design: `docs/design/netvis.md`, `docs/design/microscope-reachability-view.md`.
//!
//! ## Why this is one module and not two
//!
//! microscope asks "what can THIS machine reach"; netvis asks "what does this
//! goatnet's policy look like". Those are the same resolution — group
//! membership crossed with enabled policy rules — at two scopes. Keeping them
//! in one place means a correction to either is a correction to both, and this
//! surface has already needed several.
//!
//! ## SCOPE IS AN EXPLICIT PARAMETER WITH NO DEFAULT
//!
//! This is the load-bearing constraint and it is not stylistic. microscope's
//! graph was rewritten three times because it drew a POPULATION when a SUBJECT
//! was wanted: first groups-as-nodes (47 nodes, 115 edges for one operator),
//! then every peer the entity had ever touched — which for a platform admin is
//! every peer on the goatnet. netvis has the opposite job: the whole fabric
//! genuinely IS its subject. One resolver serving both is only correct while
//! neither can silently inherit the other's default, so [`Scope`] has no
//! `Default` impl and every caller states its intent.
//!
//! ## Honesty properties this layer inherits
//!
//! Scoped to NetBird, where `GET /api/policies` returns the COMPLETE rule set,
//! so an edge that is not drawn really is denied. That is not true one layer
//! up — Zenoh denies silently and has no central enumerable ACL — so an
//! application-domain graph must not reuse this type. The temptation to "just
//! add the fabric" will recur; it would make every absence a lie.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::netbird::{NbGroup, NbPeer, NbPolicy};

/// What the graph is being drawn FOR. No `Default` — see the module note.
#[derive(Debug, Clone)]
pub enum Scope<'a> {
    /// One machine: what can this peer reach. microscope's per-entity view.
    Peer(&'a str),
    /// An explicit set of machines — an entity's whole estate.
    Peers(&'a [String]),
    /// Groups and rules only, NO peers. netvis's unpopulated altitude: the
    /// policy as designed, readable at standup before anything is enrolled.
    Policy,
    /// Every group AND its membership. netvis's populated altitude.
    Fabric,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// A machine.
    Peer,
    /// A group, drawn as a node only at the policy/fabric altitudes where the
    /// group IS the subject. At peer scope groups are the MECHANISM of an edge
    /// and belong in its explanation, never beside it — drawing both is what
    /// produced the 47-node hairball.
    Group,
}

/// How an edge is permitted, and how confident we are that it is usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeState {
    /// Structural: this peer is a member of this group. Not a path.
    Membership,
    /// An enabled rule permits it. COMPLETE at this layer: a missing edge is a
    /// denial, not an unknown.
    Permitted,
    /// Permitted AND both ends have a connected peer. Means REACHABLE, never
    /// "in use" — nothing here observes traffic. The management API records
    /// that peers connected, never that one talked to another.
    ReachableNow,
}

impl EdgeState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Membership => "member of",
            Self::Permitted => "permitted",
            Self::ReachableNow => "permitted · reachable now",
        }
    }
    pub fn css(self) -> &'static str {
        match self {
            Self::Membership => "membership",
            Self::Permitted => "permitted",
            Self::ReachableNow => "reachable",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub kind: NodeKind,
    /// Peers: connected right now. Groups: none.
    pub connected: Option<bool>,
    /// Groups: how many peers are members, and how many are up. `Some(0)` is a
    /// DECLARED-BUT-EMPTY group and is a finding, not an absence — it is how
    /// ADR 1116's gear classes sat declared-and-uncreated without anyone
    /// noticing. Rendered explicitly, never omitted.
    pub members: Option<usize>,
    pub members_connected: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub state: EdgeState,
    /// The enabled policies permitting this edge.
    pub rules: Vec<String>,
    /// The source→destination group pairs the rules matched on.
    pub via_groups: Vec<String>,
    /// Transport limits derived from the RULE, never from its name. A policy
    /// named "…(icmp)" whose rule permits everything must not read as scoped.
    pub constraints: Vec<String>,
    /// True only when EVERY rule behind the edge is bidirectional. Asymmetry
    /// is the interesting case, so one one-way rule keeps the whole edge
    /// one-way and the return path must earn its own arrow.
    pub bidirectional: bool,
    /// The reverse pair exists as an INDEPENDENT edge — two rules, not one
    /// flagged rule. Derived, and deliberately a separate field from
    /// [`Edge::bidirectional`]: the two revoke differently. Delete the one
    /// bidirectional rule and both directions die; delete one of a reciprocal
    /// pair and reach becomes asymmetric. A picture that fuses them cannot
    /// predict what an operator is about to do.
    pub reciprocal: bool,
    /// This pair carries an unrestricted rule AND at least one scoped rule.
    /// The scoped rules are decoration: the wide one already permits what they
    /// permit. A control that controls nothing.
    pub shadowed: bool,
    /// How close the permit is to being usable right now. See [`Realization`].
    pub realization: Realization,
}

/// The realization ladder: permitted, and how alive.
///
/// This is what keeps alarm colour scarce. Many pairs can be unrestricted;
/// painting all of them red makes red mean "wallpaper". Only
/// the pairs that are unrestricted AND have a connected peer at both ends —
/// twelve — are live blast radius today.
///
/// `Realized` means PERMITTED WITH BOTH ENDS CONNECTED. It never means traffic:
/// nothing in this pipeline observes a packet.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, serde::Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum Realization {
    /// Membership was NOT READ, so where this permit sits on the ladder is
    /// unknown. It is not `Latent` (that claims the group is empty) and not
    /// `Dormant` (that claims the group has members). The three states below
    /// are all assertions about membership; this is the absence of one, and
    /// it exists because the arithmetic used to fall through to `Dormant`
    /// when `members` was `None` — a claim the payload could not support, in
    /// the one field a viewer reads as "how alive is this".
    Unknown,
    /// At least one endpoint group has no members at all. The permit is real
    /// and inert — populate the group and it wakes up.
    #[default]
    Latent,
    /// Both endpoint groups have members, but not a connected one at each end.
    Dormant,
    /// Both endpoint groups have a connected member right now.
    Realized,
}

impl Realization {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "permitted · membership was not read",
            Self::Latent => "permitted · an endpoint group is empty",
            Self::Dormant => "permitted · populated, no peer up at both ends",
            Self::Realized => "permitted · both ends connected",
        }
    }
    pub fn css(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Latent => "latent",
            Self::Dormant => "dormant",
            Self::Realized => "realized",
        }
    }
}

/// A rule that was READ, is ENABLED and is an ACCEPT — and that group space
/// cannot express, because one of its ends is a bare resource (a peer, a host,
/// a subnet) rather than a group.
///
/// It exists so the picture can say "one rule is not on this canvas, here it
/// is" instead of quietly rendering 59 of 60. That is the difference between a
/// surface you can ask "is anything missing?" and one you cannot.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UnrepresentableRule {
    pub policy: String,
    pub policy_id: String,
    /// `destination_resource` | `source_resource` | `no_sources` | `no_destinations`
    pub reason: String,
    pub sources: Vec<String>,
    pub destinations: Vec<String>,
    /// The resource the rule points at, resolved to a peer where possible.
    pub resource_kind: String,
    pub resource_id: String,
    pub resource_name: String,
    pub resource_ip: String,
    pub constraint: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MeshGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Why the graph looks as it does. Never left empty on an empty graph: a
    /// blank picture with no reason reads as "this reaches nothing", which is a
    /// claim the data may not support.
    pub notes: Vec<String>,
    /// True only when the policy set was actually read. An unread policy set
    /// must never render as "no permitted paths".
    pub policies_read: bool,
    /// Enabled accept rules that group space cannot draw. Never empty by
    /// accident: an empty vec is the claim "every read rule is on the canvas".
    pub unrepresentable: Vec<UnrepresentableRule>,
}

/// One resolved peer→peer path: from, to, the rules permitting it, the group
/// pairs they matched on, the transport constraints, and whether EVERY rule
/// behind it is bidirectional.
type ResolvedPair = (
    String,
    String,
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeSet<String>,
    bool,
);

/// (source group, dest group) -> (rules, constraints, per-rule bidirectional)
type Permits = BTreeMap<(String, String), (BTreeSet<String>, BTreeSet<String>, Vec<bool>)>;

/// Fold the enabled rule set into group→group permits.
///
/// `action: drop` is skipped: NetBird's default is accept and anything else is
/// not a permit. They were previously counted as one.
pub fn permits_from(policies: &[NbPolicy]) -> Permits {
    let mut permits: Permits = BTreeMap::new();
    for pol in policies {
        if !pol.enabled {
            continue;
        }
        for rule in &pol.rules {
            if !rule.enabled {
                continue;
            }
            if !rule.action.is_empty() && !rule.action.eq_ignore_ascii_case("accept") {
                continue;
            }
            let label = if pol.name.is_empty() {
                rule.name.clone()
            } else {
                pol.name.clone()
            };
            for s in &rule.sources {
                for d in &rule.destinations {
                    let e = permits.entry((s.name.clone(), d.name.clone())).or_default();
                    e.0.insert(label.clone());
                    e.1.insert(rule.constraint());
                    e.2.push(rule.bidirectional);
                    // A bidirectional rule also permits the REVERSE pair. Not
                    // recording it is how an asymmetric picture gets drawn as
                    // symmetric.
                    if rule.bidirectional {
                        let r = permits.entry((d.name.clone(), s.name.clone())).or_default();
                        r.0.insert(format!("{label} (reverse of a bidirectional rule)"));
                        r.1.insert(rule.constraint());
                        r.2.push(true);
                    }
                }
            }
        }
    }
    permits
}

fn group_names(p: &NbPeer) -> BTreeSet<String> {
    p.groups.iter().map(|g| g.name.clone()).collect()
}

/// The UNPOPULATED altitude: groups as nodes, enabled rules as edges.
///
/// Deliberately ignores peers except for the member counts on each node, so it
/// renders on a goatnet with nothing enrolled — which is when the segmentation
/// design is easiest to judge and hardest to see.
pub fn policy_graph(groups: &[NbGroup], policies: &[NbPolicy], peers: &[NbPeer]) -> MeshGraph {
    let mut g = MeshGraph {
        policies_read: true,
        ..Default::default()
    };
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for p in peers {
        for name in group_names(p) {
            let e = counts.entry(name).or_insert((0, 0));
            e.0 += 1;
            if p.connected {
                e.1 += 1;
            }
        }
    }
    let mut names: Vec<&str> = groups.iter().map(|x| x.name.as_str()).collect();
    names.sort();
    names.dedup();
    for n in &names {
        let (m, mc) = counts.get(*n).copied().unwrap_or((0, 0));
        g.nodes.push(Node {
            id: format!("g:{n}"),
            label: (*n).to_string(),
            kind: NodeKind::Group,
            connected: None,
            members: Some(m),
            members_connected: Some(mc),
        });
    }
    let known: BTreeSet<&str> = names.iter().copied().collect();
    let permits = permits_from(policies);
    let pair_keys: BTreeSet<(String, String)> = permits.keys().cloned().collect();
    for ((src, dst), (rules, cons, bidi)) in permits {
        // A rule naming a group the taxonomy does not declare is worth seeing,
        // not silently dropping — that is how hand-made dashboard groups hide.
        if !known.contains(src.as_str()) || !known.contains(dst.as_str()) {
            g.notes.push(format!(
                "rule references a group absent from the declared set: {src} → {dst}"
            ));
            continue;
        }
        let (sm, sc) = counts.get(&src).copied().unwrap_or((0, 0));
        let (dm, dc) = counts.get(&dst).copied().unwrap_or((0, 0));
        let live = sc > 0 && dc > 0;
        let all_bidi = !bidi.is_empty() && bidi.iter().all(|b| *b);
        let realization = if sm == 0 || dm == 0 {
            Realization::Latent
        } else if live {
            Realization::Realized
        } else {
            Realization::Dormant
        };
        let unrestricted = cons.iter().any(|c| c == "all protocols · all ports");
        g.edges.push(Edge {
            from: format!("g:{src}"),
            to: format!("g:{dst}"),
            state: if live { EdgeState::ReachableNow } else { EdgeState::Permitted },
            rules: rules.into_iter().collect(),
            via_groups: vec![format!("{src} → {dst}")],
            reciprocal: !all_bidi && pair_keys.contains(&(dst.clone(), src.clone())),
            shadowed: unrestricted && cons.len() > 1,
            realization,
            constraints: cons.into_iter().collect(),
            bidirectional: all_bidi,
        });
    }
    let empty: Vec<&str> = g
        .nodes
        .iter()
        .filter(|n| n.members == Some(0))
        .map(|n| n.label.as_str())
        .collect();
    if !empty.is_empty() {
        g.notes.push(format!(
            "{} declared group(s) have no members: {}. Empty is expected at standup and a finding later — a class declared and never created is how ADR 1116's gear classes stayed invisible.",
            empty.len(),
            empty.join(", ")
        ));
    }
    g
}

/// The PEER altitude: concrete machine → machine paths for a chosen subject.
///
/// Groups are resolved down to peer pairs and ride on the edge as explanation.
/// They are not drawn: a group is the MECHANISM of a path, not the path, and
/// the question is what a machine can reach.
pub fn peer_graph(subject: &BTreeSet<&str>, peers: &[NbPeer], policies: &[NbPolicy]) -> MeshGraph {
    let mut g = MeshGraph {
        policies_read: true,
        ..Default::default()
    };
    let permits = permits_from(policies);
    let mut reachable: BTreeMap<String, bool> = BTreeMap::new();
    let mut own: Vec<(&str, bool)> = Vec::new();
    let mut pairs: Vec<ResolvedPair> = Vec::new();

    for src in peers {
        if !subject.contains(src.name.as_str()) {
            continue;
        }
        own.push((src.name.as_str(), src.connected));
        let sg = group_names(src);
        for dst in peers {
            if dst.name == src.name {
                continue;
            }
            let dg = group_names(dst);
            let (mut rules, mut via, mut cons, mut bidi) =
                (BTreeSet::new(), BTreeSet::new(), BTreeSet::new(), Vec::new());
            for a in &sg {
                for b in &dg {
                    if let Some((r, c, bd)) = permits.get(&(a.clone(), b.clone())) {
                        rules.extend(r.iter().cloned());
                        cons.extend(c.iter().cloned());
                        bidi.extend(bd.iter().copied());
                        via.insert(format!("{a} → {b}"));
                    }
                }
            }
            if rules.is_empty() {
                continue;
            }
            reachable.insert(dst.name.clone(), dst.connected);
            pairs.push((
                src.name.clone(),
                dst.name.clone(),
                rules,
                via,
                cons,
                !bidi.is_empty() && bidi.iter().all(|b| *b),
            ));
        }
    }

    own.sort_by(|a, b| a.0.cmp(b.0));
    for (name, connected) in &own {
        g.nodes.push(Node {
            id: format!("p:{name}"),
            label: (*name).to_string(),
            kind: NodeKind::Peer,
            connected: Some(*connected),
            members: None,
            members_connected: None,
        });
    }
    for (name, connected) in &reachable {
        g.nodes.push(Node {
            id: format!("r:{name}"),
            label: name.clone(),
            kind: NodeKind::Peer,
            connected: Some(*connected),
            members: None,
            members_connected: None,
        });
    }
    let src_live: BTreeMap<&str, bool> = own.iter().map(|(n, c)| (*n, *c)).collect();
    let drawn: BTreeSet<(String, String)> =
        pairs.iter().map(|(f, t, ..)| (f.clone(), t.clone())).collect();
    for (from, to, rules, via, cons, all_bidi) in &pairs {
        let live = *src_live.get(from.as_str()).unwrap_or(&false)
            && *reachable.get(to).unwrap_or(&false);
        let unrestricted = cons.iter().any(|c| c == "all protocols · all ports");
        g.edges.push(Edge {
            from: format!("p:{from}"),
            to: format!("r:{to}"),
            state: if live { EdgeState::ReachableNow } else { EdgeState::Permitted },
            rules: rules.iter().cloned().collect(),
            via_groups: via.iter().cloned().collect(),
            reciprocal: !all_bidi && drawn.contains(&(to.clone(), from.clone())),
            shadowed: unrestricted && cons.len() > 1,
            // Peer scope has both endpoints in hand, so there is no "latent"
            // here — an empty group cannot produce a peer pair at all.
            realization: if live { Realization::Realized } else { Realization::Dormant },
            constraints: cons.iter().cloned().collect(),
            bidirectional: *all_bidi,
        });
    }

    // The module doc-comment promises a graph never renders blank without a
    // reason, and this function used to write no notes at all — so a subject
    // that reached nothing produced a wordless empty picture, which reads as
    // "denied" whether or not that is what happened.
    if own.is_empty() {
        g.notes.push(
            "no peer in the fabric matched the subject — this is a lookup miss, not a denial"
                .to_string(),
        );
    } else if g.edges.is_empty() {
        g.notes.push(format!(
            "{} subject peer(s) read against {} enabled policy set(s); no enabled rule pairs their groups with any other peer's groups. This IS a denial: NetBird returns the complete rule set.",
            own.len(),
            policies.len()
        ));
    }
    g
}

// ===========================================================================
// netvis — the whole-goatnet altitudes
// ===========================================================================

/// The netvis data contract: one struct tree, two altitudes, no hand-built
/// JSON anywhere.
///
/// ## Why the model lives beside the resolver
///
/// The server renders SVG from *this* struct and serves *this* struct as JSON.
/// One value, two encodings, so the picture and the API cannot disagree — the
/// failure mode where a page shows 59 rules and its own JSON says 60 is not
/// available.
///
/// ## The three honesty rules this module enforces mechanically
///
/// 1. **Unread is not empty.** `groups: None` produces `members: null` on every
///    node, never `0`. `policies: None` produces `policies_read: false` and a
///    zeroed ledger, and the renderer is contractually forbidden to draw a
///    frame. These are different pictures and the types keep them different.
/// 2. **The read ledger balances or the build fails.** `rendered + disabled +
///    not_group_addressable + denied + unread == total`, asserted here, so a
///    rule can never fall out of the picture quietly. A rule that is enabled,
///    accept and undrawable in group space still counts.
/// 3. **Permitted is not observed.** [`Realization::Realized`] means both ends
///    have a connected peer. Nothing here watches a packet.
///
/// ## What is computed here and what is not
///
/// Everything structural: scope-family decomposition, the band partition, the
/// canonical slot table, degree, constraint severity, declared-vs-live,
/// realization, and every finding predicate. Geometry is computed too (§B.2.4
/// is a closed formula over the slot table) but lives behind
/// [`apply_geometry`] so a layout crate can overwrite it without touching the
/// resolver.
pub mod netvis {
    use std::collections::{BTreeMap, BTreeSet};

    use serde::{Deserialize, Serialize};

    use super::{Realization, UnrepresentableRule};
    use crate::netbird::{NbGroup, NbPeer, NbPolicy};

    /// Bump on any breaking change to the emitted shape.
    pub const SCHEMA: u32 = 1;
    /// The family bucket for a group that belongs to no scope family.
    pub const UNSCOPED: &str = "<unscoped>";
    /// The suffix of a group named exactly its family.
    pub const SELF_SUFFIX: &str = "<self>";

    // -- geometry constants (§B.2.4). Panel-local y; integers only. ---------
    const LPLATE: i32 = 132;
    const PAD: i32 = 24;
    const RGUT: i32 = 24;
    const CORE_H: i32 = 56;
    const FEED_H: i32 = 40;
    const Y_RAIL1: i32 = 32;
    const Y_RAIL2: i32 = 56;
    const RAIL_H: i32 = 5;
    const Y_CORE: i32 = 92;
    const Y_HUB: i32 = 180;
    const HUB_H: i32 = 7;
    const Y_FEED: i32 = 214;
    const Y_DORM: i32 = 262;
    const PANEL_H: i32 = 302;
    const PANEL_GAP: i32 = 16;
    const RULER_H: i32 = 124;
    const RESIDUE_H: i32 = 96;

    // =======================================================================
    // Inputs
    // =======================================================================

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Altitude {
        /// Groups and rules. Renders on a goatnet with nothing enrolled.
        Policy,
        /// Groups, rules AND the machines in them.
        Fabric,
    }

    impl Altitude {
        pub fn as_str(self) -> &'static str {
            match self {
                Self::Policy => "policy",
                Self::Fabric => "fabric",
            }
        }
    }

    /// One declared group suffix from `ops/ansible/netbird_acl_taxonomy.yml`,
    /// in DECLARATION ORDER — the order is the tier ranking (§B.2.3), so the
    /// caller must not sort this.
    #[derive(Debug, Clone, Default)]
    pub struct DeclaredSuffix {
        pub suffix: String,
        pub purpose: String,
    }

    /// A row from `ops/ci/mesh-reach-declared-deviations.yaml`. A finding that
    /// matches one is rendered MUTED, never removed — hiding it is what trains
    /// people to skim.
    #[derive(Debug, Clone, Default)]
    pub struct Deviation {
        /// Peer name, group name or `src>dst` pair the row accepts.
        pub subject: String,
        pub check: String,
        pub decided_by: String,
        pub decided_on: String,
        pub reason: String,
        pub revisit_when: String,
    }

    /// Everything the builder reads.
    ///
    /// `groups` / `policies` / `peers` are `Option` on purpose: `None` means
    /// THE SOURCE WAS NOT READ and is a different fact from an empty slice.
    /// Collapsing the two is the single worst defect this contract can carry,
    /// so it is not expressible.
    #[derive(Debug, Clone)]
    pub struct Request<'a> {
        pub site: &'a str,
        pub alt: Altitude,
        /// RFC3339. Injected, never read from a clock here — a resolver that
        /// reads the clock cannot have a golden test.
        pub generated_at: &'a str,
        /// Canvas width in SVG user units.
        pub canvas_w: i32,
        /// This goatnet's own declared prefix (`ZAGROS-PROD`). Used only to
        /// decide which declared suffixes are "declared but never created".
        pub site_prefix: &'a str,
        pub groups: Option<&'a [NbGroup]>,
        pub policies: Option<&'a [NbPolicy]>,
        pub peers: Option<&'a [NbPeer]>,
        /// Read-failure reasons, verbatim. `[]` is the claim "nothing failed".
        pub notes: &'a [String],
        pub declared: &'a [DeclaredSuffix],
        /// Rule/policy name → why it exists (`netbird_acl_matrix.yml`).
        pub rule_purpose: &'a BTreeMap<String, String>,
        pub deviations: &'a [Deviation],
    }

    impl<'a> Request<'a> {
        /// A request with every source unread. Build it, then fill in what was
        /// actually read — so forgetting a source under-claims, never over-claims.
        pub fn new(site: &'a str, alt: Altitude, generated_at: &'a str) -> Self {
            Self {
                site,
                alt,
                generated_at,
                canvas_w: 1100,
                site_prefix: "",
                groups: None,
                policies: None,
                peers: None,
                notes: &[],
                declared: &[],
                rule_purpose: EMPTY_PURPOSE.get_or_init(BTreeMap::new),
                deviations: &[],
            }
        }
    }

    static EMPTY_PURPOSE: std::sync::OnceLock<BTreeMap<String, String>> =
        std::sync::OnceLock::new();

    /// The build refused rather than shipping a short page.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum BuildError {
        /// `rendered + disabled + not_group_addressable + denied + unread`
        /// did not equal `total`. HTTP 500 — never a quietly short page.
        ReadLedgerImbalance { ledger: Read, sum: u32 },
    }

    impl std::fmt::Display for BuildError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::ReadLedgerImbalance { ledger, sum } => write!(
                    f,
                    "read ledger does not balance: {sum} accounted of {} rules read",
                    ledger.total
                ),
            }
        }
    }
    impl std::error::Error for BuildError {}

    // =======================================================================
    // The emitted document (§C)
    // =======================================================================

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Doc {
        pub schema: u32,
        pub site: String,
        pub alt: String,
        pub generated_at: String,

        // ── honesty header: all five always present ──
        pub policies_read: bool,
        pub groups_read: bool,
        pub peers_read: bool,
        pub notes: Vec<String>,
        pub read: Read,

        pub counts: Counts,
        pub band_matrix: BTreeMap<String, u32>,

        pub canvas: Canvas,
        pub geom: Geom,
        pub core_ruler_h: i32,
        pub feed_ruler_h: i32,
        pub panel_h: i32,
        pub panel_gap: i32,
        pub bands: Vec<BandRow>,
        pub axes: Axes,
        pub panels: Vec<Panel>,

        pub nodes: Vec<Node>,
        pub edges: Vec<Edge>,
        pub rules: Vec<Rule>,

        pub unrepresentable: Vec<Unrepresentable>,
        pub disabled: Vec<DisabledRule>,
        pub unscoped: Vec<UnscopedGroup>,
        pub undeclared_refs: Vec<UndeclaredRef>,
        pub findings: Vec<Finding>,

        // ── fabric altitude only: ABSENT, not null ──
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signatures: Option<Vec<Signature>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub peers: Option<Vec<Peer>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub reach_bins: Option<Vec<[u32; 2]>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub reach_max: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub reach_median: Option<u32>,
    }

    /// The arithmetic identity that makes silent rule-dropping impossible.
    ///
    /// `denied` is a fifth term the prose contract omitted: a `action: drop`
    /// rule is read, is not a permit, and is neither disabled nor
    /// group-unaddressable. Folding it into either of those would be a lie in
    /// a ledger whose entire job is to not lie, so it gets its own column. It
    /// is 0 on both live goatnets.
    #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Read {
        pub total: u32,
        pub enabled: u32,
        pub disabled: u32,
        pub rendered: u32,
        pub not_group_addressable: u32,
        pub denied: u32,
        pub unread: u32,
    }

    impl Read {
        pub fn accounted(&self) -> u32 {
            self.rendered + self.disabled + self.not_group_addressable + self.denied + self.unread
        }
        pub fn balances(&self) -> bool {
            self.accounted() == self.total
        }
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct Counts {
        pub groups: u32,
        pub peers: u32,
        pub peers_connected: u32,
        pub pairs: u32,
        pub self_pairs: u32,
        pub unrestricted: u32,
        pub scoped: u32,
        pub one_way: u32,
        pub reciprocal: u32,
        pub bidirectional_flag: u32,
        /// Permitted pairs whose realization could not be decided because
        /// membership was not read. Its own column so `latent + dormant +
        /// realized + realization_unknown == pairs` holds — the same ledger
        /// discipline `Read` uses, applied to the ladder.
        pub realization_unknown: u32,
        pub latent: u32,
        pub dormant: u32,
        pub realized: u32,
        pub realized_unrestricted: u32,
        pub groups_empty: u32,
        pub groups_in_no_rule: u32,
        pub groups_reserved: u32,
        pub cross_family_edges: u32,
    }

    /// The geometry constants the client needs to draw the frame the server
    /// solved for.
    ///
    /// These were private `const`s and were never emitted, so the renderer read
    /// `D.geom.lplate` off `undefined` and the whole page fell back to its
    /// read-failure banner. The client MUST NOT hard-code them: `pitch` is
    /// derived from the canvas width and the slot count, so a copy in the JS
    /// would silently disagree the first time a goatnet had a different number
    /// of core slots. One source, serialized.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Geom {
        /// Left plate: x where the panel body starts, right of the rail labels.
        pub lplate: i32,
        /// Inner padding between the plate and the first slot.
        pub pad: i32,
        /// Right gutter reserved for the detour lane.
        pub rgut: i32,
        /// Height of a core tile row.
        pub core_h: i32,
        /// Horizontal distance between adjacent slot centres. DERIVED.
        pub pitch: i32,
    }

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Canvas {
        pub w: i32,
        pub h: i32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct BandRow {
        pub id: String,
        pub label: String,
        pub y: i32,
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct Axes {
        pub core: Vec<Slot>,
        pub feed: Vec<Slot>,
        pub rail: Vec<RailSlot>,
        pub hub: Option<HubSlot>,
        pub dorm: Vec<Slot>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Slot {
        pub suffix: String,
        pub slot: i32,
        pub x: i32,
        pub tier: u32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct RailSlot {
        pub suffix: String,
        pub rank: i32,
        pub y: i32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct HubSlot {
        pub suffix: String,
        pub y: i32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Panel {
        pub i: usize,
        pub family: String,
        pub y: i32,
        pub h: i32,
        pub groups: u32,
        pub populated: u32,
        pub pairs: u32,
        pub unrestricted: u32,
        /// Suffixes another family has and this one does not. The slot is HELD
        /// and drawn empty; collapsing it would reflow the panel and destroy
        /// the comparison the small multiples exist for.
        pub reserved: Vec<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Node {
        pub i: usize,
        pub id: String,
        pub name: String,
        /// Index into `panels`, or -1 for an unscoped group.
        pub panel: i32,
        pub family: String,
        pub suffix: String,
        pub band: String,
        pub slot: i32,
        pub x: i32,
        pub y: i32,
        pub w: i32,
        pub h: i32,
        /// `None` = the groups endpoint was NOT READ. `Some(0)` = read and
        /// empty. Two different pictures; never collapse them.
        pub members: Option<u32>,
        pub members_connected: Option<u32>,
        /// Distinct neighbour GROUPS of this node in the instance graph.
        pub deg_neighbours: u32,
        /// Distinct neighbour SUFFIXES of this node's suffix in the union
        /// template. This is the number the layout uses, and it is smaller
        /// than `deg_neighbours` — a hub suffix can be 30 here and 48 there.
        /// Emitting one number for both would misstate one of them by ~1.6×.
        pub deg_template: u32,
        /// Resolved rule instances touching this node.
        pub deg_instances: u32,
        pub r#in: u32,
        pub out: u32,
        pub declared: bool,
        pub live: bool,
        pub reserved: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub purpose: Option<String>,
        /// Unrestricted pairs that would flip to realized if this empty group
        /// were populated. Zero for a populated group.
        pub latent_paths: u32,
        pub findings: Vec<usize>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Edge {
        pub i: usize,
        pub s: usize,
        pub d: usize,
        pub route: String,
        pub path: String,
        pub dir: String,
        pub unrestricted: bool,
        pub shadowed: bool,
        pub realization: Realization,
        pub constraints: Vec<String>,
        pub rules: Vec<usize>,
        /// The same rule reached this pair as the reverse leg of a
        /// bidirectional rule. Parallel to `rules`.
        pub rules_reverse: Vec<bool>,
        pub findings: Vec<usize>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Rule {
        pub i: usize,
        pub policy: String,
        pub policy_id: String,
        pub rule_name: String,
        pub constraint: String,
        pub protocol: String,
        pub ports: Vec<String>,
        pub bidirectional: bool,
        pub enabled: bool,
        pub unrestricted: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub purpose: Option<String>,
        // The warrant lens (ADR 1129). Decorated by netvis from the port-set
        // registry after the doc is built from the live plane — absent here
        // when nobody filled them, so every other consumer of this struct
        // serializes exactly what it did before.
        //
        // `expires` and `grant_owner` are the load-bearing pair: they are
        // registry-only, stripped by generate.py before the ACL is applied, so
        // they exist NOWHERE in the live plane. A reader of the applied ACL
        // cannot tell a permanent grant from one that lapses on Friday.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub port_set: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub port_set_purpose: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub port_set_owner: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub expires: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub grant_owner: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Unrepresentable {
        pub policy: String,
        pub policy_id: String,
        pub reason: String,
        pub sources: Vec<String>,
        pub source_members: Option<u32>,
        pub target: ResourceTarget,
        pub constraint: String,
        pub enabled: bool,
        pub x: i32,
        pub y: i32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ResourceTarget {
        pub kind: String,
        pub id: String,
        pub name: String,
        pub ip: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct DisabledRule {
        pub policy: String,
        pub policy_id: String,
        pub shape: String,
        pub why: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct UnscopedGroup {
        pub name: String,
        pub members: Option<u32>,
        pub in_any_rule: bool,
        pub node: usize,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct UndeclaredRef {
        pub rule_index: usize,
        pub group_name: String,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Severity {
        Alarm,
        Review,
        Accepted,
    }

    impl Severity {
        fn rank(self) -> u8 {
            match self {
                Self::Alarm => 0,
                Self::Review => 1,
                Self::Accepted => 2,
            }
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum Subject {
        Edge { i: usize },
        Node { i: usize },
        Peer { i: usize },
        Policy { id: String },
        Global,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Finding {
        pub i: usize,
        pub id: String,
        pub severity: Severity,
        pub subject: Subject,
        pub claim: String,
        pub blast_radius: u32,
        pub blast_unit: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub accepted: Option<Accepted>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub action: Option<Action>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Accepted {
        pub decided_by: String,
        pub decided_on: String,
        pub reason: String,
        pub revisit_when: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Action {
        pub doc: String,
        pub command: String,
        pub recheck: String,
        pub declare_yaml: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Signature {
        pub i: usize,
        pub groups: Vec<usize>,
        pub count: u32,
        pub connected: u32,
        pub cross_family: bool,
        pub reach_any: u32,
        pub peers: Vec<usize>,
        pub findings: Vec<usize>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Peer {
        pub i: usize,
        pub id: String,
        pub name: String,
        pub hostname: String,
        pub ip: String,
        pub connected: bool,
        pub os: String,
        pub os_family: String,
        pub last_seen: Option<String>,
        pub created_at: Option<String>,
        pub groups: Vec<usize>,
        pub signature: usize,
        pub reach_any: u32,
        pub findings: Vec<usize>,
        /// ADR 1116 §3 — this machine's gear classes decomposed into families
        /// and slots, filled by the caller from the compiled-in taxonomy.
        ///
        /// Carried on the peer rather than derived in the client so there is
        /// ONE implementation of the decomposition. A JS re-derivation would
        /// be the second, and this tree has already paid for that lesson with
        /// the peer-name grammar.
        ///
        /// Empty for anything holding no gear class, which is most peers.
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        pub gear: serde_json::Value,
    }

    // =======================================================================
    // Algorithm F — scope-family decomposition (§B.2.1)
    // =======================================================================

    /// Which scope family each group belongs to, and the suffix it carries
    /// inside it.
    #[derive(Debug, Clone, Default)]
    pub struct Families {
        /// group name → (family, suffix)
        pub of: BTreeMap<String, (String, String)>,
        /// Every candidate prefix that scored, with its score.
        pub scores: BTreeMap<String, usize>,
        /// Families in render order: suffix-coverage desc, then name asc.
        pub order: Vec<String>,
    }

    impl Families {
        pub fn family(&self, group: &str) -> &str {
            self.of.get(group).map(|(f, _)| f.as_str()).unwrap_or(UNSCOPED)
        }
        pub fn suffix<'b>(&'b self, group: &'b str) -> &'b str {
            self.of.get(group).map(|(_, s)| s.as_str()).unwrap_or(group)
        }
    }

    /// Decompose group names into scope families.
    ///
    /// The obvious rule — "a suffix that occurs under ≥2 prefixes makes its
    /// prefix a family, longest match wins" — over-fires badly: it
    /// promotes `<PREFIX>-EXTERNAL` and `<PREFIX>-PROBATION` to families
    /// because `EXTERNAL-INFRA`'s tail `INFRA` occurs under many
    /// prefixes. That produces apparent cross-family edges and destroys the
    /// small-multiple invariant the whole layout rests on.
    ///
    /// The fix is to score a candidate prefix by how much of its suffix
    /// vocabulary is SHARED with NON-NESTED candidates, and to pick the
    /// highest-scoring cover. On both live goatnets this yields exactly two
    /// families and ZERO cross-family edges.
    pub fn decompose_families(names: &[String]) -> Families {
        // C := prefixes that are a proper '-'-delimited prefix of ≥2 names.
        let mut freq: BTreeMap<String, usize> = BTreeMap::new();
        for g in names {
            let segs: Vec<&str> = g.split('-').collect();
            for k in 1..segs.len() {
                *freq.entry(segs[..k].join("-")).or_insert(0) += 1;
            }
        }
        let candidates: Vec<String> =
            freq.into_iter().filter(|(_, c)| *c >= 2).map(|(p, _)| p).collect();

        let covers = |p: &str, g: &str| g == p || g.starts_with(&format!("{p}-"));
        let sufs: BTreeMap<String, BTreeSet<String>> = candidates
            .iter()
            .map(|p| {
                let s: BTreeSet<String> = names
                    .iter()
                    .filter(|g| covers(p, g))
                    .map(|g| {
                        if g == p {
                            SELF_SUFFIX.to_string()
                        } else {
                            g[p.len() + 1..].to_string()
                        }
                    })
                    .collect();
                (p.clone(), s)
            })
            .collect();
        let nested = |p: &str, q: &str| {
            p == q || p.starts_with(&format!("{q}-")) || q.starts_with(&format!("{p}-"))
        };

        let mut scores: BTreeMap<String, usize> = BTreeMap::new();
        for p in &candidates {
            let mut s = 0usize;
            for q in &candidates {
                if nested(p, q) {
                    continue;
                }
                s += sufs[p].intersection(&sufs[q]).count();
            }
            if s > 0 {
                scores.insert(p.clone(), s);
            }
        }

        let mut of: BTreeMap<String, (String, String)> = BTreeMap::new();
        for g in names {
            let mut best: Option<&String> = None;
            for p in scores.keys() {
                if !covers(p, g) {
                    continue;
                }
                let better = match best {
                    None => true,
                    Some(b) => (scores[p], std::cmp::Reverse(b.len()), std::cmp::Reverse(b))
                        > (scores[b], std::cmp::Reverse(p.len()), std::cmp::Reverse(p)),
                };
                if better {
                    best = Some(p);
                }
            }
            match best {
                Some(p) => {
                    let suffix = if g == p {
                        SELF_SUFFIX.to_string()
                    } else {
                        g[p.len() + 1..].to_string()
                    };
                    of.insert(g.clone(), (p.clone(), suffix));
                }
                None => {
                    of.insert(g.clone(), (UNSCOPED.to_string(), g.clone()));
                }
            }
        }

        // Render order: the family with the widest suffix vocabulary is the
        // reference and sits on top.
        let mut coverage: BTreeMap<String, usize> = BTreeMap::new();
        for (f, s) in of.values() {
            if f != UNSCOPED {
                coverage.entry(f.clone()).or_default();
                let _ = s;
            }
        }
        for f in coverage.clone().keys() {
            let n = of.values().filter(|(ff, _)| ff == f).count();
            coverage.insert(f.clone(), n);
        }
        let mut order: Vec<String> = coverage.keys().cloned().collect();
        order.sort_by(|a, b| coverage[b].cmp(&coverage[a]).then(a.cmp(b)));

        Families { of, scores, order }
    }

    // =======================================================================
    // Band partition (§B.2.2) + canonical slot table (§B.2.3)
    // =======================================================================

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub enum Band {
        Rail,
        Core,
        Hub,
        Feed,
        Dorm,
        Unscoped,
    }

    impl Band {
        pub fn id(self) -> &'static str {
            match self {
                Self::Rail => "RAIL",
                Self::Core => "CORE",
                Self::Hub => "HUB",
                Self::Feed => "FEED",
                Self::Dorm => "DORM",
                Self::Unscoped => "UNSCOPED",
            }
        }
        /// The band's name in the operator's terms. Derived from the graph,
        /// never asserted from a group name.
        pub fn label(self) -> &'static str {
            match self {
                Self::Rail => "AUTHORITY",
                Self::Core => "CORE",
                Self::Hub => "SUBSTRATE",
                Self::Feed => "FEEDERS",
                Self::Dorm => "DORMANT",
                Self::Unscoped => "UNSCOPED",
            }
        }
    }

    /// The union template: one skeleton every family is drawn against.
    #[derive(Debug, Clone, Default)]
    pub struct Template {
        pub suffixes: Vec<String>,
        pub hub: Option<String>,
        pub band: BTreeMap<String, Band>,
        /// Non-self template edges, suffix → suffix.
        pub edges: BTreeSet<(String, String)>,
        pub tin: BTreeMap<String, BTreeSet<String>>,
        pub tout: BTreeMap<String, BTreeSet<String>>,
        /// Slot index within the band.
        pub slot: BTreeMap<String, i32>,
        pub tier: BTreeMap<String, u32>,
    }

    /// Build the union template and classify every suffix into a band.
    ///
    /// Self-loops are removed BEFORE classification. Leaving them in makes
    /// `INFRA` look like a strongly-connected component of one and pushes
    /// `P2P` out of the rail band — an earlier analysis reached exactly that
    /// wrong answer.
    pub fn build_template(
        fams: &Families,
        pairs: &BTreeSet<(String, String)>,
        tier_of_root: &BTreeMap<String, u32>,
    ) -> Template {
        let mut t = Template::default();
        let mut suffixes: BTreeSet<String> = BTreeSet::new();
        for (f, s) in fams.of.values() {
            if f != UNSCOPED {
                suffixes.insert(s.clone());
            }
        }
        t.suffixes = suffixes.iter().cloned().collect();

        for (a, b) in pairs {
            let (fa, sa) = match fams.of.get(a) {
                Some(v) => v,
                None => continue,
            };
            let (fb, sb) = match fams.of.get(b) {
                Some(v) => v,
                None => continue,
            };
            if fa != fb || fa == UNSCOPED {
                continue;
            }
            if sa == sb {
                continue; // self-loop: removed before classification
            }
            t.edges.insert((sa.clone(), sb.clone()));
        }
        for s in &t.suffixes {
            t.tin.insert(s.clone(), BTreeSet::new());
            t.tout.insert(s.clone(), BTreeSet::new());
        }
        for (a, b) in &t.edges {
            t.tout.entry(a.clone()).or_default().insert(b.clone());
            t.tin.entry(b.clone()).or_default().insert(a.clone());
        }

        // hub := the biggest node whose out-neighbours are a subset of its
        // in-neighbours — a bus answers only who already talks to it.
        let empty: BTreeSet<String> = BTreeSet::new();
        let ins = |s: &str| t.tin.get(s).unwrap_or(&empty);
        let outs = |s: &str| t.tout.get(s).unwrap_or(&empty);
        let mut hub: Option<(usize, String)> = None;
        for s in &t.suffixes {
            let (i, o) = (ins(s), outs(s));
            if o.is_empty() || !o.is_subset(i) {
                continue;
            }
            let deg = i.len() + o.len();
            let better = match &hub {
                None => true,
                Some((d, n)) => deg > *d || (deg == *d && s < n),
            };
            if better {
                hub = Some((deg, s.clone()));
            }
        }
        t.hub = hub.map(|(_, n)| n);

        for s in &t.suffixes {
            let (i, o) = (ins(s), outs(s));
            let band = if Some(s) == t.hub.as_ref() {
                Band::Hub
            } else if i.is_empty() && o.is_empty() {
                Band::Dorm
            } else if i.is_empty()
                && !o.is_empty()
                && t.hub.as_ref().is_some_and(|h| o.len() == 1 && o.contains(h))
            {
                Band::Feed
            } else if o.len() >= 3 * i.len().max(1) {
                Band::Rail
            } else {
                Band::Core
            };
            t.band.insert(s.clone(), band);
        }

        // Canonical slot table: one per goatnet, shared by every panel.
        let root = |s: &str| s.split('-').next().unwrap_or(s).to_string();
        for s in &t.suffixes {
            t.tier
                .insert(s.clone(), *tier_of_root.get(&root(s)).unwrap_or(&9999));
        }
        for band in [Band::Core, Band::Feed, Band::Dorm] {
            let mut members: Vec<&String> =
                t.suffixes.iter().filter(|s| t.band[*s] == band).collect();
            members.sort_by(|a, b| {
                (t.tier[*a], root(a), (*a).clone()).cmp(&(t.tier[*b], root(b), (*b).clone()))
            });
            for (i, s) in members.iter().enumerate() {
                t.slot.insert((*s).clone(), i as i32);
            }
        }
        // Rails stack by fan-out: the widest authority on top.
        let mut rails: Vec<&String> =
            t.suffixes.iter().filter(|s| t.band[*s] == Band::Rail).collect();
        rails.sort_by(|a, b| outs(b).len().cmp(&outs(a).len()).then(a.cmp(b)));
        for (i, s) in rails.iter().enumerate() {
            t.slot.insert((*s).clone(), i as i32);
        }
        if let Some(h) = &t.hub {
            t.slot.insert(h.clone(), 0);
        }
        t
    }

    // =======================================================================
    // Build
    // =======================================================================

    /// A built `Rule` on its way into the payload.
    ///
    /// Carried `sources` / `destinations` copies too, which nothing ever read:
    /// pair accumulation walks the original `NbPolicyRule` directly, so the
    /// clones were pure duplication of the very data the pair map is keyed on.
    /// Clippy flagged them dead and CI denies warnings. Removed rather than
    /// silenced — a second copy of the group names is exactly the sort of thing
    /// that later drifts from the first.
    struct RuleRec {
        rec: Rule,
    }

    #[derive(Default)]
    struct PairAcc {
        refs: BTreeSet<(usize, bool)>,
    }

    fn os_family(os: &str) -> &'static str {
        let head = os.split_whitespace().next().unwrap_or("");
        if os.contains("Server") {
            return "server";
        }
        match head {
            "iOS" | "iPadOS" | "Android" => "mobile",
            "Darwin" | "macOS" | "Windows" => "desktop",
            // NetBird's `os` string cannot tell an Orin from a laptop, and
            // guessing would put a class on the page that the data does not
            // support. "unknown" is the honest answer for every Linux.
            _ => "unknown",
        }
    }

    /// Resolve the whole document. See the module doc for the invariants.
    pub fn build(req: &Request<'_>) -> Result<Doc, BuildError> {
        let policies_read = req.policies.is_some();
        let groups_read = req.groups.is_some();
        let peers_read = req.peers.is_some();
        let policies: &[NbPolicy] = req.policies.unwrap_or(&[]);
        let groups: &[NbGroup] = req.groups.unwrap_or(&[]);
        let peers: &[NbPeer] = req.peers.unwrap_or(&[]);

        let mut notes: Vec<String> = req.notes.to_vec();
        if !policies_read {
            notes.push(
                "the policy set was NOT read — this page makes no claim about what is permitted. \
                 An absent line here is an unknown, not a denial."
                    .to_string(),
            );
        }
        if !groups_read {
            notes.push(
                "the groups endpoint was NOT read — every membership count is null, not zero."
                    .to_string(),
            );
        }
        if groups_read && groups.is_empty() {
            notes.push(
                "the groups endpoint returned 0 groups — a real goatnet always has `All`, \
                 so this is a read failure, not an empty mesh."
                    .to_string(),
            );
        }
        if !peers_read {
            notes.push(
                "the peers endpoint was NOT read — the fabric altitude has nothing to populate \
                 with, and occupancy is unknown rather than zero."
                    .to_string(),
            );
        }

        // -- 1. read ledger + rule resolution -------------------------------
        let mut read = Read::default();
        let mut rules: Vec<RuleRec> = Vec::new();
        let mut disabled_rows: Vec<DisabledRule> = Vec::new();
        let mut unrep_raw: Vec<UnrepresentableRule> = Vec::new();
        let mut pairs: BTreeMap<(String, String), PairAcc> = BTreeMap::new();

        for pol in policies {
            for rule in &pol.rules {
                read.total += 1;
                let live = pol.enabled && rule.enabled;
                if !live {
                    read.disabled += 1;
                    let shape = format!(
                        "{} → {}",
                        join_names(&rule.sources),
                        join_names(&rule.destinations)
                    );
                    disabled_rows.push(DisabledRule {
                        policy: pol.name.clone(),
                        policy_id: pol.id.clone(),
                        shape,
                        why: if !pol.enabled {
                            "policy enabled: false".to_string()
                        } else {
                            "rule enabled: false".to_string()
                        },
                    });
                    continue;
                }
                read.enabled += 1;
                if !rule.action.is_empty() && !rule.action.eq_ignore_ascii_case("accept") {
                    read.denied += 1;
                    continue;
                }
                let addressable = !rule.sources.is_empty() && !rule.destinations.is_empty();
                if !addressable {
                    read.not_group_addressable += 1;
                    let (kind, id) = if let Some(r) = &rule.destination_resource {
                        ("destination_resource", r)
                    } else if let Some(r) = &rule.source_resource {
                        ("source_resource", r)
                    } else if rule.sources.is_empty() {
                        ("no_sources", &EMPTY_RESOURCE)
                    } else {
                        ("no_destinations", &EMPTY_RESOURCE)
                    };
                    let peer = peers.iter().find(|p| p.id == id.id);
                    unrep_raw.push(UnrepresentableRule {
                        policy: pol.name.clone(),
                        policy_id: pol.id.clone(),
                        reason: kind.to_string(),
                        sources: rule.sources.iter().map(|g| g.name.clone()).collect(),
                        destinations: rule.destinations.iter().map(|g| g.name.clone()).collect(),
                        resource_kind: id.kind.clone(),
                        resource_id: id.id.clone(),
                        resource_name: peer.map(|p| p.name.clone()).unwrap_or_default(),
                        resource_ip: peer.map(|p| p.ip.clone()).unwrap_or_default(),
                        constraint: rule.constraint(),
                    });
                    continue;
                }
                read.rendered += 1;
                let idx = rules.len();
                let name = if pol.name.is_empty() { rule.name.clone() } else { pol.name.clone() };
                rules.push(RuleRec {
                    rec: Rule {
                        i: idx,
                        policy: name.clone(),
                        policy_id: pol.id.clone(),
                        rule_name: rule.name.clone(),
                        constraint: rule.constraint(),
                        protocol: rule.protocol.clone(),
                        ports: rule.ports.clone(),
                        bidirectional: rule.bidirectional,
                        enabled: true,
                        unrestricted: rule.is_unrestricted(),
                        purpose: req
                            .rule_purpose
                            .get(&name)
                            .or_else(|| req.rule_purpose.get(&rule.name))
                            .cloned(),
                        // Filled by the surface that has the registry compiled
                        // in (netvis); the live plane cannot know them.
                        port_set: None,
                        port_set_purpose: None,
                        port_set_owner: None,
                        expires: None,
                        grant_owner: None,
                    },
                });
                for s in &rule.sources {
                    for d in &rule.destinations {
                        pairs
                            .entry((s.name.clone(), d.name.clone()))
                            .or_default()
                            .refs
                            .insert((idx, false));
                        if rule.bidirectional {
                            pairs
                                .entry((d.name.clone(), s.name.clone()))
                                .or_default()
                                .refs
                                .insert((idx, true));
                        }
                    }
                }
            }
        }
        if !read.balances() {
            return Err(BuildError::ReadLedgerImbalance { sum: read.accounted(), ledger: read });
        }

        // -- 2. group universe ---------------------------------------------
        let live_names: BTreeSet<String> = groups.iter().map(|g| g.name.clone()).collect();
        let declared_full: BTreeSet<String> = if req.site_prefix.is_empty() {
            BTreeSet::new()
        } else {
            req.declared
                .iter()
                .map(|d| format!("{}-{}", req.site_prefix, d.suffix))
                .collect()
        };
        let declared_suffixes: BTreeSet<&str> =
            req.declared.iter().map(|d| d.suffix.as_str()).collect();
        let purpose_of: BTreeMap<&str, &str> = req
            .declared
            .iter()
            .map(|d| (d.suffix.as_str(), d.purpose.as_str()))
            .collect();
        let mut tier_of_root: BTreeMap<String, u32> = BTreeMap::new();
        for (i, d) in req.declared.iter().enumerate() {
            let root = d.suffix.split('-').next().unwrap_or(&d.suffix).to_string();
            tier_of_root.entry(root).or_insert(i as u32);
        }

        let mut universe: BTreeSet<String> = live_names.clone();
        universe.extend(declared_full.iter().cloned());
        if !groups_read {
            // With no group list, the rule set is the only evidence a group
            // exists. Drawing the frame from it is better than drawing nothing
            // — but membership stays null throughout.
            for k in pairs.keys() {
                universe.insert(k.0.clone());
                universe.insert(k.1.clone());
            }
        }
        let names: Vec<String> = universe.iter().cloned().collect();

        // -- 3. families + template ----------------------------------------
        let fams = decompose_families(&names);
        let known: BTreeSet<&str> = names.iter().map(|s| s.as_str()).collect();
        let mut undeclared_refs: Vec<UndeclaredRef> = Vec::new();
        let mut drawn: BTreeMap<(String, String), PairAcc> = BTreeMap::new();
        for (k, v) in pairs {
            let mut missing = Vec::new();
            if !known.contains(k.0.as_str()) {
                missing.push(k.0.clone());
            }
            if !known.contains(k.1.as_str()) {
                missing.push(k.1.clone());
            }
            if missing.is_empty() {
                drawn.insert(k, v);
            } else {
                for g in missing {
                    for (ri, _) in &v.refs {
                        undeclared_refs.push(UndeclaredRef { rule_index: *ri, group_name: g.clone() });
                    }
                }
            }
        }
        undeclared_refs.sort_by(|a, b| {
            (a.rule_index, &a.group_name).cmp(&(b.rule_index, &b.group_name))
        });
        undeclared_refs.dedup_by(|a, b| a.rule_index == b.rule_index && a.group_name == b.group_name);

        let pair_keys: BTreeSet<(String, String)> = drawn.keys().cloned().collect();
        let tmpl = build_template(&fams, &pair_keys, &tier_of_root);

        // -- 4. membership --------------------------------------------------
        let mut members: BTreeMap<String, (u32, u32)> = BTreeMap::new();
        for p in peers {
            for g in &p.groups {
                let e = members.entry(g.name.clone()).or_insert((0, 0));
                e.0 += 1;
                if p.connected {
                    e.1 += 1;
                }
            }
        }
        // Membership is a fact about PEERS, so `peers_read` alone decides it.
        //
        // This was three branches whose first two were identical — clippy's
        // `if_same_then_else`, and CI denies warnings. The dead branch tested
        // `!groups_read && !peers_read`, which is strictly narrower than the
        // `!peers_read` immediately below it and could never be reached on its
        // own. Collapsing changes no behaviour.
        //
        // The `None` matters more than the arithmetic: null means NOT READ and
        // 0 means read-and-empty, and the client renders them differently. A
        // group whose membership we could not look up must never appear as a
        // group with no members.
        let member_count = |n: &str| -> Option<u32> {
            if !peers_read {
                None
            } else {
                Some(members.get(n).map(|(m, _)| *m).unwrap_or(0))
            }
        };
        let connected_count = |n: &str| -> Option<u32> {
            if !peers_read {
                None
            } else {
                Some(members.get(n).map(|(_, c)| *c).unwrap_or(0))
            }
        };

        // -- 5. panels + nodes ---------------------------------------------
        let mut panels: Vec<Panel> = Vec::new();
        for (i, f) in fams.order.iter().enumerate() {
            let gs: Vec<&String> =
                names.iter().filter(|g| fams.family(g) == f.as_str()).collect();
            let mine: BTreeSet<String> =
                gs.iter().map(|g| fams.suffix(g).to_string()).collect();
            let mut reserved: Vec<String> = tmpl
                .suffixes
                .iter()
                .filter(|s| !mine.contains(*s))
                .cloned()
                .collect();
            reserved.sort();
            let ps: Vec<&(String, String)> = pair_keys
                .iter()
                .filter(|(a, b)| fams.family(a) == f.as_str() && fams.family(b) == f.as_str())
                .collect();
            let unrestricted = ps
                .iter()
                .filter(|k| {
                    drawn[*k]
                        .refs
                        .iter()
                        .any(|(ri, _)| rules[*ri].rec.unrestricted)
                })
                .count() as u32;
            panels.push(Panel {
                i,
                family: f.clone(),
                y: 0,
                h: PANEL_H,
                groups: gs.len() as u32,
                populated: gs
                    .iter()
                    .filter(|g| member_count(g).unwrap_or(0) > 0)
                    .count() as u32,
                pairs: ps.len() as u32,
                unrestricted,
                reserved,
            });
        }
        let panel_index: BTreeMap<&str, i32> = fams
            .order
            .iter()
            .enumerate()
            .map(|(i, f)| (f.as_str(), i as i32))
            .collect();

        // Node order is (panel asc, band order, slot asc, name asc) and the
        // client never re-sorts: tab order, reading order and visual order
        // agree by construction.
        let band_rank = |b: Band| match b {
            Band::Rail => 0,
            Band::Core => 1,
            Band::Hub => 2,
            Band::Feed => 3,
            Band::Dorm => 4,
            Band::Unscoped => 5,
        };
        let mut ordered: Vec<&String> = names.iter().collect();
        ordered.sort_by_key(|g| {
            let f = fams.family(g);
            let s = fams.suffix(g);
            let (band, slot) = if f == UNSCOPED {
                (Band::Unscoped, 0)
            } else {
                (
                    *tmpl.band.get(s).unwrap_or(&Band::Dorm),
                    *tmpl.slot.get(s).unwrap_or(&0),
                )
            };
            (
                panel_index.get(f).copied().unwrap_or(i32::MAX),
                band_rank(band),
                slot,
                (*g).clone(),
            )
        });

        let mut node_of: BTreeMap<&str, usize> = BTreeMap::new();
        let mut nodes: Vec<Node> = Vec::new();
        for (i, g) in ordered.iter().enumerate() {
            node_of.insert(g.as_str(), i);
            let f = fams.family(g).to_string();
            let s = fams.suffix(g).to_string();
            let unscoped = f == UNSCOPED;
            let band = if unscoped {
                Band::Unscoped
            } else {
                *tmpl.band.get(&s).unwrap_or(&Band::Dorm)
            };
            let mut in_deg = 0u32;
            let mut out_deg = 0u32;
            let mut nb: BTreeSet<&str> = BTreeSet::new();
            let mut inst = 0u32;
            let mut latent_paths = 0u32;
            let m = member_count(g);
            for (k, acc) in &drawn {
                let touches_src = k.0 == **g;
                let touches_dst = k.1 == **g;
                if !touches_src && !touches_dst {
                    continue;
                }
                if touches_src {
                    out_deg += 1;
                    nb.insert(k.1.as_str());
                }
                if touches_dst {
                    in_deg += 1;
                    nb.insert(k.0.as_str());
                }
                inst += acc.refs.len() as u32;
                if m == Some(0) && acc.refs.iter().any(|(ri, _)| rules[*ri].rec.unrestricted) {
                    let other = if touches_src { &k.1 } else { &k.0 };
                    if other != *g && member_count(other).unwrap_or(0) > 0 {
                        latent_paths += 1;
                    }
                }
            }
            nb.remove(g.as_str());
            let live = live_names.contains(*g);
            let declared = declared_suffixes.contains(s.as_str()) || declared_full.contains(*g);
            nodes.push(Node {
                i,
                id: format!("g:{g}"),
                name: (*g).clone(),
                panel: if unscoped { -1 } else { panel_index[f.as_str()] },
                family: f,
                suffix: s.clone(),
                band: band.id().to_string(),
                slot: if unscoped { 0 } else { *tmpl.slot.get(&s).unwrap_or(&0) },
                x: 0,
                y: 0,
                w: 0,
                h: 0,
                members: m,
                members_connected: connected_count(g),
                deg_neighbours: nb.len() as u32,
                deg_template: tmpl
                    .tin
                    .get(&s)
                    .map(|i| i.union(tmpl.tout.get(&s).unwrap_or(i)).count())
                    .unwrap_or(0) as u32,
                deg_instances: inst,
                r#in: in_deg,
                out: out_deg,
                declared,
                live,
                reserved: false,
                purpose: purpose_of.get(s.as_str()).map(|p| (*p).to_string()),
                latent_paths,
                findings: Vec::new(),
            });
        }

        // -- 6. edges -------------------------------------------------------
        let mut edges: Vec<Edge> = Vec::new();
        for (k, acc) in &drawn {
            let s = node_of[k.0.as_str()];
            let d = node_of[k.1.as_str()];
            let unrestricted = acc.refs.iter().any(|(ri, _)| rules[*ri].rec.unrestricted);
            let scoped_too = acc.refs.iter().any(|(ri, _)| !rules[*ri].rec.unrestricted);
            let all_bidi = acc.refs.iter().all(|(ri, _)| rules[*ri].rec.bidirectional);
            let dir = if all_bidi {
                "bidirectional_flag"
            } else if pair_keys.contains(&(k.1.clone(), k.0.clone())) {
                "reciprocal"
            } else {
                "one_way"
            };
            let (sm, sc) = (nodes[s].members, nodes[s].members_connected);
            let (dm, dc) = (nodes[d].members, nodes[d].members_connected);
            // UNREAD IS NOT EMPTY, AND IT IS NOT POPULATED EITHER. This was
            // three branches with no `None` arm, so a goatnet whose peer list
            // failed to read emitted `Dormant` — "both endpoint groups have
            // members, none up" — on every edge. Nothing in the payload
            // supported that, and it is the field the client is about to draw
            // as its liveness channel.
            let realization = match (sm, dm) {
                (None, _) | (_, None) => Realization::Unknown,
                (Some(0), _) | (_, Some(0)) => Realization::Latent,
                _ if sc.unwrap_or(0) > 0 && dc.unwrap_or(0) > 0 => Realization::Realized,
                _ => Realization::Dormant,
            };
            let mut constraints: Vec<String> = acc
                .refs
                .iter()
                .map(|(ri, _)| rules[*ri].rec.constraint.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            constraints.sort();
            let route = route_class(&nodes[s], &nodes[d], &tmpl, k.0 == k.1);
            edges.push(Edge {
                i: edges.len(),
                s,
                d,
                route: route.to_string(),
                path: String::new(),
                dir: dir.to_string(),
                unrestricted,
                shadowed: unrestricted && scoped_too,
                realization,
                constraints,
                rules: acc.refs.iter().map(|(ri, _)| *ri).collect(),
                rules_reverse: acc.refs.iter().map(|(_, rev)| *rev).collect(),
                findings: Vec::new(),
            });
        }
        edges.sort_by_key(|e| (e.s, e.d));
        for (i, e) in edges.iter_mut().enumerate() {
            e.i = i;
        }

        // -- 7. fabric altitude --------------------------------------------
        let mut peer_rows: Vec<Peer> = Vec::new();
        let mut signatures: Vec<Signature> = Vec::new();
        let mut reach_bins: Vec<[u32; 2]> = Vec::new();
        let mut reach_max = 0u32;
        let mut reach_median = 0u32;
        if req.alt == Altitude::Fabric && peers_read {
            let mut sorted: Vec<&NbPeer> = peers.iter().collect();
            sorted.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
            // Any-port reach: how many OTHER peers an unrestricted rule lets
            // this one touch. Recomputed from the policy set, never taken from
            // NetBird's own accessible_peers_count, so the number and the
            // lines on the page come from one source.
            let open: BTreeSet<&(String, String)> = drawn
                .iter()
                .filter(|(_, acc)| acc.refs.iter().any(|(ri, _)| rules[*ri].rec.unrestricted))
                .map(|(k, _)| k)
                .collect();
            let gset: Vec<BTreeSet<&str>> = sorted
                .iter()
                .map(|p| p.groups.iter().map(|g| g.name.as_str()).collect())
                .collect();
            let mut reach: Vec<u32> = Vec::with_capacity(sorted.len());
            for a in 0..sorted.len() {
                let mut n = 0u32;
                for b in 0..sorted.len() {
                    if a == b {
                        continue;
                    }
                    let hit = gset[a].iter().any(|x| {
                        gset[b]
                            .iter()
                            .any(|y| open.contains(&((*x).to_string(), (*y).to_string())))
                    });
                    if hit {
                        n += 1;
                    }
                }
                reach.push(n);
            }
            let mut sig_of: BTreeMap<Vec<usize>, usize> = BTreeMap::new();
            for (i, p) in sorted.iter().enumerate() {
                let mut gidx: Vec<usize> = p
                    .groups
                    .iter()
                    .filter_map(|g| node_of.get(g.name.as_str()).copied())
                    .collect();
                gidx.sort_unstable();
                gidx.dedup();
                let si = *sig_of.entry(gidx.clone()).or_insert_with(|| {
                    let fs: BTreeSet<&str> = gidx
                        .iter()
                        .map(|n| nodes[*n].family.as_str())
                        .filter(|f| *f != UNSCOPED)
                        .collect();
                    signatures.push(Signature {
                        i: 0,
                        groups: gidx.clone(),
                        count: 0,
                        connected: 0,
                        cross_family: fs.len() >= 2,
                        reach_any: reach[i],
                        peers: Vec::new(),
                        findings: Vec::new(),
                    });
                    signatures.len() - 1
                });
                signatures[si].count += 1;
                if p.connected {
                    signatures[si].connected += 1;
                }
                signatures[si].peers.push(i);
                peer_rows.push(Peer {
                    i,
                    id: p.id.clone(),
                    name: p.name.clone(),
                    hostname: p.hostname.clone(),
                    ip: p.ip.clone(),
                    connected: p.connected,
                    os: p.os.clone(),
                    os_family: os_family(&p.os).to_string(),
                    last_seen: (!p.last_seen.is_empty()).then(|| p.last_seen.clone()),
                    created_at: (!p.created_at.is_empty()).then(|| p.created_at.clone()),
                    groups: gidx,
                    signature: si,
                    reach_any: reach[i],
                    findings: Vec::new(),
                    gear: serde_json::Value::Null,
                });
            }
            // Signature order: biggest class first, then by group set — a
            // deterministic tie-break, so the strip is in the same place
            // tomorrow.
            let mut order: Vec<usize> = (0..signatures.len()).collect();
            order.sort_by(|a, b| {
                signatures[*b]
                    .count
                    .cmp(&signatures[*a].count)
                    .then(signatures[*a].groups.cmp(&signatures[*b].groups))
            });
            let mut remap = vec![0usize; signatures.len()];
            let mut reordered: Vec<Signature> = Vec::with_capacity(signatures.len());
            for (new, old) in order.iter().enumerate() {
                remap[*old] = new;
                let mut s = signatures[*old].clone();
                s.i = new;
                reordered.push(s);
            }
            signatures = reordered;
            for p in &mut peer_rows {
                p.signature = remap[p.signature];
            }
            let mut sr = reach.clone();
            sr.sort_unstable();
            reach_max = sr.last().copied().unwrap_or(0);
            reach_median = sr.get(sr.len() / 2).copied().unwrap_or(0);
            let mut bins: BTreeMap<u32, u32> = BTreeMap::new();
            for r in &reach {
                *bins.entry(*r).or_insert(0) += 1;
            }
            reach_bins = bins.into_iter().map(|(r, c)| [r, c]).collect();
        }

        // -- 8. findings ----------------------------------------------------
        let cross_family_edges = edges
            .iter()
            .filter(|e| {
                let (a, b) = (&nodes[e.s].family, &nodes[e.d].family);
                a != b && a != UNSCOPED && b != UNSCOPED
            })
            .count() as u32;
        let findings = collect_findings(FindingCtx {
            req,
            nodes: &nodes,
            edges: &edges,
            peers: &peer_rows,
            unrep: &unrep_raw,
            cross_family_edges,
            peers_read,
        });
        for (i, f) in findings.iter().enumerate() {
            let _ = i;
            match &f.subject {
                Subject::Edge { i } => edges[*i].findings.push(f.i),
                Subject::Node { i } => nodes[*i].findings.push(f.i),
                Subject::Peer { i } => {
                    peer_rows[*i].findings.push(f.i);
                    let si = peer_rows[*i].signature;
                    if !signatures.is_empty() && !signatures[si].findings.contains(&f.i) {
                        signatures[si].findings.push(f.i);
                    }
                }
                Subject::Policy { .. } | Subject::Global => {}
            }
        }

        // -- 9. residue lanes -----------------------------------------------
        let in_rule: BTreeSet<&str> = pair_keys
            .iter()
            .flat_map(|(a, b)| [a.as_str(), b.as_str()])
            .collect();
        let unscoped: Vec<UnscopedGroup> = nodes
            .iter()
            .filter(|n| n.family == UNSCOPED)
            .map(|n| UnscopedGroup {
                name: n.name.clone(),
                members: n.members,
                in_any_rule: in_rule.contains(n.name.as_str()),
                node: n.i,
            })
            .collect();
        let unrepresentable: Vec<Unrepresentable> = unrep_raw
            .iter()
            .map(|u| Unrepresentable {
                policy: u.policy.clone(),
                policy_id: u.policy_id.clone(),
                reason: u.reason.clone(),
                sources: u.sources.clone(),
                source_members: u
                    .sources
                    .first()
                    .and_then(|s| member_count(s)),
                target: ResourceTarget {
                    kind: u.resource_kind.clone(),
                    id: u.resource_id.clone(),
                    name: u.resource_name.clone(),
                    ip: u.resource_ip.clone(),
                },
                constraint: u.constraint.clone(),
                enabled: true,
                x: 0,
                y: 0,
            })
            .collect();

        // -- 10. counts -----------------------------------------------------
        let mut counts = Counts {
            groups: names.len() as u32,
            peers: if peers_read { peers.len() as u32 } else { 0 },
            peers_connected: peers.iter().filter(|p| p.connected).count() as u32,
            pairs: edges.len() as u32,
            groups_reserved: panels.iter().map(|p| p.reserved.len() as u32).sum(),
            groups_in_no_rule: names
                .iter()
                .filter(|g| !in_rule.contains(g.as_str()))
                .count() as u32,
            cross_family_edges,
            ..Default::default()
        };
        for e in &edges {
            if e.s == e.d {
                counts.self_pairs += 1;
            }
            if e.unrestricted {
                counts.unrestricted += 1;
            } else {
                counts.scoped += 1;
            }
            match e.dir.as_str() {
                "one_way" => counts.one_way += 1,
                "reciprocal" => counts.reciprocal += 1,
                _ => counts.bidirectional_flag += 1,
            }
            match e.realization {
                Realization::Unknown => counts.realization_unknown += 1,
                Realization::Latent => counts.latent += 1,
                Realization::Dormant => counts.dormant += 1,
                Realization::Realized => {
                    counts.realized += 1;
                    if e.unrestricted {
                        counts.realized_unrestricted += 1;
                    }
                }
            }
        }
        counts.groups_empty = nodes.iter().filter(|n| n.members == Some(0)).count() as u32;

        let mut band_matrix: BTreeMap<String, u32> = BTreeMap::new();
        for e in &edges {
            let key = if e.s == e.d {
                "SELF".to_string()
            } else if nodes[e.s].family == UNSCOPED || nodes[e.d].family == UNSCOPED {
                "UNSCOPED".to_string()
            } else {
                format!("{}>{}", nodes[e.s].band, nodes[e.d].band)
            };
            *band_matrix.entry(key).or_insert(0) += 1;
        }

        // -- 11. axes + geometry --------------------------------------------
        let axes = build_axes(&tmpl);
        // geom.pitch is filled by apply_geometry, which is the only place the
        // pitch is derived; leaving it 0 here would let a caller that skips
        // apply_geometry serialize a frame the client cannot draw.
        let mut doc = Doc {
            schema: SCHEMA,
            site: req.site.to_lowercase(),
            alt: req.alt.as_str().to_string(),
            generated_at: req.generated_at.to_string(),
            policies_read,
            groups_read,
            peers_read,
            notes,
            read,
            counts,
            band_matrix,
            geom: Geom::default(),
            canvas: Canvas { w: req.canvas_w, h: 0 },
            core_ruler_h: RULER_H,
            feed_ruler_h: RULER_H,
            panel_h: PANEL_H,
            panel_gap: PANEL_GAP,
            bands: vec![
                BandRow { id: "RAIL".into(), label: Band::Rail.label().into(), y: Y_RAIL1 },
                BandRow { id: "CORE".into(), label: Band::Core.label().into(), y: Y_CORE },
                BandRow { id: "HUB".into(), label: Band::Hub.label().into(), y: Y_HUB },
                BandRow { id: "FEED".into(), label: Band::Feed.label().into(), y: Y_FEED },
                BandRow { id: "DORM".into(), label: Band::Dorm.label().into(), y: Y_DORM },
            ],
            axes,
            panels,
            nodes,
            edges,
            rules: rules.into_iter().map(|r| r.rec).collect(),
            unrepresentable,
            disabled: disabled_rows,
            unscoped,
            undeclared_refs,
            findings,
            signatures: (req.alt == Altitude::Fabric).then_some(signatures),
            peers: (req.alt == Altitude::Fabric).then_some(peer_rows),
            reach_bins: (req.alt == Altitude::Fabric).then_some(reach_bins),
            reach_max: (req.alt == Altitude::Fabric).then_some(reach_max),
            reach_median: (req.alt == Altitude::Fabric).then_some(reach_median),
        };
        apply_geometry(&mut doc);
        Ok(doc)
    }

    static EMPTY_RESOURCE: crate::netbird::NbResourceRef = crate::netbird::NbResourceRef {
        id: String::new(),
        kind: String::new(),
    };

    fn join_names(g: &[NbGroup]) -> String {
        if g.is_empty() {
            "(resource)".to_string()
        } else {
            g.iter().map(|x| x.name.as_str()).collect::<Vec<_>>().join(", ")
        }
    }

    fn route_class(s: &Node, d: &Node, tmpl: &Template, self_pair: bool) -> &'static str {
        if self_pair {
            return "selfloop";
        }
        if s.family != d.family {
            return "cross";
        }
        let _ = tmpl;
        match (s.band.as_str(), d.band.as_str()) {
            ("RAIL", "CORE") | ("CORE", "HUB") | ("HUB", "CORE") | ("FEED", "HUB") => "stub",
            ("CORE", "CORE") => "detour",
            ("RAIL", "HUB") | ("HUB", "RAIL") => "gutter",
            ("RAIL", "RAIL") => "railhop",
            _ => "detour",
        }
    }

    fn build_axes(t: &Template) -> Axes {
        let mut axes = Axes::default();
        let of_band = |band: Band| -> Vec<Slot> {
            let mut v: Vec<Slot> = t
                .suffixes
                .iter()
                .filter(|s| t.band.get(*s) == Some(&band))
                .map(|s| Slot {
                    suffix: s.clone(),
                    slot: *t.slot.get(s).unwrap_or(&0),
                    x: 0,
                    tier: *t.tier.get(s).unwrap_or(&9999),
                })
                .collect();
            v.sort_by_key(|s| s.slot);
            v
        };
        axes.core = of_band(Band::Core);
        axes.feed = of_band(Band::Feed);
        axes.dorm = of_band(Band::Dorm);
        let mut rail: Vec<RailSlot> = t
            .suffixes
            .iter()
            .filter(|s| t.band.get(*s) == Some(&Band::Rail))
            .map(|s| RailSlot {
                suffix: s.clone(),
                rank: *t.slot.get(s).unwrap_or(&0),
                y: 0,
            })
            .collect();
        rail.sort_by_key(|r| r.rank);
        for r in &mut rail {
            r.y = if r.rank == 0 { Y_RAIL1 } else { Y_RAIL2 };
        }
        axes.rail = rail;
        axes.hub = t.hub.clone().map(|h| HubSlot { suffix: h, y: Y_HUB });
        axes
    }

    /// §B.2.4 geometry: a closed formula over the slot table.
    ///
    /// Integers only, no floats, no clock, no map-iteration order — so the
    /// same input bytes give a byte-identical SVG string. Separated from the
    /// resolver so a layout crate can replace it wholesale.
    pub fn apply_geometry(doc: &mut Doc) {
        let n_core = doc.axes.core.len().max(1) as i32;
        let n_feed = doc.axes.feed.len().max(1) as i32;
        let span = doc.canvas.w - LPLATE - PAD - RGUT;
        let pitch = (span / n_core.max(n_feed)).clamp(40, 88);
        let tile_w = 32.min(pitch - 8).max(8);
        let bar_x = LPLATE + PAD;
        let bar_w = doc.canvas.w - RGUT - bar_x;
        let cx = |slot: i32| bar_x + slot * pitch + pitch / 2;

        // Publish the frame constants the client draws against. Derived here,
        // never duplicated in JS — see Geom's note.
        doc.geom = Geom { lplate: LPLATE, pad: PAD, rgut: RGUT, core_h: CORE_H, pitch };

        for s in &mut doc.axes.core {
            s.x = cx(s.slot);
        }
        for s in &mut doc.axes.feed {
            s.x = cx(s.slot);
        }
        for s in &mut doc.axes.dorm {
            s.x = cx(s.slot);
        }
        for (i, p) in doc.panels.iter_mut().enumerate() {
            p.y = RULER_H + (i as i32) * (PANEL_H + PANEL_GAP);
            p.h = PANEL_H;
        }
        let panel_y: Vec<i32> = doc.panels.iter().map(|p| p.y).collect();
        let unscoped_y = RULER_H
            + (doc.panels.len() as i32) * (PANEL_H + PANEL_GAP)
            + RULER_H;

        for n in &mut doc.nodes {
            let base = if n.panel >= 0 { panel_y[n.panel as usize] } else { unscoped_y };
            match n.band.as_str() {
                "RAIL" => {
                    n.x = bar_x;
                    n.w = bar_w;
                    n.h = RAIL_H;
                    n.y = base + if n.slot == 0 { Y_RAIL1 } else { Y_RAIL2 };
                }
                "HUB" => {
                    n.x = bar_x;
                    n.w = bar_w;
                    n.h = HUB_H;
                    n.y = base + Y_HUB;
                }
                "CORE" => {
                    n.x = cx(n.slot) - tile_w / 2;
                    n.w = tile_w;
                    n.h = CORE_H;
                    n.y = base + Y_CORE;
                }
                "FEED" => {
                    n.x = cx(n.slot) - tile_w / 2;
                    n.w = tile_w;
                    n.h = FEED_H;
                    n.y = base + Y_FEED;
                }
                "DORM" => {
                    n.x = cx(n.slot) - tile_w / 2;
                    n.w = tile_w;
                    n.h = 18;
                    n.y = base + Y_DORM;
                }
                _ => {
                    n.x = bar_x + n.slot * 120;
                    n.w = 112;
                    n.h = 18;
                    n.y = unscoped_y;
                }
            }
        }

        let geom: Vec<(i32, i32, i32, i32, String, String)> = doc
            .nodes
            .iter()
            .map(|n| (n.x, n.y, n.w, n.h, n.band.clone(), n.slot.to_string()))
            .collect();
        for e in &mut doc.edges {
            let (sx, sy, sw, sh, sb, sslot) = &geom[e.s];
            let (dx, dy, _dw, dh, db, _) = &geom[e.d];
            let scx = sx + sw / 2;
            let dcx = dx + _dw / 2;
            e.path = match e.route.as_str() {
                "selfloop" => {
                    let x = sx + 12;
                    format!("M{x},{} L{x},{} L{},{} L{},{}", sy, sy - 12, x + 12, sy - 12, x + 12, *sy)
                }
                "stub" => match (sb.as_str(), db.as_str()) {
                    ("RAIL", "CORE") => format!("M{dcx},{} L{dcx},{}", sy + sh, dy),
                    ("CORE", "HUB") => format!("M{scx},{} L{scx},{}", sy + sh, dy),
                    ("HUB", "CORE") => format!("M{dcx},{} L{dcx},{}", sy, dy + dh),
                    _ => format!("M{scx},{} L{scx},{}", sy, dy + dh),
                },
                "detour" => {
                    let lane = sy + sh + 6 + (sslot.parse::<i32>().unwrap_or(0) % 4) * 6;
                    format!("M{scx},{} L{scx},{lane} L{dcx},{lane} L{dcx},{}", sy + sh, dy + dh)
                }
                "gutter" => {
                    let gx = if sb == "RAIL" { LPLATE + 12 } else { LPLATE + 24 };
                    format!("M{gx},{} L{gx},{}", sy + sh / 2, dy + dh / 2)
                }
                "railhop" => {
                    let x = bar_x + 6;
                    format!("M{x},{} L{x},{}", sy + sh, *dy)
                }
                _ => format!("M{scx},{} L{dcx},{}", sy + sh, *dy),
            };
        }

        for (i, u) in doc.unrepresentable.iter_mut().enumerate() {
            u.x = PAD;
            u.y = unscoped_y + 40 + (i as i32) * 28;
        }
        doc.canvas.h = unscoped_y + RESIDUE_H + 40 * (doc.unrepresentable.len() as i32);
    }

    // =======================================================================
    // Findings — every deviation is a predicate (§B.4)
    // =======================================================================

    struct FindingCtx<'a> {
        req: &'a Request<'a>,
        nodes: &'a [Node],
        edges: &'a [Edge],
        peers: &'a [Peer],
        unrep: &'a [UnrepresentableRule],
        cross_family_edges: u32,
        peers_read: bool,
    }

    fn accept_row<'a>(devs: &'a [Deviation], subject: &str) -> Option<&'a Deviation> {
        devs.iter().find(|d| d.subject == subject)
    }

    fn declare_yaml(subject: &str, id: &str, who: &str) -> String {
        format!(
            "- subject: {subject}\n  check: {id}\n  decided_by: {who}\n  decided_on: \n  reason: \n  revisit_when: "
        )
    }

    fn collect_findings(c: FindingCtx<'_>) -> Vec<Finding> {
        let mut out: Vec<(Severity, u32, String, String, Finding)> = Vec::new();
        let mut push = |sev: Severity,
                        blast: u32,
                        id: &str,
                        subject_key: String,
                        f: Finding| {
            out.push((sev, blast, id.to_string(), subject_key, f));
        };
        let devs = c.req.deviations;
        let who = "operator";

        // D1 / D2 / D12 — edges
        for e in c.edges {
            let (sn, dn) = (&c.nodes[e.s], &c.nodes[e.d]);
            let key = format!("{}>{}", sn.name, dn.name);
            let blast = sn.members.unwrap_or(0) * dn.members.unwrap_or(0);
            if e.unrestricted && e.realization == Realization::Realized {
                let acc = accept_row(devs, &key);
                push(
                    if acc.is_some() { Severity::Accepted } else { Severity::Alarm },
                    blast,
                    "D1",
                    key.clone(),
                    Finding {
                        i: 0,
                        id: "D1".into(),
                        severity: if acc.is_some() { Severity::Accepted } else { Severity::Alarm },
                        subject: Subject::Edge { i: e.i },
                        claim: format!(
                            "{} → {} is unrestricted (all protocols, all ports) and both ends have a connected peer now",
                            sn.name, dn.name
                        ),
                        blast_radius: blast,
                        blast_unit: "peers at source × peers at destination".into(),
                        accepted: acc.map(to_accepted),
                        action: Some(Action {
                            doc: "docs/design/mesh-acl.md".into(),
                            command: format!("rules: {}", e.rules.len()),
                            recheck: "ops/ci/check-mesh-reach-overgrant.sh".into(),
                            declare_yaml: declare_yaml(&key, "D1", who),
                        }),
                    },
                );
            }
            if e.shadowed {
                let acc = accept_row(devs, &key);
                push(
                    if acc.is_some() { Severity::Accepted } else { Severity::Alarm },
                    blast,
                    "D2",
                    key.clone(),
                    Finding {
                        i: 0,
                        id: "D2".into(),
                        severity: if acc.is_some() { Severity::Accepted } else { Severity::Alarm },
                        subject: Subject::Edge { i: e.i },
                        claim: format!(
                            "{} → {} carries an unrestricted rule behind which {} scoped rule(s) grant nothing extra — a control that controls nothing",
                            sn.name,
                            dn.name,
                            e.constraints.len().saturating_sub(1)
                        ),
                        blast_radius: blast,
                        blast_unit: "peers at source × peers at destination".into(),
                        accepted: acc.map(to_accepted),
                        action: Some(Action {
                            doc: "ops/ansible/netbird_acl_matrix.yml".into(),
                            command: format!("rules: {}", e.rules.len()),
                            recheck: "ops/ci/check-mesh-reach-overgrant.sh".into(),
                            declare_yaml: declare_yaml(&key, "D2", who),
                        }),
                    },
                );
            }
            if sn.family != dn.family && sn.family != UNSCOPED && dn.family != UNSCOPED {
                push(
                    Severity::Alarm,
                    blast,
                    "D12",
                    key.clone(),
                    Finding {
                        i: 0,
                        id: "D12".into(),
                        severity: Severity::Alarm,
                        subject: Subject::Edge { i: e.i },
                        claim: format!(
                            "a policy edge crosses scope families: {} ({}) → {} ({})",
                            sn.name, sn.family, dn.name, dn.family
                        ),
                        blast_radius: blast,
                        blast_unit: "peers at source × peers at destination".into(),
                        accepted: None,
                        action: None,
                    },
                );
            }
        }

        // D7 / D8 / D10 — groups
        for n in c.nodes {
            let deg = n.r#in + n.out;
            if n.members == Some(0) && deg >= 8 {
                let acc = accept_row(devs, &n.name);
                push(
                    if acc.is_some() { Severity::Accepted } else { Severity::Review },
                    n.latent_paths,
                    "D7",
                    n.name.clone(),
                    Finding {
                        i: 0,
                        id: "D7".into(),
                        severity: if acc.is_some() { Severity::Accepted } else { Severity::Review },
                        subject: Subject::Node { i: n.i },
                        claim: format!(
                            "{} carries {deg} permitted path(s) and has zero members — authority with nothing behind it",
                            n.name
                        ),
                        blast_radius: n.latent_paths,
                        blast_unit: "unrestricted paths that would wake on population".into(),
                        accepted: acc.map(to_accepted),
                        action: Some(Action {
                            doc: "docs/design/mesh-acl.md".into(),
                            command: String::new(),
                            recheck: "ops/ci/check-membership-reconcile.py".into(),
                            declare_yaml: declare_yaml(&n.name, "D7", who),
                        }),
                    },
                );
            }
            if n.members == Some(0) && n.latent_paths > 0 {
                push(
                    Severity::Review,
                    n.latent_paths,
                    "D8",
                    n.name.clone(),
                    Finding {
                        i: 0,
                        id: "D8".into(),
                        severity: Severity::Review,
                        subject: Subject::Node { i: n.i },
                        claim: format!(
                            "populating {} creates {} unrestricted path(s) to groups that already have peers",
                            n.name, n.latent_paths
                        ),
                        blast_radius: n.latent_paths,
                        blast_unit: "unrestricted paths that would wake on population".into(),
                        accepted: None,
                        action: None,
                    },
                );
            }
            let residue = (n.live && !n.declared) || (n.declared && !n.live);
            if residue && !c.req.declared.is_empty() {
                push(
                    Severity::Review,
                    n.r#in + n.out,
                    "D10",
                    n.name.clone(),
                    Finding {
                        i: 0,
                        id: "D10".into(),
                        severity: Severity::Review,
                        subject: Subject::Node { i: n.i },
                        claim: if n.live {
                            format!(
                                "{} exists in NetBird and is in no declared taxonomy — nothing reviews what it grants",
                                n.name
                            )
                        } else {
                            format!(
                                "{} is declared in the taxonomy and was never created in NetBird",
                                n.name
                            )
                        },
                        blast_radius: n.r#in + n.out,
                        blast_unit: "permitted paths".into(),
                        accepted: None,
                        action: Some(Action {
                            doc: "ops/ansible/netbird_acl_taxonomy.yml".into(),
                            command: String::new(),
                            recheck: "ops/ci/check-membership-reconcile.py".into(),
                            declare_yaml: declare_yaml(&n.name, "D10", who),
                        }),
                    },
                );
            }
        }

        // D9 — read, enabled, accept, undrawable
        for u in c.unrep {
            push(
                Severity::Alarm,
                1,
                "D9",
                u.policy_id.clone(),
                Finding {
                    i: 0,
                    id: "D9".into(),
                    severity: Severity::Alarm,
                    subject: Subject::Policy { id: u.policy_id.clone() },
                    claim: format!(
                        "policy \"{}\" ({}) is enabled and permits {} → {} {} {} — group space cannot draw it, so it is named here instead of dropped",
                        u.policy,
                        u.policy_id,
                        u.sources.join(", "),
                        if u.resource_name.is_empty() { u.resource_kind.clone() } else { u.resource_name.clone() },
                        u.resource_ip,
                        u.constraint
                    ),
                    blast_radius: 1,
                    blast_unit: "rule".into(),
                    accepted: None,
                    action: Some(Action {
                        doc: "docs/design/mesh-acl.md".into(),
                        command: format!("policy {}", u.policy_id),
                        recheck: "ops/ci/check-mesh-reach-overgrant.sh".into(),
                        declare_yaml: declare_yaml(&u.policy_id, "D9", who),
                    }),
                },
            );
        }

        // D3 / D4 / D5 / D6 / D11 — peers (fabric altitude)
        if c.peers_read && !c.peers.is_empty() {
            let n_peers = c.peers.len() as u32;
            let threshold = n_peers.saturating_sub(1);
            let mut by_hostname: BTreeMap<&str, Vec<&Peer>> = BTreeMap::new();
            for p in c.peers {
                if !p.hostname.is_empty() {
                    by_hostname.entry(p.hostname.as_str()).or_default().push(p);
                }
            }
            for p in c.peers {
                let pivot = p.reach_any * 2 >= threshold && threshold > 0;
                let acc = accept_row(devs, &p.name);
                if pivot {
                    let idle = !p.connected;
                    let sev = if acc.is_some() {
                        Severity::Accepted
                    } else if idle {
                        Severity::Alarm
                    } else {
                        Severity::Review
                    };
                    push(
                        sev,
                        p.reach_any,
                        "D3",
                        p.name.clone(),
                        Finding {
                            i: 0,
                            id: "D3".into(),
                            severity: sev,
                            subject: Subject::Peer { i: p.i },
                            claim: format!(
                                "{} can reach {} of {} other peers on any port",
                                p.name,
                                p.reach_any,
                                threshold
                            ),
                            blast_radius: p.reach_any,
                            blast_unit: "peers reachable at any port".into(),
                            accepted: acc.map(to_accepted),
                            action: Some(Action {
                                doc: "docs/operations/p2p-graduation.md".into(),
                                command: format!("PUT /api/groups/<id> (full replacement member list) — peer {}", p.id),
                                recheck: "ops/ci/check-mesh-reach-overgrant.sh".into(),
                                declare_yaml: declare_yaml(&p.name, "D3", who),
                            }),
                        },
                    );
                    if idle {
                        push(
                            if acc.is_some() { Severity::Accepted } else { Severity::Alarm },
                            p.reach_any,
                            "D4",
                            p.name.clone(),
                            Finding {
                                i: 0,
                                id: "D4".into(),
                                severity: if acc.is_some() { Severity::Accepted } else { Severity::Alarm },
                                subject: Subject::Peer { i: p.i },
                                claim: format!(
                                    "{} is disconnected and still permitted any-port reach to {} peers",
                                    p.name, p.reach_any
                                ),
                                blast_radius: p.reach_any,
                                blast_unit: "peers reachable at any port".into(),
                                accepted: acc.map(to_accepted),
                                action: None,
                            },
                        );
                    }
                }
                if p.os_family == "mobile" {
                    let held: Vec<&str> = p
                        .groups
                        .iter()
                        .map(|g| c.nodes[*g].suffix.as_str())
                        .filter(|s| matches!(*s, "PLATFORM-ADMIN" | "INFRA" | "P2P"))
                        .collect();
                    if !held.is_empty() {
                        push(
                            if acc.is_some() { Severity::Accepted } else { Severity::Alarm },
                            p.reach_any,
                            "D5",
                            p.name.clone(),
                            Finding {
                                i: 0,
                                id: "D5".into(),
                                severity: if acc.is_some() { Severity::Accepted } else { Severity::Alarm },
                                subject: Subject::Peer { i: p.i },
                                claim: format!(
                                    "{} ({}) holds {} — a handset in a substrate-authority group",
                                    p.name,
                                    p.os,
                                    held.join(", ")
                                ),
                                blast_radius: p.reach_any,
                                blast_unit: "peers reachable at any port".into(),
                                accepted: acc.map(to_accepted),
                                action: Some(Action {
                                    doc: "docs/operations/p2p-graduation.md".into(),
                                    command: format!("PUT /api/groups/<id> (full replacement member list) — peer {}", p.id),
                                    recheck: "ops/ci/check-mesh-reach-overgrant.sh".into(),
                                    declare_yaml: declare_yaml(&p.name, "D5", who),
                                }),
                            },
                        );
                    }
                }
                let fams: BTreeSet<&str> = p
                    .groups
                    .iter()
                    .map(|g| c.nodes[*g].family.as_str())
                    .filter(|f| *f != UNSCOPED)
                    .collect();
                if fams.len() >= 2 && c.cross_family_edges == 0 {
                    push(
                        Severity::Review,
                        p.reach_any,
                        "D6",
                        p.name.clone(),
                        Finding {
                            i: 0,
                            id: "D6".into(),
                            severity: Severity::Review,
                            subject: Subject::Peer { i: p.i },
                            claim: format!(
                                "{} holds groups in {} scope families ({}) while no policy edge crosses a family boundary",
                                p.name,
                                fams.len(),
                                fams.iter().copied().collect::<Vec<_>>().join(" + ")
                            ),
                            blast_radius: p.reach_any,
                            blast_unit: "peers reachable at any port".into(),
                            accepted: None,
                            action: None,
                        },
                    );
                }
            }
            for (h, group) in &by_hostname {
                if group.len() < 2 {
                    continue;
                }
                let first = group[0];
                push(
                    Severity::Review,
                    group.len() as u32,
                    "D11",
                    (*h).to_string(),
                    Finding {
                        i: 0,
                        id: "D11".into(),
                        severity: Severity::Review,
                        subject: Subject::Peer { i: first.i },
                        claim: format!(
                            "{} peers share the hostname {h}: {}",
                            group.len(),
                            group.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
                        ),
                        blast_radius: group.len() as u32,
                        blast_unit: "peers sharing one hostname".into(),
                        accepted: None,
                        action: Some(Action {
                            doc: "docs/adr/1116-gear-node-class-taxonomy.md".into(),
                            command: group
                                .iter()
                                .map(|p| {
                                    format!(
                                        "{} created_at={} last_seen={}",
                                        p.name,
                                        p.created_at.clone().unwrap_or_default(),
                                        p.last_seen.clone().unwrap_or_default()
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(" | "),
                            recheck: "ops/ci/check-membership-reconcile.py".into(),
                            declare_yaml: declare_yaml(h, "D11", who),
                        }),
                    },
                );
            }
        }

        // Deterministic, tie-free: severity, then blast radius desc, then id,
        // then subject.
        out.sort_by(|a, b| {
            a.0.rank()
                .cmp(&b.0.rank())
                .then(b.1.cmp(&a.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        out.into_iter()
            .enumerate()
            .map(|(i, (_, _, _, _, mut f))| {
                f.i = i;
                f
            })
            .collect()
    }

    fn to_accepted(d: &Deviation) -> Accepted {
        Accepted {
            decided_by: d.decided_by.clone(),
            decided_on: d.decided_on.clone(),
            reason: d.reason.clone(),
            revisit_when: d.revisit_when.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netbird::{NbGroup, NbPeer, NbPolicy, NbPolicyRule};

    fn grp(n: &str) -> NbGroup {
        NbGroup { id: n.into(), name: n.into(), peers: Vec::new() }
    }
    fn peer(name: &str, connected: bool, groups: &[&str]) -> NbPeer {
        NbPeer {
            id: name.into(),
            name: name.into(),
            connected,
            groups: groups.iter().map(|g| grp(g)).collect(),
            ..Default::default()
        }
    }
    fn pol(name: &str, src: &str, dst: &str, proto: &str, bidi: bool, action: &str) -> NbPolicy {
        NbPolicy {
            id: name.into(),
            name: name.into(),
            enabled: true,
            rules: vec![NbPolicyRule {
                name: name.into(),
                enabled: true,
                sources: vec![grp(src)],
                destinations: vec![grp(dst)],
                bidirectional: bidi,
                protocol: proto.into(),
                action: action.into(),
                ..Default::default()
            }],
        }
    }

    /// A drop rule is not a permit. They were previously counted as one, which
    /// widened every picture that included them.
    #[test]
    fn drop_rules_are_not_permits() {
        let p = permits_from(&[pol("deny", "A", "B", "all", false, "drop")]);
        assert!(p.is_empty());
    }

    /// A bidirectional rule permits the REVERSE pair too. Not recording it is
    /// how an asymmetric picture gets drawn as symmetric.
    #[test]
    fn bidirectional_rules_register_the_reverse_pair() {
        let p = permits_from(&[pol("both", "A", "B", "all", true, "accept")]);
        assert!(p.contains_key(&("A".into(), "B".into())));
        assert!(p.contains_key(&("B".into(), "A".into())));
    }

    /// One-way must stay one-way — the conservative reading.
    #[test]
    fn one_way_rules_do_not_imply_a_return_path() {
        let p = permits_from(&[pol("out", "A", "B", "all", false, "accept")]);
        assert!(p.contains_key(&("A".into(), "B".into())));
        assert!(!p.contains_key(&("B".into(), "A".into())));
    }

    /// The unpopulated altitude must render with NO peers enrolled — that is
    /// when segmentation is easiest to judge and hardest to see.
    #[test]
    fn the_policy_graph_renders_with_an_empty_fabric() {
        let g = policy_graph(&[grp("A"), grp("B")], &[pol("r", "A", "B", "tcp", false, "accept")], &[]);
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.edges.len(), 1);
        assert!(g.policies_read);
    }

    /// A declared group with no members is a FINDING, surfaced explicitly —
    /// this is the shape ADR 1116's gear classes sat in unnoticed.
    #[test]
    fn declared_but_empty_groups_are_called_out() {
        let g = policy_graph(&[grp("KID")], &[], &[]);
        assert!(g.notes.iter().any(|n| n.contains("no members")));
    }

    /// A rule naming an undeclared group must be surfaced, not dropped —
    /// that is how hand-made dashboard groups hide.
    #[test]
    fn rules_referencing_undeclared_groups_are_reported() {
        let g = policy_graph(&[grp("A")], &[pol("r", "A", "GHOST", "all", false, "accept")], &[]);
        assert!(g.notes.iter().any(|n| n.contains("absent from the declared set")));
        assert!(g.edges.is_empty());
    }

    /// Peer scope draws machines, never groups — groups are the mechanism.
    #[test]
    fn the_peer_graph_draws_no_group_nodes() {
        let peers = vec![peer("laptop", true, &["A"]), peer("orin", true, &["B"])];
        let subject: BTreeSet<&str> = ["laptop"].into_iter().collect();
        let g = peer_graph(&subject, &peers, &[pol("r", "A", "B", "tcp", false, "accept")]);
        assert!(g.nodes.iter().all(|n| n.kind == NodeKind::Peer));
        assert_eq!(g.edges.len(), 1);
        assert!(!g.edges[0].constraints.is_empty(), "an edge must carry its transport limit");
    }

    /// The module doc promises a graph is never blank without a reason. This
    /// function wrote no notes at all, so a subject that reached nothing
    /// produced a wordless empty picture — which reads as "denied".
    #[test]
    fn the_peer_graph_says_why_it_is_empty() {
        let peers = vec![peer("laptop", true, &["A"]), peer("orin", true, &["B"])];
        let miss: BTreeSet<&str> = ["nobody"].into_iter().collect();
        assert!(peer_graph(&miss, &peers, &[]).notes.iter().any(|n| n.contains("lookup miss")));
        let hit: BTreeSet<&str> = ["laptop"].into_iter().collect();
        let g = peer_graph(&hit, &peers, &[]);
        assert!(g.edges.is_empty());
        assert!(g.notes.iter().any(|n| n.contains("IS a denial")));
    }

    /// Two rules pointing opposite ways is not one bidirectional rule. They
    /// revoke differently, so the picture has to be able to tell them apart.
    #[test]
    fn reciprocal_and_bidirectional_are_different_states() {
        let two = policy_graph(
            &[grp("A"), grp("B")],
            &[
                pol("out", "A", "B", "tcp", false, "accept"),
                pol("back", "B", "A", "tcp", false, "accept"),
            ],
            &[],
        );
        assert!(two.edges.iter().all(|e| e.reciprocal && !e.bidirectional));
        let one = policy_graph(
            &[grp("A"), grp("B")],
            &[pol("both", "A", "B", "tcp", true, "accept")],
            &[],
        );
        assert!(one.edges.iter().all(|e| e.bidirectional && !e.reciprocal));
    }

    /// A scoped rule sitting behind an unrestricted one grants nothing.
    #[test]
    fn a_scoped_rule_behind_an_open_one_is_shadowed() {
        let g = policy_graph(
            &[grp("A"), grp("B")],
            &[
                pol("wide", "A", "B", "all", false, "accept"),
                pol("narrow", "A", "B", "tcp", false, "accept"),
            ],
            &[],
        );
        assert_eq!(g.edges.len(), 1);
        assert!(g.edges[0].shadowed);
    }

    /// Realization is a ladder, and its top rung means BOTH ENDS CONNECTED —
    /// never "in use". An empty endpoint keeps a permit latent no matter how
    /// wide it is; that is what keeps alarm colour scarce.
    #[test]
    fn realization_separates_empty_from_dark_from_live() {
        let groups = [grp("A"), grp("B")];
        let rule = [pol("r", "A", "B", "all", false, "accept")];
        let latent = policy_graph(&groups, &rule, &[peer("x", true, &["A"])]);
        assert_eq!(latent.edges[0].realization, Realization::Latent);
        let dark = policy_graph(
            &groups,
            &rule,
            &[peer("x", true, &["A"]), peer("y", false, &["B"])],
        );
        assert_eq!(dark.edges[0].realization, Realization::Dormant);
        let live = policy_graph(
            &groups,
            &rule,
            &[peer("x", true, &["A"]), peer("y", true, &["B"])],
        );
        assert_eq!(live.edges[0].realization, Realization::Realized);
    }
}

#[cfg(test)]
mod netvis_tests {
    use std::collections::BTreeMap;

    use super::netvis::*;
    use super::Realization;
    use crate::netbird::{NbGroup, NbPolicy, NbPolicyRule};

    // ── structure ──────────────────────────────────────────────────────────

    // ── counts, direction, constraint ──────────────────────────────────────

    /// The constraint comes from the RULE. A policy named "icmp only" whose
    /// rule permits everything must render as wide open — that mistake points
    /// the wrong way, which is the only direction that matters here.
    #[test]
    fn constraint_never_derives_from_a_policy_name() {
        let d = build_small(&[grp("A"), grp("B")], &[pol("icmp only", "A", "B", "all", false)]);
        assert!(d.edges[0].unrestricted);
        assert_eq!(d.edges[0].constraints, vec!["all protocols · all ports"]);
        assert_eq!(d.rules[0].policy, "icmp only");
    }

    // ── honesty ────────────────────────────────────────────────────────────

    /// A ledger that does not balance is a page that dropped a rule. Refuse.
    #[test]
    fn an_unbalanced_ledger_refuses_to_build() {
        let ledger = Read { total: 9, rendered: 1, ..Default::default() };
        assert!(!ledger.balances());
        let e = BuildError::ReadLedgerImbalance { sum: ledger.accounted(), ledger };
        assert!(e.to_string().contains("does not balance"));
    }

    // ── findings ───────────────────────────────────────────────────────────

    // ── fabric altitude ────────────────────────────────────────────────────

    // ── helpers ────────────────────────────────────────────────────────────

    fn grp(n: &str) -> NbGroup {
        NbGroup { id: n.into(), name: n.into(), peers: Vec::new() }
    }
    fn pol(name: &str, s: &str, d: &str, proto: &str, bidi: bool) -> NbPolicy {
        NbPolicy {
            id: name.into(),
            name: name.into(),
            enabled: true,
            rules: vec![NbPolicyRule {
                name: name.into(),
                enabled: true,
                sources: vec![grp(s)],
                destinations: vec![grp(d)],
                bidirectional: bidi,
                protocol: proto.into(),
                action: "accept".into(),
                ..Default::default()
            }],
        }
    }
    fn build_small(groups: &[NbGroup], policies: &[NbPolicy]) -> Doc {
        let purpose = BTreeMap::new();
        let mut req = Request::new("test", Altitude::Policy, "t");
        req.groups = Some(groups);
        req.policies = Some(policies);
        req.rule_purpose = &purpose;
        build(&req).unwrap()
    }

    #[test]
    fn realization_is_ordered_from_inert_to_live() {
        assert!(Realization::Latent < Realization::Dormant);
        assert!(Realization::Dormant < Realization::Realized);
        assert_eq!(Realization::Realized.label(), "permitted · both ends connected");
        assert!(!Realization::Realized.label().contains("use"));
        // Unknown sits BELOW the ladder, not on it: a roll-up that takes the
        // strongest realization present must never let "not read" outrank a
        // state that was actually measured.
        assert!(Realization::Unknown < Realization::Latent);
    }

}
