//! The rotation sweep — the daemon's poll-driven decision to rotate, or not.
//!
//! `KeyService::rotate` always mints a *new* generation. A poll loop must not spam
//! generations every tick, so the daemon rotates a release-set only when its
//! resolved audience has **changed** since the last generation it published (or on
//! first sight). This is the honest poll-based realization of the design's
//! "rotate on access-rule change" trigger: no change-feed yet, so we diff.
//!
//! The audience fingerprint covers each member's id and public key, order-
//! independent — so a reorder does not trigger a needless rotation, but an add,
//! remove, or key change does.

use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use p256::elliptic_curve::sec1::ToSec1Point;

use grant_crypto::service::{recipients, AudienceResolver, BundlePublisher, KeyService, Member};

/// Which step of a release-set's sweep failed. Carried as a type rather than
/// parsed back out of the error text, so the metrics surface can count failures
/// by stage without depending on message wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureStage {
    /// The audience could not be built — fail-closed, nothing was published.
    Resolve,
    /// Mint/wrap/sign/publish failed after the audience resolved.
    Rotate,
}

/// The result of sweeping one release-set.
#[derive(Debug, PartialEq, Eq)]
pub enum SweepOutcome {
    /// The audience changed (or first sight) and a new generation was published.
    Rotated {
        release_set: String,
        generation: u64,
        members: usize,
    },
    /// The audience was unchanged since the last published generation — no rotation.
    Unchanged { release_set: String },
    /// Resolve or publish failed — the previous generation stays valid.
    Failed {
        release_set: String,
        stage: FailureStage,
        error: String,
    },
}

/// An order-independent fingerprint of an audience (member ids + public keys),
/// taken over the same recipient map the bundle wraps to.
fn fingerprint(members: &[Member]) -> u64 {
    // A BTreeMap iterates in id order, so the entries are already sorted.
    let entries: Vec<(String, Vec<u8>)> = recipients(members)
        .into_iter()
        .map(|(id, key)| (id, key.to_sec1_point(true).as_bytes().to_vec()))
        .collect();
    let mut h = DefaultHasher::new();
    entries.hash(&mut h);
    h.finish()
}

