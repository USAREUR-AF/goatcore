// SPDX-License-Identifier: Apache-2.0
//
// Everything that decides whether one sample crosses, in one pure function.
//
// The point of keeping this free of zenoh and of clocks is that the whole
// decision surface is testable without a fabric, and that a refusal is a
// VALUE — counted, labelled and logged — rather than a `continue` somewhere in
// an IO loop. F-177 is that denial is silent at both ends; the outbound half
// does not have to inherit that.
//
// Every ceiling here is the RECEIVER's, taken from the agreement. A sender
// cannot raise anyone else's ceiling, so shedding locally is not politeness —
// it is the only place the shed is visible to us at all. Shedding at their
// edge produces a denial we do not see.

use chrono::NaiveDate;
use std::collections::HashSet;

use crate::agreement::Envelope;
use crate::mapping::{KeyMap, MapError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Publish under this wire key.
    Publish(String),
    Refuse(Refusal),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Outside the declared release set — the case that must never cross.
    NotReleased,
    /// Inside the release set but with no wire name (see `MapError`).
    Unmappable(String),
    /// The agreement's `expires` date has passed. An expired agreement is not
    /// an agreement, so the flow stops without an operator touching anything.
    AgreementExpired(NaiveDate),
    /// Over the receiver's `bytes_per_message`. Never truncated — half a
    /// sample is worse than no sample, because the receiver cannot tell.
    Oversize { len: usize, limit: usize },
    /// Over the receiver's `hz` for this second.
    RateShed { limit: u64 },
    /// A new distinct wire key beyond the receiver's `distinct_keys`. Keys
    /// already seen keep flowing; the ceiling bounds cardinality, not volume.
    CardinalityShed { limit: usize },
}

impl Refusal {
    /// Stable, low-cardinality label for the metric dimension.
    pub fn label(&self) -> &'static str {
        match self {
            Refusal::NotReleased => "not_released",
            Refusal::Unmappable(_) => "unmappable",
            Refusal::AgreementExpired(_) => "agreement_expired",
            Refusal::Oversize { .. } => "oversize",
            Refusal::RateShed { .. } => "rate_shed",
            Refusal::CardinalityShed { .. } => "cardinality_shed",
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotReleased => write!(f, "not covered by the agreement's release set"),
            Refusal::Unmappable(e) => write!(f, "{e}"),
            Refusal::AgreementExpired(d) => write!(f, "the agreement expired on {d}"),
            Refusal::Oversize { len, limit } => {
                write!(f, "{len} bytes exceeds the receiver's ceiling of {limit}")
            }
            Refusal::RateShed { limit } => {
                write!(f, "over the receiver's ceiling of {limit} publications/s")
            }
            Refusal::CardinalityShed { limit } => write!(
                f,
                "a new wire key beyond the receiver's ceiling of {limit} distinct keys"
            ),
        }
    }
}

pub struct Policy {
    map: KeyMap,
    envelope: Envelope,
    expires: NaiveDate,
    // Fixed one-second window. A token bucket would smooth bursts, but the
    // receiver's declared figure is "publications per second" and a fixed
    // window is what that sentence means.
    window_start_ms: u64,
    window_count: u64,
    seen_keys: HashSet<String>,
    // The cardinality-shed warning: when it last fired, and the refusals held
    // back since (see `cardinality_shed_warning`).
    shed_warned_ms: Option<u64>,
    shed_held: u64,
}

/// At most one cardinality-shed warning per this many milliseconds.
pub const SHED_WARN_INTERVAL_MS: u64 = 60_000;

/// The operator's line for a new wire key refused at the agreement's ceiling.
pub fn cardinality_shed_message(limit: usize, held_back: u64) -> String {
    let more = if held_back > 0 {
        format!("; {held_back} more new key(s) refused the same way since the last warning")
    } else {
        String::new()
    };
    format!(
        "refused a new wire key: the agreement's distinct_keys limit of {limit} is reached, so no new key crosses (keys already seen still do){more}"
    )
}

impl Policy {
    pub fn new(map: KeyMap, envelope: Envelope, expires: NaiveDate) -> Self {
        Self {
            map,
            envelope,
            expires,
            window_start_ms: 0,
            window_count: 0,
            seen_keys: HashSet::new(),
            shed_warned_ms: None,
            shed_held: 0,
        }
    }

    pub fn map(&self) -> &KeyMap {
        &self.map
    }

    pub fn distinct_keys_seen(&self) -> usize {
        self.seen_keys.len()
    }

