//! Verifying the keyless signature over a release's SHA256SUMS inside
//! goatbuilder, so `goatbuilder update` needs no `cosign` on PATH.
//!
//! The release workflow (.github/workflows/_release-binary.yml, called by
//! goatbuilder-release.yml) runs `cosign sign-blob --bundle
//! SHA256SUMS.cosign-bundle SHA256SUMS`. That writes cosign's bundle:
//! `base64Signature`, `cert` (the Fulcio leaf, base64 of its PEM) and
//! `rekorBundle` (Rekor's SignedEntryTimestamp over the entry's payload).
//! [`verify`] checks it offline, against a Sigstore trusted root: the public
//! instance's, embedded here, plus any the operator names.
//!
//! The checks, and where each rule comes from (cosign v2 `verify-blob`,
//! pkg/cosign/verify.go and pkg/cosign/tlog.go):
//!  1. the leaf chains to a Fulcio CA of the trusted root, each certificate
//!     valid at the entry's integratedTime (cosign `ValidateAndUnpackCert`,
//!     `CheckExpiry`);
//!  2. the leaf carries the code-signing extended key usage (Fulcio issues
//!     only that), is not a CA, and marks no extension critical that this
//!     verifier does not handle (as Go's x509.Verify, which cosign uses);
//!  3. the signature over SHA256SUMS verifies with the leaf's key
//!     (`VerifyBlobSignature`);
//!  4. Rekor's SignedEntryTimestamp verifies, under the log key whose id is
//!     the payload's logID, over the RFC 8785 canonical JSON of exactly
//!     {body, integratedTime, logID, logIndex} (`VerifySET`);
//!  5. the entry's body is a hashedrekord naming this file's SHA-256, this
//!     signature and this certificate (`VerifyBundle` compares the entry with
//!     the artifact);
//!  6. the leaf's identity is the release workflow's (Fulcio's OID
//!     1.3.6.1.4.1.57264.1.* extensions, docs/oid-info.md in sigstore/fulcio):
//!     the GitHub Actions issuer, the source repository, the tag
//!     goatbuilder/v<version> as the source ref, and goatbuilder-release.yml
//!     at that tag as the Build Config URI. The certificate's SAN names the
//!     reusable workflow both goat and goatbuilder releases run, so it cannot
//!     tell them apart; the Build Config URI is the workflow that was
//!     triggered;
//!  7. the leaf carries an SCT from a Certificate Transparency log of the
//!     trusted root, valid at the SCT's time, over its precertificate (RFC
//!     6962 sections 3.2 and 3.3; cosign v2 verify-blob checks it by default,
//!     pkg/cosign/verify.go VerifyEmbeddedSCT).
//!
//! ECDSA is P-256 with SHA-256 or P-384 with SHA-384; any other key or curve
//! fails closed. Ed25519 log keys are read, but a legacy bundle's SET is
//! always from an ECDSA Rekor log, so that path is not reached by a release.

use std::path::Path;

use base64::Engine as _;
use sha2::{Digest, Sha256};
use x509_parser::prelude::*;

/// The public Sigstore instance's trusted root (sigstore/root-signing,
/// targets/trusted_root.json): Fulcio's CAs and Rekor's log keys.
const EMBEDDED_ROOT: &str = include_str!("sigstore-trusted-root.json");

/// The environment variable naming an operator-supplied trusted_root.json,
/// whose keys are trusted beside the embedded ones (an islanded site takes a
/// Sigstore key rotation without a new goatbuilder).
pub const TRUSTED_ROOT_ENV: &str = "GOATBUILDER_UPDATE_TRUSTED_ROOT";

/// GitHub Actions' OIDC issuer, the only issuer a release is signed under.
const GITHUB_ISSUER: &str = "https://token.actions.githubusercontent.com";

/// The workflow a goatbuilder release is triggered by, at the release's tag.
pub const RELEASE_WORKFLOW: &str = "goatbuilder-release.yml";
/// The tag prefix of a goatbuilder release.
pub const TAG_PREFIX: &str = "goatbuilder/v";

/// Whose signature is accepted.
#[derive(Debug, Clone)]
pub struct Expect<'a> {
    /// `<owner>/<repo>` the release was built in (update's release signer).
    pub signer: &'a str,
    /// The release's version, as version.json serves it (`0.5.1`).
    pub version: &'a str,
    /// The workflow file that was triggered ([`RELEASE_WORKFLOW`]).
    pub workflow: &'a str,
}

/// What was verified, for the update's report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub identity: String,
    pub log_index: i64,
    pub integrated_time: i64,
}

/// Which check refused, and why. `check` is one of the names in the module
/// documentation's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub check: &'static str,
    pub detail: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.check, self.detail)
    }
}

fn refuse(check: &'static str, detail: impl Into<String>) -> Refusal {
    Refusal {
        check,
        detail: detail.into(),
    }
}

/// One Rekor log key: its id (SHA-256 of the DER public key), the key, and
/// when it was valid.
#[derive(Debug, Clone)]
struct LogKey {
    id: Vec<u8>,
    spki: Vec<u8>,
    ed25519: bool,
    start: i64,
    end: Option<i64>,
}

/// One Fulcio CA: its chain (the issuing certificate first, the root last)
/// and when it was valid.
#[derive(Debug, Clone)]
struct Authority {
    chain: Vec<Vec<u8>>,
    start: i64,
    end: Option<i64>,
}

/// The trusted root: the CAs and log keys a bundle may chain to.
#[derive(Debug, Clone, Default)]
pub struct TrustRoot {
    logs: Vec<LogKey>,
    /// Certificate Transparency logs, whose SCTs a Fulcio leaf embeds.
    ctlogs: Vec<LogKey>,
    authorities: Vec<Authority>,
    /// Where each part came from, for a refusal's message.
    sources: Vec<String>,
}

fn rfc3339(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp())
}

