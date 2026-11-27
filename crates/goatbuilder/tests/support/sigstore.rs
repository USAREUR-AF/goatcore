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
    if key.is_compatible(&rcgen::PKCS_ED25519) {
        let k = ring::signature::Ed25519KeyPair::from_pkcs8_maybe_unchecked(&key.serialize_der()).unwrap();
        return k.sign(msg).as_ref().to_vec();
    }
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
    /// Carry no extended key usage.
    pub no_code_signing: bool,
    /// The OIDC issuer, when not GitHub's.
    pub issuer: Option<&'static str>,
    /// The log entry names another certificate than the bundle's.
    pub entry_of_another: bool,
    /// Let this long pass between the precertificate and the certificate,
    /// as a slow machine does (a regression test for the two reading the
    /// clock apart).
    pub pause: std::time::Duration,
    /// Name this repository's reusable workflow in the SAN instead of the
    /// calling workflow's own repository (a workflow of one repository called
    /// from another, as GitHub's certificates record it).
    pub san_repo: Option<&'static str>,
}

/// The log entry of a stand-in attestation: the log's own, or wrong in one way.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum DsseBody {
    #[default]
    Good,
    /// Of another kind than dsse.
    Kind,
    /// Naming two signatures.
    TwoSignatures,
    /// Naming another signature than the envelope's.
    OtherSignature,
    /// Naming another certificate than the bundle's.
    OtherVerifier,
}

