//! goat-provenance: per-sample provenance, `prov` v1 (ADR 1251).
//!
//! Contract: docs/design/pod-carriage-and-provenance.md §7.2. The signature
//! rides in the zenoh sample attachment under the key `prov`, is added once
//! where data enters goat, and is forwarded unchanged by pods and routers.
//!
//! - [`canonical`]: the JSON form signatures cover.
//! - [`cert`]: the goatnet CA roots, signer certificates, the signing key.
//! - [`prov`]: build, sign, attach, extract, verify, grade.
//! - [`fabric`]: certificate publication and fetch on
//!   `<anchor>/_goat/prov/cert`, the cache, and the subscriber's grader.

pub mod canonical;
pub mod cert;
pub mod fabric;
pub mod prov;

pub use cert::{check_signer_cert, CertError, Signer, SignerCert, TrustRoots};
pub use fabric::{cert_key, fetch_cert, publish_cert, CertCache, Verifier};
pub use prov::{
    attach, extract, sign, verify, Extracted, Grade, Prov, ProvError, SignedGrade, Via,
};
