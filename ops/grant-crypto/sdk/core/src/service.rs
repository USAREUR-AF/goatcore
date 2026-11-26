//! E1 — the key-distribution service core (design §3, M2.1).
//!
//! The state machine and assembly pipeline, off the read path: on a grant change
//! or rotation trigger it resolves the audience, mints a fresh DEK for the next
//! generation, wraps it to each member, signs the bundle, and publishes it on
//! `keys/<release-set>/<generation>`. The two integration seams — resolving the
//! audience and publishing to the fabric — are traits, so this logic is testable
//! in isolation; the live Zenoh publisher and HTTP resolver are concrete impls
//! added on top.
//!
//! Rotation model (per the external review's readmission point): each rotation is
//! a new generation. A revoked member is simply absent from the next generation's
//! bundle; a re-admitted member reappears in a later generation and never receives
//! the generations it was excluded from — the generation label carries revocation.

#[cfg(test)]
use p256::elliptic_curve::Generate;
use std::collections::BTreeMap;

use p256::ecdsa::{SigningKey, VerifyingKey};
use p256::PublicKey;
use zeroize::Zeroize;

use crate::{new_bundle, new_dek, sign_bundle, KeyBundle};

/// One authorized member of a release-set: its stable id and its recipient
/// public key (its SVID's EC key for the classical `v1` suite).
#[derive(Clone)]
pub struct Member {
    pub id: String,
    pub key: PublicKey,
}

/// Resolves a release-set to its current authorized audience (member set +
/// recipient keys). In production this reads the portal resolver over HTTP plus
/// the identity registry / PKI; here it is a seam so the state machine is testable.
pub trait AudienceResolver {
    fn resolve(&self, release_set: &str) -> Result<Vec<Member>, ServiceError>;
}

/// So the daemon can hold whichever resolver it selected at startup (the
/// operator-declared `ConfiguredAudience` vs the grant-row-driven resolver) behind
/// one type — mirrors the `Box<dyn RecipientKeys>` pattern.
impl AudienceResolver for Box<dyn AudienceResolver> {
    fn resolve(&self, release_set: &str) -> Result<Vec<Member>, ServiceError> {
        (**self).resolve(release_set)
    }
}

/// Publishes a key bundle onto the fabric. In production this is a native Zenoh
/// session publishing on the key topic; here it is a seam.
pub trait BundlePublisher {
    fn publish(&self, topic: &str, payload: &[u8]) -> Result<(), ServiceError>;
}

/// A key-service failure.
#[derive(Debug)]
pub enum ServiceError {
    /// The audience could not be resolved.
    Resolve(String),
    /// The bundle could not be published.
    Publish(String),
    /// A crypto operation failed (wrap / sign / serialize).
    Crypto(String),
}

impl From<crate::Error> for ServiceError {
    fn from(e: crate::Error) -> Self {
        ServiceError::Crypto(format!("{e:?}"))
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::Resolve(m) => write!(f, "resolve: {m}"),
            ServiceError::Publish(m) => write!(f, "publish: {m}"),
            ServiceError::Crypto(m) => write!(f, "crypto: {m}"),
        }
    }
}

impl std::error::Error for ServiceError {}

/// The key topic a bundle is published on — kept as one helper so publisher and
/// consumer never drift on the path shape (matches the Go reference `BundleTopic`).
pub fn bundle_topic(release_set: &str, generation: u64) -> String {
    format!("keys/{release_set}/{generation}")
}

/// The outcome of a rotation, for logging / telemetry (never carries key material).
pub struct Rotation {
    pub release_set: String,
    pub generation: u64,
    pub members: usize,
}

