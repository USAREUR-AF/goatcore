// SPDX-License-Identifier: Apache-2.0
//
// NetBird management-API client — the mesh-layer control surface goat needs
// to *eject a peer* (the authoritative, mesh-first cut on a brokerless slot-pool
// fabric: a cert is inert without a mesh path, so removing the peer is what
// actually stops a flooder).
//
// Auth: NetBird uses an `Authorization: Token <key>` header (NOT Bearer). The
// admin token is a powerful secret, so it is env-driven on the consumer side
// (never stored in the profile); an empty base-url or token yields a Disabled
// client whose every call returns `Err(NetbirdError::Disabled)`.
//
// Endpoints used (NetBird mgmt API):
//   GET    /api/peers          → list mesh peers
//   DELETE /api/peers/{id}     → eject a peer (drops its WireGuard tunnel)
//   GET    /api/peers/{id}     → one peer's settings, re-sent on update
//   PUT    /api/peers/{id}     → turn NetBird SSH on or off (settings echoed)
//   GET    /api/groups         → resolve auto-group suffixes → UUIDs (onboard)
//   GET    /api/groups/{id}    → a group's network resources, re-sent on update
//   PUT    /api/groups/{id}    → replace a group's peers (resources echoed)
//   POST   /api/setup-keys     → mint a peer's mesh enrollment key (onboard)

use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

#[derive(Debug, Error)]
pub enum NetbirdError {
    #[error("netbird client disabled (no mgmt url or api token configured)")]
    Disabled,
    #[error("netbird mgmt API returned {status}")]
    Http { status: StatusCode },
    #[error("peer not found")]
    NotFound,
    #[error("network or parse error: {0}")]
    Transient(String),
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct NbGroup {
    /// Group UUID — what `POST /api/setup-keys` wants in `auto_groups`.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Member peers (populated by `GET /api/groups`; empty in the nested
    /// `NbPeer.groups` projection, which omits the field). Null-tolerated —
    /// same live-API quirk class as `SetupKeyMeta.auto_groups`.
    #[serde(default, deserialize_with = "null_to_empty_vec")]
    pub peers: Vec<NbGroupPeer>,
}

/// Peer reference as embedded in a group's `peers` array (`GET /api/groups`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NbGroupPeer {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// A policy as returned by `GET /api/policies` — only what the membership
/// transfer engine's coverage gate reads: which groups any ENABLED policy
/// references. Rule sources/destinations come back as group objects.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NbPolicy {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, deserialize_with = "null_to_empty_vec")]
    pub rules: Vec<NbPolicyRule>,
}

// Default is derived so test fixtures can spread `..Default::default()` and
// not break every time a field is added — which is exactly what adding the
// transport fields did to goat-cli. The derived values are the NARROW ones:
// bidirectional=false, empty protocol (read as "unstated"), no ports, no
// action. A fixture that forgets a field therefore under-states reach rather
// than over-stating it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NbPolicyRule {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, deserialize_with = "null_to_empty_vec")]
    pub sources: Vec<NbGroup>,
    #[serde(default, deserialize_with = "null_to_empty_vec")]
    pub destinations: Vec<NbGroup>,
    /// Whether the rule permits the REVERSE direction too.
    ///
    /// Load-bearing for any honest display of a policy: `A → B` and `A ↔ B`
    /// are different grants, and a reader shown only an arrow cannot tell
    /// which they are looking at. Defaults FALSE — the narrower reading — so
    /// a field the API stops sending never silently widens what we claim.
    #[serde(default)]
    pub bidirectional: bool,
    /// `tcp` / `udp` / `icmp` / `all`. Empty is treated as `all` by NetBird.
    #[serde(default)]
    pub protocol: String,
    /// Individual ports the rule is limited to. Empty = every port.
    #[serde(default, deserialize_with = "null_to_empty_vec")]
    pub ports: Vec<String>,
    /// Port ranges, the other half of the same constraint. Aliased because
    /// NetBird has shipped both spellings; missing the camelCase one would
    /// silently drop a real limit and render the rule as wider than it is.
    #[serde(
        default,
        alias = "portRanges",
        deserialize_with = "null_to_empty_vec"
    )]
    pub port_ranges: Vec<NbPortRange>,
    /// `accept` / `drop`. Defaults to accept, which is what NetBird sends.
    #[serde(default)]
    pub action: String,
    /// A rule can target a single RESOURCE (a peer, a host, a subnet) instead
    /// of a group, for example source `All`, destination a single peer.
    ///
    /// Read explicitly because the alternative is silence. A group-to-group
    /// picture cannot draw this rule, and a picture that drops a live rule
    /// without saying so is worse than one that never claimed completeness.
    #[serde(default, alias = "destinationResource")]
    pub destination_resource: Option<NbResourceRef>,
    #[serde(default, alias = "sourceResource")]
    pub source_resource: Option<NbResourceRef>,
}

/// A non-group rule endpoint (`{"id": "...", "type": "peer"}`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NbResourceRef {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub kind: String,
}

/// An inclusive port range on a policy rule.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NbPortRange {
    #[serde(default)]
    pub start: u32,
    #[serde(default)]
    pub end: u32,
}

impl NbPolicyRule {
    /// True when the rule limits NOTHING: every protocol, every port.
    ///
    /// Derived from the rule exactly as [`NbPolicyRule::constraint`] is, and
    /// for the same reason — a policy NAME that says "(icmp)" is a label. This
    /// is the predicate the surface encodes as stroke weight, so getting it
    /// from the name would draw an open door as a locked one.
    ///
    /// Narrow-by-default: an unstated protocol is NetBird's `all`, so it
    /// counts as unrestricted. That over-states the finding, never the safety.
    pub fn is_unrestricted(&self) -> bool {
        let all_proto = self.protocol.is_empty() || self.protocol.eq_ignore_ascii_case("all");
        all_proto && self.ports.is_empty() && self.port_ranges.is_empty()
    }

