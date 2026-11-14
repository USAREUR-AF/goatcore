// SPDX-License-Identifier: Apache-2.0
//
// Per-peer registry entry. Records pub/sub declarations + schema-decl /
// admin-namespace-derived metadata. Role classification is computed from
// pub/sub overlap.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::keyexpr::keyexprs_related;
use crate::types::{
    is_reserved_namespace, NodeClass, NodeKind, PeerId, Role, TopicKey, INFRA_VENDORS,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    /// Peer ZID (hex string).
    pub peer_id: PeerId,
    /// Vendor/principal extracted from the goat cert URI-SAN (per-goatnet
    /// scheme — `urn:goat-nova:vendor=<v>` on nova-a, `urn:goat:zagros:org=<v>` on
    /// ZAGROS), or `None` when not running over mTLS (e.g. local dev).
    pub vendor: Option<String>,
    /// URI-SANs observed on this peer's cert (raw strings).
    pub uri_sans: Vec<String>,
    /// Topics this peer has been observed publishing to.
    pub publishes: HashSet<TopicKey>,
    /// Topics this peer has been observed subscribing to.
    pub subscribes: HashSet<TopicKey>,
    /// First observation timestamp (millis since UNIX epoch — for
    /// serialization across the WS).
    pub first_seen_ms: u64,
    /// Last observation timestamp.
    pub last_seen_ms: u64,
    /// Friendly per-peer name declared in the schema registration
    /// (e.g. "tarsier-alpha"). Decoupled from `peer_id` (which is the
    /// Zenoh ZID after admin-namespace discovery and the topic-prefix
    /// fallback otherwise) so the frontend can render a human label
    /// without leaking 32-hex-char ZIDs into the side panel. Falls back
    /// to peer_id when absent.
    pub display_name: Option<String>,
    /// Operating site declared in the schema registration
    /// (e.g. `alpha-fob`, `theater`).
    pub site: Option<String>,
    /// Role class declared in the schema registration
    /// (e.g. `sensor`, `effector`, `fusion`, `c2`, `observer`).
    pub role_class: Option<String>,
    /// Payload format declared in the schema registration
    /// (e.g. `protobuf`, `cbor`, `json`).
    pub format_hint: Option<String>,
    /// True when the peer's schema registration carried
    /// `synthetic: true` — drives the NOTIONAL/REAL toggle in the
    /// frontend so notional traffic can be filtered out once real
    /// operational publishers join the fabric.
    pub synthetic: Option<bool>,
    /// Explicit self-declaration of infrastructure status, carried in the
    /// schema-decl JSON `infra` field. When present it is the
    /// highest-priority signal in [`PeerInfo::node_class`] — lets a
    /// participant opt itself in (or out) of the infra class regardless of
    /// the derived heuristics. `None` for the common case (class is then
    /// derived from node_kind / vendor / reserved-namespace publishing).
    pub infra: Option<bool>,
    /// Substrate-level identity: router / peer / subscriber-only.
    /// Set when an admin-namespace sample confirms the node kind; left
    /// as `None` for nodes created only from schema-decl or topic-prefix
    /// fallback (those carry richer logical metadata instead).
    pub node_kind: Option<NodeKind>,
    /// Provenance of the most-recent metadata refresh: `"schema_decl"`,
    /// `"admin"`, or `"data_tail"`. Schema-decl wins on conflict, so a
    /// node first seen via admin and later refined by schema-decl ends
    /// up with `"schema_decl"`. Surfaced for operator-side debugging.
    pub attest_source: Option<String>,
    /// Endpoint locator string (e.g. `tcp/198.51.100.7:7447`) extracted
    /// from the admin payload — handy in the side panel when triaging
    /// "which peer is that?".
    pub locator: Option<String>,
}

impl PeerInfo {
    pub fn new(peer_id: PeerId, now_ms: u64) -> Self {
        Self {
            peer_id,
            vendor: None,
            uri_sans: Vec::new(),
            publishes: HashSet::new(),
            subscribes: HashSet::new(),
            first_seen_ms: now_ms,
            last_seen_ms: now_ms,
            display_name: None,
            site: None,
            role_class: None,
            format_hint: None,
            synthetic: None,
            infra: None,
            node_kind: None,
            attest_source: None,
            locator: None,
        }
    }