/// The per-site key-distribution service. Holds its signing identity and the
/// current generation per release-set. It never retains a DEK — each is minted,
/// wrapped into the bundle, and dropped.
pub struct KeyService {
    signer: SigningKey,
    signer_id: String,
    /// The key-service's own SVID (PEM), attached to each published bundle so
    /// members can validate the signer against the SPIRE root without a side
    /// channel (self-describing authenticity). `None` = don't attach.
    signer_svid: Option<String>,
    generations: BTreeMap<String, u64>,
    /// Every bundle this service has published or adopted, by (release-set,
    /// generation). Wrapped DEKs only: the plaintext DEK is never kept. Held so
    /// the bundles can be re-signed when the signer's SVID renews — a bundle
    /// carries the SVID it was signed under, and a reader refuses it once that
    /// SVID expires, which for a SPIRE SVID is about an hour.
    retained: BTreeMap<(String, u64), KeyBundle>,
}

impl KeyService {
    /// Create a service with its key-service signing identity.
    pub fn new(signer: SigningKey, signer_id: impl Into<String>) -> Self {
        KeyService {
            signer,
            signer_id: signer_id.into(),
            signer_svid: None,
            generations: BTreeMap::new(),
            retained: BTreeMap::new(),
        }
    }

    /// Replace the signing key and the SVID attached to bundles, after the SVID
    /// renews. Bundles already published keep their old signature until
    /// [`KeyService::republish`] re-signs them.
    pub fn set_signer(&mut self, signer: SigningKey, svid_pem: Option<String>) {
        self.signer = signer;
        self.signer_svid = svid_pem;
    }

    /// Take a bundle this service published before it restarted (read back from
    /// the fabric's `keys/**` storage) into its keeping: generations continue
    /// above it instead of starting again at 1, and [`KeyService::republish`]
    /// re-signs it. The caller authenticates the bundle first.
    pub fn adopt(&mut self, bundle: KeyBundle) {
        let gen = self.generations.entry(bundle.rs.clone()).or_insert(0);
        *gen = (*gen).max(bundle.gen);
        self.retained
            .entry((bundle.rs.clone(), bundle.gen))
            .or_insert(bundle);
    }

    /// Re-sign every retained bundle with the current signer and SVID and publish
    /// it again on its own `keys/<rs>/<gen>` topic. Wrapped keys and membership
    /// are unchanged; only the signature and the attached SVID are new. Returns
    /// how many were published; stops at the first publish failure.
    pub fn republish(&mut self, publisher: &dyn BundlePublisher) -> Result<usize, ServiceError> {
        let mut n = 0;
        for ((rs, gen), bundle) in self.retained.iter_mut() {
            sign_bundle(bundle, &self.signer_id, &self.signer)?;
            bundle.signer_svid = self.signer_svid.clone();
            publisher.publish(&bundle_topic(rs, *gen), &serialize_bundle(bundle)?)?;
            n += 1;
        }
        Ok(n)
    }

    /// Attach the key-service's own SVID (PEM) so every published bundle is
    /// self-describing — members obtain the verifying key by validating it.
    pub fn with_signer_svid(mut self, svid_pem: impl Into<String>) -> Self {
        self.signer_svid = Some(svid_pem.into());
        self
    }

    /// The public verifying key members use to authenticate this service's bundles.
    pub fn verifying_key(&self) -> &VerifyingKey {
        self.signer.verifying_key()
    }

    /// The current generation for a release-set (0 = none minted yet).
    pub fn current_generation(&self, release_set: &str) -> u64 {
        self.generations.get(release_set).copied().unwrap_or(0)
    }

    /// Rotate a release-set: resolve the audience, mint the next-generation DEK,
    /// wrap it to every member, sign the bundle, and publish it. Fail-closed —
    /// any wrap failure aborts the whole rotation (nothing is published).
    pub fn rotate(
        &mut self,
        release_set: &str,
        resolver: &dyn AudienceResolver,
        publisher: &dyn BundlePublisher,
    ) -> Result<Rotation, ServiceError> {
        let members = resolver.resolve(release_set)?;
        self.rotate_to(release_set, &members, publisher)
    }