    /// The rule's actual transport constraint, in the form an operator reads.
    ///
    /// This is DERIVED FROM THE RULE, not from its name. The graph previously
    /// showed strings like "(icmp)" and "(web/dns/fabric tcp)" that came from
    /// the human-written policy NAME — a label, not the enforced constraint.
    /// A policy called "…(icmp)" whose rule permits all protocols would have
    /// rendered as restrictive while being wide open, which is the worst
    /// direction for a surface people use to reason about posture.
    pub fn constraint(&self) -> String {
        let proto = if self.protocol.is_empty() || self.protocol == "all" {
            "all protocols".to_string()
        } else {
            self.protocol.to_uppercase()
        };
        let mut ports: Vec<String> = self.ports.clone();
        ports.extend(
            self.port_ranges
                .iter()
                .map(|r| format!("{}-{}", r.start, r.end)),
        );
        if ports.is_empty() {
            // ICMP has no ports, so "all ports" would be nonsense there.
            if self.protocol.eq_ignore_ascii_case("icmp") {
                proto
            } else {
                format!("{proto} · all ports")
            }
        } else {
            format!("{proto} {}", ports.join(","))
        }
    }
}

/// Request body for `POST /api/setup-keys` (onboard mesh-enrollment leg).
/// Mirrors the playbook's Phase 1.4 body. `auto_groups` are resolved group
/// UUIDs (see [`resolve_group_ids`]); `key_type` is `reusable` when
/// `usage_limit > 1`, else `one-off`.
#[derive(Debug, Clone, Serialize)]
pub struct NewSetupKey {
    pub name: String,
    #[serde(rename = "type")]
    pub key_type: String,
    /// TTL in seconds.
    pub expires_in: u64,
    pub revoked: bool,
    pub auto_groups: Vec<String>,
    pub usage_limit: u32,
    pub ephemeral: bool,
    /// AutoPeerName prefix (patched goat-maintainer mgmt image combines it with the
    /// joining peer's hostname as `<auto_peer_name>-<hostname>`).
    pub auto_peer_name: String,
}

/// Response from `POST /api/setup-keys`. `key` is the one-time secret the peer
/// runs `netbird up --setup-key <key>` with; the rest is metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct SetupKeyResult {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub state: String,
}

/// Resolve manifest auto-group *suffixes* (e.g. `OPERATOR`) to NetBird group
/// UUIDs against the fetched group list. Each suffix expands to the
/// fully-qualified `<site_prefix>-<suffix>` name (the convention in
/// `netbird_acl_taxonomy.yml`), then looks up its UUID. Pure over the fetched
/// groups so it is unit-testable without a mgmt server. Returns `Ok(uuids)` in
/// suffix order, or `Err(missing_full_names)` listing every group that wasn't
/// found (fail-loud, mirroring the playbook's Phase 1.3 assert).
pub fn resolve_group_ids(
    groups: &[NbGroup],
    site_prefix: &str,
    suffixes: &[String],
) -> Result<Vec<String>, Vec<String>> {
    let mut ids = Vec::with_capacity(suffixes.len());
    let mut missing = Vec::new();
    for suffix in suffixes {
        let full = format!("{site_prefix}-{suffix}");
        match groups.iter().find(|g| g.name == full) {
            Some(g) if !g.id.is_empty() => ids.push(g.id.clone()),
            _ => missing.push(full),
        }
    }
    if missing.is_empty() {
        Ok(ids)
    } else {
        Err(missing)
    }
}

/// A mesh peer as returned by `GET /api/peers`. We deserialize only the
/// operator-relevant fields; everything else is ignored.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NbPeer {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub hostname: String,
    /// Mesh IP (the 100.64.x.x address on wt0) — NOT a LAN address.
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub last_seen: String,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub groups: Vec<NbGroup>,
    /// When the peer was enrolled. Read so a duplicate-hostname finding can
    /// show both peers' ages side by side instead of offering a delete button.
    #[serde(default)]
    pub created_at: String,
    /// goat-client / netbird version. Deserialized, not rendered on netvis —
    /// version skew is a fleet-health question, not a segmentation one.
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub login_expired: bool,
    #[serde(default)]
    pub ephemeral: bool,
    /// NetBird's own reachability count. NOT used for netvis's reach figures —
    /// those are recomputed from the policy set so the number on the page and
    /// the lines on the page cannot disagree.
    #[serde(default)]
    pub accessible_peers_count: u32,
    /// The peer's NetBird SSH server setting (ADR 1216 port 22022).
    #[serde(default)]
    pub ssh_enabled: bool,
    /// The peer drops off the mesh when its login expires (only for peers a
    /// user added; ADR 1216 devices must have this off).
    #[serde(default)]
    pub login_expiration_enabled: bool,
    /// The peer drops off the mesh after a period of inactivity.
    #[serde(default)]
    pub inactivity_expiration_enabled: bool,
}

/// What [`NetbirdClient::set_peer_ssh`] found and did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerSshChange {
    pub name: String,
    /// False when the peer already had the requested setting: nothing was sent.
    pub changed: bool,
    pub login_expiration_enabled: bool,
    pub inactivity_expiration_enabled: bool,
}

impl NbPeer {
    pub fn group_names(&self) -> Vec<&str> {
        self.groups.iter().map(|g| g.name.as_str()).collect()
    }
    /// Does this peer match an operator-supplied selector? Matches an exact
    /// peer id, an exact mesh IP, or a case-insensitive substring of the
    /// name/hostname (so an operator can cut by the name they see in `ls`).
    pub fn matches(&self, sel: &str) -> bool {
        let s = sel.to_lowercase();
        self.id == sel
            || self.ip == sel
            || self.name.to_lowercase().contains(&s)
            || self.hostname.to_lowercase().contains(&s)
    }
}

#[derive(Clone)]
struct Inner {
    http: Client,
    base_url: String,
    token: String,
}

/// Client for the NetBird management API. Construct with the mgmt origin
/// (e.g. `https://netbird.example.net`) + an admin token; either-empty
/// → a Disabled client.
pub struct NetbirdClient {
    inner: Option<Inner>,
}

