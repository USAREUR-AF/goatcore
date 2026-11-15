// SPDX-License-Identifier: Apache-2.0
//
// The composed agreement is the ONLY configuration input for what crosses.
//
// docs/design/federation-package-spec.md §8 defines it; the byte-exact
// conformance vectors in ops/federation/vectors/ are the contract. The same
// document produces the access rules, so the release set the republisher
// forwards and the prefix the far edge admits cannot drift apart by being
// typed twice.
//
// Strictness is deliberately asymmetric. `flows` — the member that governs
// what leaves — is strictly typed and refuses unknown members. `parties` and
// `transport` are read loosely, because this process does not act on them and
// refusing over a member it never reads would be a false alarm.

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use serde::Deserialize;

/// The only agreement version this build understands. A different value is a
/// refusal, never a best-effort parse: two versions producing two behaviours
/// is exactly the silent divergence the spec's single-lineage rule exists to
/// prevent.
pub const AGREEMENT_SPEC: &str = "federation-agreement/v1";

#[derive(Debug, Clone, Deserialize)]
pub struct Agreement {
    pub spec: String,
    /// `YYYY-MM-DD`. The earlier of the two packages' expiry.
    pub expires: String,
    #[serde(default)]
    pub flows: Vec<Flow>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flow {
    pub from: String,
    pub to: String,
    /// Derived `from/<from>/**`. Carried in the agreement, and re-derived and
    /// compared here — a mismatch means the agreement was hand-edited.
    pub publish_under: String,
    /// The originating party's local staging prefixes. Sorted, de-duplicated,
    /// non-empty per spec §8.1.
    pub release: Vec<String>,
    pub envelope: Envelope,
}

/// The RECEIVER's declared ceilings. A sender cannot raise anyone else's
/// ceiling, so these are limits on us, not permissions for us.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub hz: u64,
    pub bytes_per_message: usize,
    pub distinct_keys: usize,
}

impl Agreement {
    pub fn from_json(raw: &str) -> Result<Self> {
        let ag: Agreement = serde_json::from_str(raw).context(
            "parsing the composed agreement. If this failed on an unknown member inside \
             `flows`, the producing implementation and docs/design/federation-package-spec.md \
             §8.1 disagree, and the spec is the contract",
        )?;
        if ag.spec != AGREEMENT_SPEC {
            bail!(
                "agreement declares spec {:?}; this build implements {:?} only",
                ag.spec,
                AGREEMENT_SPEC
            );
        }
        ag.expiry_date()?;
        Ok(ag)
    }

    pub fn expiry_date(&self) -> Result<NaiveDate> {
        NaiveDate::parse_from_str(&self.expires, "%Y-%m-%d")
            .with_context(|| format!("agreement `expires` {:?} is not YYYY-MM-DD", self.expires))
    }

