//! The `prov` v1 object: build and sign it, carry it in the sample
//! attachment, verify it, and grade a sample.

use crate::canonical;
use crate::cert::{anchor_of_cn, is_anchor, Signer, SignerCert};
use base64::Engine as _;
use p256::ecdsa::signature::{Signer as _, Verifier as _};
use p256::ecdsa::Signature;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// The attachment key provenance sits under.
pub const ATTACHMENT_KEY: &str = "prov";
/// The signature-domain prefix (§7.2).
pub const SIG_PREFIX: &[u8] = b"goat-prov-v1\n";
/// The only version this crate writes and reads.
pub const VERSION: u64 = 1;

/// How an ingress adapter received a keyless source's frame.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Via {
    pub iface: String,
    pub port: u16,
    pub from: String,
}

/// The two grades a signature can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignedGrade {
    DeviceSigned,
    AdapterAttested,
}

impl SignedGrade {
    pub fn as_str(self) -> &'static str {
        match self {
            SignedGrade::DeviceSigned => "device-signed",
            SignedGrade::AdapterAttested => "adapter-attested",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "device-signed" => Some(SignedGrade::DeviceSigned),
            "adapter-attested" => Some(SignedGrade::AdapterAttested),
            _ => None,
        }
    }
}

/// `prov` v1 as it rides in the attachment.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Prov {
    pub v: u64,
    pub grade: String,
    pub src: String,
    pub signer: String,
    pub ts: String,
    pub key: String,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<Via>,
    #[serde(default)]
    pub sig: String,
}

/// Why a `prov` could not be built, or did not verify.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProvError {
    #[error("attachment is not a JSON object: {0}")]
    AttachmentNotObject(String),
    #[error("prov is malformed: {0}")]
    Malformed(String),
    #[error("prov version {0} is not supported (this build reads v1)")]
    Version(u64),
    #[error("unknown grade {0:?}")]
    UnknownGrade(String),
    #[error("signed for key {signed:?} but the sample arrived on {got:?}")]
    KeyMismatch { signed: String, got: String },
    #[error("payload digest does not match prov.sha256")]
    DigestMismatch,
    #[error("key {key:?} is not under the source slot {src:?}")]
    KeyNotUnderSource { key: String, src: String },
    #[error("src {0:?} is not a slot anchor")]
    BadSource(String),
    #[error("device-signed but the signer {signer:?} is not the source {src:?}")]
    SignerNotSource { signer: String, src: String },
    #[error("{0} requires via")]
    ViaMissing(&'static str),
    #[error("device-signed must not carry via")]
    ViaUnexpected,
    #[error("prov.signer {signer:?} is not the certificate CN {cn:?}")]
    SignerCnMismatch { signer: String, cn: String },
    #[error("prov.ts {0:?} is not an RFC 3339 time")]
    BadTime(String),
    #[error("signed at {0} outside the signer certificate's validity")]
    OutsideValidity(String),
    #[error("signature does not verify")]
    BadSignature,
    #[error("signer: {0}")]
    Signer(String),
}

/// The anchor a key is written under: the first segment, or the second for
/// `volatile/<anchor>/…` (ADR 1032).
pub fn key_anchor(key: &str) -> Option<&str> {
    let mut parts = key.split('/');
    let first = parts.next()?;
    if first == "volatile" {
        parts.next()
    } else {
        Some(first)
    }
}

/// The source slot of an interim pod key `<pod-slot>/routed/<source-slot>/…`
/// (design §2), volatile-aware.
pub fn routed_source(key: &str) -> Option<&str> {
    let rest = key.strip_prefix("volatile/").unwrap_or(key);
    let mut parts = rest.split('/');
    let pod = parts.next()?;
    if !is_anchor(pod) || parts.next()? != "routed" {
        return None;
    }
    parts.next().filter(|s| is_anchor(s))
}

