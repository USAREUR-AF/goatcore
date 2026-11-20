// Slot alias resolver (ADR 1039 / docs/design/slot-alias-display-layer.md).
//
// Short human aliases (`jane-operator`) are projected over the UUID slot
// anchor (a 32-hex id). The UUID stays the sole
// canonical key in every machine path; this resolver is the boundary that
// translates between the two — `resolve_input` at every input (alias → UUID,
// BEFORE any wire/ACL action) and `render` at every output (UUID → alias,
// AFTER the canonical value is settled). The alias is never produced for a
// wire key, an ACL rule, a cert, or `--json`.
//
// Two invariants make this safe rather than fragile:
//   1. A token is classified alias-vs-UUID by SHAPE alone (a 32-hex string is
//      always the anchor; anything else is an alias candidate). The portal's
//      ValidAlias rejects 32-hex aliases, so the shapes are disjoint.
//   2. An unknown alias FAILS CLOSED (`Err(UnknownAlias)`), never a silent
//      passthrough — a typo can't become a phantom prefix the router silently
//      denies (which would recreate F-177 one layer up).
//
// When the portal can't be reached the registry is simply empty: it resolves
// only UUID-shaped input and renders UUIDs truncated. Aliases never block.

use crate::portal::AliasRow;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A canonical identity paired with its display alias — the **canonical
/// output shape** of the slot-alias layer (ADR 1039). Computed once at the
/// backend boundary (`AliasRegistry::label`) and serialized to a UI; the
/// frontend never resolves, it just renders `alias` if present else a
/// truncated `id`. Lets every serializing surface (panoscope, stethoscope,
/// future ones) ship ONE identity shape instead of ad-hoc per-surface fields.
///
/// `id` is the canonical UUID — the machine key, deep-link, copy target; it is
/// never dropped (the §3 invariant). `alias` is the operator-facing name, or
/// `None` when no alias is known (a random ZID / unknown anchor / portal-down),
/// in which case the consumer falls back to `id`. `alias` is skipped when
/// `None` so a UUID-only payload is the natural degraded shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Labeled {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

impl Labeled {
    /// Render to a single human string: the alias if known, else the truncated
    /// id. The one place a Labeled collapses to text — mirrors what a frontend
    /// `alias ?? truncate(id)` helper does, for server-side/terminal callers.
    pub fn display(&self) -> String {
        match &self.alias {
            Some(a) => a.clone(),
            None => truncate(&self.id),
        }
    }
}