impl TrustRoot {
    /// Parse a Sigstore trusted_root.json (media type
    /// application/vnd.dev.sigstore.trustedroot+json;version=0.1).
    pub fn parse(text: &str, source: &str) -> Result<Self, String> {
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("{source} is not JSON: {e}"))?;
        let b64 = |x: &serde_json::Value| -> Result<Vec<u8>, String> {
            base64::engine::general_purpose::STANDARD
                .decode(x.as_str().unwrap_or(""))
                .map_err(|e| format!("{source}: {e}"))
        };
        let window = |w: &serde_json::Value| -> Result<(i64, Option<i64>), String> {
            let start = w["start"]
                .as_str()
                .and_then(rfc3339)
                .ok_or_else(|| format!("{source}: a validFor without a start"))?;
            // An `end` that is there must parse: a window read as endless
            // would trust a retired key.
            let end = match w.get("end") {
                None | Some(serde_json::Value::Null) => None,
                Some(e) => Some(e.as_str().and_then(rfc3339).ok_or_else(|| {
                    format!("{source}: a validFor end that is not an RFC 3339 time: {e}")
                })?),
            };
            Ok((start, end))
        };
        let mut root = TrustRoot {
            sources: vec![source.to_string()],
            ..Default::default()
        };
        for (list, ct) in [("tlogs", false), ("ctlogs", true)] {
            for t in v[list].as_array().into_iter().flatten() {
                let details = t["publicKey"]["keyDetails"].as_str().unwrap_or("");
                let ed25519 = match details {
                    "PKIX_ECDSA_P256_SHA_256" => false,
                    "PKIX_ED25519" => true,
                    _ => continue,
                };
                let (start, end) = window(&t["publicKey"]["validFor"])?;
                let key = LogKey {
                    id: b64(&t["logId"]["keyId"])?,
                    spki: b64(&t["publicKey"]["rawBytes"])?,
                    ed25519,
                    start,
                    end,
                };
                if ct {
                    root.ctlogs.push(key);
                } else {
                    root.logs.push(key);
                }
            }
        }
        for c in v["certificateAuthorities"].as_array().into_iter().flatten() {
            let chain = c["certChain"]["certificates"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|x| b64(&x["rawBytes"]))
                .collect::<Result<Vec<_>, _>>()?;
            if chain.is_empty() {
                continue;
            }
            let (start, end) = window(&c["validFor"])?;
            root.authorities.push(Authority { chain, start, end });
        }
        if root.logs.is_empty() || root.authorities.is_empty() {
            return Err(format!(
                "{source} names no Rekor log key or no Fulcio certificate authority"
            ));
        }
        Ok(root)
    }

    /// The embedded public Sigstore root, plus the operator's file when
    /// `GOATBUILDER_UPDATE_TRUSTED_ROOT` names one.
    pub fn resolve() -> Result<Self, String> {
        let mut root = Self::parse(EMBEDDED_ROOT, "the embedded Sigstore trusted root")?;
        if let Some(p) = std::env::var_os(TRUSTED_ROOT_ENV).filter(|p| !p.is_empty()) {
            let path = Path::new(&p);
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("{TRUSTED_ROOT_ENV}={}: {e}", path.display()))?;
            root.add(Self::parse(
                &text,
                &format!("{TRUSTED_ROOT_ENV}={}", path.display()),
            )?);
        }
        Ok(root)
    }

    fn add(&mut self, other: TrustRoot) {
        self.logs.extend(other.logs);
        self.ctlogs.extend(other.ctlogs);
        self.authorities.extend(other.authorities);
        self.sources.extend(other.sources);
    }

    fn named(&self) -> String {
        self.sources.join(" and ")
    }
}

/// Verify `bundle` (SHA256SUMS.cosign-bundle) over `sums` (SHA256SUMS) for
/// `expect`, against `root`. Nothing is fetched: everything checked is in the
/// bundle and the trusted root.
pub fn verify(
    sums: &[u8],
    bundle: &str,
    expect: &Expect,
    root: &TrustRoot,
) -> Result<Verified, Refusal> {
    let b: serde_json::Value = serde_json::from_str(bundle).map_err(|e| {
        refuse(
            "bundle",
            format!("SHA256SUMS.cosign-bundle is not JSON: {e}"),
        )
    })?;
    let field = |v: &serde_json::Value, name: &str| -> Result<String, Refusal> {
        v.get(name)
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .ok_or_else(|| refuse("bundle", format!("SHA256SUMS.cosign-bundle has no {name}")))
    };
    let sig = b64(&field(&b, "base64Signature")?, "base64Signature")?;
    let cert_pem = b64(&field(&b, "cert")?, "cert")?;
    let leaf = pem_to_der(&cert_pem)
        .ok_or_else(|| refuse("bundle", "the bundle's cert is not a PEM certificate"))?;
    let rekor = b
        .get("rekorBundle")
        .ok_or_else(|| refuse("bundle", "SHA256SUMS.cosign-bundle has no rekorBundle"))?;
    let set = b64(
        &field(rekor, "SignedEntryTimestamp")?,
        "SignedEntryTimestamp",
    )?;
    let payload = rekor
        .get("Payload")
        .ok_or_else(|| refuse("bundle", "the rekorBundle has no Payload"))?;

    // 4. Rekor's promise: the SET over the canonical payload, under a log key
    //    of the trusted root valid at the entry's time.
    let canon = set_payload(payload)?;
    let time = payload["integratedTime"].as_i64().unwrap_or_default();
    let index = payload["logIndex"].as_i64().unwrap_or_default();
    let log_id = payload["logID"].as_str().unwrap_or_default();
    check_set(&canon, &set, log_id, time, root)?;

    // 5. The entry is for this file, this signature and this certificate.
    let body_text = b64(payload["body"].as_str().unwrap_or_default(), "Payload.body")?;
    let body: serde_json::Value = serde_json::from_slice(&body_text)
        .map_err(|e| refuse("entry", format!("the Rekor entry's body is not JSON: {e}")))?;
    let digest = Sha256::digest(sums);
    check_entry(&body, &digest, &sig, &leaf)?;

    // 1. and 2. The certificate chain at the entry's time, and its usage.
    let issuer = chain_to(&leaf, time, root)?;
    let (_, cert) = X509Certificate::from_der(&leaf).map_err(|e| {
        refuse(
            "chain",
            format!("the signing certificate does not parse: {e}"),
        )
    })?;
    check_leaf(&cert)?;
    // 7. The leaf was logged: an embedded SCT from a trusted CT log verifies
    //    over its precertificate (cosign v2 verify-blob checks this by
    //    default: pkg/cosign/verify.go, VerifyEmbeddedSCT).
    check_sct(&cert, &issuer, root)?;
    let code_signing = matches!(cert.extended_key_usage(), Ok(Some(eku)) if eku.value.code_signing);
    if !code_signing {
        return Err(refuse(
            "chain",
            "the signing certificate is not for code signing (no extended key usage codeSigning)",
        ));
    }

    // 3. The signature over SHA256SUMS, by the certificate's key.
    if !verify_raw(cert.public_key(), sums, &sig) {
        return Err(refuse(
            "signature",
            "the signature does not verify over this SHA256SUMS with the signing certificate's key",
        ));
    }

    // 6. Who signed: the release workflow, at the served version's tag.
    let ids = identity_of(&cert)?;
    let identity = check_identity(&ids, expect)?;
    Ok(Verified {
        identity,
        log_index: index,
        integrated_time: time,
    })
}

