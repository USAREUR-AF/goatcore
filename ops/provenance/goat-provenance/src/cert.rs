//! Signer certificates: the goat leaf that signs `prov`, its chain to the
//! goatnet CA, and the signing side that holds the leaf's private key.

use p256::ecdsa::{SigningKey, VerifyingKey};
use std::path::Path;

/// Why a signer certificate or key was refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CertError {
    #[error("not a PEM CERTIFICATE: {0}")]
    NotPem(String),
    #[error("not an X.509 certificate: {0}")]
    NotX509(String),
    #[error("certificate has no common name")]
    NoCn,
    #[error("certificate CN {got:?} is not the signer {want:?}")]
    WrongCn { got: String, want: String },
    #[error("certificate CN {0:?} is not a goat slot leaf (<32 hex>[.g<N>])")]
    NotSlotLeaf(String),
    #[error("certificate does not chain to the goatnet CA")]
    Unchained,
    #[error("certificate key is not ECDSA P-256 (provenance v1 signs with P-256 only)")]
    NotP256,
    #[error("no CA root could be read from the trust bundle")]
    NoRoots,
    #[error("private key: {0}")]
    BadKey(String),
    #[error("private key does not belong to the certificate")]
    KeyMismatch,
    #[error("read {path}: {err}")]
    Io { path: String, err: String },
}

/// The goatnet CA roots a signer certificate must chain to (the profile's
/// `ca-roots.pem`). Every certificate in the bundle is a trust anchor.
#[derive(Clone)]
pub struct TrustRoots {
    ders: Vec<Vec<u8>>,
}

impl TrustRoots {
    /// Read every CERTIFICATE block from a PEM bundle.
    pub fn from_pem(pem_bytes: &[u8]) -> Result<Self, CertError> {
        let blocks = pem::parse_many(pem_bytes).map_err(|e| CertError::NotPem(e.to_string()))?;
        let ders: Vec<Vec<u8>> = blocks
            .into_iter()
            .filter(|b| b.tag() == "CERTIFICATE")
            .map(|b| b.into_contents())
            .filter(|der| x509_parser::parse_x509_certificate(der).is_ok())
            .collect();
        if ders.is_empty() {
            return Err(CertError::NoRoots);
        }
        Ok(Self { ders })
    }

    pub fn from_file(path: &Path) -> Result<Self, CertError> {
        let bytes = std::fs::read(path).map_err(|e| CertError::Io {
            path: path.display().to_string(),
            err: e.to_string(),
        })?;
        Self::from_pem(&bytes)
    }

    fn chains(&self, leaf: &x509_parser::certificate::X509Certificate<'_>) -> bool {
        self.ders.iter().any(|der| {
            let Ok((_, root)) = x509_parser::parse_x509_certificate(der) else {
                return false;
            };
            root.subject() == leaf.issuer()
                && leaf.verify_signature(Some(root.public_key())).is_ok()
        })
    }
}

/// A signer certificate that parsed, chains to the goatnet CA and carries a
/// P-256 key.
#[derive(Clone, Debug)]
pub struct SignerCert {
    pub cn: String,
    pub vk: VerifyingKey,
    /// Validity window, Unix seconds.
    pub not_before: i64,
    pub not_after: i64,
    /// The certificate as published (PEM).
    pub pem: String,
}

fn first_cn(cert: &x509_parser::certificate::X509Certificate<'_>) -> Option<String> {
    cert.subject()
        .iter_common_name()
        .next()
        .and_then(|a| a.as_str().ok())
        .map(str::to_string)
}

