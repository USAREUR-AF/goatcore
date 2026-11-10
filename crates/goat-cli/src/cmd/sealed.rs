//! The grant-keyed (supplemental) encryption glue for `goat pub --sealed` and
//! `goat sub --open` — the machine-plane read cut that travels WITH the data
//! (ADR 1094/1102, docs/design/grant-keyed-encryption.md).
//!
//! What it does, and what it is NOT:
//! - It fetches the release-set's key bundles off `keys/<rs>/**`, **verifies
//!   each signer's SVID against a SPIRE trust bundle** (fail-closed — a bundle
//!   from an untrusted signer is dropped, never used to wrap or open), unwraps
//!   the caller's OWN slot with the profile's enrollment key, and seals/opens.
//! - The unwrap key is the profile's own `mtls.key.pem`; the member id is the
//!   profile cert's slot anchor (CN). No new key material, no new trust surface
//!   — the enrollment identity IS the recipient identity by design.
//! - It does NOT invent a fabric session: the caller passes the already-open
//!   `goat` zenoh session (the same mTLS path every other pub/sub op uses).
//!
//! The signer trust inputs (the SPIRE bundle, trust domain, key-service SPIFFE
//! id) are goatnet config, not per-user secrets; they arrive as flags. A future
//! step folds them into the profile so `--sealed`/`--open` need no flags on a
//! goatnet that ships them in the bundle.

use std::time::Duration;

use grant_crypto::datapath::{
    acquire_stream_key, decode_message, encode_message, Context, KeyStore,
};
use grant_crypto::KeyBundle;
use grant_keyd::svid::{verify_bundle_self, SvidValidator, TrustBundle};
use p256::SecretKey;

use crate::profile::Profile;

/// The signer-trust inputs for a sealed op (the same set the grant-keyd
/// examples take). `key_service_id` defaults to `spiffe://<trust_domain>/grant-keyd`.
pub struct SealTrust {
    pub spire_bundle: String,
    pub trust_domain: String,
    pub key_service_id: String,
}

impl SealTrust {
    /// Resolve the trust inputs from flags. `key_service_id` defaults to the
    /// conventional `spiffe://<td>/grant-keyd` when not given.
    pub fn from_flags(
        spire_bundle: Option<String>,
        trust_domain: Option<String>,
        key_service_id: Option<String>,
    ) -> anyhow::Result<Self> {
        let spire_bundle = spire_bundle.ok_or_else(|| {
            anyhow::anyhow!(
                "--sealed/--open needs the key service's SPIRE trust bundle: pass \
                 --key-service-bundle <spire-root.pem> (the bundle grant-keyd signs with; \
                 on a goatnet's Thor it is the grant-keyd spire-bundle.pem)"
            )
        })?;
        let trust_domain = trust_domain.ok_or_else(|| {
            anyhow::anyhow!(
                "--sealed/--open needs --trust-domain <td> (e.g. zagros.goat.example)"
            )
        })?;
        let key_service_id =
            key_service_id.unwrap_or_else(|| format!("spiffe://{trust_domain}/grant-keyd"));
        Ok(Self {
            spire_bundle,
            trust_domain,
            key_service_id,
        })
    }

    fn validator(&self) -> anyhow::Result<SvidValidator> {
        // The authored message below used to never fire for a missing/
        // unreadable bundle file: `?` on the read short-circuited BEFORE
        // `.map_err()` got a chance to run (map_err only wrapped
        // `from_pem`'s own error), so a missing file surfaced as a bare,
        // unattributed io::Error instead of naming the path.
        use anyhow::Context;
        let pem = std::fs::read_to_string(&self.spire_bundle)
            .with_context(|| format!("read SPIRE bundle {}", self.spire_bundle))?;
        let bundle = TrustBundle::from_pem(&pem).map_err(|e| {
            anyhow::anyhow!("load key-service SPIRE bundle {}: {e:?}", self.spire_bundle)
        })?;
        Ok(SvidValidator::new(self.trust_domain.clone(), bundle))
    }
}

/// The caller's own member identity for a sealed op: its slot anchor (cert CN)
/// and its enrollment private key, both from the active profile.
pub struct SelfMember {
    pub anchor: String,
    pub secret: SecretKey,
}

impl SelfMember {
    pub fn from_profile(profile: &Profile) -> anyhow::Result<Self> {
        let anchor = crate::cmd::pubsub::own_anchor(profile).ok_or_else(|| {
            anyhow::anyhow!(
                "your profile cert carries no slot anchor (32-hex CN) — the sealed data path \
                 keys the audience on the slot anchor, so a legacy/shared cert cannot participate"
            )
        })?;
        let pem = {
            use anyhow::Context;
            std::fs::read_to_string(profile.key_path())
                .with_context(|| format!("read {}", profile.key_path().display()))?
        };
        let secret = load_secret(&pem).ok_or_else(|| {
            anyhow::anyhow!(
                "could not read a P-256 private key from {} — the enrollment key is the unwrap key",
                profile.key_path().display()
            )
        })?;
        Ok(Self { anchor, secret })
    }
}

fn load_secret(pem: &str) -> Option<SecretKey> {
    use p256::pkcs8::DecodePrivateKey;
    SecretKey::from_pkcs8_pem(pem)
        .or_else(|_| SecretKey::from_sec1_pem(pem))
        .ok()
}