    /// Compute role classification per the v1 lexical-overlap heuristic.
    ///
    /// `source` = publishes only, no subscriptions.
    /// `sink` = subscribes only, no publications.
    /// `deriver` = publishes on a topic AND subscribes on a
    /// lexically-related prefix — fusion / correlation / translation /
    /// C2-from-sensors. Not a relay (Zenoh handles routing).
    /// `unknown` = neither.
    pub fn role(&self) -> Role {
        let has_pubs = !self.publishes.is_empty();
        let has_subs = !self.subscribes.is_empty();
        match (has_pubs, has_subs) {
            (false, false) => Role::Unknown,
            (true, false) => Role::Source,
            (false, true) => Role::Sink,
            (true, true) => {
                // Deriver if any publish-key is lexically related to
                // any subscribe-key (one is a prefix of the other after
                // stripping wildcards). v1 collapses unrelated-tree
                // pub+sub into deriver too; refine when Block 23
                // lineage lands.
                let _related = self
                    .publishes
                    .iter()
                    .any(|p| self.subscribes.iter().any(|s| keyexprs_related(p, s)));
                Role::Deriver
            }
        }
    }

    /// Classify this peer as underlying infrastructure vs an application
    /// data participant. The single authoritative classification — the
    /// frontend reads the serialized result instead of re-deriving from
    /// node_kind / vendor / publishes, so "is this substrate?" is decided
    /// in exactly one place. Priority order:
    ///
    /// 1. **Explicit self-declaration** — `infra: true`/`false` in the
    ///    schema-decl wins outright, so a participant can opt in or out.
    /// 2. **Substrate node-kind** — a Zenoh `Router` or a subscriber-only
    ///    observer (`Subscriber`, e.g. the audit-subscriber) is substrate.
    /// 3. **Known-infra vendor** — [`INFRA_VENDORS`] (e.g. `trashgoat`).
    /// 4. **Reserved-namespace publishing** — a peer that publishes
    ///    *only* under reserved operator prefixes ([`is_reserved_namespace`],
    ///    `ops/` + `attest/`, plus the deprecated `system/`). The "only" is
    ///    deliberate: an ordinary
    ///    vendor that also declares a schema under `attest/...` (every
    ///    well-behaved vendor does) is NOT infra — its mission-data
    ///    publishes keep it an application participant.
    ///
    /// Anything else is [`NodeClass::App`].
    pub fn node_class(&self) -> NodeClass {
        if self.is_infra() {
            NodeClass::Infra
        } else {
            NodeClass::App
        }
    }

    fn is_infra(&self) -> bool {
        // 1. Explicit self-declaration overrides every heuristic.
        if let Some(flag) = self.infra {
            return flag;
        }
        // 2. Substrate node-kinds (router / subscriber-only observer).
        if matches!(self.node_kind, Some(NodeKind::Router) | Some(NodeKind::Subscriber)) {
            return true;
        }
        // 3. Known platform-internal vendor (trashgoat …).
        if let Some(v) = &self.vendor {
            if INFRA_VENDORS.contains(&v.as_str()) {
                return true;
            }
        }
        // 4. Publishes ONLY under reserved operator namespaces. Requires
        //    at least one publish — a peer with no publishes falls through
        //    to App unless an earlier signal already classed it infra.
        !self.publishes.is_empty() && self.publishes.iter().all(|t| is_reserved_namespace(t))
    }

    /// Stable sub-type key for an infrastructure node, or `None` when the
    /// peer is an application participant. The frontend maps the key to a
    /// human label + one-line "what this is / why it's not a data
    /// participant" explainer (so the presentation copy lives frontend-
    /// side while the classification stays here). Keys:
    /// `router` · `subscriber` · `chaos` · `schema-registry` ·
    /// `operator-system` · `infra` (generic self-declared).
    pub fn node_subkind(&self) -> Option<&'static str> {
        if self.node_class() != NodeClass::Infra {
            return None;
        }
        if matches!(self.node_kind, Some(NodeKind::Router)) {
            return Some("router");
        }
        if matches!(self.node_kind, Some(NodeKind::Subscriber)) {
            return Some("subscriber");
        }
        if self.vendor.as_deref() == Some("trashgoat") {
            return Some("chaos");
        }
        if !self.publishes.is_empty() {
            if self.publishes.iter().all(|t| t.starts_with("attest/")) {
                return Some("schema-registry");
            }
            // ops/ is the canonical operator/control namespace; system/ is
            // its deprecated predecessor (kept until migration completes).
            if self
                .publishes
                .iter()
                .all(|t| t.starts_with("ops/") || t.starts_with("system/"))
            {
                return Some("operator-system");
            }
        }
        // Self-declared infra with no other distinguishing signal.
        Some("infra")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_role_classification() {
        let mut p = PeerInfo::new("p1".into(), 0);
        assert_eq!(p.role(), Role::Unknown);

        p.publishes.insert("org11x/lattice/raw".into());
        assert_eq!(p.role(), Role::Source);

        p.subscribes.insert("org11x/lattice/raw".into());
        assert_eq!(p.role(), Role::Deriver);

        let mut sink = PeerInfo::new("p2".into(), 0);
        sink.subscribes.insert("org11x/lattice/raw".into());
        assert_eq!(sink.role(), Role::Sink);
    }