/// A slot anchor is the first 32 chars of a sha256 hex digest — exactly 32
/// lowercase hex characters. An alias can never take this shape (portal
/// `ValidAlias` rejects it), so shape alone classifies a token.
pub fn is_anchor_shaped(token: &str) -> bool {
    token.len() == 32 && token.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The slot anchor a fabric leaf's Subject CN names (GHSA-rr3p). The portal
/// mints a slot's leaf with CN `<anchor>` at generation 0 and `<anchor>.g<N>`
/// after the slot's first assignment, rotation or collection; the namespace,
/// URI-SAN and every registry key stay the bare anchor. Returns the anchor for
/// either form and `None` for any other CN (a `<vendor>-trial-peer` or infra
/// CN, or a malformed generation such as `.g0` or `.g01`).
pub fn anchor_of_leaf_cn(cn: &str) -> Option<&str> {
    let (head, gen) = match cn.split_once(".g") {
        Some((h, g)) => (h, Some(g)),
        None => (cn, None),
    };
    if !is_anchor_shaped(head) {
        return None;
    }
    match gen {
        None => Some(head),
        Some(g)
            if !g.is_empty()
                && g.len() <= 18
                && !g.starts_with('0')
                && g.bytes().all(|b| b.is_ascii_digit()) =>
        {
            Some(head)
        }
        Some(_) => None,
    }
}

/// Left-pad a MINIMAL-hex token back to the 32-char anchor shape, or `None`
/// when it is not short lowercase hex.
///
/// A Zenoh ZID is a u128 printed as minimal hex, so a session pinned to an
/// anchor that begins with `0` reports a ZID of 31 chars or fewer — the pin
/// strips the leading zeros because zenoh rejects them outright. The
/// value is identical; only the spelling is short. Panoscope keys a peer on
/// `src_zid` when a sample carries one (`zenoh_client.rs`) and then asks this
/// registry for that peer's alias, so without re-padding, precisely the
/// identities affected by F-350 would render as raw hex where every other peer
/// shows its operator alias.
///
/// Shape-only, and callers must adopt the padded form ONLY on a registry hit —
/// see `AliasRegistry::lookup`. That keeps a short hex-shaped vendor name (`cafe`) from
/// silently becoming `0000…cafe`; it can only resolve if a real anchor sits at
/// that exact value, which for a sha256-derived pool is not a collision anyone
/// will meet.
pub fn pad_zid_to_anchor(token: &str) -> Option<String> {
    if token.is_empty() || token.len() >= 32 {
        return None;
    }
    if !token
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    Some(format!("{}{token}", "0".repeat(32 - token.len())))
}

/// Strip a leading slot-anchor segment from a key expression:
/// `<anchor>/tarsier/obs/v1` → `tarsier/obs/v1`. Keys whose first segment is
/// not anchor-shaped come back verbatim (per-vendor namespace, `ops/`, …); a
/// bare anchor with no taxonomy strips to `""`. Shape-classified only — no
/// registry needed, so it works portal-down and in surfaces with no portal
/// client (flashlight).
pub fn strip_anchor(key: &str) -> &str {
    match key.split_once('/') {
        Some((head, tail)) if is_anchor_shaped(head) => tail,
        None if is_anchor_shaped(key) => "",
        _ => key,
    }
}

/// The vendor segment of a key under both namespace flavours: the first
/// segment for per-vendor keys (`tarsier/obs/v1` → `tarsier`), the segment
/// AFTER the anchor for slot-pool keys (`<anchor>/tarsier/obs/v1` →
/// `tarsier`). Empty for a bare anchor.
pub fn vendor_segment(key: &str) -> &str {
    strip_anchor(key).split('/').next().unwrap_or("")
}

/// True when `vendor` names this key under either namespace flavour: it
/// equals the key's first segment (per-vendor goatnets, or an
/// operator passing a raw anchor) OR its anchor-stripped vendor segment
/// (slot-pool goatnets, where keys are `<anchor>/<vendor>/…` per
/// ADR 1025). This is THE vendor-matching rule for every inspection surface —
/// matching the first segment alone goes silently blind on slot-pool keys.
pub fn key_vendor_matches(key: &str, vendor: &str) -> bool {
    if vendor.is_empty() {
        return false;
    }
    key.split('/').next().unwrap_or("") == vendor || vendor_segment(key) == vendor
}

/// Truncate a UUID anchor for display fallback: first 8 chars + ellipsis.
/// Non-anchor input is returned verbatim (render only ever sees a canonical
/// key, but be total).
fn truncate(uuid: &str) -> String {
    if is_anchor_shaped(uuid) {
        format!("{}…", &uuid[..8])
    } else {
        uuid.to_string()
    }
}

/// Why `resolve_input` refused a token. Fail-closed by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// The token isn't UUID-shaped and matches no known alias. `suggestions`
    /// are near-matches to surface in the error ("did you mean …").
    UnknownAlias {
        input: String,
        suggestions: Vec<String>,
    },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::UnknownAlias { input, suggestions } => {
                write!(f, "no slot aliased `{input}`")?;
                if !suggestions.is_empty() {
                    write!(f, " — did you mean: {}", suggestions.join(", "))?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ResolveError {}

/// Why the alias registry holds what it holds — the honesty arm of the
/// display layer (ADR 1039 §3 fail-closed rendering).
///
/// An empty registry renders every anchor as a truncated UUID. That is the
/// *correct* degraded behaviour, but on its own it is indistinguishable from
/// "this goatnet has no aliases assigned" — an unset `PANOSCOPE_PORTAL_API_KEY`
/// looks the same, and nobody can
/// tell by looking. A surface that renders UUIDs must be able to say *which*
/// of these four it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AliasRegistryState {
    /// No portal API key configured — the refresher never polls at all.
    /// A deploy gap, not a runtime fault: the fix is wiring the key and
    /// re-applying the role, never waiting.
    Disabled,
    /// Configured, but no poll has ever succeeded. Either the portal has
    /// not come up yet, or the key it holds is not the one we present.
    Unreached,
    /// The most recent poll succeeded. `aliases == 0` here genuinely means
    /// the goatnet has assigned no aliases.
    Ok,
    /// At least one poll succeeded and a later one failed — the registry is
    /// last-good, deliberately kept rather than blanked back to UUIDs.
    Stale,
}

impl AliasRegistryState {
    /// Whether an operator should act. `Ok` never needs attention; `Stale`
    /// is self-healing on the next poll but worth surfacing; the other two
    /// mean names will never appear without an intervention.
    pub fn needs_attention(&self) -> bool {
        !matches!(self, AliasRegistryState::Ok)
    }
}

/// Serializable health of a surface's alias registry — what `/api/legend`
/// (and any sibling surface's equivalent) reports so an operator reading
/// UUIDs on screen can tell WHY in one glance instead of an archaeology dig
/// through container env and `debug!`-level logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasRegistryHealth {
    pub state: AliasRegistryState,
    /// Aliases currently held. Meaningful only alongside `state` — 0 under
    /// `Disabled` says nothing about the goatnet.
    pub aliases: usize,
    /// Wallclock ms of the last successful poll; `None` until one lands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ok_ms: Option<u64>,
    /// The most recent failure, verbatim, when `state` is `Unreached` or
    /// `Stale`. Cleared on the next success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl Default for AliasRegistryHealth {
    /// Pre-first-poll default. `Disabled` is set explicitly by the refresher
    /// once it learns the portal client is unconfigured.
    fn default() -> Self {
        Self {
            state: AliasRegistryState::Unreached,
            aliases: 0,
            last_ok_ms: None,
            last_error: None,
        }
    }
}

impl AliasRegistryHealth {
    /// Record a successful poll.
    pub fn record_ok(&mut self, aliases: usize, at_ms: u64) {
        self.state = AliasRegistryState::Ok;
        self.aliases = aliases;
        self.last_ok_ms = Some(at_ms);
        self.last_error = None;
    }

    /// Record a failed poll. Keeps the alias count — the registry itself is
    /// deliberately left last-good — and distinguishes never-worked
    /// (`Unreached`) from worked-then-broke (`Stale`).
    pub fn record_error(&mut self, error: String) {
        self.state = if self.last_ok_ms.is_some() {
            AliasRegistryState::Stale
        } else {
            AliasRegistryState::Unreached
        };
        self.last_error = Some(error);
    }

    /// Record that the portal client is unconfigured (no API key).
    pub fn record_disabled(&mut self) {
        self.state = AliasRegistryState::Disabled;
        self.aliases = 0;
        self.last_error = None;
    }
}

/// Bidirectional alias↔UUID map, goatnet-local. Built from the portal's
/// `GET /api/aliases`; an empty registry is the portal-down/degraded state.
#[derive(Debug, Clone, Default)]
pub struct AliasRegistry {
    by_alias: HashMap<String, String>, // alias → uuid
    by_uuid: HashMap<String, String>,  // uuid → alias
}

/// Top-level key-expression segments that are fabric namespaces, not
/// identities. Kept in step with the portal's `ValidVendorPrefix` reserved
/// list (ADR 0844 / ADR 1035): `ops` is the canonical operator/control
/// namespace, `attest` carries schema declarations, `system` is the
/// deprecated operator predecessor still reserved through the migration.
pub const RESERVED_ROOTS: [&str; 3] = ["ops", "attest", "system"];

impl AliasRegistry {
    /// An empty registry — the degraded state when the portal is unreachable.
    /// Resolves only UUID-shaped input; renders UUIDs truncated.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from the portal's alias rows. Rows whose alias is UUID-shaped are
    /// dropped (defensive — the portal already rejects them) so the shape
    /// classifier can never be fooled.
    pub fn from_rows(rows: impl IntoIterator<Item = AliasRow>) -> Self {
        let mut by_alias = HashMap::new();
        let mut by_uuid = HashMap::new();
        for r in rows {
            if r.alias.is_empty() || is_anchor_shaped(&r.alias) {
                continue;
            }
            by_alias.insert(r.alias.clone(), r.slot_id.clone());
            by_uuid.insert(r.slot_id, r.alias);
        }
        Self { by_alias, by_uuid }
    }

    /// Number of aliases known (0 = degraded/empty).
    pub fn len(&self) -> usize {
        self.by_alias.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_alias.is_empty()
    }

    /// Resolve one input token to its canonical UUID. UUID-shaped → verbatim;
    /// an alias → its UUID; anything else → `Err(UnknownAlias)` (fail closed).
    pub fn resolve_input(&self, token: &str) -> Result<String, ResolveError> {
        if is_anchor_shaped(token) {
            return Ok(token.to_string());
        }
        // A pure Zenoh wildcard leading segment (`*` / `**`) is a
        // key-expression token, never an identity alias — pass it through
        // rather than fail closed as an unknown alias, so `goat replay '**'`
        // and `goat sub '*/...'` work on a goatnet that has aliases. A
        // wildcard is read-only matching; it cannot become a phantom write
        // prefix, so the F-177 fail-closed below does not apply.
        if token == "*" || token == "**" {
            return Ok(token.to_string());
        }
        if let Some(uuid) = self.by_alias.get(token) {
            return Ok(uuid.clone());
        }
        // A RESERVED top-level namespace is a fabric root, never an identity.
        // The portal refuses to register any of them as an org prefix
        // (`ValidVendorPrefix`, ops/goat-zenoh-portal/internal/registry/
        // registry.go), so one can never become a phantom write prefix and
        // the F-177 fail-closed rule below has nothing to protect. Without
        // this, every health-surface key on a slot-pool goatnet — the whole
        // `ops/<domain>/health/**` family the runbooks are written around —
        // died on "no slot aliased `ops`", and the documented commands only
        // worked with a `--literal` none of them carry.
        //
        // The alias lookup above runs FIRST, so a goatnet that somehow does
        // carry an `ops` alias keeps resolving it: this is additive.
        if RESERVED_ROOTS.contains(&token) {
            return Ok(token.to_string());
        }
        // No alias matched. With NO aliases at all — a per-vendor goatnet,
        // or a degraded state where the portal was unreachable — we have
        // no basis to reject: pass the token through as a literal key. Fail
        // closed ONLY when aliases exist, so a typo against a KNOWN set can't
        // silently become a phantom prefix (the F-177 trap). An empty registry
        // is not a known set.
        if self.by_alias.is_empty() {
            return Ok(token.to_string());
        }
        Err(ResolveError::UnknownAlias {
            input: token.to_string(),
            suggestions: self.suggest(token),
        })
    }

    /// The alias for a canonical UUID if (and only if) one is known, else
    /// `None`. Unlike `render`, this does NOT fall back to a truncated UUID —
    /// the caller decides the fallback. Used where a display field wants to
    /// stay absent (serialize-skip) when no alias applies, so the consumer's
    /// own UUID-rendering path is reached unchanged (panoscope `slot_alias`).
    pub fn alias_of(&self, uuid: &str) -> Option<String> {
        self.lookup(uuid).map(|(_, alias)| alias.clone())
    }

    /// The UUID-side lookup every display path funnels through: verbatim first,
    /// then — only if that misses — the leading-zero-padded form of a
    /// minimal-hex token (`pad_zid_to_anchor`), so a session ZID pinned to a
    /// `0`-leading anchor still finds its alias.
    ///
    /// Adopting the padded form ONLY on a hit is the whole safety argument:
    /// anything that resolved before resolves identically, and anything unknown
    /// stays unknown, so no caller's fallback rendering changes.
    ///
    /// UUID side only. `resolve_input` — operator-typed tokens — is deliberately
    /// untouched and still classifies by shape alone, because a token a person
    /// typed must fail closed, not get silently reinterpreted.
    /// Returns the CANONICAL anchor alongside the alias, because the two can
    /// differ here: a display that says `glass-display (405fc463…)` would be
    /// quoting the ZID's minimal-hex spelling, which is not the anchor anyone
    /// can look up in the registry or an audit trail. Resolving through the pad
    /// means we know the canonical form, so we show it.
    fn lookup(&self, uuid: &str) -> Option<(&str, &String)> {
        if let Some((canon, alias)) = self.by_uuid.get_key_value(uuid) {
            return Some((canon.as_str(), alias));
        }
        let padded = pad_zid_to_anchor(uuid)?;
        self.by_uuid
            .get_key_value(padded.as_str())
            .map(|(canon, alias)| (canon.as_str(), alias))
    }

    /// Pair a canonical UUID anchor with its alias into the [`Labeled`] output
    /// shape — for an *identity* (a peer/slot anchor). `id` is preserved
    /// verbatim (canonical), `alias` is `alias_of(uuid)`. The anchor-identity
    /// constructor; see [`AliasRegistry::label_key`] for full key expressions.
    pub fn label(&self, uuid: &str) -> Labeled {
        Labeled {
            id: uuid.to_string(),
            alias: self.alias_of(uuid),
        }
    }

    /// Pair a canonical key expression with its alias-projected form into the
    /// [`Labeled`] shape — for a *topic/key* (`<anchor>/<taxonomy>`), where the
    /// leading anchor projects to an alias but the taxonomy stays verbatim.
    /// `id` is the canonical key; `alias` is `render_key(key)` **only when it
    /// differs** from the canonical (i.e. the leading segment resolved to a
    /// known alias) — `None` otherwise, so a consumer's `alias ?? id` renders
    /// the canonical key when there's nothing to project. The key-expression
    /// constructor; see [`AliasRegistry::label`] for bare anchors.
    pub fn label_key(&self, key: &str) -> Labeled {
        let projected = self.render_key(key);
        let alias = if projected != key {
            Some(projected)
        } else {
            None
        };
        Labeled {
            id: key.to_string(),
            alias,
        }
    }

    /// Render a canonical UUID for humans: its alias if known, else the
    /// truncated UUID. Never errors — output is best-effort by design.
    pub fn render(&self, uuid: &str) -> String {
        self.lookup(uuid)
            .map(|(_, alias)| alias.clone())
            .unwrap_or_else(|| truncate(uuid))
    }

    /// Render with both names for disambiguation / audit views:
    /// `alias (e7a9c75b…)`, or just the truncated UUID when no alias is known.
    pub fn render_both(&self, uuid: &str) -> String {
        match self.lookup(uuid) {
            Some((canon, alias)) => format!("{alias} ({})", truncate(canon)),
            None => truncate(uuid),
        }
    }

    /// Resolve a key expression's LEADING segment (the slot anchor) to its
    /// UUID, leaving the trailing taxonomy (ADR 0844, `class/site/id/…`)
    /// untouched. `jane-operator/obs/v1` → `<uuid>/obs/v1`. The wire key is
    /// always UUID-anchored. Fails closed if the leading segment is an unknown
    /// alias.
    pub fn resolve_key(&self, key: &str) -> Result<String, ResolveError> {
        match key.split_once('/') {
            Some((head, tail)) => Ok(format!("{}/{}", self.resolve_input(head)?, tail)),
            None => self.resolve_input(key),
        }
    }

    /// Reverse of `resolve_key` for display: render the leading UUID segment as
    /// its alias, leave the taxonomy verbatim. `<uuid>/obs/v1` →
    /// `jane-operator/obs/v1`. Never errors.
    pub fn render_key(&self, key: &str) -> String {
        match key.split_once('/') {
            Some((head, tail)) => format!("{}/{}", self.render(head), tail),
            None => self.render(key),
        }
    }

    /// Up to 5 "did you mean" candidates: known aliases that contain the input
    /// as a substring, else sharing the first character. Best-effort hinting.
    fn suggest(&self, input: &str) -> Vec<String> {
        let mut hits: Vec<String> = self
            .by_alias
            .keys()
            .filter(|a| a.contains(input))
            .cloned()
            .collect();
        if hits.is_empty() {
            if let Some(first) = input.chars().next() {
                hits = self
                    .by_alias
                    .keys()
                    .filter(|a| a.starts_with(first))
                    .cloned()
                    .collect();
            }
        }
        hits.sort();
        hits.truncate(5);
        hits
    }
}

#[cfg(test)]
mod tests {
    /// The reserved fabric roots are namespaces, not identities. Before this,
    /// `ops/mesh/health/**` — the key every health runbook is written around —
    /// failed closed on a slot-pool goatnet with "no slot aliased `ops`".
    #[test]
    fn reserved_roots_pass_through_on_a_goatnet_with_aliases() {
        let reg = AliasRegistry::from_rows([AliasRow {
            slot_id: "e7a9c75b532e3d434bb09fefd3ce1ec7".to_string(),
            alias: "maintainer-operator".into(),
        }]);
        for key in [
            "ops/mesh/health/**",
            "ops/consistency/report/**",
            "attest/schema/v1",
            "system/legacy/v1",
        ] {
            assert_eq!(reg.resolve_key(key).unwrap(), key, "reserved root refused: {key}");
        }
        // Still fail-closed for anything that is NOT reserved and NOT known:
        // a typo must not become a phantom write prefix.
        assert!(reg.resolve_key("opsz/mesh/health/**").is_err());
        assert!(reg.resolve_key("maintainer-operatr/obs/v1").is_err());
        // And a real alias still resolves.
        assert_eq!(
            reg.resolve_key("maintainer-operator/obs/v1").unwrap(),
            "e7a9c75b532e3d434bb09fefd3ce1ec7/obs/v1"
        );
    }

    /// An alias lookup wins over the reserved list, so a goatnet that somehow
    /// carries an `ops` alias is unaffected by the pass-through.
    #[test]
    fn a_registered_alias_still_wins_over_the_reserved_list() {
        let reg = AliasRegistry::from_rows([AliasRow {
            slot_id: "e7a9c75b532e3d434bb09fefd3ce1ec7".to_string(),
            alias: "ops".into(),
        }]);
        assert_eq!(
            reg.resolve_key("ops/mesh/health/**").unwrap(),
            "e7a9c75b532e3d434bb09fefd3ce1ec7/mesh/health/**"
        );
    }

    use super::*;

    const ANCHOR: &str = "e7a9c75b532e3d434bb09fefd3ce1ec7";

    #[test]
    fn health_starts_unreached_not_ok() {
        // The whole point of the arm: a registry that has never been filled
        // must not claim `Ok` with zero aliases, which reads as "no aliases
        // are assigned on this goatnet".
        let h = AliasRegistryHealth::default();
        assert_eq!(h.state, AliasRegistryState::Unreached);
        assert_eq!(h.aliases, 0);
        assert!(h.last_ok_ms.is_none());
        assert!(h.state.needs_attention());
    }

    #[test]
    fn health_records_ok_and_clears_prior_error() {
        let mut h = AliasRegistryHealth::default();
        h.record_error("connection refused".into());
        h.record_ok(7, 1_754_000_000_000);
        assert_eq!(h.state, AliasRegistryState::Ok);
        assert_eq!(h.aliases, 7);
        assert_eq!(h.last_ok_ms, Some(1_754_000_000_000));
        assert!(h.last_error.is_none());
        assert!(!h.state.needs_attention());
    }

    #[test]
    fn health_ok_with_zero_aliases_is_a_real_answer() {
        // A goatnet that genuinely assigns no aliases IS `Ok` — the state,
        // not the count, is what distinguishes it from a deploy gap.
        let mut h = AliasRegistryHealth::default();
        h.record_ok(0, 1);
        assert_eq!(h.state, AliasRegistryState::Ok);
        assert!(!h.state.needs_attention());
    }

    #[test]
    fn health_error_before_any_success_is_unreached_not_stale() {
        let mut h = AliasRegistryHealth::default();
        h.record_error("401 unauthorized".into());
        assert_eq!(h.state, AliasRegistryState::Unreached);
        assert_eq!(h.last_error.as_deref(), Some("401 unauthorized"));
    }

    #[test]
    fn health_error_after_success_is_stale_and_keeps_the_count() {
        // The registry itself is deliberately kept last-good across a failed
        // poll, so the count must survive alongside the stale marker.
        let mut h = AliasRegistryHealth::default();
        h.record_ok(7, 1);
        h.record_error("504 gateway timeout".into());
        assert_eq!(h.state, AliasRegistryState::Stale);
        assert_eq!(h.aliases, 7);
        assert_eq!(h.last_ok_ms, Some(1));
    }

    #[test]
    fn health_disabled_is_distinct_from_unreached() {
        // `disabled` = no API key was ever rendered into the container, so no
        // poll is even attempted. `unreached` = we tried and could not.
        // Conflating them sends an operator to the wrong fix.
        let mut h = AliasRegistryHealth::default();
        h.record_disabled();
        assert_eq!(h.state, AliasRegistryState::Disabled);
        assert!(h.state.needs_attention());
    }

    #[test]
    fn health_state_serializes_kebab_case() {
        // The frontend switches on these exact strings.
        let json = serde_json::to_string(&AliasRegistryState::Disabled).unwrap();
        assert_eq!(json, "\"disabled\"");
        let json = serde_json::to_string(&AliasRegistryState::Unreached).unwrap();
        assert_eq!(json, "\"unreached\"");
    }

    fn reg() -> AliasRegistry {
        AliasRegistry::from_rows([
            AliasRow { slot_id: ANCHOR.to_string(), alias: "maintainer-operator".into() },
            AliasRow { slot_id: "0405fc463154cb88502bc25fa9f0989a".into(), alias: "glass-display".into() },
        ])
    }

    #[test]
    fn strip_anchor_slot_pool_key() {
        assert_eq!(strip_anchor(&format!("{ANCHOR}/tarsier/obs/v1")), "tarsier/obs/v1");
        // Per-vendor key: untouched.
        assert_eq!(strip_anchor("tarsier/obs/v1"), "tarsier/obs/v1");
        // Bare anchor: empty taxonomy.
        assert_eq!(strip_anchor(ANCHOR), "");
        // Non-anchor first segment of anchor-ish length stays.
        assert_eq!(strip_anchor("ops/fabric/health/site-a"), "ops/fabric/health/site-a");
    }

    #[test]
    fn vendor_segment_both_flavours() {
        assert_eq!(vendor_segment(&format!("{ANCHOR}/tarsier/bravo-fob/contacts/v1")), "tarsier");
        assert_eq!(vendor_segment("tarsier/bravo-fob/contacts/v1"), "tarsier");
        assert_eq!(vendor_segment(ANCHOR), "");
        assert_eq!(vendor_segment("ops/fabric/health"), "ops");
    }

    #[test]
    fn key_vendor_matches_both_flavours() {
        let anchored = format!("{ANCHOR}/tarsier/bravo-fob/contacts/v1");
        // Slot-pool key matched by vendor segment — the F-195 blind spot.
        assert!(key_vendor_matches(&anchored, "tarsier"));
        // Raw-anchor lookups keep working (anchor-named vendor rows).
        assert!(key_vendor_matches(&anchored, ANCHOR));
        // Per-vendor key matched by first segment.
        assert!(key_vendor_matches("tarsier/obs/v1", "tarsier"));
        // No false positives: vendor name buried after a NON-anchor first
        // segment must not match (`ops/tarsier/...` is not vendor tarsier).
        assert!(!key_vendor_matches("ops/tarsier/health", "tarsier"));
        assert!(!key_vendor_matches(&anchored, "bravo-fob"));
        assert!(!key_vendor_matches(&anchored, ""));
    }

    #[test]
    fn anchor_shape_classifier() {
        assert!(is_anchor_shaped(ANCHOR));
        // GHSA-rr3p: a leaf CN carries the slot generation; the anchor is the
        // part before it.
        assert_eq!(anchor_of_leaf_cn(ANCHOR), Some(ANCHOR));
        assert_eq!(anchor_of_leaf_cn(&format!("{ANCHOR}.g7")), Some(ANCHOR));
        assert_eq!(anchor_of_leaf_cn(&format!("{ANCHOR}.g12345")), Some(ANCHOR));
        for bad in [
            ".g0",
            ".g01",
            ".g",
            ".gx",
            ".g-1",
            ".unassigned",
            "-trial-peer",
        ] {
            assert_eq!(anchor_of_leaf_cn(&format!("{ANCHOR}{bad}")), None, "{bad}");
        }
        assert_eq!(anchor_of_leaf_cn("maintainer-operator"), None);
        assert!(!is_anchor_shaped("maintainer-operator"));
        assert!(!is_anchor_shaped("ABCDEF0123456789ABCDEF0123456789")); // uppercase isn't anchor hex
        assert!(!is_anchor_shaped("e7a9c75b")); // too short
        assert!(!is_anchor_shaped("g7a9c75b532e3d434bb09fefd3ce1ec7")); // 'g' not hex
    }

    #[test]
    fn resolve_uuid_passthrough() {
        // A UUID-shaped token resolves to itself even with an empty registry.
        assert_eq!(AliasRegistry::empty().resolve_input(ANCHOR).unwrap(), ANCHOR);
    }

    #[test]
    fn resolve_alias_to_uuid() {
        assert_eq!(reg().resolve_input("maintainer-operator").unwrap(), ANCHOR);
    }

    #[test]
    fn resolve_unknown_fails_closed_with_suggestions() {
        let err = reg().resolve_input("maintainer-oper").unwrap_err();
        match err {
            ResolveError::UnknownAlias { input, suggestions } => {
                assert_eq!(input, "maintainer-oper");
                assert!(suggestions.contains(&"maintainer-operator".to_string()));
            }
        }
    }

    #[test]
    fn empty_registry_passes_tokens_through() {
        // With no aliases (per-vendor goatnet, or portal-down degraded), there
        // is no basis to reject — a literal key like `org11x` must pass
        // through verbatim, and UUIDs too. Fail-closed needs a KNOWN alias set.
        let e = AliasRegistry::empty();
        assert_eq!(e.resolve_input("org11x").unwrap(), "org11x");
        assert_eq!(e.resolve_input(ANCHOR).unwrap(), ANCHOR);
        assert_eq!(e.resolve_key("org11x/obs/v1").unwrap(), "org11x/obs/v1");
    }

    #[test]
    fn render_known_and_unknown() {
        let r = reg();
        assert_eq!(r.render(ANCHOR), "maintainer-operator");
        // Unknown UUID falls back to truncation, never errors.
        assert_eq!(r.render("ffffffffffffffffffffffffffffffff"), "ffffffff…");
        assert_eq!(r.render_both(ANCHOR), "maintainer-operator (e7a9c75b…)");
    }

    #[test]
    fn key_rewrite_leading_segment_only() {
        let r = reg();
        // Input: alias leading segment → UUID, taxonomy untouched.
        assert_eq!(
            r.resolve_key("maintainer-operator/obs/v1").unwrap(),
            format!("{ANCHOR}/obs/v1")
        );
        // Output: UUID leading segment → alias, taxonomy untouched.
        assert_eq!(r.render_key(&format!("{ANCHOR}/obs/v1")), "maintainer-operator/obs/v1");
        // A bare token (no slash) round-trips too.
        assert_eq!(r.resolve_key("maintainer-operator").unwrap(), ANCHOR);
    }

    #[test]
    fn key_rewrite_unknown_leading_alias_fails_closed() {
        // A typo in the leading segment must not silently pass through as a
        // literal prefix (that's the F-177 trap).
        assert!(reg().resolve_key("maintainer-oper/obs/v1").is_err());
    }

    #[test]
    fn wildcard_leading_segment_passes_through_even_with_aliases() {
        // `*` / `**` are key-expression wildcards, not alias attempts — they
        // pass through unchanged even when the registry HAS aliases, so
        // `goat replay '**'` / `goat sub '*/...'` work on a goatnet with
        // aliases. The fail-closed gate still catches a real typo.
        let r = reg();
        assert_eq!(r.resolve_input("**").unwrap(), "**");
        assert_eq!(r.resolve_input("*").unwrap(), "*");
        assert_eq!(r.resolve_key("**").unwrap(), "**");
        assert_eq!(r.resolve_key("*/data/v1").unwrap(), "*/data/v1");
        // The pass-through didn't widen the gate: a typo'd alias still errors.
        assert!(r.resolve_key("maintainer-oper/obs/v1").is_err());
    }

    #[test]
    fn label_carries_pair_and_displays_alias_or_truncation() {
        let r = reg();
        // Known anchor → both fields, display = alias.
        let known = r.label(ANCHOR);
        assert_eq!(known.id, ANCHOR);
        assert_eq!(known.alias.as_deref(), Some("maintainer-operator"));
        assert_eq!(known.display(), "maintainer-operator");
        // Unknown → id preserved, alias None, display = truncated id.
        let unknown = r.label("ffffffffffffffffffffffffffffffff");
        assert_eq!(unknown.id, "ffffffffffffffffffffffffffffffff");
        assert_eq!(unknown.alias, None);
        assert_eq!(unknown.display(), "ffffffff…");
        // alias=None is skipped on the wire (UUID-only is the degraded shape).
        let json = serde_json::to_string(&unknown).unwrap();
        assert_eq!(json, r#"{"id":"ffffffffffffffffffffffffffffffff"}"#);
        // A known label round-trips with the alias present.
        let known_json = serde_json::to_string(&known).unwrap();
        assert!(known_json.contains(r#""alias":"maintainer-operator""#));
    }

    #[test]
    fn label_key_projects_only_when_leading_alias_known() {
        let r = reg();
        // Known leading anchor → alias is the projected key; id stays canonical.
        // Key consumers render `alias ?? id` (not display(), which is the
        // bare-anchor convenience). Known leading anchor → alias is the
        // projected key; id stays canonical.
        let known = r.label_key(&format!("{ANCHOR}/obs/v1"));
        assert_eq!(known.id, format!("{ANCHOR}/obs/v1"));
        assert_eq!(known.alias.as_deref(), Some("maintainer-operator/obs/v1"));
        // Unknown leading segment against a populated registry: render_key
        // truncates it, which differs from the canonical, so alias = the
        // truncated projection (degraded display, still UUID-derived).
        let unknown = r.label_key("ffffffffffffffffffffffffffffffff/obs/v1");
        assert_eq!(unknown.alias.as_deref(), Some("ffffffff…/obs/v1"));
        // Empty registry + short non-anchor head → no projection, alias None
        // (consumer falls back to the canonical id).
        let plain = AliasRegistry::empty().label_key("drs/obs/v1");
        assert_eq!(plain.id, "drs/obs/v1");
        assert_eq!(plain.alias, None);
    }

    #[test]
    fn alias_of_is_some_only_when_known() {
        let r = reg();
        // Known anchor → Some(alias). Unknown / non-anchor → None (the caller
        // falls back, e.g. panoscope's slot_alias stays absent so the existing
        // truncated-UUID display path is reached unchanged).
        assert_eq!(r.alias_of(ANCHOR).as_deref(), Some("maintainer-operator"));
        assert_eq!(r.alias_of("ffffffffffffffffffffffffffffffff"), None);
        assert_eq!(r.alias_of("some-random-zid"), None);
        assert_eq!(AliasRegistry::empty().alias_of(ANCHOR), None);
    }

    #[test]
    fn pad_zid_to_anchor_is_shape_only() {
        // The F-350 shape: zenoh prints a pinned `0`-leading anchor as minimal
        // hex, one char short per stripped zero.
        assert_eq!(
            pad_zid_to_anchor("405fc463154cb88502bc25fa9f0989a").as_deref(),
            Some("0405fc463154cb88502bc25fa9f0989a")
        );
        // Already anchor-length, or longer: not ours to touch.
        assert_eq!(pad_zid_to_anchor(ANCHOR), None);
        assert_eq!(pad_zid_to_anchor(&"a".repeat(33)), None);
        // Not lowercase hex, or empty.
        assert_eq!(pad_zid_to_anchor("maintainer-operator"), None);
        assert_eq!(pad_zid_to_anchor("ABCDEF"), None);
        assert_eq!(pad_zid_to_anchor(""), None);
    }

    #[test]
    fn minimal_hex_zid_of_a_known_leading_zero_anchor_still_renders_its_alias() {
        let r = reg();
        // What panoscope actually holds for this peer after F-350: the ZID as
        // zenoh prints it, one leading zero gone.
        let trimmed = "405fc463154cb88502bc25fa9f0989a";
        assert_eq!(r.alias_of(trimmed).as_deref(), Some("glass-display"));
        assert_eq!(r.render(trimmed), "glass-display");
        assert_eq!(r.render_both(trimmed), "glass-display (0405fc46…)");
        // `label` picks it up through alias_of; `id` stays verbatim — the
        // caller's own identifier is never rewritten underneath it.
        let labeled = r.label(trimmed);
        assert_eq!(labeled.id, trimmed);
        assert_eq!(labeled.alias.as_deref(), Some("glass-display"));
    }

    #[test]
    fn padding_never_invents_a_match_for_unknown_or_short_hex() {
        let r = reg();
        // Short hex that pads to no known anchor: unchanged behavior, and the
        // truncated-UUID fallback still renders.
        assert_eq!(r.alias_of("cafe"), None);
        assert_eq!(r.render("cafe"), truncate("cafe"));
        // A trimmed form of an anchor NOT in the registry stays unknown.
        assert_eq!(r.alias_of("fffffffffffffffffffffffffffffff"), None);
        // The non-zero-leading anchor is unaffected in every direction.
        assert_eq!(r.alias_of(ANCHOR).as_deref(), Some("maintainer-operator"));
    }

    #[test]
    fn from_rows_drops_uuid_shaped_aliases() {
        // Defensive: a row whose "alias" is UUID-shaped is ignored so the
        // classifier can't be fooled.
        let r = AliasRegistry::from_rows([AliasRow {
            slot_id: ANCHOR.to_string(),
            alias: "ffffffffffffffffffffffffffffffff".into(),
        }]);
        assert_eq!(r.len(), 0);
    }
}