fn key_is_under(key: &str, src: &str) -> bool {
    key_anchor(key) == Some(src) || routed_source(key) == Some(src)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// SHA-256 of a payload, lowercase hex.
pub fn payload_digest(payload: &[u8]) -> String {
    hex(&Sha256::digest(payload))
}

/// The bytes a signature covers: the prefix, then the canonical form of the
/// `prov` object without `sig`.
pub fn signed_bytes(prov_without_sig: &Value) -> Vec<u8> {
    let mut out = SIG_PREFIX.to_vec();
    out.extend_from_slice(&canonical::to_vec(prov_without_sig));
    out
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

/// Build and sign `prov` for a payload about to be published on `key`.
///
/// Refuses what a verifier would refuse: a device-signed sample whose source
/// is not the signer's own slot, a key outside the source's slot, `via` on the
/// wrong grade.
pub fn sign(
    signer: &Signer,
    grade: SignedGrade,
    src: &str,
    key: &str,
    payload: &[u8],
    via: Option<Via>,
) -> Result<Prov, ProvError> {
    sign_at(signer, grade, src, key, payload, via, &now_rfc3339())
}

/// [`sign`] with an explicit timestamp.
pub fn sign_at(
    signer: &Signer,
    grade: SignedGrade,
    src: &str,
    key: &str,
    payload: &[u8],
    via: Option<Via>,
    ts: &str,
) -> Result<Prov, ProvError> {
    let mut prov = Prov {
        v: VERSION,
        grade: grade.as_str().to_string(),
        src: src.to_string(),
        signer: signer.cn.clone(),
        ts: ts.to_string(),
        key: key.to_string(),
        sha256: payload_digest(payload),
        via,
        sig: String::new(),
    };
    check_semantics(&prov, grade)?;
    let mut v = serde_json::to_value(&prov).map_err(|e| ProvError::Malformed(e.to_string()))?;
    if let Value::Object(m) = &mut v {
        m.remove("sig");
    }
    let sig: Signature = signer.signing_key().sign(&signed_bytes(&v));
    prov.sig = base64::engine::general_purpose::STANDARD.encode(sig.to_der().as_bytes());
    Ok(prov)
}

fn check_semantics(p: &Prov, grade: SignedGrade) -> Result<(), ProvError> {
    if !is_anchor(&p.src) {
        return Err(ProvError::BadSource(p.src.clone()));
    }
    if !key_is_under(&p.key, &p.src) {
        return Err(ProvError::KeyNotUnderSource {
            key: p.key.clone(),
            src: p.src.clone(),
        });
    }
    match grade {
        SignedGrade::DeviceSigned => {
            if p.via.is_some() {
                return Err(ProvError::ViaUnexpected);
            }
            if anchor_of_cn(&p.signer) != Some(p.src.as_str()) {
                return Err(ProvError::SignerNotSource {
                    signer: p.signer.clone(),
                    src: p.src.clone(),
                });
            }
        }
        SignedGrade::AdapterAttested => {
            if p.via.is_none() {
                return Err(ProvError::ViaMissing("adapter-attested"));
            }
        }
    }
    Ok(())
}

/// Put `prov` into a sample attachment, keeping any other keys an existing
/// attachment holds. The result is canonical JSON.
pub fn attach(existing: Option<&[u8]>, prov: &Prov) -> Result<Vec<u8>, ProvError> {
    let mut obj = match existing {
        None => Map::new(),
        Some([]) => Map::new(),
        Some(b) => match serde_json::from_slice::<Value>(b) {
            Ok(Value::Object(m)) => m,
            Ok(_) => return Err(ProvError::AttachmentNotObject("not an object".into())),
            Err(e) => return Err(ProvError::AttachmentNotObject(e.to_string())),
        },
    };
    let pv = serde_json::to_value(prov).map_err(|e| ProvError::Malformed(e.to_string()))?;
    obj.insert(ATTACHMENT_KEY.to_string(), pv);
    Ok(canonical::to_vec(&Value::Object(obj)))
}

/// What a sample's attachment says about provenance.
#[derive(Debug, PartialEq)]
pub enum Extracted {
    /// No attachment, or one without a `prov` key.
    Absent,
    /// An attachment that is not a JSON object, or a `prov` that is not one.
    Malformed(String),
    /// The raw `prov` object, exactly as received.
    Present(Value),
}

pub fn extract(attachment: Option<&[u8]>) -> Extracted {
    let Some(bytes) = attachment.filter(|b| !b.is_empty()) else {
        return Extracted::Absent;
    };
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(mut m)) => match m.remove(ATTACHMENT_KEY) {
            None => Extracted::Absent,
            Some(v @ Value::Object(_)) => Extracted::Present(v),
            Some(_) => Extracted::Malformed("prov is not an object".into()),
        },
        // An attachment some other component wrote in another format carries
        // no provenance; it is not a broken one.
        Ok(_) | Err(_) => Extracted::Absent,
    }
}