    // ---- node_class: infra vs application data participant ----

    #[test]
    fn plain_vendor_is_app() {
        let mut p = PeerInfo::new("p".into(), 0);
        p.vendor = Some("caracal".into());
        p.publishes.insert("caracal/tracks/entity/v1".into());
        assert_eq!(p.node_class(), NodeClass::App);
        assert_eq!(p.node_subkind(), None);
    }

    #[test]
    fn router_and_subscriber_are_infra() {
        let mut router = PeerInfo::new("r".into(), 0);
        router.node_kind = Some(NodeKind::Router);
        assert_eq!(router.node_class(), NodeClass::Infra);
        assert_eq!(router.node_subkind(), Some("router"));

        let mut sub = PeerInfo::new("s".into(), 0);
        sub.node_kind = Some(NodeKind::Subscriber);
        sub.subscribes.insert("**".into());
        assert_eq!(sub.node_class(), NodeClass::Infra);
        assert_eq!(sub.node_subkind(), Some("subscriber"));
    }

    #[test]
    fn zenoh_peer_kind_alone_is_not_infra() {
        // A plain Zenoh peer process with no other infra signal is an
        // application participant — only Router/Subscriber are substrate.
        let mut p = PeerInfo::new("p".into(), 0);
        p.node_kind = Some(NodeKind::Peer);
        p.vendor = Some("caracal".into());
        p.publishes.insert("caracal/x/v1".into());
        assert_eq!(p.node_class(), NodeClass::App);
    }

    #[test]
    fn trashgoat_vendor_is_infra_chaos() {
        let mut p = PeerInfo::new("t".into(), 0);
        p.vendor = Some("trashgoat".into());
        p.publishes.insert("trashgoat/exp-1/junk/v1".into());
        assert_eq!(p.node_class(), NodeClass::Infra);
        assert_eq!(p.node_subkind(), Some("chaos"));
    }

    #[test]
    fn reserved_namespace_only_publisher_is_infra() {
        // Publishes ONLY under attest/ → schema-registry infra.
        let mut p = PeerInfo::new("a".into(), 0);
        p.publishes.insert("attest/schema/caracal/sys/v1".into());
        assert_eq!(p.node_class(), NodeClass::Infra);
        assert_eq!(p.node_subkind(), Some("schema-registry"));

        // Publishes ONLY under ops/ (canonical operator namespace) →
        // operator-system infra.
        let mut o = PeerInfo::new("o".into(), 0);
        o.publishes.insert("ops/trashgoat/control/1/v1".into());
        assert_eq!(o.node_class(), NodeClass::Infra);
        assert_eq!(o.node_subkind(), Some("operator-system"));

        // Deprecated system/ predecessor still classifies during the
        // migration window (ADR 1035).
        let mut s = PeerInfo::new("s".into(), 0);
        s.publishes.insert("system/health/router/v1".into());
        assert_eq!(s.node_class(), NodeClass::Infra);
        assert_eq!(s.node_subkind(), Some("operator-system"));
    }

    #[test]
    fn vendor_also_declaring_schema_stays_app() {
        // The "reserved-only" rule: a real vendor that publishes mission
        // data AND a schema-decl under attest/ is still an application
        // participant — its non-reserved publishes disqualify it from the
        // reserved-only infra rule.
        let mut p = PeerInfo::new("v".into(), 0);
        p.vendor = Some("caracal".into());
        p.publishes.insert("caracal/tracks/entity/v1".into());
        p.publishes.insert("attest/schema/caracal/tracks/v1".into());
        assert_eq!(p.node_class(), NodeClass::App);
        assert_eq!(p.node_subkind(), None);
    }

    #[test]
    fn explicit_infra_flag_overrides_heuristics() {
        // infra: true on an otherwise-ordinary vendor → Infra (generic).
        let mut p = PeerInfo::new("x".into(), 0);
        p.vendor = Some("caracal".into());
        p.publishes.insert("caracal/x/v1".into());
        p.infra = Some(true);
        assert_eq!(p.node_class(), NodeClass::Infra);
        assert_eq!(p.node_subkind(), Some("infra"));

        // infra: false forces App even on a known-infra vendor.
        let mut t = PeerInfo::new("t".into(), 0);
        t.vendor = Some("trashgoat".into());
        t.infra = Some(false);
        assert_eq!(t.node_class(), NodeClass::App);
    }

    #[test]
    fn no_signal_peer_is_app() {
        // Bare peer, nothing observed yet — App, not infra (no false
        // positives from the empty-publishes branch).
        let p = PeerInfo::new("p".into(), 0);
        assert_eq!(p.node_class(), NodeClass::App);
    }
}
