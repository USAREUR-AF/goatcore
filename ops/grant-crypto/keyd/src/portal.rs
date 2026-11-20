//! Portal-CA cert validation — the **portal-minted member** half of the durable
//! key source (WS2 of the alignment plan; closes the F-267 "only the unvalidated
//! stopgap resolves an audience" gap for the real sharing population).
//!
//! [`crate::svid::SvidRecipientKeys`] durably resolves SPIRE workloads — but the
//! actual sharing audience (partner/coalition orgs, human clients, non-SPIRE
//! machines) is **portal-minted**: leaves signed by the goatnet's offline CA
//! (ADR 0406/0407, `cert-sign`), carrying the write-anchor as **CN** (slot-pool
//! shape, ADR 1025) and the identity URI-SAN `<prefix><anchor>` (e.g.
//! `urn:goat:<site>:org=acme`). Those certs have no SPIFFE path and never chain to
//! SPIRE, so the SVID source rightly rejects them.
//!
//! [`PortalCertRecipientKeys`] is their validated peer: same vetted chain
//! validation (rustls-webpki — signature, validity, EKU, intermediates), anchored
//! to the **portal offline-CA root** instead of SPIRE, and the identity binding is
//! **member-id = CN = URI-SAN suffix** — the §3a "member id is the write-anchor"
//! invariant, enforced at key-resolve time rather than doc-asserted. The chain
//! accepts Ed25519 issuer signatures (the dev/prod offline root is Ed25519, with
//! an optional ECDSA-P256 intermediate — F-090), but the **leaf key must be
//! P-256** (the recipient contract; a pre-F-270 Ed25519 leaf is refused with a
//! rotation pointer). Fail-closed throughout.

use std::path::PathBuf;

use p256::pkcs8::DecodePublicKey;
use p256::PublicKey;
use rustls_pki_types::{CertificateDer, UnixTime};
use webpki::{anchor_from_trusted_cert, EndEntityCert, KeyUsage};
use x509_cert::der::asn1::{PrintableStringRef, Utf8StringRef};
use x509_cert::der::oid::{AssociatedOid, ObjectIdentifier};
use x509_cert::der::{Decode, DecodePem, Encode};
use x509_cert::ext::pkix::{name::GeneralName, SubjectAltName};
use x509_cert::Certificate;

use grant_crypto::service::ServiceError;

use crate::svid::TrustBundle;

/// id-at-commonName (2.5.4.3).
const OID_CN: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.3");

/// Chain signature algorithms for portal-minted leaves: the offline root signs
/// with Ed25519 (ADR 0406 dev/prod ceremony); the F-090 intermediate and the
/// post-F-270 leaves are ECDSA-P256 (P-384 allowed forward-compatibly).
static SIG_ALGS: &[&dyn rustls_pki_types::SignatureVerificationAlgorithm] = &[
    webpki::ring::ED25519,
    webpki::ring::ECDSA_P256_SHA256,
    webpki::ring::ECDSA_P384_SHA384,
];

fn crypto(msg: impl Into<String>) -> ServiceError {
    ServiceError::Crypto(format!("portal-cert: {}", msg.into()))
}

/// Validates portal-minted leaves against the offline-CA root and the
/// member-id↔CN↔URI-SAN identity binding.
pub struct PortalCertValidator {
    /// The per-goatnet identity URI-SAN prefix, e.g. `urn:goat:<site>:org=`. The
    /// leaf must carry exactly `<prefix><member-id>`.
    uri_prefix: String,
    bundle: TrustBundle,
}

impl PortalCertValidator {
    pub fn new(uri_prefix: impl Into<String>, bundle: TrustBundle) -> Self {
        Self {
            uri_prefix: uri_prefix.into(),
            bundle,
        }
    }

    /// Validate a member's cert PEM (leaf first; any further CERTIFICATE blocks
    /// are treated as intermediates) and return its P-256 recipient key.
    pub fn validate(&self, pem: &str, member_id: &str) -> Result<PublicKey, ServiceError> {
        self.validate_at(pem, member_id, UnixTime::now())
    }