/// Parse the typed fields of a raw `prov` without verifying anything.
pub fn parse(raw: &Value) -> Result<Prov, ProvError> {
    let p: Prov =
        serde_json::from_value(raw.clone()).map_err(|e| ProvError::Malformed(e.to_string()))?;
    if p.v != VERSION {
        return Err(ProvError::Version(p.v));
    }
    Ok(p)
}

/// Verify a raw `prov` against the sample it arrived with and the signer's
/// (already chain-checked) certificate.
///
/// The signature is checked over the object as received, minus `sig`, so a
/// field this build does not know is still covered.
pub fn verify(
    raw: &Value,
    sample_key: &str,
    payload: &[u8],
    cert: &SignerCert,
) -> Result<Prov, ProvError> {
    let p = parse(raw)?;
    let grade =
        SignedGrade::parse(&p.grade).ok_or_else(|| ProvError::UnknownGrade(p.grade.clone()))?;
    if p.signer != cert.cn {
        return Err(ProvError::SignerCnMismatch {
            signer: p.signer.clone(),
            cn: cert.cn.clone(),
        });
    }
    if p.key != sample_key {
        return Err(ProvError::KeyMismatch {
            signed: p.key.clone(),
            got: sample_key.to_string(),
        });
    }
    if p.sha256 != payload_digest(payload) {
        return Err(ProvError::DigestMismatch);
    }
    check_semantics(&p, grade)?;
    let ts = chrono::DateTime::parse_from_rfc3339(&p.ts)
        .map_err(|_| ProvError::BadTime(p.ts.clone()))?
        .timestamp();
    if ts < cert.not_before || ts > cert.not_after {
        return Err(ProvError::OutsideValidity(p.ts.clone()));
    }
    let der = base64::engine::general_purpose::STANDARD
        .decode(p.sig.as_bytes())
        .map_err(|_| ProvError::BadSignature)?;
    let sig = Signature::from_der(&der).map_err(|_| ProvError::BadSignature)?;
    let mut unsigned = raw.clone();
    if let Value::Object(m) = &mut unsigned {
        m.remove("sig");
    }
    cert.vk
        .verify(&signed_bytes(&unsigned), &sig)
        .map_err(|_| ProvError::BadSignature)?;
    Ok(p)
}

/// A sample's provenance grade (§3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grade {
    DeviceSigned {
        signer: String,
    },
    AdapterAttested {
        signer: String,
        via: Via,
    },
    /// No signature; the sample arrived through a pod.
    PodVouchedOnly,
    /// No signature, no pod.
    Unsigned,
    /// A `prov` is present and did not verify.
    Invalid(String),
}

impl Grade {
    /// One short label for a human line.
    pub fn label(&self) -> String {
        match self {
            Grade::DeviceSigned { signer } => format!("device-signed by {signer}"),
            Grade::AdapterAttested { signer, via } => format!(
                "adapter-attested by {signer} ({} port {} from {})",
                via.iface, via.port, via.from
            ),
            Grade::PodVouchedOnly => "pod-vouched only".to_string(),
            Grade::Unsigned => "unsigned".to_string(),
            Grade::Invalid(why) => format!("INVALID: {why}"),
        }
    }

    /// The grade word alone, for machine output.
    pub fn word(&self) -> &'static str {
        match self {
            Grade::DeviceSigned { .. } => "device-signed",
            Grade::AdapterAttested { .. } => "adapter-attested",
            Grade::PodVouchedOnly => "pod-vouched-only",
            Grade::Unsigned => "unsigned",
            Grade::Invalid(_) => "invalid",
        }
    }

    /// The grade of a verified `prov`.
    pub fn of_verified(p: &Prov) -> Grade {
        match (SignedGrade::parse(&p.grade), &p.via) {
            (Some(SignedGrade::AdapterAttested), Some(via)) => Grade::AdapterAttested {
                signer: p.signer.clone(),
                via: via.clone(),
            },
            _ => Grade::DeviceSigned {
                signer: p.signer.clone(),
            },
        }
    }

    /// The grade of a sample without `prov`. Through a pod means the caller
    /// knows it (`through_pod`) or the key is an interim
    /// `<pod-slot>/routed/<source-slot>/…` key.
    pub fn without_prov(key: &str, through_pod: bool) -> Grade {
        if through_pod || routed_source(key).is_some() {
            Grade::PodVouchedOnly
        } else {
            Grade::Unsigned
        }
    }
}
