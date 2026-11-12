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
        Sigstore {
            root,
            root_key,
            inter,
            inter_key,
            log_key,
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
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut p = rcgen::CertificateParams::new(vec![]).unwrap();
        let reusable = format!(
            "https://github.com/{}/.github/workflows/_release-binary.yml@refs/tags/{}",
            w.repo, w.tag
        );
        p.subject_alt_names = vec![rcgen::SanType::URI(
            rcgen::Ia5String::try_from(reusable).unwrap(),
        )];
        p.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::CodeSigning];
        p.not_before = time::OffsetDateTime::now_utc() - time::Duration::minutes(5);
        p.not_after = time::OffsetDateTime::now_utc() + time::Duration::minutes(5);
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