fn b64(s: &str, what: &str) -> Result<Vec<u8>, Refusal> {
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| refuse("bundle", format!("the bundle's {what} is not base64: {e}")))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn when(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| t.to_string())
}

/// The payload a SignedEntryTimestamp signs: the RFC 8785 (JCS) canonical
/// JSON of exactly {body, integratedTime, logID, logIndex}. JCS orders keys by
/// UTF-16 code units, which for these four is body, integratedTime, logID,
/// logIndex ("logID" < "logIndex" at 'D' < 'n'). Rekor signs this object
/// (rekor pkg/api/entries.go, the SET over the entry's canonical JSON), and
/// cosign checks it the same way (pkg/cosign/tlog.go, VerifySET). Any other
/// field makes the payload not one Rekor signed.
fn set_payload(p: &serde_json::Value) -> Result<Vec<u8>, Refusal> {
    let obj = p
        .as_object()
        .ok_or_else(|| refuse("timestamp", "the rekorBundle's Payload is not an object"))?;
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    if keys != ["body", "integratedTime", "logID", "logIndex"] {
        return Err(refuse(
            "timestamp",
            format!("the rekorBundle's Payload has the fields {keys:?}, not exactly body, integratedTime, logID and logIndex"),
        ));
    }
    let (Some(body), Some(time), Some(id), Some(index)) = (
        p["body"].as_str(),
        p["integratedTime"].as_i64(),
        p["logID"].as_str(),
        p["logIndex"].as_i64(),
    ) else {
        return Err(refuse(
            "timestamp",
            "the rekorBundle's Payload fields have the wrong types",
        ));
    };
    let s = |x: &str| serde_json::to_string(x).unwrap_or_default();
    Ok(format!(
        "{{\"body\":{},\"integratedTime\":{time},\"logID\":{},\"logIndex\":{index}}}",
        s(body),
        s(id)
    )
    .into_bytes())
}

fn check_set(
    canon: &[u8],
    set: &[u8],
    log_id: &str,
    time: i64,
    root: &TrustRoot,
) -> Result<(), Refusal> {
    let id = unhex(log_id)
        .ok_or_else(|| refuse("timestamp", format!("the logID {log_id} is not hex")))?;
    let Some(key) = root
        .logs
        .iter()
        .find(|k| k.id == id && k.start <= time && k.end.is_none_or(|e| time <= e))
    else {
        return Err(refuse(
            "timestamp",
            format!(
                "the entry's Rekor log key {log_id} is in no trusted root valid at {} ({}). If Sigstore rotated its log key, set {TRUSTED_ROOT_ENV} to a trusted_root.json that holds it",
                when(time),
                root.named()
            ),
        ));
    };
    let ok = match SubjectPublicKeyInfo::from_der(&key.spki) {
        Ok((_, spki)) if key.ed25519 => ring::signature::UnparsedPublicKey::new(
            &ring::signature::ED25519,
            &spki.subject_public_key.data,
        )
        .verify(canon, set)
        .is_ok(),
        Ok((_, spki)) => verify_raw(&spki, canon, set),
        Err(_) => false,
    };
    if !ok {
        return Err(refuse(
            "timestamp",
            format!("Rekor's SignedEntryTimestamp does not verify under log key {log_id}: the entry's payload is not the one the log signed"),
        ));
    }
    Ok(())
}

/// The entry is a hashedrekord v0.0.1 that names this file's SHA-256, this
/// signature and this certificate (rekor pkg/types/hashedrekord; cosign
/// compares the entry with the artifact, signature and certificate it holds).
fn check_entry(
    body: &serde_json::Value,
    digest: &[u8],
    sig: &[u8],
    leaf: &[u8],
) -> Result<(), Refusal> {
    if body["kind"] != "hashedrekord" || body["apiVersion"] != "0.0.1" {
        return Err(refuse(
            "entry",
            format!(
                "the Rekor entry is {} {}, not hashedrekord 0.0.1",
                body["kind"], body["apiVersion"]
            ),
        ));
    }
    let spec = &body["spec"];
    let want = hex(digest);
    if spec["data"]["hash"]["algorithm"] != "sha256"
        || spec["data"]["hash"]["value"].as_str() != Some(want.as_str())
    {
        return Err(refuse(
            "entry",
            format!(
                "the Rekor entry names the hash {} {}, not this SHA256SUMS's sha256 {want}",
                spec["data"]["hash"]["algorithm"], spec["data"]["hash"]["value"]
            ),
        ));
    }
    let named_sig = base64::engine::general_purpose::STANDARD
        .decode(spec["signature"]["content"].as_str().unwrap_or_default())
        .unwrap_or_default();
    if named_sig != sig {
        return Err(refuse(
            "entry",
            "the Rekor entry names another signature than the bundle's",
        ));
    }
    let named_cert = base64::engine::general_purpose::STANDARD
        .decode(
            spec["signature"]["publicKey"]["content"]
                .as_str()
                .unwrap_or_default(),
        )
        .ok()
        .and_then(|p| pem_to_der(&p));
    if named_cert.as_deref() != Some(leaf) {
        return Err(refuse(
            "entry",
            "the Rekor entry names another certificate than the bundle's",
        ));
    }
    Ok(())
}

/// A certificate's own validity covers `t`.
fn valid_at(c: &X509Certificate, t: i64) -> bool {
    let v = c.validity();
    v.not_before.timestamp() <= t && t <= v.not_after.timestamp()
}

/// ECDSA (P-256 with SHA-256, P-384 with SHA-384) over `msg` by `spki`.
fn verify_raw(spki: &SubjectPublicKeyInfo, msg: &[u8], sig: &[u8]) -> bool {
    use x509_parser::oid_registry::{OID_EC_P256, OID_NIST_EC_P384};
    let curve = spki
        .algorithm
        .parameters
        .as_ref()
        .and_then(|p| p.as_oid().ok());
    let alg: &dyn ring::signature::VerificationAlgorithm = match curve {
        Some(c) if c == OID_EC_P256 => &ring::signature::ECDSA_P256_SHA256_ASN1,
        Some(c) if c == OID_NIST_EC_P384 => &ring::signature::ECDSA_P384_SHA384_ASN1,
        _ => return false,
    };
    ring::signature::UnparsedPublicKey::new(alg, &spki.subject_public_key.data)
        .verify(msg, sig)
        .is_ok()
}