    fn validate_at(
        &self,
        pem: &str,
        member_id: &str,
        now: UnixTime,
    ) -> Result<PublicKey, ServiceError> {
        let mut certs = parse_pem_certs(pem)?;
        if certs.is_empty() {
            return Err(crypto("no CERTIFICATE blocks"));
        }
        let leaf = certs.remove(0);
        let intermediates: Vec<CertificateDer<'static>> = certs
            .iter()
            .map(|c| der_of(c).map(CertificateDer::from))
            .collect::<Result<_, _>>()?;
        let leaf_der = CertificateDer::from(der_of(&leaf)?);

        // (1) Vetted chain validation to the PORTAL offline-CA root: signature,
        //     validity window, client-auth usage, intermediates.
        let anchors: Vec<_> = self
            .bundle
            .roots()
            .iter()
            .map(|r| anchor_from_trusted_cert(r).map_err(|e| crypto(format!("bad root: {e}"))))
            .collect::<Result<_, _>>()?;
        let ee = EndEntityCert::try_from(&leaf_der)
            .map_err(|e| crypto(format!("leaf not a cert: {e}")))?;
        ee.verify_for_usage(
            SIG_ALGS,
            &anchors,
            &intermediates,
            now,
            KeyUsage::client_auth(),
            None,
            None,
        )
        .map_err(|e| crypto(format!("chain invalid: {e}")))?;

        // (2) Identity binding, both halves — the §3a member-id = write-anchor
        //     invariant. CN is what the router ACL admits; the URI-SAN is the
        //     frozen identity grammar. Either mismatching the member id means
        //     this cert cannot stand in for that member.
        //     The CN carries the slot's leaf generation, `<anchor>.g<N>`
        //     (GHSA-rr3p); the member id is the anchor before it.
        let cn = subject_cn(&leaf)?;
        if cn_anchor(&cn) != member_id {
            return Err(crypto(format!(
                "CN {cn:?} != member id {member_id:?} (anchor binding)"
            )));
        }
        let expected_uri = format!("{}{}", self.uri_prefix, member_id);
        let uri = single_prefixed_uri(&leaf, &self.uri_prefix)?;
        if uri != expected_uri {
            return Err(crypto(format!(
                "identity URI-SAN {uri:?} != expected {expected_uri:?}"
            )));
        }

        // (3) The recipient key: the leaf's P-256 SPKI. A pre-F-270 Ed25519 leaf
        //     lands here — name the remedy, since the fleet carries such certs.
        let spki_der = leaf
            .tbs_certificate
            .subject_public_key_info
            .to_der()
            .map_err(|e| crypto(format!("encode spki: {e}")))?;
        PublicKey::from_public_key_der(&spki_der).map_err(|_| {
            crypto(format!(
                "leaf key for {member_id:?} is not P-256 (EC-only). A leaf minted before the \
                 F-270 fix is Ed25519 and can never unwrap — rotate it (in-place re-issue, \
                 ADR 1044)"
            ))
        })
    }
}

/// Recipient keys for portal-minted members: `<dir>/<member-id>.pem`, validated
/// against the portal offline-CA root and the CN/URI-SAN identity binding. The
/// validated peer of [`crate::svid::SvidRecipientKeys`] for the non-SPIRE
/// population; the same fail-closed posture (an unresolvable member is dropped
/// from the wrap set, never wrapped to a guessed key).
pub struct PortalCertRecipientKeys {
    dir: PathBuf,
    validator: PortalCertValidator,
}

impl PortalCertRecipientKeys {
    pub fn new(dir: impl Into<PathBuf>, validator: PortalCertValidator) -> Self {
        Self {
            dir: dir.into(),
            validator,
        }
    }
}

impl crate::recipient::RecipientKeys for PortalCertRecipientKeys {
    fn key(&self, member_id: &str) -> Result<PublicKey, ServiceError> {
        let path = crate::recipient::member_pem_path(&self.dir, member_id)?;
        let pem = std::fs::read_to_string(&path)
            .map_err(|e| crypto(format!("read {}: {e}", path.display())))?;
        self.validator.validate(&pem, member_id)
    }
}

/// The U7 "publish my key" distribution half: a [`PortalCertRecipientKeys`]
/// that first refreshes each member's PEM from the portal's by-anchor
/// recipient-cert store (`GET /api/access/recipient-cert`) into `cert_dir`,
/// then resolves through the validated source as usual.
///
/// Trust posture is unchanged — the fetch is DISTRIBUTION only. Whatever the
/// portal serves is written to disk and then re-validated (offline-CA chain +
/// CN/URI-SAN binding + P-256) by the inner validator before it can ever be a
/// wrap target; a portal serving the wrong cert fails the identity binding and
/// the member fails closed. When the portal is unreachable the cached file (the
/// last synced or hand-staged PEM) still resolves, so a portal outage degrades
/// to the pre-U7 behavior instead of stalling rotation.
pub struct PortalSyncedRecipientKeys {
    client: crate::directory::AudienceClient,
    dir: PathBuf,
    inner: PortalCertRecipientKeys,
}