/// The checkpoint of a Sigstore bundle v0.3: the log's own, or one that is
/// wrong in one way and otherwise well made.
#[derive(Clone, Copy, Default)]
pub enum Checkpoint {
    #[default]
    Good,
    /// Signed by another key, under the log's name and key hint.
    OtherKey,
    /// Signed by the log, for another root.
    OtherRoot,
    /// Signed by the log, for another size.
    OtherSize,
    /// Signed by the log, under this origin.
    Origin(&'static str),
    /// Signed by the log, under another signer's name.
    OtherName,
    /// Signed by the log, with another key's hint.
    OtherHint,
    /// The log's own, with another witness's line before it.
    Witnessed,
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

    /// One whose Rekor log has an Ed25519 key.
    pub fn with_ed25519_log() -> Self {
        let mut s = Self::new();
        s.log_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
        s
    }

    /// The trusted_root.json naming this CA and log key.
    pub fn trusted_root(&self) -> String {
        let spki = self.log_key.public_key_der();
        let details = if self.log_key.is_compatible(&rcgen::PKCS_ED25519) {
            "PKIX_ED25519"
        } else {
            "PKIX_ECDSA_P256_SHA_256"
        };
        serde_json::json!({
            "mediaType": "application/vnd.dev.sigstore.trustedroot+json;version=0.1",
            "tlogs": [{
                "baseUrl": "https://rekor.example.org",
                "hashAlgorithm": "SHA2_256",
                "publicKey": {
                    "rawBytes": b64(&spki),
                    "keyDetails": details,
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
    /// `now` is read once per bundle: the precertificate and the certificate
    /// must carry the same validity, or the SCT over the one does not match
    /// the other's TBS.
    fn leaf_params(
        &self,
        w: &Workflow,
        leaf: Leaf,
        now: time::OffsetDateTime,
    ) -> rcgen::CertificateParams {
        let mut p = rcgen::CertificateParams::new(vec![]).unwrap();
        let reusable = format!(
            "https://github.com/{}/.github/workflows/_release-binary.yml@refs/tags/{}",
            leaf.san_repo.unwrap_or(&w.repo),
            w.tag
        );
        p.subject_alt_names = vec![rcgen::SanType::URI(
            rcgen::Ia5String::try_from(reusable).unwrap(),
        )];
        if !leaf.no_code_signing {
            p.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::CodeSigning];
        }
        p.serial_number = Some(rcgen::SerialNumber::from(vec![0x42, 0x17]));
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
            (8, leaf.issuer.unwrap_or("https://token.actions.githubusercontent.com").to_string()),
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

    /// A Sigstore bundle v0.3 over `sums`, signed as `w`.
    pub fn bundle_v03(&self, sums: &[u8], w: &Workflow) -> String {
        self.bundle_v03_with(sums, w, Leaf::default(), Checkpoint::Good)
    }

    /// [`Self::bundle_v03`] whose leaf is `leaf`: [`Self::bundle_with`]'s parts
    /// in the newer shape, the entry the fourth of a log of seven.
    pub fn bundle_v03_with(&self, sums: &[u8], w: &Workflow, leaf: Leaf, cp: Checkpoint) -> String {
        self.v03(sums, w, leaf, cp, None, DsseBody::Good)
    }

    /// A GitHub artifact attestation, in its shape: SLSA provenance v1 of
    /// `files` (name, bytes), built by `w` at its tag, in a DSSE envelope.
    pub fn attestation_v03(&self, files: &[(&str, &[u8])], w: &Workflow) -> String {
        self.attestation_v03_with(files, w, DsseBody::Good)
    }

    /// [`Self::attestation_v03`] whose log entry is `body`.
    pub fn attestation_v03_with(&self, files: &[(&str, &[u8])], w: &Workflow, body: DsseBody) -> String {
        let statement = serde_json::json!({
            "_type": "https://in-toto.io/Statement/v1",
            "subject": files.iter().map(|(n, b)| serde_json::json!({"name": n, "digest": {"sha256": format!("{:x}", Sha256::digest(b))}})).collect::<Vec<_>>(),
            "predicateType": "https://slsa.dev/provenance/v1",
            "predicate": {"buildDefinition": {"buildType": "https://actions.github.io/buildtypes/workflow/v1",
                "externalParameters": {"workflow": {"ref": format!("refs/tags/{}", w.tag), "repository": format!("https://github.com/{}", w.repo), "path": format!(".github/workflows/{}", w.workflow)}}},
                "runDetails": {"builder": {"id": "stand-in"}}}
        })
        .to_string();
        self.v03(b"", w, Leaf::default(), Checkpoint::Good, Some(statement.as_bytes()), body)
    }

    fn v03(&self, sums: &[u8], w: &Workflow, leaf: Leaf, cp: Checkpoint, dsse: Option<&[u8]>, body: DsseBody) -> String {
        let un64 = |s: &str| {
            base64::engine::general_purpose::STANDARD
                .decode(s)
                .unwrap()
        };
        let old: serde_json::Value =
            serde_json::from_str(&self.signed_with(sums, w, leaf, dsse, body)).unwrap();
        let p = &old["rekorBundle"]["Payload"];
        let pem = un64(old["cert"].as_str().unwrap());
        let (_, pem) = x509_parser::pem::parse_x509_pem(&pem).unwrap();
        let leaf_hash = |b: &[u8]| -> Vec<u8> {
            Sha256::new().chain_update([0u8]).chain_update(b).finalize().to_vec()
        };
        let pair = |l: &[u8], r: &[u8]| -> Vec<u8> {
            Sha256::new()
                .chain_update([1u8])
                .chain_update(l)
                .chain_update(r)
                .finalize()
                .to_vec()
        };
        let body = un64(p["body"].as_str().unwrap());
        let h: Vec<Vec<u8>> = (0..7u8)
            .map(|i| if i == 3 { leaf_hash(&body) } else { leaf_hash(&[i]) })
            .collect();
        let (n01, n23, n45) = (pair(&h[0], &h[1]), pair(&h[2], &h[3]), pair(&h[4], &h[5]));
        let (left, right) = (pair(&n01, &n23), pair(&n45, &h[6]));
        let root = pair(&left, &right);
        let other = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let origin = if let Checkpoint::Origin(o) = cp { o } else { "rekor.example.org - 1234" };
        let size = if matches!(cp, Checkpoint::OtherSize) { 8 } else { 7 };
        let said = if matches!(cp, Checkpoint::OtherRoot) { &left } else { &root };
        let text = format!("{origin}\n{size}\n{}\n", b64(said));
        let hint = if matches!(cp, Checkpoint::OtherHint) { &other } else { &self.log_key };
        let mut note = un64(p_id(hint).as_str())[..4].to_vec();
        note.extend(sign(if matches!(cp, Checkpoint::OtherKey) { &other } else { &self.log_key }, text.as_bytes()));
        let name = if matches!(cp, Checkpoint::OtherName) { "witness.example.org" } else { "rekor.example.org" };
        let mut lines = format!("\u{2014} {name} {}\n", b64(&note));
        if matches!(cp, Checkpoint::Witnessed) {
            lines = format!("\u{2014} witness.example.org {}\n{lines}", b64(&sign(&other, b"x")));
        }
        let mut b = serde_json::json!({
            "mediaType": "application/vnd.dev.sigstore.bundle.v0.3+json",
            "verificationMaterial": {
                "certificate": {"rawBytes": b64(&pem.contents)},
                "tlogEntries": [{
                    "logIndex": "7",
                    "logId": {"keyId": p_id(&self.log_key)},
                    "kindVersion": {"kind": if dsse.is_some() { "dsse" } else { "hashedrekord" }, "version": "0.0.1"},
                    "integratedTime": p["integratedTime"].to_string(),
                    "inclusionPromise": {"signedEntryTimestamp": old["rekorBundle"]["SignedEntryTimestamp"]},
                    "inclusionProof": {
                        "logIndex": "3",
                        "rootHash": b64(&root),
                        "treeSize": "7",
                        "hashes": [b64(&h[2]), b64(&n01), b64(&right)],
                        "checkpoint": {"envelope": format!("{text}\n{lines}")}
                    },
                    "canonicalizedBody": p["body"]
                }],
                "timestampVerificationData": {"rfc3161Timestamps": []}
            },
            "messageSignature": {
                "messageDigest": {"algorithm": "SHA2_256", "digest": b64(&Sha256::digest(sums))},
                "signature": old["base64Signature"]
            }
        });
        if let Some(payload) = dsse {
            b.as_object_mut().unwrap().remove("messageSignature");
            b["dsseEnvelope"] = serde_json::json!({
                "payload": b64(payload),
                "payloadType": "application/vnd.in-toto+json",
                "signatures": [{"sig": old["base64Signature"]}]
            });
        }
        b.to_string()
    }

    /// A cosign bundle whose leaf is `leaf`.
    pub fn bundle_with(&self, sums: &[u8], w: &Workflow, leaf: Leaf) -> String {
        self.signed_with(sums, w, leaf, None, DsseBody::Good)
    }

    /// [`Self::bundle_with`]'s parts; with `dsse` (a payload of in-toto's type),
    /// the signature is over DSSE's encoding of it and the entry is a dsse one.
    fn signed_with(&self, sums: &[u8], w: &Workflow, leaf: Leaf, dsse: Option<&[u8]>, kind: DsseBody) -> String {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        // RFC 6962 section 3.2: the log signs the precertificate's TBS, which
        // is the final certificate's TBS without the SCT list. The precert is
        // built from the same parameters, so the two TBS differ only by it.
        let now = time::OffsetDateTime::now_utc()
            .replace_nanosecond(0)
            .unwrap();
        let pre = self
            .leaf_params(w, leaf, now)
            .signed_by(&key, &self.inter, &self.inter_key)
            .unwrap();
        std::thread::sleep(leaf.pause);
        let mut p = self.leaf_params(w, leaf, now);
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
        let other = self.leaf_params(w, Leaf::default(), now).signed_by(&key, &self.inter, &self.inter_key).unwrap();
        let entry_of_another = leaf.entry_of_another;
        let leaf = p.signed_by(&key, &self.inter, &self.inter_key).unwrap();
        let pem = leaf.pem();
        let entry_pem = if entry_of_another { other.pem() } else { leaf.pem() };
        let (sig, body) = match dsse {
            None => {
                let sig = sign(&key, sums);
                (sig.clone(), serde_json::json!({
                    "apiVersion": "0.0.1",
                    "kind": "hashedrekord",
                    "spec": {
                        "data": {"hash": {"algorithm": "sha256", "value": format!("{:x}", Sha256::digest(sums))}},
                        "signature": {"content": b64(&sig), "publicKey": {"content": b64(entry_pem.as_bytes())}}
                    }
                }))
            }
            Some(payload) => {
                let t = "application/vnd.in-toto+json";
                let mut pae = format!("DSSEv1 {} {t} {} ", t.len(), payload.len()).into_bytes();
                pae.extend_from_slice(payload);
                let sig = sign(&key, &pae);
                let named = if kind == DsseBody::OtherSignature { sign(&key, b"other") } else { sig.clone() };
                let verifier = if kind == DsseBody::OtherVerifier { other.pem() } else { entry_pem.clone() };
                let one = serde_json::json!({"signature": b64(&named), "verifier": b64(verifier.as_bytes())});
                let sigs = if kind == DsseBody::TwoSignatures { vec![one.clone(), one] } else { vec![one] };
                (sig.clone(), serde_json::json!({
                    "apiVersion": "0.0.1",
                    "kind": if kind == DsseBody::Kind { "intoto" } else { "dsse" },
                    "spec": {
                        "envelopeHash": {"algorithm": "sha256", "value": "0".repeat(64)},
                        "payloadHash": {"algorithm": "sha256", "value": format!("{:x}", Sha256::digest(payload))},
                        "signatures": sigs
                    }
                }))
            }
        };
        let body = body.to_string();
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

/// A log key's id, base64.
fn p_id(k: &rcgen::KeyPair) -> String {
    b64(&Sha256::digest(k.public_key_der()))
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