/// The leaf chains, at `t`, to a Fulcio CA of the trusted root valid at `t`:
/// each certificate signed by the next, each a CA but the leaf, each valid at
/// `t` (cosign ValidateAndUnpackCert and CheckExpiry: a Fulcio leaf lives ten
/// minutes, so the entry's time, not today, is what it must cover).
fn chain_to(leaf: &[u8], t: i64, root: &TrustRoot) -> Result<Vec<u8>, Refusal> {
    let (_, cert) = X509Certificate::from_der(leaf).map_err(|e| {
        refuse(
            "chain",
            format!("the signing certificate does not parse: {e}"),
        )
    })?;
    if !valid_at(&cert, t) {
        return Err(refuse(
            "chain",
            format!(
                "the signing certificate was valid from {} to {}, and Rekor recorded the entry at {}",
                cert.validity().not_before,
                cert.validity().not_after,
                when(t)
            ),
        ));
    }
    for a in root
        .authorities
        .iter()
        .filter(|a| a.start <= t && a.end.is_none_or(|e| t <= e))
    {
        let parsed: Vec<X509Certificate> = a
            .chain
            .iter()
            .filter_map(|d| X509Certificate::from_der(d).ok().map(|(_, c)| c))
            .collect();
        if parsed.len() != a.chain.len() || parsed.is_empty() {
            continue;
        }
        let mut child = &cert;
        let mut ok = true;
        for issuer in &parsed {
            let is_ca = matches!(issuer.basic_constraints(), Ok(Some(bc)) if bc.value.ca);
            if child.issuer().as_raw() != issuer.subject().as_raw()
                || !is_ca
                || !valid_at(issuer, t)
                || child.verify_signature(Some(issuer.public_key())).is_err()
            {
                ok = false;
                break;
            }
            child = issuer;
        }
        // The last certificate is the root: self-issued, self-signed.
        if ok
            && child.issuer().as_raw() == child.subject().as_raw()
            && child.verify_signature(None).is_ok()
        {
            // The leaf's issuer, whose key hash the SCT is over.
            return Ok(a.chain[0].clone());
        }
    }
    Err(refuse(
        "chain",
        format!(
            "the signing certificate (issuer {}) chains to no Fulcio CA of {} valid at {}. If Sigstore rotated its CA, set {TRUSTED_ROOT_ENV} to a trusted_root.json that holds it",
            cert.issuer(),
            root.named(),
            when(t)
        ),
    ))
}

/// The extensions a leaf may mark critical: the ones this verifier reads or
/// enforces. Any other critical extension refuses the leaf, as Go's
/// x509.Verify does for an unhandled critical extension (cosign verifies
/// certificates with it).
const HANDLED_CRITICAL: &[&str] = &[
    "2.5.29.15", // keyUsage
    "2.5.29.17", // subjectAltName (critical when the subject is empty, as Fulcio's is)
    "2.5.29.19", // basicConstraints
    "2.5.29.37", // extKeyUsage
];

/// The leaf is an end-entity certificate: not a CA, and with no critical
/// extension the verifier does not handle.
fn check_leaf(c: &X509Certificate) -> Result<(), Refusal> {
    if matches!(c.basic_constraints(), Ok(Some(bc)) if bc.value.ca) {
        return Err(refuse(
            "chain",
            "the signing certificate is a CA (basicConstraints CA:TRUE), not a Fulcio signing leaf",
        ));
    }
    if let Some(e) = c
        .extensions()
        .iter()
        .find(|e| e.critical && !HANDLED_CRITICAL.contains(&e.oid.to_id_string().as_str()))
    {
        return Err(refuse(
            "chain",
            format!(
                "the signing certificate marks the extension {} critical, which this verifier does not handle",
                e.oid.to_id_string()
            ),
        ));
    }
    Ok(())
}

/// The SCT list extension (RFC 6962 section 3.3).
const SCT_LIST_OID: &str = "1.3.6.1.4.1.11129.2.4.2";

/// One DER element at the start of `b`: (tag, header length, content length).
fn der_tlv(b: &[u8]) -> Option<(u8, usize, usize)> {
    let tag = *b.first()?;
    let first = *b.get(1)?;
    if first < 0x80 {
        return Some((tag, 2, first as usize));
    }
    let n = (first & 0x7f) as usize;
    if n == 0 || n > 3 || b.len() < 2 + n {
        return None;
    }
    let len = b[2..2 + n]
        .iter()
        .fold(0usize, |a, &x| (a << 8) | x as usize);
    Some((tag, 2 + n, len))
}

fn der_len(n: usize) -> Vec<u8> {
    match n {
        0..=0x7f => vec![n as u8],
        0x80..=0xff => vec![0x81, n as u8],
        0x100..=0xffff => vec![0x82, (n >> 8) as u8, n as u8],
        _ => vec![0x83, (n >> 16) as u8, (n >> 8) as u8, n as u8],
    }
}

/// The DER elements of a constructed element's content.
fn der_children(mut b: &[u8]) -> Option<Vec<&[u8]>> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let (_, h, l) = der_tlv(b)?;
        if b.len() < h + l {
            return None;
        }
        out.push(&b[..h + l]);
        b = &b[h + l..];
    }
    Some(out)
}

fn der_wrap(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend(der_len(content.len()));
    out.extend_from_slice(content);
    out
}

/// The precertificate's TBS: the leaf's TBSCertificate with the SCT list
/// extension removed and every length above it re-encoded (RFC 6962 section
/// 3.2; certificate-transparency-go x509util / ctutil.VerifySCT build the
/// same bytes).
fn precert_tbs(tbs: &[u8], sct_ext_oid: &[u8]) -> Option<Vec<u8>> {
    let (tag, h, l) = der_tlv(tbs)?;
    if tag != 0x30 || tbs.len() != h + l {
        return None;
    }
    let mut fields = Vec::new();
    let mut found = false;
    for f in der_children(&tbs[h..])? {
        if f[0] != 0xa3 {
            fields.extend_from_slice(f);
            continue;
        }
        let (_, eh, _) = der_tlv(f)?;
        let seq = &f[eh..];
        let (_, sh, _) = der_tlv(seq)?;
        let mut kept = Vec::new();
        for ext in der_children(&seq[sh..])? {
            let (_, xh, _) = der_tlv(ext)?;
            let oid = der_children(&ext[xh..])?.first().copied()?;
            if oid == sct_ext_oid {
                found = true;
            } else {
                kept.extend_from_slice(ext);
            }
        }
        fields.extend(der_wrap(0xa3, &der_wrap(0x30, &kept)));
    }
    found.then(|| der_wrap(0x30, &fields))
}