/// Parse a signer certificate, check that it chains to `roots`, that its key
/// is P-256 and, when given, that its CN is `expected_cn`.
pub fn check_signer_cert(
    pem_text: &str,
    roots: &TrustRoots,
    expected_cn: Option<&str>,
) -> Result<SignerCert, CertError> {
    let blob = pem::parse(pem_text.as_bytes()).map_err(|e| CertError::NotPem(e.to_string()))?;
    if blob.tag() != "CERTIFICATE" {
        return Err(CertError::NotPem(format!("block is {}", blob.tag())));
    }
    let (_, leaf) = x509_parser::parse_x509_certificate(blob.contents())
        .map_err(|e| CertError::NotX509(e.to_string()))?;
    let cn = first_cn(&leaf).ok_or(CertError::NoCn)?;
    if let Some(want) = expected_cn {
        if cn != want {
            return Err(CertError::WrongCn {
                got: cn,
                want: want.to_string(),
            });
        }
    }
    if !roots.chains(&leaf) {
        return Err(CertError::Unchained);
    }
    let vk = grant_crypto::verifying_key_from_spki(leaf.public_key().raw)
        .map_err(|_| CertError::NotP256)?;
    let v = leaf.validity();
    Ok(SignerCert {
        cn,
        vk,
        not_before: v.not_before.timestamp(),
        not_after: v.not_after.timestamp(),
        pem: pem_text.to_string(),
    })
}

/// The slot anchor of a goat leaf CN: `<32 lowercase hex>` optionally
/// followed by `.g<N>` (the generation). Mirrors
/// `panoscope_core::alias::anchor_of_leaf_cn`.
pub fn anchor_of_cn(cn: &str) -> Option<&str> {
    let (head, gen) = match cn.split_once(".g") {
        Some((h, g)) => (h, Some(g)),
        None => (cn, None),
    };
    if !is_anchor(head) {
        return None;
    }
    match gen {
        None => Some(head),
        Some(g)
            if !g.is_empty()
                && g.len() <= 18
                && !g.starts_with('0')
                && g.bytes().all(|b| b.is_ascii_digit()) =>
        {
            Some(head)
        }
        Some(_) => None,
    }
}

/// True for a 32-character lowercase hex slot anchor.
pub fn is_anchor(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The signing side: a goat leaf certificate and its private key.
pub struct Signer {
    pub cn: String,
    pub anchor: String,
    pub cert_pem: String,
    key: SigningKey,
}

impl Signer {
    /// Build a signer from PEM text. The key may be PKCS#8 or SEC1 and must be
    /// the certificate's own P-256 key; the CN must be a slot leaf.
    pub fn from_pem(cert_pem: &str, key_pem: &str) -> Result<Self, CertError> {
        use p256::pkcs8::DecodePrivateKey;
        let blob = pem::parse(cert_pem.as_bytes()).map_err(|e| CertError::NotPem(e.to_string()))?;
        let (_, leaf) = x509_parser::parse_x509_certificate(blob.contents())
            .map_err(|e| CertError::NotX509(e.to_string()))?;
        let cn = first_cn(&leaf).ok_or(CertError::NoCn)?;
        let anchor = anchor_of_cn(&cn)
            .ok_or_else(|| CertError::NotSlotLeaf(cn.clone()))?
            .to_string();
        let cert_vk = grant_crypto::verifying_key_from_spki(leaf.public_key().raw)
            .map_err(|_| CertError::NotP256)?;
        let secret = p256::SecretKey::from_pkcs8_pem(key_pem)
            .or_else(|_| p256::SecretKey::from_sec1_pem(key_pem))
            .map_err(|e| CertError::BadKey(e.to_string()))?;
        let key = SigningKey::from(secret);
        if *key.verifying_key() != cert_vk {
            return Err(CertError::KeyMismatch);
        }
        Ok(Self {
            cn,
            anchor,
            cert_pem: cert_pem.to_string(),
            key,
        })
    }

    pub fn from_files(cert: &Path, key: &Path) -> Result<Self, CertError> {
        let read = |p: &Path| {
            std::fs::read_to_string(p).map_err(|e| CertError::Io {
                path: p.display().to_string(),
                err: e.to_string(),
            })
        };
        Self::from_pem(&read(cert)?, &read(key)?)
    }

    pub(crate) fn signing_key(&self) -> &SigningKey {
        &self.key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_of_cn_accepts_slot_leaves_only() {
        let a = "0123456789abcdef0123456789abcdef";
        assert_eq!(anchor_of_cn(a), Some(a));
        assert_eq!(anchor_of_cn(&format!("{a}.g3")), Some(a));
        assert_eq!(anchor_of_cn(&format!("{a}.g03")), None);
        assert_eq!(anchor_of_cn(&format!("{a}.g")), None);
        assert_eq!(anchor_of_cn("acme-trial-peer"), None);
        assert_eq!(anchor_of_cn(&a.to_uppercase()), None);
    }
}