    /// The one flow that originates with us. Exactly one, or a refusal.
    ///
    /// `flows: []` and a flow that only arrives are both valid agreements
    /// (spec §8.1) and neither is an error — but neither gives this process
    /// anything to do, so it says so and exits rather than idling as a
    /// healthy-looking no-op.
    pub fn outbound_flow(&self, our_sovereign: &str) -> Result<&Flow> {
        let mine: Vec<&Flow> = self
            .flows
            .iter()
            .filter(|f| f.from == our_sovereign)
            .collect();
        match mine.as_slice() {
            [f] => {
                let derived = format!("from/{}/**", f.from);
                if f.publish_under != derived {
                    bail!(
                        "flow declares publish_under {:?} but the derived value is {:?}; the \
                         prefix is derived, never declared, so this agreement was edited",
                        f.publish_under,
                        derived
                    );
                }
                if f.release.is_empty() {
                    bail!("flow {} -> {} declares an empty release set", f.from, f.to);
                }
                Ok(f)
            }
            [] => {
                bail!(
                "no flow in this agreement originates with {our_sovereign:?}. Parties present: \
                 {:?}. Nothing crosses outbound; there is nothing for this process to do",
                self.flows.iter().map(|f| f.from.as_str()).collect::<Vec<_>>()
            )
            }
            many => bail!(
                "{} flows originate with {our_sovereign:?}; the schema allows at most one per \
                 party",
                many.len()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VECTOR: &str = include_str!("../../vectors/compose-full/agreement.pretty.json");

    #[test]
    fn conformance_vector_parses_and_yields_our_flow() {
        let ag = Agreement::from_json(VECTOR).expect("compose-full vector must parse");
        let f = ag.outbound_flow("xaa").unwrap();
        assert_eq!(f.to, "xab");
        assert_eq!(f.publish_under, "from/xaa/**");
        assert_eq!(f.release, vec!["release/weather/**"]);
        assert_eq!(f.envelope.hz, 50);
        assert_eq!(f.envelope.bytes_per_message, 262_144);
        assert_eq!(f.envelope.distinct_keys, 2000);
        // The mirrored flow is the partner's, and is read by the same code
        // path with no perspective flag.
        let g = ag.outbound_flow("xab").unwrap();
        assert_eq!(g.publish_under, "from/xab/**");
        assert_eq!(g.envelope.hz, 100);
    }

    #[test]
    fn empty_flows_is_valid_but_gives_us_nothing_to_do() {
        let raw = include_str!("../../vectors/compose-no-flows/agreement.pretty.json");
        let ag = Agreement::from_json(raw).expect("no-flows vector is a VALID agreement");
        let err = ag.outbound_flow("xaa").unwrap_err().to_string();
        assert!(err.contains("no flow"), "{err}");
    }

    /// The vector as an agreement/v2 (ADR 1242): v1 with `spec` changed and,
    /// in one party, the next CA it announces.
    fn as_v2(raw: &str) -> String {
        let raw = raw.replace("federation-agreement/v1", "federation-agreement/v2");
        let at = raw
            .find("\"ca_fingerprint\"")
            .expect("a party names its CA");
        let next = format!("\"ca_fingerprint_next\": \"sha256:{}\", ", "ab".repeat(32));
        format!("{}{}{}", &raw[..at], next, &raw[at..])
    }

    #[test]
    fn an_agreement_v2_is_read_like_v1_and_its_next_ca_is_not_acted_on() {
        let v1 = Agreement::from_json(VECTOR).unwrap();
        let v2 = Agreement::from_json(&as_v2(VECTOR)).expect("an agreement/v2 must parse");
        assert_eq!(v2.spec, "federation-agreement/v2");
        assert_eq!(v2.expires, v1.expires);
        for us in ["xaa", "xab"] {
            let (a, b) = (v1.outbound_flow(us).unwrap(), v2.outbound_flow(us).unwrap());
            assert_eq!(
                (&a.to, &a.publish_under, &a.release),
                (&b.to, &b.publish_under, &b.release)
            );
            assert_eq!(a.envelope.hz, b.envelope.hz);
            assert_eq!(a.envelope.bytes_per_message, b.envelope.bytes_per_message);
            assert_eq!(a.envelope.distinct_keys, b.envelope.distinct_keys);
        }
    }

    #[test]
    fn a_spec_version_this_build_does_not_implement_is_refused_not_best_effort() {
        for other in [
            "federation-agreement/v3",
            "federation-agreement/v0",
            "federation-package/v1",
        ] {
            let raw = VECTOR.replace("federation-agreement/v1", other);
            let err = Agreement::from_json(&raw).unwrap_err().to_string();
            assert!(
                err.contains(other) && err.contains("federation-agreement/v2"),
                "{other}: {err}"
            );
        }
    }

    #[test]
    fn a_hand_edited_publish_under_is_refused() {
        let raw = VECTOR.replace("\"from/xaa/**\"", "\"from/somebody-else/**\"");
        let ag = Agreement::from_json(&raw).unwrap();
        let err = ag.outbound_flow("xaa").unwrap_err().to_string();
        assert!(err.contains("derived"), "{err}");
    }

    #[test]
    fn an_unknown_member_inside_a_flow_is_refused() {
        let raw = VECTOR.replace(
            "\"from\": \"xaa\",",
            "\"from\": \"xaa\", \"allow_all\": true,",
        );
        // `{:#}` walks the anyhow chain to the serde message naming the member.
        let err = format!("{:#}", Agreement::from_json(&raw).unwrap_err());
        assert!(err.contains("allow_all"), "{err}");
    }

    #[test]
    fn expiry_parses_from_the_vector() {
        let ag = Agreement::from_json(VECTOR).unwrap();
        assert_eq!(
            ag.expiry_date().unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 20).unwrap()
        );
    }
}