/// The leaf carries an SCT, from a CT log of the trusted root valid at the
/// SCT's time, over the precertificate entry (RFC 6962 sections 3.2 and 3.3):
/// version 0, certificate_timestamp, the SCT's timestamp, precert_entry, the
/// SHA-256 of the issuer's SubjectPublicKeyInfo, the precert TBS (24-bit
/// length) and the SCT's extensions. One that verifies is enough, as cosign
/// requires (VerifyEmbeddedSCT).
fn check_sct(c: &X509Certificate, issuer_der: &[u8], root: &TrustRoot) -> Result<(), Refusal> {
    let Some(ext) = c
        .extensions()
        .iter()
        .find(|e| e.oid.to_id_string() == SCT_LIST_OID)
    else {
        return Err(refuse(
            "sct",
            "the signing certificate carries no SCT: it was not logged in a Certificate Transparency log",
        ));
    };
    let bad = |why: &str| refuse("sct", format!("the signing certificate's SCT list {why}"));
    // extnValue holds an OCTET STRING whose content is the TLS-encoded list.
    let (tag, h, l) = der_tlv(ext.value).ok_or_else(|| bad("is not DER"))?;
    if tag != 0x04 || ext.value.len() != h + l {
        return Err(bad("is not an OCTET STRING"));
    }
    let list = &ext.value[h..];
    let total = u16::from_be_bytes([
        *list.first().ok_or_else(|| bad("is empty"))?,
        *list.get(1).ok_or_else(|| bad("is empty"))?,
    ]) as usize;
    if list.len() != 2 + total {
        return Err(bad("has the wrong length"));
    }
    let (_, issuer) = X509Certificate::from_der(issuer_der)
        .map_err(|_| bad("names an issuer that does not parse"))?;
    let issuer_key_hash = Sha256::digest(issuer.public_key().raw);
    // The DER of the SCT list extension's OID, to take it out of the TBS.
    let oid_der = der_wrap(0x06, ext.oid.as_bytes());
    let pre = precert_tbs(c.tbs_certificate.as_ref(), &oid_der).ok_or_else(|| {
        bad("cannot be removed from the certificate to rebuild its precertificate")
    })?;
    let mut rest = &list[2..];
    let mut seen = Vec::new();
    while rest.len() >= 2 {
        let n = u16::from_be_bytes([rest[0], rest[1]]) as usize;
        let sct = rest
            .get(2..2 + n)
            .ok_or_else(|| bad("has an SCT past its end"))?;
        rest = &rest[2 + n..];
        // version(1) log_id(32) timestamp(8) extensions(2+n) hash(1) sig(1) signature(2+n)
        if sct.len() < 1 + 32 + 8 + 2 || sct[0] != 0 {
            continue;
        }
        let log_id = &sct[1..33];
        let ts = u64::from_be_bytes(sct[33..41].try_into().unwrap_or_default());
        let el = u16::from_be_bytes([sct[41], sct[42]]) as usize;
        let Some(exts) = sct.get(43..43 + el) else {
            continue;
        };
        let tail = &sct[43 + el..];
        if tail.len() < 4 {
            continue;
        }
        let sl = u16::from_be_bytes([tail[2], tail[3]]) as usize;
        let Some(sig) = tail.get(4..4 + sl) else {
            continue;
        };
        seen.push(hex(log_id));
        let t = (ts / 1000) as i64;
        let Some(key) = root
            .ctlogs
            .iter()
            .find(|k| k.id == log_id && k.start <= t && k.end.is_none_or(|e| t <= e))
        else {
            continue;
        };
        let mut signed = vec![0u8, 0u8];
        signed.extend(ts.to_be_bytes());
        signed.extend([0u8, 1u8]);
        signed.extend(issuer_key_hash);
        signed.extend(&(pre.len() as u32).to_be_bytes()[1..]);
        signed.extend(&pre);
        signed.extend((el as u16).to_be_bytes());
        signed.extend(exts);
        if let Ok((_, spki)) = SubjectPublicKeyInfo::from_der(&key.spki) {
            if !key.ed25519 && verify_raw(&spki, &signed, sig) {
                return Ok(());
            }
        }
    }
    Err(refuse(
        "sct",
        format!(
            "no SCT of the signing certificate verifies under a CT log of {} (logs named: {}). If Sigstore rotated its CT log key, set {TRUSTED_ROOT_ENV} to a trusted_root.json that holds it",
            root.named(),
            if seen.is_empty() { "none".into() } else { seen.join(", ") }
        ),
    ))
}

/// A Fulcio leaf's workflow identity (sigstore/fulcio docs/oid-info.md).
struct Identity {
    issuer: String,
    source_repo: String,
    source_ref: String,
    build_config: String,
    san: String,
}

/// A DER UTF8String's text (Fulcio's 1.3.6.1.4.1.57264.1.8 and later
/// extensions are DER-encoded; 1.1 to 1.6 are the raw bytes).
fn der_utf8(v: &[u8]) -> Option<String> {
    let (&tag, rest) = v.split_first()?;
    if tag != 0x0c {
        return None;
    }
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (first as usize, rest)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 2 || rest.len() < n {
            return None;
        }
        let len = rest[..n].iter().fold(0usize, |a, &b| (a << 8) | b as usize);
        (len, &rest[n..])
    };
    (rest.len() == len).then(|| String::from_utf8(rest.to_vec()).ok())?
}