impl NetbirdClient {
    pub fn new(base_url: String, token: String) -> Self {
        if base_url.is_empty() || token.is_empty() {
            return Self { inner: None };
        }
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            // The mgmt cert is publicly-trusted (LE), so default verification
            // is correct here — no danger_accept_invalid_certs.
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            inner: Some(Inner {
                http,
                base_url: base_url.trim_end_matches('/').to_string(),
                token,
            }),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    pub fn base_url(&self) -> Option<&str> {
        self.inner.as_ref().map(|i| i.base_url.as_str())
    }

    /// `GET /api/peers` → all mesh peers.
    pub async fn list_peers(&self) -> Result<Vec<NbPeer>, NetbirdError> {
        let inner = self.inner.as_ref().ok_or(NetbirdError::Disabled)?;
        let url = format!("{}/api/peers", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<Vec<NbPeer>>()
                .await
                .map_err(|e| NetbirdError::Transient(format!("decode: {e}"))),
            s => {
                warn!(url = %url, status = %s, "list_peers non-2xx");
                Err(NetbirdError::Http { status: s })
            }
        }
    }

    /// `DELETE /api/peers/{id}` → eject the peer; its WireGuard tunnel drops.
    /// This is the authoritative mesh-first cut.
    pub async fn delete_peer(&self, id: &str) -> Result<(), NetbirdError> {
        let inner = self.inner.as_ref().ok_or(NetbirdError::Disabled)?;
        let url = format!("{}/api/peers/{}", inner.base_url, id);
        let resp = inner
            .http
            .delete(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::NO_CONTENT => Ok(()),
            StatusCode::NOT_FOUND => Err(NetbirdError::NotFound),
            s => {
                warn!(id, url = %url, status = %s, "delete_peer non-2xx");
                Err(NetbirdError::Http { status: s })
            }
        }
    }

    /// Turn a peer's NetBird SSH server on or off (`PUT /api/peers/{id}`).
    ///
    /// The peer PUT replaces the peer's settings: `name`, `ssh_enabled`,
    /// `login_expiration_enabled` and `inactivity_expiration_enabled` are all
    /// required, and a field sent wrong is a field changed. So the peer is read
    /// with `GET /api/peers/{id}` immediately before the PUT and every other
    /// setting is sent back as read; only `ssh_enabled` differs. If the read
    /// fails, or the record lacks any of those fields, nothing is written. If
    /// the peer already has the requested setting, nothing is written either.
    /// `ip` and `approval_required` are optional in the request and omitted,
    /// which leaves them as they are.
    pub async fn set_peer_ssh(&self, id: &str, enabled: bool) -> Result<PeerSshChange, NetbirdError> {
        #[derive(Deserialize)]
        struct PeerSettings {
            name: Option<String>,
            ssh_enabled: Option<bool>,
            login_expiration_enabled: Option<bool>,
            inactivity_expiration_enabled: Option<bool>,
        }
        let inner = self.inner.as_ref().ok_or(NetbirdError::Disabled)?;
        let url = format!("{}/api/peers/{}", inner.base_url, id);
        let resp = inner
            .http
            .get(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        let cur = match resp.status() {
            StatusCode::OK => resp
                .json::<PeerSettings>()
                .await
                .map_err(|e| NetbirdError::Transient(format!("decode: {e}")))?,
            StatusCode::NOT_FOUND => return Err(NetbirdError::NotFound),
            s => {
                warn!(id, url = %url, status = %s, "peer read before update non-2xx");
                return Err(NetbirdError::Http { status: s });
            }
        };
        let (Some(name), Some(ssh), Some(login), Some(inactivity)) = (
            cur.name,
            cur.ssh_enabled,
            cur.login_expiration_enabled,
            cur.inactivity_expiration_enabled,
        ) else {
            return Err(NetbirdError::Transient(
                "the peer record lacks name, ssh_enabled, login_expiration_enabled or \
                 inactivity_expiration_enabled; nothing was written"
                    .into(),
            ));
        };
        let change = PeerSshChange {
            name: name.clone(),
            changed: ssh != enabled,
            login_expiration_enabled: login,
            inactivity_expiration_enabled: inactivity,
        };
        if !change.changed {
            return Ok(change);
        }
        let body = serde_json::json!({
            "name": name,
            "ssh_enabled": enabled,
            "login_expiration_enabled": login,
            "inactivity_expiration_enabled": inactivity,
        });
        let resp = inner
            .http
            .put(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::NO_CONTENT => Ok(change),
            StatusCode::NOT_FOUND => Err(NetbirdError::NotFound),
            s => {
                warn!(id, url = %url, status = %s, "set_peer_ssh non-2xx");
                Err(NetbirdError::Http { status: s })
            }
        }
    }

    /// `GET /api/groups` → all groups on this mgmt server. Used by the onboard
    /// to resolve auto-group suffixes to UUIDs (see [`resolve_group_ids`]).
    pub async fn list_groups(&self) -> Result<Vec<NbGroup>, NetbirdError> {
        let inner = self.inner.as_ref().ok_or(NetbirdError::Disabled)?;
        let url = format!("{}/api/groups", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<Vec<NbGroup>>()
                .await
                .map_err(|e| NetbirdError::Transient(format!("decode: {e}"))),
            s => {
                warn!(url = %url, status = %s, "list_groups non-2xx");
                Err(NetbirdError::Http { status: s })
            }
        }
    }

    /// `GET /api/policies` → all policies. The membership transfer engine's
    /// coverage gate reads which groups any enabled policy references, so a
    /// move can refuse a destination that would leave peers policy-uncovered.
    pub async fn list_policies(&self) -> Result<Vec<NbPolicy>, NetbirdError> {
        let inner = self.inner.as_ref().ok_or(NetbirdError::Disabled)?;
        let url = format!("{}/api/policies", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<Vec<NbPolicy>>()
                .await
                .map_err(|e| NetbirdError::Transient(format!("decode: {e}"))),
            s => {
                warn!(url = %url, status = %s, "list_policies non-2xx");
                Err(NetbirdError::Http { status: s })
            }
        }
    }

    /// `GET /api/users` → every account on the goatnet, human and service.
    ///
    /// Read for ONE reason: a NetBird account id is the only strong key the
    /// activity log carries. `/api/events` records `initiator_id`, and on an
    /// OIDC deployment that id is an opaque UUID — the same shape as a Zenoh
    /// write anchor. Without the user list there is no way to tell those two
    /// apart from the string alone, so a forensic selector either guesses or
    /// answers for the wrong kind of thing.
    ///
    /// `name` and `email` come back EMPTY for every OIDC account (NetBird does
    /// not persist the identity provider's claims). Callers must not treat a
    /// blank name as "no such user" — `is_service_user` and `last_login` are
    /// the fields that actually distinguish an account, and the person behind
    /// one is resolved through Kratos or the portal, never from here.
    pub async fn list_users(&self) -> Result<Vec<NbUser>, NetbirdError> {
        let inner = self.inner.as_ref().ok_or(NetbirdError::Disabled)?;
        let url = format!("{}/api/users", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<Vec<NbUser>>()
                .await
                .map_err(|e| NetbirdError::Transient(format!("decode: {e}"))),
            s => {
                warn!(url = %url, status = %s, "list_users non-2xx");
                Err(NetbirdError::Http { status: s })
            }
        }
    }

    /// `PUT /api/users/{id}` — set a user's dashboard role (`admin` / `user`)
    /// and blocked flag. NetBird replaces the whole record, so the caller passes
    /// the user's current `auto_groups` back unchanged. Never called with
    /// `owner`: ownership moves only by the current owner's own action.
    pub async fn update_user(
        &self,
        id: &str,
        role: &str,
        auto_groups: &[String],
        is_blocked: bool,
    ) -> Result<(), NetbirdError> {
        let inner = self.inner.as_ref().ok_or(NetbirdError::Disabled)?;
        let url = format!("{}/api/users/{}", inner.base_url, id);
        let body = serde_json::json!({
            "role": role, "auto_groups": auto_groups, "is_blocked": is_blocked,
        });
        let resp = inner
            .http
            .put(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::NO_CONTENT => Ok(()),
            StatusCode::NOT_FOUND => Err(NetbirdError::NotFound),
            s => {
                warn!(id, url = %url, status = %s, "update_user non-2xx");
                Err(NetbirdError::Http { status: s })
            }
        }
    }

    /// `GET /api/events` → the mesh activity log, newest-first as NetBird
    /// serves it.
    ///
    /// Typed rather than left as raw JSON because three callers were already
    /// indexing `e["activity_code"]` and `e["initiator_id"]` by hand, and a
    /// string key that silently yields `None` on a rename is how an audit
    /// surface reports "no activity" for an account that was busy.
    ///
    /// `meta` stays untyped on purpose: NetBird puts a different shape in it
    /// per activity code (a policy name, a peer fqdn+ip, a setup key name),
    /// and enumerating those here would drop every code we had not met.
    pub async fn list_events(&self) -> Result<Vec<NbEvent>, NetbirdError> {
        let inner = self.inner.as_ref().ok_or(NetbirdError::Disabled)?;
        let url = format!("{}/api/events", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<Vec<NbEvent>>()
                .await
                .map_err(|e| NetbirdError::Transient(format!("decode: {e}"))),
            s => {
                warn!(url = %url, status = %s, "list_events non-2xx");
                Err(NetbirdError::Http { status: s })
            }
        }
    }

    /// `PUT /api/groups/{id}` with the full desired peer set — the membership
    /// transfer primitive. NetBird's PUT replaces the group's peer list, so the
    /// caller sends the complete post-move set (mirror-first discipline lives in
    /// the caller: add to the destination and VERIFY before shrinking the
    /// source). `name` is echoed back because the PUT wants the full body.
    ///
    /// The PUT replaces the WHOLE group: NetBird's handler sets `resources` to
    /// an empty list when the body omits it, so a membership change used to
    /// strip every network resource attached to the group (#2074). The group's
    /// `resources` are therefore read with `GET /api/groups/{id}` immediately
    /// before the PUT and sent back unchanged, the same rule as
    /// `ops/scripts/lifecycle-sweep.py` `_group_put_body`. `name`, `peers` and
    /// `resources` are every field the group request body carries. If the read
    /// fails, nothing is written: a PUT without the resources is the wipe.
    pub async fn update_group_peers(
        &self,
        id: &str,
        name: &str,
        peer_ids: &[String],
    ) -> Result<(), NetbirdError> {
        let inner = self.inner.as_ref().ok_or(NetbirdError::Disabled)?;
        let url = format!("{}/api/groups/{}", inner.base_url, id);
        let resources = self.group_resources(id).await?;
        let body = serde_json::json!({ "name": name, "peers": peer_ids, "resources": resources });
        let resp = inner
            .http
            .put(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::NO_CONTENT => Ok(()),
            StatusCode::NOT_FOUND => Err(NetbirdError::NotFound),
            s => {
                warn!(id, url = %url, status = %s, "update_group_peers non-2xx");
                Err(NetbirdError::Http { status: s })
            }
        }
    }

    /// `GET /api/groups/{id}` → the group's attached network resources, exactly
    /// as NetBird returns them (`[{id, type}]`), for [`Self::update_group_peers`]
    /// to send back. `null` or absent reads as none.
    async fn group_resources(&self, id: &str) -> Result<Vec<serde_json::Value>, NetbirdError> {
        #[derive(Deserialize)]
        struct GroupResources {
            #[serde(default, deserialize_with = "null_to_empty_vec")]
            resources: Vec<serde_json::Value>,
        }
        let inner = self.inner.as_ref().ok_or(NetbirdError::Disabled)?;
        let url = format!("{}/api/groups/{}", inner.base_url, id);
        let resp = inner
            .http
            .get(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<GroupResources>()
                .await
                .map(|g| g.resources)
                .map_err(|e| NetbirdError::Transient(format!("decode: {e}"))),
            StatusCode::NOT_FOUND => Err(NetbirdError::NotFound),
            s => {
                warn!(id, url = %url, status = %s, "group read before update non-2xx");
                Err(NetbirdError::Http { status: s })
            }
        }
    }

    /// `POST /api/setup-keys` → mint a peer's mesh-enrollment key. The native
    /// onboard's NetBird leg (mirrors the playbook's Phase 1.4). Returns the
    /// `SetupKeyResult` whose `key` is the one-time secret for `netbird up`.
    pub async fn create_setup_key(
        &self,
        req: NewSetupKey,
    ) -> Result<SetupKeyResult, NetbirdError> {
        let inner = self.inner.as_ref().ok_or(NetbirdError::Disabled)?;
        let url = format!("{}/api/setup-keys", inner.base_url);
        let resp = inner
            .http
            .post(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .header("Content-Type", "application/json")
            .json(&req)
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => resp
                .json::<SetupKeyResult>()
                .await
                .map_err(|e| NetbirdError::Transient(format!("decode: {e}"))),
            s => {
                warn!(url = %url, status = %s, "create_setup_key non-2xx");
                Err(NetbirdError::Http { status: s })
            }
        }
    }

    /// `GET /api/setup-keys` → all setup keys on this mgmt server. The native
    /// revoke's best-effort key-hygiene leg (mirrors the playbook's Phase R1.4)
    /// lists them so it can revoke the ones a departing principal minted.
    pub async fn list_setup_keys(&self) -> Result<Vec<SetupKeyMeta>, NetbirdError> {
        let inner = self.inner.as_ref().ok_or(NetbirdError::Disabled)?;
        let url = format!("{}/api/setup-keys", inner.base_url);
        let resp = inner
            .http
            .get(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => resp
                .json::<Vec<SetupKeyMeta>>()
                .await
                .map_err(|e| NetbirdError::Transient(format!("decode: {e}"))),
            s => {
                warn!(url = %url, status = %s, "list_setup_keys non-2xx");
                Err(NetbirdError::Http { status: s })
            }
        }
    }

    /// `PUT /api/setup-keys/{id}` with `revoked: true` → stop the key minting
    /// further peers. Best-effort hygiene (the peer DELETE is the authoritative
    /// disconnect; a revoked key only blocks NEW joins), so the native revoke
    /// leg tolerates failure here — mirrors the playbook's `failed_when: false`
    /// on Phase R1.5. NetBird's PUT wants the full body, so `name` + `auto_groups`
    /// are echoed back from the listed key.
    pub async fn revoke_setup_key(&self, key: &SetupKeyMeta) -> Result<(), NetbirdError> {
        let inner = self.inner.as_ref().ok_or(NetbirdError::Disabled)?;
        let url = format!("{}/api/setup-keys/{}", inner.base_url, key.id);
        let body = serde_json::json!({
            "name": key.name,
            "auto_groups": key.auto_groups,
            "revoked": true,
        });
        let resp = inner
            .http
            .put(&url)
            .header("Authorization", format!("Token {}", inner.token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| NetbirdError::Transient(e.to_string()))?;
        match resp.status() {
            StatusCode::OK | StatusCode::NO_CONTENT => Ok(()),
            StatusCode::NOT_FOUND => Err(NetbirdError::NotFound),
            s => {
                warn!(id = %key.id, url = %url, status = %s, "revoke_setup_key non-2xx");
                Err(NetbirdError::Http { status: s })
            }
        }
    }
}

/// Metadata of an existing setup key from `GET /api/setup-keys`. Only the
/// fields the revoke-hygiene leg needs: the id (to target the PUT), the name
/// (to match the departing principal's `peer-<handle>…` convention and to echo
/// An account as served by `GET /api/users`.
///
/// On an OIDC deployment `name` and `email` are blank for every human account;
/// see [`NetbirdClient::list_users`] for why that is not a bug to work around.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct NbUser {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
    /// `owner` / `admin` / `user`.
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub is_service_user: bool,
    #[serde(default)]
    pub status: String,
    /// Zero-value (`0001-01-01T00:00:00Z`) for an account that has never
    /// completed an interactive login — which is how a PAT-only or service
    /// account reads. Kept as the raw string so that sentinel survives.
    #[serde(default)]
    pub last_login: String,
    /// Group ids NetBird auto-assigns to peers this user enrols. Echoed back
    /// unchanged on a role update, because `PUT /api/users/{id}` replaces the
    /// whole record and an omitted list would clear it.
    #[serde(default, deserialize_with = "null_to_empty_vec")]
    pub auto_groups: Vec<String>,
    #[serde(default)]
    pub is_blocked: bool,
}

/// One row of `GET /api/events`, the mesh activity log.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct NbEvent {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub timestamp: String,
    /// e.g. `policy.add`, `peer.group.delete`, `setupkey.revoke`.
    #[serde(default)]
    pub activity_code: String,
    /// The human activity label, e.g. `Peer added`. Read alongside the code
    /// because the membership converger (`goat warrant apply --membership`)
    /// joins a peer to its minting setup key through its `Peer added` row, the
    /// same join `ops/ci/check-membership-reconcile.py` makes on this field.
    #[serde(default)]
    pub activity: String,
    /// WHO acted. An account id for a dashboard/API action; a SETUP KEY id for
    /// `peer.setupkey.add`, because NetBird records the enrolling key rather
    /// than the person who minted it. Chasing that second case to a person
    /// needs the `setupkey.add` row that created the key.
    #[serde(default)]
    pub initiator_id: String,
    /// WHAT was acted on — a policy id, a peer id, a setup key id.
    #[serde(default)]
    pub target_id: String,
    /// Per-code payload. Shape varies by `activity_code`; see
    /// [`NetbirdClient::list_events`].
    #[serde(default)]
    pub meta: serde_json::Value,
}

/// back in the PUT body), and auto_groups (echoed back so the PUT preserves them).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SetupKeyMeta {
    pub id: String,
    #[serde(default)]
    pub name: String,
    // NetBird sometimes serializes an ungrouped key's `auto_groups` as an
    // explicit `null` (not `[]` / omitted); a bare `#[serde(default)]` doesn't
    // coerce that, so null-tolerate it to an empty vec — otherwise the whole
    // list decode fails on one such key and the hygiene leg is skipped.
    #[serde(default, deserialize_with = "null_to_empty_vec")]
    pub auto_groups: Vec<String>,
    #[serde(default)]
    pub revoked: bool,
}