impl PortalSyncedRecipientKeys {
    pub fn new(
        client: crate::directory::AudienceClient,
        dir: impl Into<PathBuf>,
        validator: PortalCertValidator,
    ) -> Self {
        let dir = dir.into();
        Self {
            client,
            dir: dir.clone(),
            inner: PortalCertRecipientKeys::new(dir, validator),
        }
    }

    /// Sync one member's PEM from the portal into `cert_dir` (atomic tmp+rename;
    /// write skipped when the content is unchanged). Errors are returned for the
    /// caller to decide fallback — a failed sync is not yet a failed resolve.
    fn sync(&self, member_id: &str) -> Result<(), ServiceError> {
        let path = crate::recipient::member_pem_path(&self.dir, member_id)?;
        let fetched = self.client.recipient_cert(member_id)?;
        if let Ok(existing) = std::fs::read_to_string(&path) {
            if existing == fetched {
                return Ok(());
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| crypto(format!("create {}: {e}", parent.display())))?;
        }
        let tmp = path.with_extension("pem.tmp");
        std::fs::write(&tmp, &fetched)
            .map_err(|e| crypto(format!("stage {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            crypto(format!("publish {}: {e}", path.display()))
        })?;
        Ok(())
    }
}

impl crate::recipient::RecipientKeys for PortalSyncedRecipientKeys {
    fn key(&self, member_id: &str) -> Result<PublicKey, ServiceError> {
        if let Err(e) = self.sync(member_id) {
            // Portal unreachable / cert not yet persisted there: fall back to
            // the cached (or hand-staged) file, still fully validated. Say so —
            // a quiet fallback would mask a dead sync path.
            eprintln!("grant-keyd: recipient-cert sync for {member_id:?} failed ({e:?}); using cached cert_dir copy if present");
        }
        self.inner.key(member_id)
    }
}

/// Parse every CERTIFICATE block in a PEM string, in order.
fn parse_pem_certs(pem: &str) -> Result<Vec<Certificate>, ServiceError> {
    let mut out = Vec::new();
    let mut rest = pem;
    while let Some(start) = rest.find("-----BEGIN CERTIFICATE-----") {
        let after = &rest[start..];
        let end = after
            .find("-----END CERTIFICATE-----")
            .ok_or_else(|| crypto("unterminated CERTIFICATE block"))?;
        let block = &after[..end + "-----END CERTIFICATE-----".len()];
        out.push(
            Certificate::from_pem(block.as_bytes())
                .map_err(|e| crypto(format!("parse cert: {e}")))?,
        );
        rest = &after[end + "-----END CERTIFICATE-----".len()..];
    }
    Ok(out)
}

fn der_of(cert: &Certificate) -> Result<Vec<u8>, ServiceError> {
    cert.to_der()
        .map_err(|e| crypto(format!("encode cert: {e}")))
}

/// Extract the subject CN (UTF8String or PrintableString — Go's `cert-sign`
/// emits PrintableString for ASCII anchors). Exactly one CN required.
/// The anchor a slot leaf CN names: the CN with a `.g<N>` generation suffix
/// (N a decimal without a leading zero) removed, or the CN unchanged when it
/// carries none (generation 0, or a non-slot CN, which then fails the binding
/// exactly as before).
fn cn_anchor(cn: &str) -> &str {
    match cn.rsplit_once(".g") {
        Some((head, gen))
            if !gen.is_empty()
                && !gen.starts_with('0')
                && gen.bytes().all(|b| b.is_ascii_digit()) =>
        {
            head
        }
        _ => cn,
    }
}

fn subject_cn(cert: &Certificate) -> Result<String, ServiceError> {
    let mut found: Option<String> = None;
    for rdn in cert.tbs_certificate.subject.0.iter() {
        for atv in rdn.0.iter() {
            if atv.oid != OID_CN {
                continue;
            }
            let s = atv
                .value
                .decode_as::<Utf8StringRef>()
                .map(|v| v.to_string())
                .or_else(|_| {
                    atv.value
                        .decode_as::<PrintableStringRef>()
                        .map(|v| v.to_string())
                })
                .map_err(|_| crypto("CN is not a UTF8/Printable string"))?;
            if found.is_some() {
                return Err(crypto("subject has multiple CNs"));
            }
            found = Some(s);
        }
    }
    found.ok_or_else(|| crypto("subject has no CN"))
}

/// Extract the single URI-SAN starting with `prefix`, refusing zero or multiple
/// (the portal mints exactly one identity URI-SAN per leaf).
fn single_prefixed_uri(cert: &Certificate, prefix: &str) -> Result<String, ServiceError> {
    let exts = cert
        .tbs_certificate
        .extensions
        .as_ref()
        .ok_or_else(|| crypto("leaf has no extensions (no SAN)"))?;
    let mut found: Option<String> = None;
    for ext in exts.iter() {
        if ext.extn_id != SubjectAltName::OID {
            continue;
        }
        let san = SubjectAltName::from_der(ext.extn_value.as_bytes())
            .map_err(|e| crypto(format!("parse SAN: {e}")))?;
        for gn in san.0.iter() {
            if let GeneralName::UniformResourceIdentifier(uri) = gn {
                let uri = uri.as_str();
                if uri.starts_with(prefix) {
                    if found.is_some() {
                        return Err(crypto("leaf has multiple identity URI-SANs"));
                    }
                    found = Some(uri.to_string());
                }
            }
        }
    }
    found.ok_or_else(|| crypto(format!("leaf has no URI-SAN with prefix {prefix:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipient::RecipientKeys;
    use rcgen::{
        BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
        IsCa, KeyPair, SanType,
    };

    const PREFIX: &str = "urn:goat:zagros:org=";

    struct Ca {
        cert: rcgen::Certificate,
        key: KeyPair,
    }
    /// The offline-CA root: Ed25519, like ca-init's dev/prod ceremonies.
    fn root() -> Ca {
        let key = KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
        let mut p = CertificateParams::new(vec![]).unwrap();
        p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let cert = p.self_signed(&key).unwrap();
        Ca { cert, key }
    }

    /// A portal-shaped leaf: CN = anchor, URI-SAN = PREFIX+anchor, DNS-SAN,
    /// clientAuth — the exact shape certmint mints (post-F-270: P-256 key).
    fn leaf_params(anchor: &str) -> CertificateParams {
        let mut p = CertificateParams::new(vec![]).unwrap();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, anchor);
        p.distinguished_name = dn;
        p.subject_alt_names = vec![
            SanType::DnsName(format!("client.{anchor}.zagros.peer").try_into().unwrap()),
            SanType::URI(format!("{PREFIX}{anchor}").try_into().unwrap()),
        ];
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        p
    }
    fn portal_leaf(ca: &Ca, anchor: &str) -> String {
        let key = KeyPair::generate().unwrap(); // ECDSA-P256, rcgen's default
        leaf_params(anchor)
            .signed_by(&key, &ca.cert, &ca.key)
            .unwrap()
            .pem()
    }

    fn validator(ca: &Ca) -> PortalCertValidator {
        PortalCertValidator::new(PREFIX, TrustBundle::from_pem(&ca.cert.pem()).unwrap())
    }

    #[test]
    fn a_valid_portal_leaf_validates_and_its_key_wraps() {
        let ca = root();
        let leaf = portal_leaf(&ca, "acme");
        let key = validator(&ca).validate(&leaf, "acme").expect("valid leaf");
        // The resolved key is a usable recipient through the crypto core.
        let dek = grant_crypto::new_dek();
        grant_crypto::wrap_dek(&dek, &key).expect("wraps");
    }

    #[test]
    fn a_leaf_from_a_different_root_is_refused() {
        let real = root();
        let attacker = root();
        let leaf = portal_leaf(&attacker, "acme");
        assert!(validator(&real).validate(&leaf, "acme").is_err());
    }

    #[test]
    fn a_cn_mismatch_is_refused() {
        // The file claims to be member "acme" but the cert's CN is someone else —
        // the anchor binding must reject it (no key confusion across members).
        let ca = root();
        let leaf = portal_leaf(&ca, "other-org");
        assert!(validator(&ca).validate(&leaf, "acme").is_err());
    }

    /// GHSA-rr3p: a slot leaf's CN carries its generation, `<anchor>.g<N>`;
    /// the binding is to the anchor before it, and the URI-SAN stays exact.
    fn generation_leaf(ca: &Ca, anchor: &str, cn: &str) -> String {
        let key = KeyPair::generate().unwrap();
        let mut p = leaf_params(anchor);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, cn);
        p.distinguished_name = dn;
        p.signed_by(&key, &ca.cert, &ca.key).unwrap().pem()
    }

    #[test]
    fn a_generation_leaf_binds_to_its_anchor() {
        let ca = root();
        let v = validator(&ca);
        assert!(v
            .validate(&generation_leaf(&ca, "acme", "acme.g3"), "acme")
            .is_ok());
        assert!(v
            .validate(&generation_leaf(&ca, "acme", "acme.g12"), "acme")
            .is_ok());
        for bad in [
            "acme.g0",
            "acme.g03",
            "acme.g",
            "acme.gx",
            "acme.g3.g4",
            "other.g3",
        ] {
            assert!(
                v.validate(&generation_leaf(&ca, "acme", bad), "acme")
                    .is_err(),
                "CN {bad} must not bind to acme"
            );
        }
    }

    #[test]
    fn a_uri_san_mismatch_is_refused() {
        // CN matches but the identity URI-SAN names a different anchor: forged
        // or mis-minted — refused.
        let ca = root();
        let key = KeyPair::generate().unwrap();
        let mut p = leaf_params("acme");
        p.subject_alt_names = vec![SanType::URI(
            format!("{PREFIX}other-org").try_into().unwrap(),
        )];
        let leaf = p.signed_by(&key, &ca.cert, &ca.key).unwrap().pem();
        assert!(validator(&ca).validate(&leaf, "acme").is_err());
    }

    #[test]
    fn a_pre_f270_ed25519_leaf_is_refused_with_a_rotation_pointer() {
        let ca = root();
        let key = KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
        let leaf = leaf_params("acme")
            .signed_by(&key, &ca.cert, &ca.key)
            .unwrap()
            .pem();
        let err = validator(&ca).validate(&leaf, "acme").unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("not P-256"), "got: {msg}");
        assert!(msg.contains("rotate"), "remedy named: {msg}");
    }

    #[test]
    fn a_chain_through_an_ecdsa_intermediate_validates() {
        // The F-090 shape: Ed25519 root → ECDSA-P256 intermediate → leaf. The
        // member file carries leaf + intermediate; the root is the anchor.
        let ca = root();
        let int_key = KeyPair::generate().unwrap();
        let mut int_p = CertificateParams::new(vec![]).unwrap();
        int_p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let intermediate = int_p.signed_by(&int_key, &ca.cert, &ca.key).unwrap();

        let leaf_key = KeyPair::generate().unwrap();
        let leaf = leaf_params("acme")
            .signed_by(&leaf_key, &intermediate, &int_key)
            .unwrap();

        let pem = format!("{}{}", leaf.pem(), intermediate.pem());
        validator(&ca)
            .validate(&pem, "acme")
            .expect("chain via intermediate");
    }

    /// Cross-system probe: a leaf minted by the REAL `cert-sign` against the REAL
    /// `ca-init` Ed25519 root must validate here (Go's DER — PrintableString CN,
    /// AKI/SKI, key-usage bits — not rcgen's). Env-gated because it needs those
    /// artifacts; produce them with ops/grant-crypto/conformance/portal-mint/run.sh
    /// mechanics, then:
    ///   PORTAL_CONFORMANCE_CERT=<leaf.pem> PORTAL_CONFORMANCE_ROOT=<root-cert.pem> \
    ///   PORTAL_CONFORMANCE_MEMBER=<anchor> PORTAL_CONFORMANCE_PREFIX=<uri-prefix> \
    ///   cargo test -p grant-keyd portal::a_real_cert_sign_leaf -- --ignored
    #[test]
    #[ignore = "needs PORTAL_CONFORMANCE_* env pointing at real cert-sign artifacts"]
    fn a_real_cert_sign_leaf_validates() {
        let cert = std::fs::read_to_string(std::env::var("PORTAL_CONFORMANCE_CERT").unwrap())
            .expect("read leaf");
        let root_pem = std::fs::read_to_string(std::env::var("PORTAL_CONFORMANCE_ROOT").unwrap())
            .expect("read root");
        let member = std::env::var("PORTAL_CONFORMANCE_MEMBER").unwrap();
        let prefix = std::env::var("PORTAL_CONFORMANCE_PREFIX")
            .unwrap_or_else(|_| "urn:goat:zagros:org=".into());
        let v = PortalCertValidator::new(prefix, TrustBundle::from_pem(&root_pem).unwrap());
        let key = v.validate(&cert, &member).expect("real cert-sign leaf");
        let dek = grant_crypto::new_dek();
        grant_crypto::wrap_dek(&dek, &key).expect("wraps");
    }

    // --- PortalSyncedRecipientKeys (U7 publish-my-key distribution) ---

    use crate::directory::AudienceClient;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// One-shot HTTP stub serving `body` with `status` (real ureq → real TCP).
    fn cert_stub(status: &'static str, body: String) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/x-pem-file\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes());
            let _ = sock.flush();
        });
        (format!("http://{addr}"), handle)
    }

    fn empty_dir() -> std::path::PathBuf {
        use p256::elliptic_curve::Generate;
        use p256::SecretKey;
        let seed = hex::encode(&SecretKey::generate().to_bytes()[..8]);
        let dir = std::env::temp_dir().join(format!("keyd-syncedkeys-{seed}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn synced_source_fetches_writes_and_validates_a_fresh_member() {
        // Empty cert_dir; the portal serves acme's leaf → resolve succeeds and
        // the PEM is cached on disk for the portal-down case.
        let ca = root();
        let leaf = portal_leaf(&ca, "acme");
        let (base, handle) = cert_stub("200 OK", leaf.clone());
        let dir = empty_dir();
        let keys =
            PortalSyncedRecipientKeys::new(AudienceClient::new(base, "tok"), &dir, validator(&ca));
        use crate::recipient::RecipientKeys;
        keys.key("acme")
            .expect("freshly-minted member resolves with zero hand-staging");
        assert_eq!(
            std::fs::read_to_string(dir.join("acme.pem")).expect("cached"),
            leaf,
            "fetched PEM cached in cert_dir"
        );
        handle.join().unwrap();
    }

    #[test]
    fn synced_source_falls_back_to_the_cached_file_when_the_portal_is_down() {
        let ca = root();
        let dir = empty_dir();
        std::fs::write(dir.join("acme.pem"), portal_leaf(&ca, "acme")).unwrap();
        // A closed port: connection refused on fetch.
        let keys = PortalSyncedRecipientKeys::new(
            AudienceClient::new("http://127.0.0.1:1", "tok"),
            &dir,
            validator(&ca),
        );
        use crate::recipient::RecipientKeys;
        keys.key("acme")
            .expect("cached (hand-staged) cert still resolves");
    }

    #[test]
    fn synced_source_fails_closed_when_portal_404s_and_no_cache_exists() {
        let ca = root();
        let (base, handle) = cert_stub("404 Not Found", "no cert".into());
        let keys = PortalSyncedRecipientKeys::new(
            AudienceClient::new(base, "tok"),
            empty_dir(),
            validator(&ca),
        );
        use crate::recipient::RecipientKeys;
        assert!(
            keys.key("acme").is_err(),
            "no portal cert + no cache = fail closed"
        );
        handle.join().unwrap();
    }

    #[test]
    fn a_portal_serving_the_wrong_cert_cannot_redirect_the_wrap() {
        // The portal (or an on-path attacker behind it) serves a cert for a
        // DIFFERENT member: the fetch succeeds, the identity binding refuses.
        let ca = root();
        let (base, handle) = cert_stub("200 OK", portal_leaf(&ca, "elsewhere"));
        let keys = PortalSyncedRecipientKeys::new(
            AudienceClient::new(base, "tok"),
            empty_dir(),
            validator(&ca),
        );
        use crate::recipient::RecipientKeys;
        assert!(
            keys.key("acme").is_err(),
            "identity binding must refuse a swapped cert"
        );
        handle.join().unwrap();
    }

    #[test]
    fn the_recipient_source_resolves_and_fails_closed() {
        let ca = root();
        let seed = {
            use p256::elliptic_curve::Generate;
            use p256::SecretKey;
            hex::encode(&SecretKey::generate().to_bytes()[..8])
        };
        let dir = std::env::temp_dir().join(format!("keyd-portalkeys-{seed}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("acme.pem"), portal_leaf(&ca, "acme")).unwrap();
        // A file whose cert is for a different member — must not resolve as acme2.
        std::fs::write(dir.join("acme2.pem"), portal_leaf(&ca, "elsewhere")).unwrap();

        let keys = PortalCertRecipientKeys::new(&dir, validator(&ca));
        keys.key("acme").expect("validated portal member");
        assert!(keys.key("acme2").is_err(), "identity mismatch fails closed");
        assert!(keys.key("nobody").is_err(), "missing fails closed");
        assert!(keys.key("../escape").is_err(), "traversal refused");
    }
}
