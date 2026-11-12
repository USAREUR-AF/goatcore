//! A stand-in Sigstore for the update tests: a Fulcio-shaped CA (root and
//! intermediate), a Rekor-shaped log key, and a signer that writes a cosign
//! bundle over SHA256SUMS for a workflow identity. Its trusted_root.json is
//! given to the binary under test through GOATBUILDER_UPDATE_TRUSTED_ROOT,
//! so a test can serve a release that verifies, or one that does not, with no
//! network and no cosign.

#![allow(dead_code)]

use base64::Engine as _;
use sha2::{Digest, Sha256};

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

/// A DER UTF8String, as Fulcio writes its 1.3.6.1.4.1.57264.1.8+ extensions.
fn utf8(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = vec![0x0c];
    if b.len() < 0x80 {
        out.push(b.len() as u8);
    } else {
        out.extend([0x82, (b.len() >> 8) as u8, b.len() as u8]);
    }
    out.extend_from_slice(b);
    out
}

fn oid(last: u64) -> Vec<u64> {
    vec![1, 3, 6, 1, 4, 1, 57264, 1, last]
}

fn sign(key: &rcgen::KeyPair, msg: &[u8]) -> Vec<u8> {
    let rng = ring::rand::SystemRandom::new();
    let k = ring::signature::EcdsaKeyPair::from_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &key.serialize_der(),
        &rng,
    )
    .unwrap();
    k.sign(&rng, msg).unwrap().as_ref().to_vec()
}

/// Who a stand-in release is signed as.
#[derive(Clone)]
pub struct Workflow {
    pub repo: String,
    pub workflow: String,
    pub tag: String,
}

impl Workflow {
    /// goatbuilder's release workflow in `repo` at `goatbuilder/v<version>`.
    pub fn release(repo: &str, version: &str) -> Self {
        Workflow {
            repo: repo.into(),
            workflow: "goatbuilder-release.yml".into(),
            tag: format!("goatbuilder/v{version}"),
        }
    }
}

pub struct Sigstore {
    root: rcgen::Certificate,
    root_key: rcgen::KeyPair,
    inter: rcgen::Certificate,
    inter_key: rcgen::KeyPair,
    log_key: rcgen::KeyPair,
    ct_key: rcgen::KeyPair,
}