fn null_to_empty_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_when_unconfigured() {
        assert!(!NetbirdClient::new(String::new(), String::new()).is_enabled());
        assert!(!NetbirdClient::new("https://x".into(), String::new()).is_enabled());
        assert!(NetbirdClient::new("https://x".into(), "tok".into()).is_enabled());
    }

    #[test]
    fn selector_matches_id_ip_and_name_substring() {
        let p = NbPeer {
            id: "abc123".into(),
            name: "qos-demo-cloud-flooder".into(),
            hostname: "qos-demo-cloud-flooder".into(),
            ip: "100.127.0.46".into(),
            connected: true,
            last_seen: String::new(),
            os: String::new(),
            groups: vec![],
            ..Default::default()
        };
        assert!(p.matches("abc123")); // exact id
        assert!(p.matches("100.127.0.46")); // exact mesh ip
        assert!(p.matches("cloud-flooder")); // name substring
        assert!(p.matches("QOS-DEMO")); // case-insensitive
        assert!(!p.matches("192.168.1.2")); // a LAN IP must NOT match a mesh peer
        assert!(!p.matches("other"));
    }

    fn grp(id: &str, name: &str) -> NbGroup {
        NbGroup { id: id.into(), name: name.into(), peers: vec![] }
    }

    #[test]
    fn group_peers_tolerate_null_and_absent() {
        // GET /api/groups embeds `peers`; the nested NbPeer.groups projection
        // omits it, and the live API may serialize it as explicit null.
        let body = r#"[
            {"id":"g1","name":"ZAGROS-DEVELOPER","peers":[{"id":"p1","name":"bob-laptop"},{"id":"p2"}]},
            {"id":"g2","name":"ZAGROS-MOON","peers":null},
            {"id":"g3","name":"ZAGROS-POD"}
        ]"#;
        let groups: Vec<NbGroup> = serde_json::from_str(body).unwrap();
        assert_eq!(groups[0].peers.len(), 2);
        assert_eq!(groups[0].peers[0].id, "p1");
        assert!(groups[1].peers.is_empty());
        assert!(groups[2].peers.is_empty());
    }

    #[test]
    fn policy_decode_reads_enabled_and_rule_groups() {
        let body = r#"[
            {"id":"pol1","name":"all → infra","enabled":true,
             "rules":[{"name":"r","enabled":true,
                       "sources":[{"id":"g1","name":"ZAGROS-DEVELOPER"}],
                       "destinations":[{"id":"g9","name":"ZAGROS-INFRA"}]}]},
            {"id":"pol2","name":"Default","enabled":false,"rules":null}
        ]"#;
        let pols: Vec<NbPolicy> = serde_json::from_str(body).unwrap();
        assert!(pols[0].enabled);
        assert_eq!(pols[0].rules[0].sources[0].id, "g1");
        assert!(!pols[1].enabled);
        assert!(pols[1].rules.is_empty());
    }

    #[test]
    fn resolve_group_ids_expands_suffix_and_maps_uuid() {
        let groups = vec![
            grp("uuid-op", "ZAGROS-OPERATOR"),
            grp("uuid-vd", "ZAGROS-VENDOR"),
            grp("uuid-other", "ZAGROS-ADMIN"),
        ];
        let got = resolve_group_ids(&groups, "ZAGROS", &["OPERATOR".into(), "VENDOR".into()]).unwrap();
        // UUIDs returned in suffix order, after <site_prefix>-<suffix> expansion.
        assert_eq!(got, vec!["uuid-op", "uuid-vd"]);
    }

    #[test]
    fn resolve_group_ids_fails_loud_on_missing() {
        let groups = vec![grp("uuid-op", "ZAGROS-OPERATOR")];
        let err = resolve_group_ids(&groups, "ZAGROS", &["OPERATOR".into(), "NOPE".into()]).unwrap_err();
        assert_eq!(err, vec!["ZAGROS-NOPE"]); // reports the full name that wasn't found
    }

    #[test]
    fn resolve_group_ids_rejects_empty_uuid() {
        // A group present by name but with no UUID is treated as missing.
        let groups = vec![grp("", "ZAGROS-OPERATOR")];
        let err = resolve_group_ids(&groups, "ZAGROS", &["OPERATOR".into()]).unwrap_err();
        assert_eq!(err, vec!["ZAGROS-OPERATOR"]);
    }

    #[tokio::test]
    async fn disabled_client_short_circuits_groups_and_setup_keys() {
        let c = NetbirdClient::new(String::new(), String::new());
        assert!(matches!(c.list_groups().await, Err(NetbirdError::Disabled)));
        let req = NewSetupKey {
            name: "x".into(),
            key_type: "one-off".into(),
            expires_in: 86400,
            revoked: false,
            auto_groups: vec![],
            usage_limit: 1,
            ephemeral: false,
            auto_peer_name: "x".into(),
        };
        assert!(matches!(c.create_setup_key(req).await, Err(NetbirdError::Disabled)));
    }

    #[test]
    fn new_setup_key_serializes_type_as_type_field() {
        // The JSON key must be `type` (Rust keyword) — verify the rename.
        let req = NewSetupKey {
            name: "peer-x".into(),
            key_type: "reusable".into(),
            expires_in: 2_592_000,
            revoked: false,
            auto_groups: vec!["uuid-1".into()],
            usage_limit: 5,
            ephemeral: false,
            auto_peer_name: "peer-x".into(),
        };
        let j = serde_json::to_value(&req).unwrap();
        assert_eq!(j["type"], "reusable");
        assert_eq!(j["usage_limit"], 5);
        assert_eq!(j["expires_in"], 2_592_000);
        assert_eq!(j["auto_groups"][0], "uuid-1");
    }

    #[test]
    fn setup_key_meta_tolerates_null_auto_groups() {
        // NetBird's live `/api/setup-keys` serializes an ungrouped key's
        // auto_groups as explicit `null`; the whole list decode used to fail on
        // it. Null → empty vec; a present list still decodes; extra fields ignored.
        let body = r#"[
            {"id":"a","name":"peer-x","auto_groups":null,"revoked":false,"extra":1},
            {"id":"b","name":"peer-y","auto_groups":["g1","g2"],"revoked":true}
        ]"#;
        let keys: Vec<SetupKeyMeta> = serde_json::from_str(body).unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys[0].auto_groups.is_empty());
        assert_eq!(keys[1].auto_groups, vec!["g1", "g2"]);
        assert!(keys[1].revoked);
    }
}

