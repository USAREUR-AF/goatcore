// SPDX-License-Identifier: Apache-2.0
//
// Staging name -> wire name.
//
// Two names, two jobs, and conflating them is the error this module exists to
// make impossible (spec §3.2):
//
//   release/weather/forecast     the LOCAL STAGING name. Never leaves us.
//   from/<us>/weather/forecast   the WIRE name. What the partner sees, and the
//                                only origin claim the far edge ENFORCES.
//
// THE RULE, in one sentence a partner can implement from the agreement alone:
// the wire key is `from/<our sovereign_id>/` followed by the staging key with
// its FIRST path segment removed.
//
// Why the first segment goes. `release` is our staging root — our vocabulary,
// chosen by us, meaningless to the receiver, and the one part of the key the
// spec says never crosses. Everything below it is the topic we agreed to
// export, and is exactly what the partner asked for. Dropping the whole
// literal prefix of the matched pattern instead (`release/weather`) would map
// `release/weather/forecast` and `release/imagery/forecast` onto one wire key
// and silently merge two feeds; keeping the whole path instead would put our
// staging vocabulary on the wire. Removing one segment is the only rule that
// is both structure-preserving and stateless.
//
// Injectivity is not assumed, it is CHECKED at startup: if two release
// prefixes in the agreement would map onto intersecting wire keys, the process
// refuses to start rather than merging two feeds under one name.

use anyhow::{bail, Result};
use zenoh::key_expr::{keyexpr, OwnedKeyExpr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    /// The router handed us a key no release prefix covers. Defence in depth:
    /// the subscriptions are per-prefix, so this should be unreachable — and
    /// if it is ever reached, forwarding would be an undeclared disclosure.
    NotReleased,
    /// A single-segment staging key has nothing left after the staging root is
    /// removed, so it has no wire name.
    NoRemainder,
    /// The mapped key is not a legal key-expression.
    Malformed(String),
}

impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapError::NotReleased => write!(f, "not covered by any release prefix"),
            MapError::NoRemainder => write!(f, "no path remains after the staging root"),
            MapError::Malformed(e) => write!(f, "maps to a malformed key-expression: {e}"),
        }
    }
}

/// The reason a release prefix is unusable. Every one of these is a startup
/// refusal, never a runtime surprise.
#[derive(Debug)]
pub struct KeyMap {
    wire_root: String,
    release: Vec<OwnedKeyExpr>,
}

impl KeyMap {
    pub fn new(sovereign_id: &str, release: &[String]) -> Result<Self> {
        if sovereign_id.is_empty() || sovereign_id.contains('/') || sovereign_id.contains('*') {
            bail!("sovereign_id {sovereign_id:?} is not a single literal key segment");
        }
        let mut parsed = Vec::with_capacity(release.len());
        for r in release {
            let ke = OwnedKeyExpr::new(r.as_str()).map_err(|e| {
                anyhow::anyhow!("release prefix {r:?} is not a key-expression: {e}")
            })?;
            let first = ke.as_str().split('/').next().unwrap_or("");
            if first.contains('*') {
                bail!(
                    "release prefix {r:?} begins with a wildcard segment. The mapping removes \
                     the first segment, which is only well defined when that segment is literal"
                );
            }
            if !ke.as_str().contains('/') {
                bail!(
                    "release prefix {r:?} is a single segment; there is nothing below the \
                     staging root to release"
                );
            }
            parsed.push(ke);
        }
        if parsed.is_empty() {
            bail!("the agreement's release set for our flow is empty");
        }

        // Injectivity. Two staging prefixes whose remainders intersect would
        // put two different local feeds on one wire key — a silent merge, and
        // an unrecoverable one, since the receiver cannot tell them apart.
        for i in 0..parsed.len() {
            for j in (i + 1)..parsed.len() {
                let (ri, rj) = (strip_root(&parsed[i]), strip_root(&parsed[j]));
                let (ki, kj) = (keyexpr::new(ri.as_str()), keyexpr::new(rj.as_str()));
                if let (Ok(ki), Ok(kj)) = (ki, kj) {
                    if ki.intersects(kj) {
                        bail!(
                            "release prefixes {:?} and {:?} both map onto {:?}; two feeds would \
                             arrive under one wire name and the receiver could not tell them \
                             apart. Rename one staging root's contents, or split the peering",
                            parsed[i].as_str(),
                            parsed[j].as_str(),
                            format!("from/{sovereign_id}/{ri}")
                        );
                    }
                }
            }
        }

        Ok(Self {
            wire_root: format!("from/{sovereign_id}"),
            release: parsed,
        })
    }

    /// The key-expressions to subscribe to locally — the release set verbatim.
    pub fn subscriptions(&self) -> &[OwnedKeyExpr] {
        &self.release
    }