    /// Rotate a release-set to an audience the caller has already resolved:
    /// mint the next-generation DEK, wrap it to exactly `members`, sign, and
    /// publish. A caller that decides whether to rotate from a resolved audience
    /// passes that same audience here, so the bundle wraps what it inspected
    /// rather than a second, possibly different, read.
    pub fn rotate_to(
        &mut self,
        release_set: &str,
        members: &[Member],
        publisher: &dyn BundlePublisher,
    ) -> Result<Rotation, ServiceError> {
        let generation = self.current_generation(release_set) + 1;

        let recipients = recipients(members);

        // The DEK is minted, wrapped into the bundle, and dropped — the service never
        // retains it. Zeroize the local copy now that it is wrapped (best-effort hygiene,
        // matching the "minted, wrapped, dropped" contract).
        let mut dek = new_dek();
        let mut bundle = new_bundle(release_set, generation, &dek, &recipients)?;
        dek.zeroize();
        sign_bundle(&mut bundle, &self.signer_id, &self.signer)?;
        if let Some(svid) = &self.signer_svid {
            crate::set_signer_svid(&mut bundle, svid.clone());
        }

        let payload = serialize_bundle(&bundle)?;
        publisher.publish(&bundle_topic(release_set, generation), &payload)?;

        self.generations.insert(release_set.to_string(), generation);
        self.retained
            .insert((release_set.to_string(), generation), bundle);
        Ok(Rotation {
            release_set: release_set.to_string(),
            generation,
            members: recipients.len(),
        })
    }
}

/// The recipient map a bundle wraps to, keyed by member id. A repeated id keeps
/// its last key, so this map, not the member list, is the wrapped audience.
pub fn recipients(members: &[Member]) -> BTreeMap<String, PublicKey> {
    members.iter().map(|m| (m.id.clone(), m.key)).collect()
}