    /// Called once per cardinality-shed refusal. `Some(n)` when this one should
    /// be logged as a warning, `n` being the refusals held back since the last
    /// warning; `None` when it falls inside the interval and is only counted.
    pub fn cardinality_shed_warning(&mut self, now_ms: u64) -> Option<u64> {
        match self.shed_warned_ms {
            Some(t) if now_ms.saturating_sub(t) < SHED_WARN_INTERVAL_MS => {
                self.shed_held += 1;
                None
            }
            _ => {
                self.shed_warned_ms = Some(now_ms);
                Some(std::mem::take(&mut self.shed_held))
            }
        }
    }

    /// `now_ms` is any monotonic millisecond counter; `today` is UTC.
    ///
    /// Order matters and is deliberate: the release-set check runs FIRST, so a
    /// key that must never cross is refused for that reason and not masked by
    /// a rate shed that happened to fire on the same sample.
    pub fn decide(
        &mut self,
        staging_key: &str,
        payload_len: usize,
        now_ms: u64,
        today: NaiveDate,
    ) -> Decision {
        let wire = match self.map.map(staging_key) {
            Ok(w) => w,
            Err(MapError::NotReleased) => return Decision::Refuse(Refusal::NotReleased),
            Err(e) => return Decision::Refuse(Refusal::Unmappable(e.to_string())),
        };

        if today > self.expires {
            return Decision::Refuse(Refusal::AgreementExpired(self.expires));
        }

        if payload_len > self.envelope.bytes_per_message {
            return Decision::Refuse(Refusal::Oversize {
                len: payload_len,
                limit: self.envelope.bytes_per_message,
            });
        }

        if !self.seen_keys.contains(&wire) {
            if self.seen_keys.len() >= self.envelope.distinct_keys {
                return Decision::Refuse(Refusal::CardinalityShed {
                    limit: self.envelope.distinct_keys,
                });
            }
            self.seen_keys.insert(wire.clone());
        }

        if now_ms.saturating_sub(self.window_start_ms) >= 1000 {
            self.window_start_ms = now_ms;
            self.window_count = 0;
        }
        if self.window_count >= self.envelope.hz {
            return Decision::Refuse(Refusal::RateShed {
                limit: self.envelope.hz,
            });
        }
        self.window_count += 1;

        Decision::Publish(wire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(release: &[&str], envelope: Envelope) -> Policy {
        let map = KeyMap::new(
            "zagros",
            &release.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
        .unwrap();
        Policy::new(
            map,
            envelope,
            NaiveDate::from_ymd_opt(2026, 11, 20).unwrap(),
        )
    }

    fn roomy() -> Envelope {
        Envelope {
            hz: 1000,
            bytes_per_message: 1 << 20,
            distinct_keys: 1000,
        }
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 5).unwrap()
    }

    #[test]
    fn a_released_sample_gets_its_wire_key() {
        let mut p = policy(&["release/weather/**"], roomy());
        assert_eq!(
            p.decide("release/weather/forecast", 12, 0, today()),
            Decision::Publish("from/zagros/weather/forecast".into())
        );
    }

    #[test]
    fn two_release_prefixes_both_flow() {
        let mut p = policy(&["release/weather/**", "release/imagery/**"], roomy());
        assert_eq!(
            p.decide("release/weather/forecast", 1, 0, today()),
            Decision::Publish("from/zagros/weather/forecast".into())
        );
        assert_eq!(
            p.decide("release/imagery/tile/7", 1, 0, today()),
            Decision::Publish("from/zagros/imagery/tile/7".into())
        );
    }

    #[test]
    fn an_undeclared_key_is_refused_for_that_reason_and_no_other() {
        let mut p = policy(&["release/weather/**"], roomy());
        // Oversize AND undeclared: the refusal must name the release set, so
        // that raising a ceiling can never turn into a disclosure.
        assert_eq!(
            p.decide("release/secret/keys", 1 << 30, 0, today()),
            Decision::Refuse(Refusal::NotReleased)
        );
    }

    #[test]
    fn an_expired_agreement_stops_the_flow_with_no_operator_action() {
        let mut p = policy(&["release/weather/**"], roomy());
        let after = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        match p.decide("release/weather/forecast", 1, 0, after) {
            Decision::Refuse(Refusal::AgreementExpired(_)) => {}
            other => panic!("{other:?}"),
        }
        // Still valid on the expiry date itself.
        let on = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        assert!(matches!(
            p.decide("release/weather/forecast", 1, 0, on),
            Decision::Publish(_)
        ));
    }

    #[test]
    fn oversize_is_dropped_whole_never_truncated() {
        let mut p = policy(
            &["release/weather/**"],
            Envelope {
                bytes_per_message: 100,
                ..roomy()
            },
        );
        assert_eq!(
            p.decide("release/weather/f", 101, 0, today()),
            Decision::Refuse(Refusal::Oversize {
                len: 101,
                limit: 100
            })
        );
        assert!(matches!(
            p.decide("release/weather/f", 100, 0, today()),
            Decision::Publish(_)
        ));
    }

    #[test]
    fn the_rate_ceiling_sheds_and_the_next_second_recovers() {
        let mut p = policy(&["release/weather/**"], Envelope { hz: 2, ..roomy() });
        assert!(matches!(
            p.decide("release/weather/f", 1, 0, today()),
            Decision::Publish(_)
        ));
        assert!(matches!(
            p.decide("release/weather/f", 1, 10, today()),
            Decision::Publish(_)
        ));
        assert_eq!(
            p.decide("release/weather/f", 1, 20, today()),
            Decision::Refuse(Refusal::RateShed { limit: 2 })
        );
        assert!(matches!(
            p.decide("release/weather/f", 1, 1_000, today()),
            Decision::Publish(_)
        ));
    }

    #[test]
    fn cardinality_bounds_new_keys_but_not_existing_ones() {
        let mut p = policy(
            &["release/weather/**"],
            Envelope {
                distinct_keys: 2,
                ..roomy()
            },
        );
        assert!(matches!(
            p.decide("release/weather/a", 1, 0, today()),
            Decision::Publish(_)
        ));
        assert!(matches!(
            p.decide("release/weather/b", 1, 0, today()),
            Decision::Publish(_)
        ));
        assert_eq!(
            p.decide("release/weather/c", 1, 0, today()),
            Decision::Refuse(Refusal::CardinalityShed { limit: 2 })
        );
        // The two already-admitted keys keep flowing — the ceiling bounds
        // cardinality, not throughput.
        assert!(matches!(
            p.decide("release/weather/a", 1, 0, today()),
            Decision::Publish(_)
        ));
        assert_eq!(p.distinct_keys_seen(), 2);
    }

    /// A new key refused at the distinct-keys ceiling is data that never crosses,
    /// so an operator must see it: the first refusal warns, the next ones within a
    /// minute are counted into the following warning instead of logged each.
    #[test]
    fn a_cardinality_shed_warns_at_most_once_a_minute_and_counts_the_rest() {
        let mut p = policy(
            &["release/weather/**"],
            Envelope {
                distinct_keys: 1,
                ..roomy()
            },
        );
        assert!(matches!(
            p.decide("release/weather/a", 1, 0, today()),
            Decision::Publish(_)
        ));
        assert_eq!(
            p.decide("release/weather/b", 1, 0, today()),
            Decision::Refuse(Refusal::CardinalityShed { limit: 1 })
        );
        assert_eq!(
            p.cardinality_shed_warning(0),
            Some(0),
            "the first refusal warns"
        );
        assert_eq!(
            p.cardinality_shed_warning(1_000),
            None,
            "held back within the minute"
        );
        assert_eq!(p.cardinality_shed_warning(59_999), None);
        assert_eq!(
            p.cardinality_shed_warning(60_000),
            Some(2),
            "the next warning counts the two held back"
        );
        assert_eq!(p.cardinality_shed_warning(60_001), None);
    }

    #[test]
    fn the_cardinality_warning_names_the_agreements_limit() {
        let w = cardinality_shed_message(5000, 3);
        assert!(w.contains("distinct_keys") && w.contains("5000"), "{w}");
        assert!(w.contains("3 more"), "{w}");
        assert!(
            !cardinality_shed_message(5000, 0).contains("more"),
            "no count when none was held back"
        );
    }

    #[test]
    fn refusal_labels_are_stable_and_low_cardinality() {
        for r in [
            Refusal::NotReleased,
            Refusal::Unmappable("x".into()),
            Refusal::AgreementExpired(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
            Refusal::Oversize { len: 1, limit: 0 },
            Refusal::RateShed { limit: 1 },
            Refusal::CardinalityShed { limit: 1 },
        ] {
            assert!(!r.label().is_empty());
            assert!(!r.to_string().is_empty());
        }
    }
}