#[cfg(test)]
mod rule_constraint_tests {
    use super::{NbPolicyRule, NbPortRange};

    fn rule(protocol: &str, ports: &[&str], ranges: &[(u32, u32)]) -> NbPolicyRule {
        NbPolicyRule {
            name: String::new(),
            enabled: true,
            sources: Vec::new(),
            destinations: Vec::new(),
            bidirectional: false,
            protocol: protocol.into(),
            ports: ports.iter().map(|s| s.to_string()).collect(),
            port_ranges: ranges
                .iter()
                .map(|(s, e)| NbPortRange { start: *s, end: *e })
                .collect(),
            action: String::new(),
            ..Default::default()
        }
    }

    /// The predicate the surface draws as stroke weight. Same source as
    /// `constraint()`: the rule, never its name.
    #[test]
    fn unrestricted_is_all_protocols_and_all_ports() {
        assert!(rule("", &[], &[]).is_unrestricted());
        assert!(rule("all", &[], &[]).is_unrestricted());
        assert!(rule("ALL", &[], &[]).is_unrestricted());
        assert!(!rule("tcp", &[], &[]).is_unrestricted());
        assert!(!rule("all", &["443"], &[]).is_unrestricted());
        assert!(!rule("all", &[], &[(1, 65535)]).is_unrestricted());
    }

