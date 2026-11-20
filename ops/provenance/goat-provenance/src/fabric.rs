//! Signer certificates on the fabric: publication on
//! `<signer anchor>/_goat/prov/cert`, fetch, a small cache, and the
//! per-sample grader that subscribers use.

use crate::cert::{anchor_of_cn, check_signer_cert, Signer, SignerCert, TrustRoots};
use crate::prov::{self, Extracted, Grade};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Where a signer's certificate is published: a stored key in its own namespace.
pub fn cert_key(anchor: &str) -> String {
    format!("{anchor}/_goat/prov/cert")
}

/// How long a fetch waits for the stored certificate.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(3);

/// Fetch the PEM published on `cert_key(anchor)`. `None` when nothing answers.
pub async fn fetch_cert(
    session: &zenoh::Session,
    anchor: &str,
    timeout: Duration,
) -> Option<String> {
    let replies = session.get(cert_key(anchor)).timeout(timeout).await.ok()?;
    while let Ok(reply) = replies.recv_async().await {
        if let Ok(sample) = reply.result() {
            if let Ok(text) = String::from_utf8(sample.payload().to_bytes().to_vec()) {
                return Some(text);
            }
        }
    }
    None
}

/// Publish the signer's certificate once: put it only when the stored value is
/// missing or different. Returns whether a put was made.
pub async fn publish_cert(session: &zenoh::Session, signer: &Signer) -> Result<bool, String> {
    let current = fetch_cert(session, &signer.anchor, FETCH_TIMEOUT).await;
    if current.as_deref().map(str::trim) == Some(signer.cert_pem.trim()) {
        return Ok(false);
    }
    session
        .put(
            cert_key(&signer.anchor),
            signer.cert_pem.clone().into_bytes(),
        )
        .await
        .map_err(|e| format!("put {}: {e}", cert_key(&signer.anchor)))?;
    Ok(true)
}

enum Entry {
    Good(SignerCert, Instant),
    Missing(String, Instant),
}

/// A bounded cache of verified signer certificates, keyed by signer CN.
///
/// A refusal is cached too, for a shorter time, so a stream of samples from
/// one signer whose certificate is absent costs one query per `miss_ttl`, not
/// one per sample (client demand must not amplify into fabric load, ADR 1123).
pub struct CertCache {
    roots: TrustRoots,
    entries: HashMap<String, Entry>,
    pub ttl: Duration,
    pub miss_ttl: Duration,
    pub capacity: usize,
}

impl CertCache {
    pub fn new(roots: TrustRoots) -> Self {
        Self {
            roots,
            entries: HashMap::new(),
            ttl: Duration::from_secs(600),
            miss_ttl: Duration::from_secs(30),
            capacity: 1024,
        }
    }

    /// The verified certificate for `signer_cn`, fetching it with `fetch`
    /// (given the signer's anchor) when it is not cached.
    pub async fn get<F, Fut>(&mut self, signer_cn: &str, fetch: F) -> Result<SignerCert, String>
    where
        F: FnOnce(String) -> Fut,
        Fut: std::future::Future<Output = Option<String>>,
    {
        let now = Instant::now();
        match self.entries.get(signer_cn) {
            Some(Entry::Good(c, at)) if now.duration_since(*at) < self.ttl => return Ok(c.clone()),
            Some(Entry::Missing(why, at)) if now.duration_since(*at) < self.miss_ttl => {
                return Err(why.clone())
            }
            _ => {}
        }
        let Some(anchor) = anchor_of_cn(signer_cn) else {
            return Err(format!("signer {signer_cn:?} is not a goat slot leaf"));
        };
        let result = match fetch(anchor.to_string()).await {
            None => Err(format!("no signer certificate on {}", cert_key(anchor))),
            Some(pem) => check_signer_cert(&pem, &self.roots, Some(signer_cn))
                .map_err(|e| format!("signer certificate on {}: {e}", cert_key(anchor))),
        };
        if self.entries.len() >= self.capacity {
            self.entries.clear();
        }
        let entry = match &result {
            Ok(c) => Entry::Good(c.clone(), now),
            Err(why) => Entry::Missing(why.clone(), now),
        };
        self.entries.insert(signer_cn.to_string(), entry);
        result
    }
}

/// Grades samples for a subscriber: extract `prov`, fetch and cache the
/// signer certificate, verify.
pub struct Verifier {
    session: zenoh::Session,
    cache: CertCache,
}

impl Verifier {
    pub fn new(session: zenoh::Session, roots: TrustRoots) -> Self {
        Self {
            session,
            cache: CertCache::new(roots),
        }
    }

    pub async fn grade(&mut self, key: &str, payload: &[u8], attachment: Option<&[u8]>) -> Grade {
        let raw = match prov::extract(attachment) {
            Extracted::Absent => return Grade::without_prov(key, false),
            Extracted::Malformed(why) => return Grade::Invalid(why),
            Extracted::Present(v) => v,
        };
        let parsed = match prov::parse(&raw) {
            Ok(p) => p,
            Err(e) => return Grade::Invalid(e.to_string()),
        };
        let session = self.session.clone();
        let cert = self
            .cache
            .get(&parsed.signer, |anchor| async move {
                fetch_cert(&session, &anchor, FETCH_TIMEOUT).await
            })
            .await;
        let cert = match cert {
            Ok(c) => c,
            Err(why) => return Grade::Invalid(why),
        };
        match prov::verify(&raw, key, payload, &cert) {
            Ok(p) => Grade::of_verified(&p),
            Err(e) => Grade::Invalid(e.to_string()),
        }
    }
}