/// How a stand-in leaf differs from a good one.
#[derive(Clone, Copy, Default)]
pub struct Leaf {
    /// The leaf's embedded SCT.
    pub sct: Sct,
    /// Carry an unknown extension marked critical.
    pub unknown_critical: bool,
    /// Be a CA (basicConstraints CA:TRUE).
    pub ca: bool,
    /// Expire before the entry is integrated.
    pub expired: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Sct {
    /// Signed by the trusted CT log over the precertificate.
    #[default]
    Valid,
    /// No SCT list extension.
    None,
    /// Signed by a CT log key the trusted root does not hold.
    UnknownLog,
    /// Signed by the trusted log over other bytes.
    OverOther,
}

fn ca(name: &str) -> rcgen::CertificateParams {
    let mut p = rcgen::CertificateParams::new(vec![]).unwrap();
    p.distinguished_name
        .push(rcgen::DnType::OrganizationName, "sigstore.example.org");
    p.distinguished_name.push(rcgen::DnType::CommonName, name);
    p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    p.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    p.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(1);
    p
}

impl Sigstore {
    pub fn new() -> Self {
        let root_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).unwrap();
        let root = ca("stand-in-root").self_signed(&root_key).unwrap();
        let inter_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).unwrap();
        let inter = ca("stand-in-intermediate")
            .signed_by(&inter_key, &root, &root_key)
            .unwrap();
        let log_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let ct_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        Sigstore {
            root,
            root_key,
            inter,
            inter_key,
            log_key,
            ct_key,
        }
    }

    /// The trusted_root.json naming this CA and log key.
    pub fn trusted_root(&self) -> String {
        let spki = self.log_key.public_key_der();
        serde_json::json!({
            "mediaType": "application/vnd.dev.sigstore.trustedroot+json;version=0.1",
            "tlogs": [{
                "baseUrl": "https://rekor.example.org",
                "hashAlgorithm": "SHA2_256",
                "publicKey": {
                    "rawBytes": b64(&spki),
                    "keyDetails": "PKIX_ECDSA_P256_SHA_256",
                    "validFor": {"start": "2020-01-01T00:00:00Z"}
                },
                "logId": {"keyId": b64(&Sha256::digest(&spki))}
            }],
            "ctlogs": [{
                "baseUrl": "https://ctfe.example.org",
                "hashAlgorithm": "SHA2_256",
                "publicKey": {
                    "rawBytes": b64(&self.ct_key.public_key_der()),
                    "keyDetails": "PKIX_ECDSA_P256_SHA_256",
                    "validFor": {"start": "2020-01-01T00:00:00Z"}
                },
                "logId": {"keyId": b64(&Sha256::digest(self.ct_key.public_key_der()))}
            }],
            "certificateAuthorities": [{
                "subject": {"organization": "sigstore.example.org", "commonName": "stand-in"},
                "uri": "https://fulcio.example.org",
                "certChain": {"certificates": [
                    {"rawBytes": b64(self.inter.der())},
                    {"rawBytes": b64(self.root.der())}
                ]},
                "validFor": {"start": "2020-01-01T00:00:00Z"}
            }]
        })
        .to_string()
    }

    /// A cosign bundle over `sums`, signed as `w`.
    pub fn bundle(&self, sums: &[u8], w: &Workflow) -> String {
        self.bundle_with(sums, w, Leaf::default())
    }

    /// The leaf's parameters, without its SCT list.
    fn leaf_params(&self, w: &Workflow, leaf: Leaf) -> rcgen::CertificateParams {
        let mut p = rcgen::CertificateParams::new(vec![]).unwrap();
        let reusable = format!(
            "https://github.com/{}/.github/workflows/_release-binary.yml@refs/tags/{}",
            w.repo, w.tag
        );
        p.subject_alt_names = vec![rcgen::SanType::URI(
            rcgen::Ia5String::try_from(reusable).unwrap(),
        )];
        p.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::CodeSigning];
        p.serial_number = Some(rcgen::SerialNumber::from(vec![0x42, 0x17]));
        let now = time::OffsetDateTime::now_utc()
            .replace_nanosecond(0)
            .unwrap();
        if leaf.expired {
            p.not_before = now - time::Duration::minutes(30);
            p.not_after = now - time::Duration::minutes(20);
        } else {
            p.not_before = now - time::Duration::minutes(5);
            p.not_after = now + time::Duration::minutes(5);
        }
        if leaf.ca {
            p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        }
        for (n, v) in [
            (8, "https://token.actions.githubusercontent.com".to_string()),
            (12, format!("https://github.com/{}", w.repo)),
            (14, format!("refs/tags/{}", w.tag)),
            (
                18,
                format!(
                    "https://github.com/{}/.github/workflows/{}@refs/tags/{}",
                    w.repo, w.workflow, w.tag
                ),
            ),
        ] {
            p.custom_extensions
                .push(rcgen::CustomExtension::from_oid_content(&oid(n), utf8(&v)));
        }
        if leaf.unknown_critical {
            let mut e = rcgen::CustomExtension::from_oid_content(
                &[1, 3, 6, 1, 4, 1, 99999, 1],
                vec![0x05, 0x00],
            );
            e.set_criticality(true);
            p.custom_extensions.push(e);
        }
        p
    }

    /// A cosign bundle whose leaf is `leaf`.
    pub fn bundle_with(&self, sums: &[u8], w: &Workflow, leaf: Leaf) -> String {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        // RFC 6962 section 3.2: the log signs the precertificate's TBS, which
        // is the final certificate's TBS without the SCT list. The precert is
        // built from the same parameters, so the two TBS differ only by it.
        let pre = self
            .leaf_params(w, leaf)
            .signed_by(&key, &self.inter, &self.inter_key)
            .unwrap();
        let mut p = self.leaf_params(w, leaf);
        if leaf.sct != Sct::None {
            let (_, parsed) = x509_parser::parse_x509_certificate(pre.der()).unwrap();
            let tbs = parsed.tbs_certificate.as_ref().to_vec();
            let other = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
            let log = if leaf.sct == Sct::UnknownLog {
                &other
            } else {
                &self.ct_key
            };
            let signed_tbs = if leaf.sct == Sct::OverOther {
                b"other".to_vec()
            } else {
                tbs
            };
            let list = sct_list(log, self.inter.der(), &signed_tbs);
            let mut content = vec![0x04];
            content.extend(der_len(list.len()));
            content.extend(list);
            p.custom_extensions
                .push(rcgen::CustomExtension::from_oid_content(
                    &[1, 3, 6, 1, 4, 1, 11129, 2, 4, 2],
                    content,
                ));
        }
        let leaf = p.signed_by(&key, &self.inter, &self.inter_key).unwrap();
        let pem = leaf.pem();
        let sig = sign(&key, sums);
        let body = serde_json::json!({
            "apiVersion": "0.0.1",
            "kind": "hashedrekord",
            "spec": {
                "data": {"hash": {"algorithm": "sha256", "value": format!("{:x}", Sha256::digest(sums))}},
                "signature": {"content": b64(&sig), "publicKey": {"content": b64(pem.as_bytes())}}
            }
        })
        .to_string();
        let body = b64(body.as_bytes());
        let time = time::OffsetDateTime::now_utc().unix_timestamp();
        let log_id = format!("{:x}", Sha256::digest(self.log_key.public_key_der()));
        let canon = format!(
            "{{\"body\":\"{body}\",\"integratedTime\":{time},\"logID\":\"{log_id}\",\"logIndex\":7}}"
        );
        let set = sign(&self.log_key, canon.as_bytes());
        serde_json::json!({
            "base64Signature": b64(&sig),
            "cert": b64(pem.as_bytes()),
            "rekorBundle": {
                "SignedEntryTimestamp": b64(&set),
                "Payload": {"body": body, "integratedTime": time, "logIndex": 7, "logID": log_id}
            }
        })
        .to_string()
    }
}