    /// The constraint must come from the RULE. Policy NAMES carry strings like
    /// "(icmp)" that look like limits and are only labels — a policy so named
    /// whose rule permits everything would otherwise render as restrictive
    /// while being wide open.
    #[test]
    fn an_unrestricted_rule_says_so_plainly() {
        assert_eq!(rule("", &[], &[]).constraint(), "all protocols · all ports");
        assert_eq!(rule("all", &[], &[]).constraint(), "all protocols · all ports");
    }

    #[test]
    fn ports_and_ranges_both_appear() {
        assert_eq!(rule("tcp", &["443"], &[]).constraint(), "TCP 443");
        assert_eq!(rule("tcp", &[], &[(8000, 8100)]).constraint(), "TCP 8000-8100");
        assert_eq!(rule("udp", &["53"], &[(100, 200)]).constraint(), "UDP 53,100-200");
    }

    /// ICMP has no ports, so "all ports" there would be nonsense presented as
    /// precision.
    #[test]
    fn icmp_does_not_claim_ports() {
        assert_eq!(rule("icmp", &[], &[]).constraint(), "ICMP");
    }

    /// Defaults must read NARROW: a field the API stops sending must never
    /// silently widen what the page claims.
    #[test]
    fn direction_defaults_to_one_way() {
        assert!(!rule("tcp", &[], &[]).bidirectional);
    }
}