fn identity_of(c: &X509Certificate) -> Result<Identity, Refusal> {
    let ext = |oid: &str, der: bool| -> Option<String> {
        c.extensions()
            .iter()
            .find(|e| e.oid.to_id_string() == oid)
            .and_then(|e| {
                if der {
                    der_utf8(e.value)
                } else {
                    String::from_utf8(e.value.to_vec()).ok()
                }
            })
    };
    let need = |v: Option<String>, what: &str| {
        v.ok_or_else(|| {
            refuse(
                "identity",
                format!("the signing certificate carries no {what}"),
            )
        })
    };
    let issuer = ext("1.3.6.1.4.1.57264.1.8", true).or_else(|| ext("1.3.6.1.4.1.57264.1.1", false));
    let san = match c.subject_alternative_name() {
        Ok(Some(s)) => s.value.general_names.iter().find_map(|g| match g {
            GeneralName::URI(u) => Some(u.to_string()),
            _ => None,
        }),
        _ => None,
    };
    Ok(Identity {
        issuer: need(issuer, "OIDC issuer (1.3.6.1.4.1.57264.1.8)")?,
        source_repo: need(
            ext("1.3.6.1.4.1.57264.1.12", true),
            "source repository (1.3.6.1.4.1.57264.1.12)",
        )?,
        source_ref: need(
            ext("1.3.6.1.4.1.57264.1.14", true),
            "source ref (1.3.6.1.4.1.57264.1.14)",
        )?,
        build_config: need(
            ext("1.3.6.1.4.1.57264.1.18", true),
            "Build Config URI (1.3.6.1.4.1.57264.1.18)",
        )?,
        san: san.unwrap_or_default(),
    })
}

/// The identity is the release workflow's: GitHub's issuer, the signer's
/// repository (GitHub names are matched without case), the tag of the served
/// version, and the release workflow at that tag. Returns the Build Config URI.
fn check_identity(i: &Identity, e: &Expect) -> Result<String, Refusal> {
    if i.issuer != GITHUB_ISSUER {
        return Err(refuse(
            "identity",
            format!("the OIDC issuer is {}, not {GITHUB_ISSUER}", i.issuer),
        ));
    }
    let repo = format!("https://github.com/{}", e.signer);
    if !i.source_repo.eq_ignore_ascii_case(&repo) {
        return Err(refuse(
            "identity",
            format!(
                "the source repository is {}, not {repo} (the release signer)",
                i.source_repo
            ),
        ));
    }
    let tag = format!("refs/tags/{TAG_PREFIX}{}", e.version);
    if i.source_ref != tag {
        return Err(refuse(
            "identity",
            format!("the source ref is {}, not {tag}", i.source_ref),
        ));
    }
    let tail = format!("/.github/workflows/{}@{tag}", e.workflow);
    let matches = i
        .build_config
        .strip_suffix(&tail)
        .is_some_and(|head| head.eq_ignore_ascii_case(&repo));
    if !matches {
        return Err(refuse(
            "identity",
            format!(
                "the Build Config URI is {}, not {repo}{tail} (the certificate's SAN, {}, is the reusable workflow every release runs)",
                i.build_config, i.san
            ),
        ));
    }
    Ok(i.build_config.clone())
}

/// The DER of the first certificate in a PEM text.
fn pem_to_der(pem: &[u8]) -> Option<Vec<u8>> {
    let (_, p) = x509_parser::pem::parse_x509_pem(pem).ok()?;
    (p.label == "CERTIFICATE").then_some(p.contents)
}

/// The update tests' stand-in Sigstore, for the verifier's own tests.
#[cfg(test)]
#[path = "../../tests/support/sigstore.rs"]
mod stand_in;

#[cfg(test)]
mod tests {
    use super::*;

    const SUMS: &[u8] = include_bytes!("../../tests/fixtures/signature/v0.5.1/SHA256SUMS");
    const BUNDLE: &str =
        include_str!("../../tests/fixtures/signature/v0.5.1/SHA256SUMS.cosign-bundle");