    /// `from/<sov>/**` — what the far edge grants us and nothing wider. Every
    /// mapped key is checked against it before it is published, so a mapping
    /// bug becomes a local refusal rather than a write the partner refuses
    /// silently.
    pub fn wire_prefix(&self) -> String {
        format!("{}/**", self.wire_root)
    }

    pub fn map(&self, staging_key: &str) -> Result<String, MapError> {
        let ke = keyexpr::new(staging_key).map_err(|e| MapError::Malformed(e.to_string()))?;
        if !self.release.iter().any(|r| r.includes(ke)) {
            return Err(MapError::NotReleased);
        }
        let remainder = match staging_key.split_once('/') {
            Some((_root, rest)) if !rest.is_empty() => rest,
            _ => return Err(MapError::NoRemainder),
        };
        let wire = format!("{}/{}", self.wire_root, remainder);
        let wire_ke =
            keyexpr::new(wire.as_str()).map_err(|e| MapError::Malformed(e.to_string()))?;
        let bound = OwnedKeyExpr::new(self.wire_prefix()).expect("wire prefix is well formed");
        if !bound.includes(wire_ke) {
            return Err(MapError::Malformed(format!(
                "{wire} escapes {}",
                self.wire_prefix()
            )));
        }
        Ok(wire)
    }
}

/// Everything after the first `/`-segment.
fn strip_root(ke: &OwnedKeyExpr) -> String {
    ke.as_str()
        .split_once('/')
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zagros(release: &[&str]) -> KeyMap {
        KeyMap::new(
            "zagros",
            &release.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
        .unwrap()
    }

    #[test]
    fn the_drill_sample_gets_the_wire_name_not_its_staging_name() {
        // ops/federation-drill/cloud/RESULTS.md case 4 crossed as
        // `release/weather/forecast`. This is what it must cross as.
        let m = zagros(&["release/weather/**"]);
        assert_eq!(
            m.map("release/weather/forecast").unwrap(),
            "from/zagros/weather/forecast"
        );
    }

    #[test]
    fn structure_below_the_staging_root_is_preserved_verbatim() {
        let m = zagros(&["release/weather/**"]);
        assert_eq!(
            m.map("release/weather/eu/de/hourly").unwrap(),
            "from/zagros/weather/eu/de/hourly"
        );
    }

    #[test]
    fn two_release_prefixes_both_map() {
        let m = zagros(&["release/weather/**", "release/imagery/**"]);
        assert_eq!(
            m.map("release/weather/forecast").unwrap(),
            "from/zagros/weather/forecast"
        );
        assert_eq!(
            m.map("release/imagery/tile/7").unwrap(),
            "from/zagros/imagery/tile/7"
        );
    }

    #[test]
    fn a_key_outside_the_release_set_is_never_republished() {
        // RESULTS.md case 6: `release/secret/keys` was declared nowhere and
        // must not cross. Same for anything under a different root.
        let m = zagros(&["release/weather/**"]);
        assert_eq!(m.map("release/secret/keys"), Err(MapError::NotReleased));
        assert_eq!(m.map("ops/fabric/hijack"), Err(MapError::NotReleased));
        assert_eq!(m.map("release/weatherly/x"), Err(MapError::NotReleased));
    }

    #[test]
    fn a_bare_staging_root_has_no_wire_name() {
        let m = zagros(&["release/**"]);
        assert_eq!(m.map("release"), Err(MapError::NoRemainder));
        assert_eq!(
            m.map("release/weather/forecast").unwrap(),
            "from/zagros/weather/forecast"
        );
    }

    #[test]
    fn colliding_release_prefixes_are_refused_at_startup() {
        let err = KeyMap::new(
            "zagros",
            &["release/weather/**".into(), "export/weather/**".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("from/zagros/weather/**"), "{err}");
    }

    #[test]
    fn non_colliding_prefixes_under_different_roots_are_allowed() {
        let m = KeyMap::new(
            "zagros",
            &["release/weather/**".into(), "export/imagery/**".into()],
        )
        .unwrap();
        assert_eq!(m.map("export/imagery/x").unwrap(), "from/zagros/imagery/x");
    }

    #[test]
    fn a_wildcard_staging_root_is_refused() {
        let err = KeyMap::new("zagros", &["**".into()])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("single segment") || err.contains("wildcard"),
            "{err}"
        );
        let err = KeyMap::new("zagros", &["*/weather/**".into()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("wildcard"), "{err}");
    }

    #[test]
    fn every_mapped_key_lands_inside_the_prefix_the_far_edge_grants() {
        let m = zagros(&["release/weather/**"]);
        let bound = OwnedKeyExpr::new(m.wire_prefix()).unwrap();
        for k in ["release/weather/a", "release/weather/a/b/c"] {
            let wire = m.map(k).unwrap();
            assert!(
                bound.includes(keyexpr::new(wire.as_str()).unwrap()),
                "{wire}"
            );
        }
    }
}