fn der_len(n: usize) -> Vec<u8> {
    if n < 0x80 {
        vec![n as u8]
    } else if n < 0x100 {
        vec![0x81, n as u8]
    } else {
        vec![0x82, (n >> 8) as u8, n as u8]
    }
}

/// A TLS-encoded SignedCertificateTimestampList holding one v1 SCT by `log`
/// over the precert entry for `tbs`, issued by `issuer` (RFC 6962 3.2).
fn sct_list(log: &rcgen::KeyPair, issuer: &[u8], tbs: &[u8]) -> Vec<u8> {
    let (_, iss) = x509_parser::parse_x509_certificate(issuer).unwrap();
    let issuer_key_hash = Sha256::digest(iss.public_key().raw);
    let ts: u64 = (time::OffsetDateTime::now_utc().unix_timestamp() as u64) * 1000;
    let mut signed = vec![0u8, 0u8];
    signed.extend(ts.to_be_bytes());
    signed.extend([0u8, 1u8]);
    signed.extend(issuer_key_hash);
    signed.extend(&(tbs.len() as u32).to_be_bytes()[1..]);
    signed.extend(tbs);
    signed.extend([0u8, 0u8]);
    let sig = sign(log, &signed);
    let mut sct = vec![0u8];
    sct.extend(Sha256::digest(log.public_key_der()));
    sct.extend(ts.to_be_bytes());
    sct.extend([0u8, 0u8]);
    sct.extend([4u8, 3u8]);
    sct.extend((sig.len() as u16).to_be_bytes());
    sct.extend(sig);
    let mut list = (sct.len() as u16 + 2).to_be_bytes().to_vec();
    list.extend((sct.len() as u16).to_be_bytes());
    list.extend(sct);
    list
}