    fn release() -> Expect<'static> {
        Expect {
            signer: "USAREUR-AF/goatcore",
            version: "0.5.1",
            workflow: RELEASE_WORKFLOW,
        }
    }

    fn root() -> TrustRoot {
        TrustRoot::parse(EMBEDDED_ROOT, "embedded").unwrap()
    }

    fn json(b: &str) -> serde_json::Value {
        serde_json::from_str(b).unwrap()
    }

    fn b64e(b: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(b)
    }

    fn b64d(s: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD.decode(s).unwrap()
    }

    /// The published v0.5.1 SHA256SUMS and its bundle verify, offline, with
    /// no cosign.
    #[test]
    fn the_published_0_5_1_signature_is_accepted() {
        let v = verify(SUMS, BUNDLE, &release(), &root()).unwrap();
        assert_eq!(v.integrated_time, 1794414398);
        assert!(
            v.identity
                .contains("goatbuilder-release.yml@refs/tags/goatbuilder/v0.5.1"),
            "{v:?}"
        );
    }

    /// Every goatbuilder release published with a signature (0.4.1, 0.4.2,
    /// 0.4.3 and 0.5.1; 0.5.0 was tagged and never published) verifies.
    #[test]
    fn every_published_release_signature_is_accepted() {
        for (v, sums, bundle) in [
            (
                "0.4.1",
                &include_bytes!("../../tests/fixtures/signature/v0.4.1/SHA256SUMS")[..],
                include_str!("../../tests/fixtures/signature/v0.4.1/SHA256SUMS.cosign-bundle"),
            ),
            (
                "0.4.2",
                &include_bytes!("../../tests/fixtures/signature/v0.4.2/SHA256SUMS")[..],
                include_str!("../../tests/fixtures/signature/v0.4.2/SHA256SUMS.cosign-bundle"),
            ),
            (
                "0.4.3",
                &include_bytes!("../../tests/fixtures/signature/v0.4.3/SHA256SUMS")[..],
                include_str!("../../tests/fixtures/signature/v0.4.3/SHA256SUMS.cosign-bundle"),
            ),
            ("0.5.1", SUMS, BUNDLE),
        ] {
            let e = Expect {
                version: v,
                ..release()
            };
            let got = verify(sums, bundle, &e, &root());
            assert!(got.is_ok(), "{v}: {got:?}");
        }
    }

    /// A repository's name is matched as GitHub matches it, without case.
    #[test]
    fn the_signer_is_matched_without_case() {
        let e = Expect {
            signer: "usareur-af/goatcore",
            ..release()
        };
        assert!(verify(SUMS, BUNDLE, &e, &root()).is_ok());
    }

    #[test]
    fn another_signer_repository_is_refused() {
        let e = Expect {
            signer: "example-org/goatcore",
            ..release()
        };
        let r = verify(SUMS, BUNDLE, &e, &root()).unwrap_err();
        assert_eq!(r.check, "identity", "{r}");
        assert!(r.detail.contains("source repository"), "{r}");
    }

    /// goat's release runs the same reusable workflow (the same SAN); only the
    /// Build Config URI tells it from goatbuilder's.
    #[test]
    fn another_release_workflow_is_refused() {
        let e = Expect {
            workflow: "goat-release.yml",
            ..release()
        };
        let r = verify(SUMS, BUNDLE, &e, &root()).unwrap_err();
        assert_eq!(r.check, "identity", "{r}");
        assert!(r.detail.contains("Build Config URI"), "{r}");
    }

    /// The signature is for the served version's tag, so an older release's
    /// signed SHA256SUMS cannot pass for a newer one.
    #[test]
    fn another_version_s_tag_is_refused() {
        let e = Expect {
            version: "0.5.2",
            ..release()
        };
        let r = verify(SUMS, BUNDLE, &e, &root()).unwrap_err();
        assert_eq!(r.check, "identity", "{r}");
        assert!(r.detail.contains("source ref"), "{r}");
    }

    /// A branch, not a goatbuilder/v* tag, is not a release.
    #[test]
    fn a_source_ref_that_is_a_branch_is_refused() {
        let leaf = leaf_der();
        let (_, cert) = X509Certificate::from_der(&leaf).unwrap();
        let mut ids = identity_of(&cert).unwrap();
        ids.source_ref = "refs/heads/main".into();
        let r = check_identity(&ids, &release()).unwrap_err();
        assert!(r.detail.contains("source ref"), "{r}");
        ids.source_ref = "refs/tags/goatbuilder/v0.5.1".into();
        ids.build_config =
            "https://github.com/USAREUR-AF/goatcore/.github/workflows/goatbuilder-release.yml@refs/heads/main".into();
        let r = check_identity(&ids, &release()).unwrap_err();
        assert!(r.detail.contains("Build Config URI"), "{r}");
        ids.build_config = "https://github.com/USAREUR-AF/goatcore/.github/workflows/goatbuilder-release.yml@refs/tags/goatbuilder/v0.5.1".into();
        ids.issuer = "https://accounts.example.org".into();
        let r = check_identity(&ids, &release()).unwrap_err();
        assert!(r.detail.contains("issuer"), "{r}");
    }

    #[test]
    fn a_changed_sha256sums_is_refused() {
        let mut sums = SUMS.to_vec();
        sums[0] ^= 1;
        let r = verify(&sums, BUNDLE, &release(), &root()).unwrap_err();
        assert!(r.check == "signature" || r.check == "entry", "{r}");
    }

    #[test]
    fn a_changed_signed_entry_timestamp_is_refused() {
        let mut b = json(BUNDLE);
        let mut set = b64d(b["rekorBundle"]["SignedEntryTimestamp"].as_str().unwrap());
        let n = set.len() - 2;
        set[n] ^= 1;
        b["rekorBundle"]["SignedEntryTimestamp"] = b64e(&set).into();
        let r = verify(SUMS, &b.to_string(), &release(), &root()).unwrap_err();
        assert_eq!(r.check, "timestamp", "{r}");
    }

    /// The SET is over the RFC 8785 canonical JSON of exactly the four
    /// payload fields, keys in UTF-16 order: body, integratedTime, logID,
    /// logIndex (cosign pkg/cosign/tlog.go VerifySET).
    #[test]
    fn the_signed_entry_timestamp_covers_exactly_the_four_fields_in_canonical_order() {
        let b = json(BUNDLE);
        let p = &b["rekorBundle"]["Payload"];
        let canon = set_payload(p).unwrap();
        let want = format!(
            "{{\"body\":\"{}\",\"integratedTime\":{},\"logID\":\"{}\",\"logIndex\":{}}}",
            p["body"].as_str().unwrap(),
            p["integratedTime"],
            p["logID"].as_str().unwrap(),
            p["logIndex"]
        );
        assert_eq!(String::from_utf8(canon).unwrap(), want);
        // A field changed or added under the SET is refused.
        for (k, v) in [
            ("integratedTime", serde_json::json!(1794414399)),
            ("logIndex", serde_json::json!(1)),
        ] {
            let mut c = b.clone();
            c["rekorBundle"]["Payload"][k] = v;
            let r = verify(SUMS, &c.to_string(), &release(), &root()).unwrap_err();
            assert_eq!(r.check, "timestamp", "{k}: {r}");
        }
        let mut c = b.clone();
        c["rekorBundle"]["Payload"]["extra"] = "x".into();
        assert!(set_payload(&c["rekorBundle"]["Payload"]).is_err());
    }

    /// The entry's body must name this file's hash, this signature and this
    /// certificate; a body naming another hash is refused even where its SET
    /// would verify.
    #[test]
    fn a_rekor_body_naming_another_hash_is_refused() {
        let b = json(BUNDLE);
        let body: serde_json::Value =
            serde_json::from_slice(&b64d(b["rekorBundle"]["Payload"]["body"].as_str().unwrap()))
                .unwrap();
        let sig = b64d(b["base64Signature"].as_str().unwrap());
        let leaf = leaf_der();
        assert!(check_entry(&body, &Sha256::digest(SUMS), &sig, &leaf).is_ok());
        let r = check_entry(&body, &Sha256::digest(b"another file"), &sig, &leaf).unwrap_err();
        assert_eq!(r.check, "entry", "{r}");
        assert!(r.detail.contains("hash"), "{r}");
        let r = check_entry(&body, &Sha256::digest(SUMS), &sig[1..], &leaf).unwrap_err();
        assert!(r.detail.contains("signature"), "{r}");
        let mut kind = body.clone();
        kind["kind"] = "rekord".into();
        assert!(check_entry(&kind, &Sha256::digest(SUMS), &sig, &leaf).is_err());
    }

    /// Every certificate of the chain must be valid when the entry was made:
    /// the leaf for its ten minutes, the intermediate and the root too.
    #[test]
    fn each_certificate_must_be_valid_at_the_entry_s_time() {
        let leaf = leaf_der();
        let t = 1794414398;
        assert!(chain_to(&leaf, t, &root()).is_ok());
        // Within the intermediate's life but after the leaf's ten minutes.
        let r = chain_to(&leaf, t + 3600, &root()).unwrap_err();
        assert_eq!(r.check, "chain", "{r}");
        // An authority whose validFor ended before the entry is not used.
        let mut r2 = root();
        for a in &mut r2.authorities {
            a.end = Some(t - 1);
        }
        assert!(chain_to(&leaf, t, &r2).is_err());
        // The intermediate's own notAfter is checked: an entry dated after it
        // is refused even with the leaf's window moved (checked directly).
        let ca = &root().authorities[1].chain[0];
        let (_, inter) = X509Certificate::from_der(ca).unwrap();
        let after = inter.validity().not_after.timestamp() + 1;
        assert!(!valid_at(&inter, after));
        assert!(valid_at(&inter, t));
    }

    /// A certificate not issued by a trusted Fulcio CA is refused, however
    /// well formed.
    #[test]
    fn a_certificate_not_signed_by_the_intermediate_is_refused() {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut p = rcgen::CertificateParams::new(vec![]).unwrap();
        p.distinguished_name
            .push(rcgen::DnType::OrganizationName, "sigstore.dev");
        p.distinguished_name
            .push(rcgen::DnType::CommonName, "sigstore-intermediate");
        let forged = p.self_signed(&key).unwrap();
        let r = chain_to(forged.der(), 1794414398, &root()).unwrap_err();
        assert_eq!(r.check, "chain", "{r}");
    }

    /// An operator's trusted root adds keys beside the embedded ones; without
    /// the log key a bundle names, the refusal names the key and the option.
    #[test]
    fn a_missing_log_key_is_named_with_the_option_to_supply_one() {
        let mut r = root();
        r.logs.clear();
        r.logs.push(LogKey {
            id: vec![0; 32],
            spki: vec![],
            ed25519: false,
            start: 0,
            end: None,
        });
        let e = verify(SUMS, BUNDLE, &release(), &r).unwrap_err();
        assert_eq!(e.check, "timestamp", "{e}");
        assert!(e.detail.contains("c0d23d6ad406973f"), "{e}");
        assert!(e.detail.contains(TRUSTED_ROOT_ENV), "{e}");
        // The operator's file is read and added.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("trusted_root.json");
        std::fs::write(&p, EMBEDDED_ROOT).unwrap();
        let extra = TrustRoot::parse(&std::fs::read_to_string(&p).unwrap(), "operator").unwrap();
        r.add(extra);
        assert!(verify(SUMS, BUNDLE, &release(), &r).is_ok());
    }

    #[test]
    fn a_bundle_that_is_not_one_is_refused() {
        for b in ["", "{}", "{\"cert\":\"x\"}"] {
            let r = verify(SUMS, b, &release(), &root()).unwrap_err();
            assert_eq!(r.check, "bundle", "{b:?}: {r}");
        }
    }

    use super::stand_in;

    /// A stand-in Sigstore's release, verified against its trusted root alone.
    fn stand_in(leaf: stand_in::Leaf) -> Result<Verified, Refusal> {
        let s = stand_in::Sigstore::new();
        let w = stand_in::Workflow::release("example-org/releases", "9.9.9");
        let bundle = s.bundle_with(SUMS, &w, leaf);
        let root = TrustRoot::parse(&s.trusted_root(), "stand-in").unwrap();
        let e = Expect {
            signer: "example-org/releases",
            version: "9.9.9",
            workflow: RELEASE_WORKFLOW,
        };
        verify(SUMS, &bundle, &e, &root)
    }

    /// The published releases' SCTs are checked, not skipped: without the
    /// trusted root's CT logs, 0.5.1 is refused at the SCT.
    #[test]
    fn the_published_signature_needs_its_ct_log() {
        let mut r = root();
        r.ctlogs.clear();
        let e = verify(SUMS, BUNDLE, &release(), &r).unwrap_err();
        assert_eq!(e.check, "sct", "{e}");
        assert!(
            e.detail
                .contains("dd3d306ac6c7113263191e1c99673702a24a5eb8de3cadff878a72802f29ee8e"),
            "{e}"
        );
    }

    #[test]
    fn a_stand_in_release_with_a_valid_sct_is_accepted() {
        let v = stand_in(stand_in::Leaf::default());
        assert!(v.is_ok(), "{v:?}");
    }

    /// cosign v2 verify-blob requires an embedded SCT from a trusted CT log
    /// (pkg/cosign/verify.go, VerifyEmbeddedSCT; RFC 6962 section 3.2).
    #[test]
    fn a_leaf_without_an_sct_is_refused() {
        let r = stand_in(stand_in::Leaf {
            sct: stand_in::Sct::None,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(r.check, "sct", "{r}");
    }

    #[test]
    fn an_sct_from_a_log_the_trusted_root_does_not_hold_is_refused() {
        let r = stand_in(stand_in::Leaf {
            sct: stand_in::Sct::UnknownLog,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(r.check, "sct", "{r}");
        assert!(r.detail.contains(TRUSTED_ROOT_ENV), "{r}");
    }

    #[test]
    fn an_sct_over_another_certificate_is_refused() {
        let r = stand_in(stand_in::Leaf {
            sct: stand_in::Sct::OverOther,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(r.check, "sct", "{r}");
    }

    /// As Go's x509.Verify does, a leaf with a critical extension the
    /// verifier does not handle is refused.
    #[test]
    fn a_leaf_with_an_unknown_critical_extension_is_refused() {
        let r = stand_in(stand_in::Leaf {
            unknown_critical: true,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(r.check, "chain", "{r}");
        assert!(r.detail.contains("critical"), "{r}");
    }

    #[test]
    fn a_leaf_that_is_a_ca_is_refused() {
        let r = stand_in(stand_in::Leaf {
            ca: true,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(r.check, "chain", "{r}");
        assert!(r.detail.contains("CA"), "{r}");
    }

    /// Through verify(): a leaf whose ten minutes ended before Rekor
    /// integrated the entry.
    #[test]
    fn a_leaf_expired_at_the_entry_s_time_is_refused() {
        let r = stand_in(stand_in::Leaf {
            expired: true,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(r.check, "chain", "{r}");
        assert!(r.detail.contains("Rekor recorded the entry at"), "{r}");
    }

    /// An operator's trusted root with an `end` that does not parse is
    /// refused, rather than read as a window with no end.
    #[test]
    fn a_trusted_root_with_an_unreadable_end_is_refused() {
        let mut v: serde_json::Value = serde_json::from_str(EMBEDDED_ROOT).unwrap();
        v["tlogs"][0]["publicKey"]["validFor"]["end"] = "the end of time".into();
        let e = TrustRoot::parse(&v.to_string(), "operator").unwrap_err();
        assert!(e.contains("end"), "{e}");
        let mut v: serde_json::Value = serde_json::from_str(EMBEDDED_ROOT).unwrap();
        v["certificateAuthorities"][1]["validFor"]["end"] = 7.into();
        assert!(TrustRoot::parse(&v.to_string(), "operator").is_err());
    }

    fn leaf_der() -> Vec<u8> {
        let b = json(BUNDLE);
        pem_to_der(&b64d(b["cert"].as_str().unwrap())).unwrap()
    }
}