#[cfg(test)]
mod group_update_tests {
    use super::{NetbirdClient, NetbirdError};
    use reqwest::StatusCode;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// One request as the fixture server saw it.
    #[derive(Debug, Clone)]
    struct SeenRequest {
        method: String,
        path: String,
        body: String,
    }

    type Seen = Arc<Mutex<Vec<SeenRequest>>>;

    /// A NetBird mgmt stand-in on a loopback port. Each connection carries one
    /// request (`Connection: close`); `reply` picks the status line and body
    /// from the method and path. Every request is recorded BEFORE its answer is
    /// written, so once the client call returns the log is complete, however
    /// many requests the client chose to send.
    #[allow(clippy::disallowed_methods)] // test-only server; dies with the test runtime
    async fn fixture_mgmt(reply: fn(&str, &str) -> (&'static str, String)) -> (String, Seen) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen: Seen = Arc::default();
        let log = Arc::clone(&seen);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let mut bytes = Vec::new();
                let (head_end, content_length) = loop {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).await.unwrap();
                    assert_ne!(n, 0, "client closed before the request head ended");
                    bytes.extend_from_slice(&buf[..n]);
                    assert!(bytes.len() < 65536);
                    if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&bytes[..i]).to_ascii_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .map(|v| v.trim().parse::<usize>().unwrap())
                            .unwrap_or(0);
                        break (i + 4, len);
                    }
                };
                while bytes.len() < head_end + content_length {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).await.unwrap();
                    assert_ne!(n, 0, "client closed before the body ended");
                    bytes.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&bytes[..head_end]).to_string();
                let mut first = head.lines().next().unwrap().split_whitespace();
                let method = first.next().unwrap().to_string();
                let path = first.next().unwrap().to_string();
                let body = String::from_utf8_lossy(&bytes[head_end..]).to_string();
                let (status, reply_body) = reply(&method, &path);
                log.lock().unwrap().push(SeenRequest { method, path, body });
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply_body}",
                    reply_body.len()
                );
                socket.write_all(resp.as_bytes()).await.unwrap();
            }
        });
        (base, seen)
    }

    fn requests(seen: &Seen) -> Vec<SeenRequest> {
        seen.lock().unwrap().clone()
    }

    fn put_body(seen: &Seen) -> serde_json::Value {
        let reqs = requests(seen);
        let put = reqs
            .iter()
            .find(|r| r.method == "PUT")
            .unwrap_or_else(|| panic!("no PUT was sent: {reqs:?}"));
        assert_eq!(put.path, "/api/groups/grp-1");
        serde_json::from_str(&put.body).unwrap()
    }

    /// #2074: NetBird's group PUT replaces the whole group, and a body without
    /// `resources` empties them. A membership change must carry the group's
    /// current network resources back unchanged.
    #[tokio::test]
    async fn update_group_peers_keeps_the_groups_network_resources() {
        fn reply(method: &str, _path: &str) -> (&'static str, String) {
            match method {
                "GET" => (
                    "200 OK",
                    serde_json::json!({
                        "id": "grp-1",
                        "name": "fixture-group",
                        "peers": [{"id": "peer-a", "name": "peer-a"}],
                        "resources": [
                            {"id": "res-1", "type": "host"},
                            {"id": "res-2", "type": "subnet"}
                        ]
                    })
                    .to_string(),
                ),
                _ => ("200 OK", "{}".to_string()),
            }
        }
        let (base, seen) = fixture_mgmt(reply).await;
        let nb = NetbirdClient::new(base, "fixture-token".into());
        tokio::time::timeout(
            Duration::from_secs(5),
            nb.update_group_peers("grp-1", "fixture-group", &["peer-a".into(), "peer-b".into()]),
        )
        .await
        .expect("client did not finish")
        .expect("update_group_peers failed");
        let body = put_body(&seen);
        assert_eq!(body["name"], "fixture-group");
        assert_eq!(body["peers"], serde_json::json!(["peer-a", "peer-b"]));
        assert_eq!(
            body["resources"],
            serde_json::json!([
                {"id": "res-1", "type": "host"},
                {"id": "res-2", "type": "subnet"}
            ]),
            "the PUT dropped the group's network resources: {body}"
        );
    }

    /// A group with no resources (the common case today) still sends an
    /// explicit empty list, which is what NetBird would set anyway.
    #[tokio::test]
    async fn update_group_peers_tolerates_null_resources() {
        fn reply(method: &str, _path: &str) -> (&'static str, String) {
            match method {
                "GET" => (
                    "200 OK",
                    r#"{"id":"grp-1","name":"fixture-group","peers":null,"resources":null}"#
                        .to_string(),
                ),
                _ => ("200 OK", "{}".to_string()),
            }
        }
        let (base, seen) = fixture_mgmt(reply).await;
        let nb = NetbirdClient::new(base, "fixture-token".into());
        nb.update_group_peers("grp-1", "fixture-group", &[]).await.unwrap();
        assert_eq!(put_body(&seen)["resources"], serde_json::json!([]));
    }

    /// If the current group cannot be read, nothing is written: a PUT without
    /// the resources would be the very wipe this guards against.
    #[tokio::test]
    async fn update_group_peers_writes_nothing_when_the_read_fails() {
        fn not_found(_m: &str, _p: &str) -> (&'static str, String) {
            ("404 Not Found", "{}".to_string())
        }
        let (base, seen) = fixture_mgmt(not_found).await;
        let nb = NetbirdClient::new(base, "fixture-token".into());
        let err = nb
            .update_group_peers("grp-gone", "fixture-group", &["peer-a".into()])
            .await
            .unwrap_err();
        assert!(matches!(err, NetbirdError::NotFound), "got {err:?}");
        let reqs = requests(&seen);
        assert_eq!(reqs.len(), 1, "{reqs:?}");
        assert_eq!(reqs[0].method, "GET");
        assert_eq!(reqs[0].path, "/api/groups/grp-gone");

        fn unavailable(_m: &str, _p: &str) -> (&'static str, String) {
            ("503 Service Unavailable", "{}".to_string())
        }
        let (base, seen) = fixture_mgmt(unavailable).await;
        let nb = NetbirdClient::new(base, "fixture-token".into());
        let err = nb
            .update_group_peers("grp-1", "fixture-group", &["peer-a".into()])
            .await
            .unwrap_err();
        assert!(
            matches!(err, NetbirdError::Http { status } if status == StatusCode::SERVICE_UNAVAILABLE),
            "got {err:?}"
        );
        let reqs = requests(&seen);
        assert!(reqs.iter().all(|r| r.method == "GET"), "a PUT followed a failed read: {reqs:?}");
    }

    fn peer_body(ssh: bool, login: bool, inactivity: bool) -> String {
        serde_json::json!({
            "id": "peer-1", "name": "machine-switch-a-host", "ip": "100.127.0.67",
            "ssh_enabled": ssh,
            "login_expiration_enabled": login,
            "inactivity_expiration_enabled": inactivity,
            "approval_required": false,
            "groups": [{"id": "g1", "name": "SITE_ONE-NETWORK"}]
        })
        .to_string()
    }

    /// The peer PUT replaces every required setting, so only `ssh_enabled` may
    /// differ from what was read: an expiry flag sent wrong would be changed.
    #[tokio::test]
    async fn set_peer_ssh_changes_only_ssh_and_echoes_the_rest() {
        fn reply(method: &str, _path: &str) -> (&'static str, String) {
            match method {
                "GET" => ("200 OK", peer_body(false, true, false)),
                _ => ("200 OK", "{}".to_string()),
            }
        }
        let (base, seen) = fixture_mgmt(reply).await;
        let nb = NetbirdClient::new(base, "fixture-token".into());
        let change = tokio::time::timeout(Duration::from_secs(5), nb.set_peer_ssh("peer-1", true))
            .await
            .expect("client did not finish")
            .expect("set_peer_ssh failed");
        assert!(change.changed);
        assert_eq!(change.name, "machine-switch-a-host");
        assert!(change.login_expiration_enabled);
        assert!(!change.inactivity_expiration_enabled);
        let reqs = requests(&seen);
        assert_eq!(reqs.len(), 2, "{reqs:?}");
        assert_eq!((reqs[0].method.as_str(), reqs[0].path.as_str()), ("GET", "/api/peers/peer-1"));
        assert_eq!((reqs[1].method.as_str(), reqs[1].path.as_str()), ("PUT", "/api/peers/peer-1"));
        let body: serde_json::Value = serde_json::from_str(&reqs[1].body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "name": "machine-switch-a-host",
                "ssh_enabled": true,
                "login_expiration_enabled": true,
                "inactivity_expiration_enabled": false
            })
        );
    }

    /// Already on: the read is the only request.
    #[tokio::test]
    async fn set_peer_ssh_writes_nothing_when_already_set() {
        fn reply(_m: &str, _p: &str) -> (&'static str, String) {
            ("200 OK", peer_body(true, false, false))
        }
        let (base, seen) = fixture_mgmt(reply).await;
        let nb = NetbirdClient::new(base, "fixture-token".into());
        let change = nb.set_peer_ssh("peer-1", true).await.unwrap();
        assert!(!change.changed);
        let reqs = requests(&seen);
        assert!(reqs.iter().all(|r| r.method == "GET"), "{reqs:?}");
    }

    /// A record missing a required setting is never guessed at.
    #[tokio::test]
    async fn set_peer_ssh_writes_nothing_when_the_record_is_incomplete() {
        fn reply(_m: &str, _p: &str) -> (&'static str, String) {
            ("200 OK", r#"{"id":"peer-1","name":"x","ssh_enabled":false}"#.to_string())
        }
        let (base, seen) = fixture_mgmt(reply).await;
        let nb = NetbirdClient::new(base, "fixture-token".into());
        let err = nb.set_peer_ssh("peer-1", true).await.unwrap_err();
        assert!(
            matches!(err, NetbirdError::Transient(ref m) if m.contains("nothing was written")),
            "got {err:?}"
        );
        assert!(requests(&seen).iter().all(|r| r.method == "GET"));
    }

    /// A failed read writes nothing; a 403 on the PUT comes back as the status.
    #[tokio::test]
    async fn set_peer_ssh_reports_read_and_write_refusals() {
        fn not_found(_m: &str, _p: &str) -> (&'static str, String) {
            ("404 Not Found", "{}".to_string())
        }
        let (base, seen) = fixture_mgmt(not_found).await;
        let nb = NetbirdClient::new(base, "fixture-token".into());
        assert!(matches!(nb.set_peer_ssh("gone", true).await.unwrap_err(), NetbirdError::NotFound));
        assert!(requests(&seen).iter().all(|r| r.method == "GET"));

        fn forbidden_put(method: &str, _p: &str) -> (&'static str, String) {
            match method {
                "GET" => ("200 OK", peer_body(false, false, false)),
                _ => ("403 Forbidden", "{}".to_string()),
            }
        }
        let (base, _seen) = fixture_mgmt(forbidden_put).await;
        let nb = NetbirdClient::new(base, "fixture-token".into());
        let err = nb.set_peer_ssh("peer-1", true).await.unwrap_err();
        assert!(
            matches!(err, NetbirdError::Http { status } if status == StatusCode::FORBIDDEN),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn set_peer_ssh_on_a_disabled_client_sends_nothing() {
        let nb = NetbirdClient::new(String::new(), String::new());
        assert!(matches!(nb.set_peer_ssh("peer-1", true).await.unwrap_err(), NetbirdError::Disabled));
    }
}