/// Holds the key service and its integration seams, plus the last-published
/// audience fingerprint per release-set.
pub struct Daemon<R: AudienceResolver, P: BundlePublisher> {
    svc: KeyService,
    resolver: R,
    publisher: P,
    fingerprints: BTreeMap<String, u64>,
    /// Unix seconds at which a release-set's resolved audience was first seen to
    /// differ from its published generation, and has not been published since.
    /// Absent = the published generation matches the resolved audience.
    ///
    /// This is the **revocation-latency** signal, and it is the one thing the
    /// existing surface could not say. `rotations_total` and the last-rotation
    /// clock both describe rotations that *happened*; nothing described a
    /// rotation that was due and did not. While an entry is set, the live
    /// generation still wraps the DEK to a member the grant rows no longer
    /// include — a revoke that has been issued and has not taken effect. A
    /// rotate failure leaves it set on purpose, so the drift accrues and is
    /// alertable, rather than being spent as a single error tick.
    unpublished_since: BTreeMap<String, u64>,
    /// Wall time of the last successful rotate, per release-set. Rotation cost
    /// is O(audience size) — flat DEK wrapping — so this is the series that
    /// shows the flat-wrap ceiling approaching on a real audience rather than
    /// in a benchmark.
    last_rotation_ms: BTreeMap<String, u64>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl<R: AudienceResolver, P: BundlePublisher> Daemon<R, P> {
    pub fn new(svc: KeyService, resolver: R, publisher: P) -> Self {
        Self {
            svc,
            resolver,
            publisher,
            fingerprints: BTreeMap::new(),
            unpublished_since: BTreeMap::new(),
            last_rotation_ms: BTreeMap::new(),
        }
    }

    /// Per-release-set unix second at which a still-unpublished audience change
    /// was first observed. Empty = every swept release-set is in sync.
    pub fn unpublished_since(&self) -> &BTreeMap<String, u64> {
        &self.unpublished_since
    }

    /// Per-release-set wall time of the last successful rotate, milliseconds.
    pub fn last_rotation_ms(&self) -> &BTreeMap<String, u64> {
        &self.last_rotation_ms
    }

    /// Sweep every release-set once: resolve its audience, and rotate only if it
    /// changed since the last generation. Never returns Err — each release-set's
    /// outcome is reported independently so one failure doesn't stall the others.
    pub fn sweep(&mut self, release_sets: &[String]) -> Vec<SweepOutcome> {
        // Disjoint field borrows so `svc.rotate_to(.., publisher)` type-checks.
        let Self {
            svc,
            resolver,
            publisher,
            fingerprints,
            unpublished_since,
            last_rotation_ms,
        } = self;
        // Drop bookkeeping for release-sets that have left the sweep list, so a
        // retired set does not leave a drift series pinned at its last value.
        unpublished_since.retain(|rs, _| release_sets.contains(rs));
        last_rotation_ms.retain(|rs, _| release_sets.contains(rs));
        let mut outcomes = Vec::with_capacity(release_sets.len());
        for rs in release_sets {
            let audience = match resolver.resolve(rs) {
                Ok(a) => a,
                Err(e) => {
                    outcomes.push(SweepOutcome::Failed {
                        release_set: rs.clone(),
                        stage: FailureStage::Resolve,
                        error: format!("resolve: {e:?}"),
                    });
                    continue;
                }
            };
            let fp = fingerprint(&audience);
            if fingerprints.get(rs) == Some(&fp) {
                // Published generation matches the resolved audience: in sync.
                unpublished_since.remove(rs);
                outcomes.push(SweepOutcome::Unchanged {
                    release_set: rs.clone(),
                });
                continue;
            }
            // Changed (or first sight): rotate to the audience just resolved and
            // fingerprinted. Resolving again here could wrap a different
            // audience from the one whose fingerprint is stored below, and a
            // later sweep would then call the live bundle unchanged.
            //
            // Stamp the change BEFORE attempting to publish, and only if it is
            // not already stamped: a rotate that keeps failing must show one
            // drift interval growing from when the change was first seen, not
            // reset to zero on every retry.
            unpublished_since.entry(rs.clone()).or_insert_with(now_secs);
            let started = Instant::now();
            match svc.rotate_to(rs, &audience, publisher) {
                Ok(r) => {
                    fingerprints.insert(rs.clone(), fp);
                    unpublished_since.remove(rs);
                    last_rotation_ms.insert(rs.clone(), started.elapsed().as_millis() as u64);
                    outcomes.push(SweepOutcome::Rotated {
                        release_set: r.release_set,
                        generation: r.generation,
                        members: r.members,
                    });
                }
                Err(e) => outcomes.push(SweepOutcome::Failed {
                    release_set: rs.clone(),
                    stage: FailureStage::Rotate,
                    error: format!("rotate: {e:?}"),
                }),
            }
        }
        outcomes
    }

    /// Replace the signer after its SVID renews (see [`crate::signer`]).
    pub fn set_signer(&mut self, signer: p256::ecdsa::SigningKey, svid_pem: Option<String>) {
        self.svc.set_signer(signer, svid_pem);
    }

    /// Take a bundle published before a restart into the service's keeping.
    pub fn adopt(&mut self, bundle: grant_crypto::KeyBundle) {
        self.svc.adopt(bundle);
    }

    /// Re-sign and republish every retained bundle under the current signer.
    pub fn republish(&mut self) -> Result<usize, grant_crypto::service::ServiceError> {
        self.svc.republish(&self.publisher)
    }

    /// The publisher, for reading back stored bundles.
    pub fn publisher(&self) -> &P {
        &self.publisher
    }

    /// The key-service's public verifying key (for wiring / diagnostics).
    pub fn verifying_key(&self) -> &p256::ecdsa::VerifyingKey {
        self.svc.verifying_key()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grant_crypto::service::{Member, ServiceError};
    use p256::ecdsa::SigningKey;
    use p256::elliptic_curve::Generate;
    use p256::SecretKey;
    use std::cell::RefCell;

    fn member(id: &str) -> Member {
        Member {
            id: id.into(),
            key: SecretKey::generate().public_key(),
        }
    }

    // A resolver whose audience is swappable, to drive change-detection.
    struct MutResolver(RefCell<Vec<Member>>);
    impl AudienceResolver for MutResolver {
        fn resolve(&self, _rs: &str) -> Result<Vec<Member>, ServiceError> {
            Ok(self.0.borrow().clone())
        }
    }

    #[derive(Default)]
    struct CountingPublisher(RefCell<usize>);
    impl BundlePublisher for CountingPublisher {
        fn publish(&self, _topic: &str, _payload: &[u8]) -> Result<(), ServiceError> {
            *self.0.borrow_mut() += 1;
            Ok(())
        }
    }

    #[test]
    fn rotates_only_when_the_audience_changes() {
        let a = member("a");
        let b = member("b");
        let resolver = MutResolver(RefCell::new(vec![a.clone(), b.clone()]));
        let svc = KeyService::new(SigningKey::generate(), "ks");
        // We can't read the publisher's count through the moved value, so keep the
        // audience swaps observable via SweepOutcome instead.
        let mut d = Daemon::new(svc, resolver, CountingPublisher::default());
        let sets = vec!["rs".to_string()];

        // First sight → rotate to gen 1.
        assert_eq!(
            d.sweep(&sets),
            vec![SweepOutcome::Rotated {
                release_set: "rs".into(),
                generation: 1,
                members: 2
            }]
        );
        // Unchanged → no rotation.
        assert_eq!(
            d.sweep(&sets),
            vec![SweepOutcome::Unchanged {
                release_set: "rs".into()
            }]
        );
        // Reorder only → still unchanged (fingerprint is order-independent).
        d.resolver.0.borrow_mut().swap(0, 1);
        assert_eq!(
            d.sweep(&sets),
            vec![SweepOutcome::Unchanged {
                release_set: "rs".into()
            }]
        );
        // Remove a member → rotate to gen 2.
        d.resolver.0.borrow_mut().pop();
        assert_eq!(
            d.sweep(&sets),
            vec![SweepOutcome::Rotated {
                release_set: "rs".into(),
                generation: 2,
                members: 1
            }]
        );
    }

    #[test]
    fn a_failed_rotate_leaves_the_set_drifting_until_it_lands() {
        // The revocation-latency case. A publish failure must not be spent as a
        // single error tick: the release-set stays stamped as unpublished so the
        // drift keeps accruing, and the stamp must not reset on each retry or a
        // permanently-stuck rotation would read as permanently fresh.
        struct FlakyPublisher(RefCell<bool>);
        impl BundlePublisher for FlakyPublisher {
            fn publish(&self, _topic: &str, _payload: &[u8]) -> Result<(), ServiceError> {
                if *self.0.borrow() {
                    Ok(())
                } else {
                    Err(ServiceError::Publish("router refused".into()))
                }
            }
        }

        let resolver = MutResolver(RefCell::new(vec![member("a"), member("b")]));
        let svc = KeyService::new(SigningKey::generate(), "ks");
        let mut d = Daemon::new(svc, resolver, FlakyPublisher(RefCell::new(false)));
        let sets = vec!["rs".to_string()];

        // First sight, publish refused: reported as a rotate failure AND stamped.
        let out = d.sweep(&sets);
        assert!(matches!(
            out[0],
            SweepOutcome::Failed {
                stage: FailureStage::Rotate,
                ..
            }
        ));
        let first_stamp = *d
            .unpublished_since()
            .get("rs")
            .expect("a failed rotate stamps the set as unpublished");

        // Retry, still refused: same stamp, so the drift interval grows from
        // when the change was first seen rather than restarting.
        d.sweep(&sets);
        assert_eq!(
            d.unpublished_since().get("rs"),
            Some(&first_stamp),
            "a retry must not reset the drift stamp"
        );

        // Publisher recovers: the rotation lands and the set is back in sync.
        *d.publisher.0.borrow_mut() = true;
        let out = d.sweep(&sets);
        assert!(matches!(out[0], SweepOutcome::Rotated { .. }));
        assert!(
            !d.unpublished_since().contains_key("rs"),
            "a landed rotation clears the drift"
        );
        assert!(
            d.last_rotation_ms().contains_key("rs"),
            "a landed rotation records its cost"
        );
    }

    #[test]
    fn an_unchanged_set_is_never_drifting() {
        let resolver = MutResolver(RefCell::new(vec![member("a")]));
        let svc = KeyService::new(SigningKey::generate(), "ks");
        let mut d = Daemon::new(svc, resolver, CountingPublisher::default());
        let sets = vec!["rs".to_string()];
        d.sweep(&sets); // first sight -> rotates
        d.sweep(&sets); // unchanged
        assert!(d.unpublished_since().is_empty());
    }

    #[test]
    fn a_retired_release_set_leaves_no_drift_series_behind() {
        // Otherwise a set dropped from the sweep list keeps a drift gauge pinned
        // at its last value forever, and the alert on it never clears.
        struct DeadPublisher;
        impl BundlePublisher for DeadPublisher {
            fn publish(&self, _topic: &str, _payload: &[u8]) -> Result<(), ServiceError> {
                Err(ServiceError::Publish("down".into()))
            }
        }
        let resolver = MutResolver(RefCell::new(vec![member("a")]));
        let svc = KeyService::new(SigningKey::generate(), "ks");
        let mut d = Daemon::new(svc, resolver, DeadPublisher);
        d.sweep(&["rs".to_string()]);
        assert!(d.unpublished_since().contains_key("rs"));
        d.sweep(&["other".to_string()]);
        assert!(
            !d.unpublished_since().contains_key("rs"),
            "a set that left the sweep list drops its drift series"
        );
    }

    #[test]
    fn the_published_bundle_wraps_exactly_the_audience_that_was_fingerprinted() {
        // The audience source can change between two reads in one sweep (a
        // grant row removed and restored, a cache refresh). The sweep must read
        // it once: the generation it publishes wraps the members it
        // fingerprinted, so a later sweep that sees the same audience does not
        // leave a bundle live that wraps a member outside it.
        struct SequenceResolver {
            calls: RefCell<usize>,
            first: Vec<Member>,
            later: Vec<Member>,
        }
        impl AudienceResolver for SequenceResolver {
            fn resolve(&self, _rs: &str) -> Result<Vec<Member>, ServiceError> {
                let mut n = self.calls.borrow_mut();
                *n += 1;
                Ok(if *n == 1 {
                    self.first.clone()
                } else {
                    self.later.clone()
                })
            }
        }
        #[derive(Default)]
        struct CapturePublisher(RefCell<Vec<Vec<u8>>>);
        impl BundlePublisher for CapturePublisher {
            fn publish(&self, _topic: &str, payload: &[u8]) -> Result<(), ServiceError> {
                self.0.borrow_mut().push(payload.to_vec());
                Ok(())
            }
        }

        let a = member("a");
        let c = member("c");
        let resolver = SequenceResolver {
            calls: RefCell::new(0),
            first: vec![a.clone()],
            later: vec![a.clone(), c.clone()],
        };
        let svc = KeyService::new(SigningKey::generate(), "ks");
        let mut d = Daemon::new(svc, resolver, CapturePublisher::default());
        let sets = vec!["rs".to_string()];

        let out = d.sweep(&sets);
        assert_eq!(
            out,
            vec![SweepOutcome::Rotated {
                release_set: "rs".into(),
                generation: 1,
                members: 1
            }]
        );
        assert_eq!(
            *d.resolver.calls.borrow(),
            1,
            "one resolve per release-set per sweep"
        );
        let published = d.publisher.0.borrow();
        let bundle: grant_crypto::KeyBundle = serde_json::from_slice(&published[0]).unwrap();
        let wrapped: Vec<&String> = bundle.members.keys().collect();
        assert_eq!(
            wrapped,
            vec!["a"],
            "the bundle wraps the fingerprinted audience only"
        );
    }

    #[test]
    fn a_resolve_failure_is_reported_not_fatal() {
        struct FailResolver;
        impl AudienceResolver for FailResolver {
            fn resolve(&self, _rs: &str) -> Result<Vec<Member>, ServiceError> {
                Err(ServiceError::Resolve("boom".into()))
            }
        }
        let svc = KeyService::new(SigningKey::generate(), "ks");
        let mut d = Daemon::new(svc, FailResolver, CountingPublisher::default());
        let out = d.sweep(&["rs".to_string()]);
        assert!(matches!(
            out[0],
            SweepOutcome::Failed {
                stage: FailureStage::Resolve,
                ..
            }
        ));
    }
}