/// The release-set for a key: `rs ≡ producer anchor` = the leading segment.
/// Refuses a wildcard leading segment (the audience is per producer, so a
/// sealed op must name one concrete producer subtree).
pub fn release_set_of(key: &str) -> anyhow::Result<String> {
    let seg = key.split('/').next().unwrap_or("");
    if seg.is_empty() || seg.contains('*') {
        anyhow::bail!(
            "sealed pub/sub needs a concrete producer anchor as the first key segment \
             (rs ≡ producer); got {key:?}"
        );
    }
    Ok(seg.to_string())
}

/// Fetch + verify the release-set's key bundles off `keys/<rs>/**`. Only bundles
/// whose signer SVID chains to the SPIRE root are returned — fail-closed on the
/// trust anchor. `session.get` is the async goat path.
pub async fn fetch_verified_bundles(
    session: &zenoh::Session,
    rs: &str,
    trust: &SealTrust,
    router: &str,
) -> anyhow::Result<Vec<KeyBundle>> {
    let validator = trust.validator()?;
    let replies = session
        .get(format!("keys/{rs}/**"))
        .timeout(Duration::from_secs(10))
        .await
        .map_err(|e| crate::classify::classify_zenoh_err(e, router, "query key bundles"))?;
    let mut bundles = Vec::new();
    while let Ok(reply) = replies.recv_async().await {
        // Both drops below used to be totally silent — collapsing several
        // very different causes (a router ACL denial per F-177, a malformed
        // bundle, a version-skew wire-format mismatch) into one downstream
        // symptom: "fewer key bundles than expected, no idea why".
        let sample = match reply.result() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  reply error on keys/{rs}/**: {e:?} (a router ACL denial, F-177, reads this way too)");
                continue;
            }
        };
        let wire = sample.payload().to_bytes().to_vec();
        let bundle: KeyBundle = match serde_json::from_slice(&wire) {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "  skip undecodable key bundle on `{}`: {e}",
                    sample.key_expr().as_str()
                );
                continue;
            }
        };
        match verify_bundle_self(&bundle, &validator, &trust.key_service_id) {
            Ok(()) => bundles.push(bundle),
            Err(e) => eprintln!(
                "  reject key bundle {} gen={}: signer not trusted ({e:?})",
                bundle.rs, bundle.gen
            ),
        }
    }
    Ok(bundles)
}

/// Producer side: acquire the current DEK by unwrapping our own slot from the
/// newest verified bundle, seal `plaintext` for `keyexpr`, and return the
/// self-describing envelope wire bytes to PUT. Fail-closed when we hold no slot
/// (not in the audience) or no verified bundle exists.
pub async fn seal_for(
    session: &zenoh::Session,
    keyexpr: &str,
    plaintext: &[u8],
    me: &SelfMember,
    trust: &SealTrust,
    router: &str,
) -> anyhow::Result<Vec<u8>> {
    let rs = release_set_of(keyexpr)?;
    if rs != me.anchor {
        anyhow::bail!(
            "sealed publish must be under your own producer anchor {} (you are the release-set \
             member that holds the DEK); got a key under {rs:?}",
            me.anchor
        );
    }
    let bundles = fetch_verified_bundles(session, &rs, trust, router).await?;
    if bundles.is_empty() {
        anyhow::bail!(
            "no verified key bundle for release-set {rs:?} — is grant-keyd publishing keys/{rs}/**, \
             and are you in the audience? (grant yourself/your org first, then retry)"
        );
    }
    let mut sk = acquire_stream_key(&rs, &bundles, &me.anchor, &me.secret).map_err(|e| {
        anyhow::anyhow!(
            "no slot for {} in any bundle — the producer must be in its own release-set audience: {e:?}",
            me.anchor
        )
    })?;
    // seal_current, not seal: refuse if a newer generation exists for this
    // release set. This path re-fetches per message so it would be safe either
    // way, but the demonstrated path should be the correct one — a producer that
    // holds its key is the case F-498 is about.
    let env = sk
        .seal_current(Context(keyexpr), plaintext, &bundles)
        .map_err(|e| match e {
            grant_crypto::Error::Superseded => anyhow::anyhow!(
                "the key for {rs} was superseded while sealing — the owner changed who may read. \
                 Re-run to pick up the new generation."
            ),
            other => anyhow::anyhow!("seal: {other:?}"),
        })?;
    encode_message(&env).map_err(|e| anyhow::anyhow!("encode envelope: {e:?}"))
}

/// Consumer side: build a `KeyStore` admitting every slot we can open from the
/// verified bundles. `admitted()` reports how many generations we hold — zero
/// means every read will be DENIED (the expected outcome for a non-member).
pub async fn build_keystore(
    session: &zenoh::Session,
    rs: &str,
    me: &SelfMember,
    trust: &SealTrust,
    router: &str,
) -> anyhow::Result<(KeyStore, usize)> {
    let bundles = fetch_verified_bundles(session, rs, trust, router).await?;
    let mut store = KeyStore::new();
    let mut admitted = 0usize;
    for bundle in &bundles {
        match store.admit(bundle, &me.anchor, &me.secret) {
            Ok(true) => admitted += 1,
            Ok(false) => {}
            Err(e) => eprintln!("  bundle {} gen={}: bad slot: {e:?}", bundle.rs, bundle.gen),
        }
    }
    Ok((store, admitted))
}

/// Open one received sample body with the keystore. Returns the plaintext, or a
/// reason it could not be opened (wrong key, unknown generation, not-an-envelope).
pub fn open_sample(store: &KeyStore, keyexpr: &str, body: &[u8]) -> Result<Vec<u8>, String> {
    let env = decode_message(body).map_err(|_| "not a sealed envelope".to_string())?;
    store
        .open(Context(keyexpr), &env)
        .map_err(|e| format!("cannot open (no key for this generation, or not a member): {e:?}"))
}