/// Serialize a bundle for the wire (JSON, matching the Go reference format).
pub fn serialize_bundle(bundle: &KeyBundle) -> Result<Vec<u8>, ServiceError> {
    serde_json::to_vec(bundle).map_err(|e| ServiceError::Crypto(format!("serialize: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{unwrap_dek, verify_bundle};
    use p256::SecretKey;
    use std::cell::RefCell;

    // A member and its retained secret, so the test can unwrap what the service wraps.
    struct TestMember {
        secret: SecretKey,
        member: Member,
    }
    fn member(id: &str) -> TestMember {
        let secret = SecretKey::generate();
        let member = Member {
            id: id.into(),
            key: secret.public_key(),
        };
        TestMember { secret, member }
    }

    struct FakeResolver(Vec<Member>);
    impl AudienceResolver for FakeResolver {
        fn resolve(&self, _rs: &str) -> Result<Vec<Member>, ServiceError> {
            Ok(self.0.clone())
        }
    }

    #[derive(Default)]
    struct CapturePublisher(RefCell<Vec<(String, Vec<u8>)>>);
    impl BundlePublisher for CapturePublisher {
        fn publish(&self, topic: &str, payload: &[u8]) -> Result<(), ServiceError> {
            self.0
                .borrow_mut()
                .push((topic.to_string(), payload.to_vec()));
            Ok(())
        }
    }

    fn service() -> KeyService {
        KeyService::new(SigningKey::generate(), "key-service")
    }

    #[test]
    fn rotate_publishes_a_signed_bundle_members_can_open() {
        let a = member("a");
        let b = member("b");
        let resolver = FakeResolver(vec![a.member.clone(), b.member.clone()]);
        let pubr = CapturePublisher::default();
        let mut svc = service();

        let r = svc.rotate("coalition-air", &resolver, &pubr).unwrap();
        assert_eq!(r.generation, 1);
        assert_eq!(r.members, 2);

        let (topic, payload) = pubr.0.borrow()[0].clone();
        assert_eq!(topic, "keys/coalition-air/1");
        let bundle: KeyBundle = serde_json::from_slice(&payload).unwrap();

        // The bundle is authenticated by the key-service.
        verify_bundle(&bundle, svc.verifying_key()).unwrap();
        // Each member can recover the DEK; both get the same key.
        let dek_a = unwrap_dek(&bundle.members["a"], &a.secret).unwrap();
        let dek_b = unwrap_dek(&bundle.members["b"], &b.secret).unwrap();
        assert_eq!(dek_a, dek_b);
    }

    #[test]
    fn generation_increments_and_revocation_readmission_track() {
        let a = member("a");
        let b = member("b");
        let c = member("c");
        let pubr = CapturePublisher::default();
        let mut svc = service();

        // gen 1: {a,b,c}
        let full = FakeResolver(vec![a.member.clone(), b.member.clone(), c.member.clone()]);
        assert_eq!(svc.rotate("rs", &full, &pubr).unwrap().generation, 1);

        // gen 2: revoke c -> {a,b}
        let revoked = FakeResolver(vec![a.member.clone(), b.member.clone()]);
        assert_eq!(svc.rotate("rs", &revoked, &pubr).unwrap().generation, 2);

        // gen 3: readmit c -> {a,b,c}
        assert_eq!(svc.rotate("rs", &full, &pubr).unwrap().generation, 3);

        let published = pubr.0.borrow();
        let gen_of = |i: usize| -> KeyBundle { serde_json::from_slice(&published[i].1).unwrap() };
        // gen 2 excludes the revoked member; c cannot unwrap gen 2.
        let g2 = gen_of(1);
        assert!(!g2.members.contains_key("c"));
        assert!(unwrap_dek(g2.members.get("c").unwrap_or(&g2.members["a"]), &c.secret).is_err());
        // c is back in gen 3 and unwraps it.
        let g3 = gen_of(2);
        assert!(g3.members.contains_key("c"));
        unwrap_dek(&g3.members["c"], &c.secret).unwrap();
    }

    #[test]
    fn an_adopted_bundle_moves_the_next_generation_above_it() {
        // A restarted service that adopts generation 2 from storage must mint 3,
        // not 1: a fresh 1 would leave the older 2 as the newest generation, and
        // producers seal under the newest one they can see.
        let a = member("a");
        let pubr = CapturePublisher::default();
        let mut before = service();
        let resolver = FakeResolver(vec![a.member.clone()]);
        before.rotate("rs", &resolver, &pubr).unwrap();
        before.rotate("rs", &resolver, &pubr).unwrap();
        let g2: KeyBundle = serde_json::from_slice(&pubr.0.borrow()[1].1).unwrap();

        let mut after = service();
        after.adopt(g2);
        assert_eq!(after.current_generation("rs"), 2);
        assert_eq!(after.rotate("rs", &resolver, &pubr).unwrap().generation, 3);
    }

    #[test]
    fn republish_resigns_every_retained_bundle_under_the_new_signer() {
        let a = member("a");
        let pubr = CapturePublisher::default();
        let mut svc = service();
        let resolver = FakeResolver(vec![a.member.clone()]);
        svc.rotate("rs", &resolver, &pubr).unwrap();
        svc.rotate("rs", &resolver, &pubr).unwrap();
        let before: Vec<KeyBundle> = pubr
            .0
            .borrow()
            .iter()
            .map(|(_, p)| serde_json::from_slice(p).unwrap())
            .collect();

        let renewed = SigningKey::generate();
        let renewed_vk = *renewed.verifying_key();
        svc.set_signer(renewed, Some("renewed-svid".into()));
        let republished = CapturePublisher::default();
        assert_eq!(svc.republish(&republished).unwrap(), 2);

        let out = republished.0.borrow();
        assert_eq!(out[0].0, "keys/rs/1");
        assert_eq!(out[1].0, "keys/rs/2");
        for (i, (_, payload)) in out.iter().enumerate() {
            let b: KeyBundle = serde_json::from_slice(payload).unwrap();
            verify_bundle(&b, &renewed_vk).unwrap();
            assert!(verify_bundle(&b, svc.verifying_key()).is_ok());
            assert_eq!(b.signer_svid.as_deref(), Some("renewed-svid"));
            // Same wrapped key: the member opens the same DEK as before.
            assert_eq!(
                unwrap_dek(&b.members["a"], &a.secret).unwrap(),
                unwrap_dek(&before[i].members["a"], &a.secret).unwrap()
            );
        }
    }
}
