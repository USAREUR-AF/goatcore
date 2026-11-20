// SPDX-License-Identifier: Apache-2.0
//
// A TLS client configuration for talking HTTPS to a goat peer that serves
// its own goat leaf certificate, such as a pod's local API (ADR 1251,
// docs/design/pod-carriage-and-provenance.md §7.4). Such a leaf names the
// peer by its slot (CN, URI SAN), not by the address dialled (127.0.0.1, a
// mesh address), so ordinary hostname verification cannot pass. This
// verifier keeps every other check — the chain to the goatnet CA roots,
// validity, signatures — and replaces the hostname check with a caller's
// check of the leaf itself (for `goat pod`: its CN is the pod's slot), the
// same way the fabric authenticates on the certificate, not a hostname.
//
// The client presents its own goat leaf (client-certificate sign-in).
// Hand the result to `reqwest::ClientBuilder::use_preconfigured_tls`.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error, RootCertStore, SignatureScheme};

/// The TLS library these types come from, for a caller (or a test) that
/// needs to name them.
pub use rustls;

/// Checks the server's leaf (DER) once its chain has verified; `Err` names
/// why the leaf is not the peer the caller meant to reach.
pub type LeafCheck = Arc<dyn Fn(&[u8]) -> std::result::Result<(), String> + Send + Sync>;

#[derive(Debug)]
struct ChainThenLeaf {
    inner: Arc<WebPkiServerVerifier>,
    check: LeafCheckDebug,
}

struct LeafCheckDebug(LeafCheck);

impl std::fmt::Debug for LeafCheckDebug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LeafCheck")
    }
}

impl ServerCertVerifier for ChainThenLeaf {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, Error> {
        // webpki checks the chain before the name, so a name mismatch means
        // the chain, validity and usage all passed.
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(_)
            | Err(Error::InvalidCertificate(CertificateError::NotValidForName))
            | Err(Error::InvalidCertificate(CertificateError::NotValidForNameContext { .. })) => {}
            Err(e) => return Err(e),
        }
        (self.check.0)(end_entity.as_ref()).map_err(Error::General)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// Build the client configuration: trust `roots_pem` (every certificate in
/// it), sign in with `cert_pem` + `key_pem`, and accept the server's leaf
/// when it chains to a root and `check` accepts it.
pub fn client_config(
    roots_pem: &[u8],
    cert_pem: &[u8],
    key_pem: &[u8],
    check: LeafCheck,
) -> Result<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut roots = RootCertStore::empty();
    for c in CertificateDer::pem_slice_iter(roots_pem) {
        roots
            .add(c.context("read a CA root")?)
            .context("add a CA root")?;
    }
    if roots.is_empty() {
        return Err(anyhow!("no CA root certificate in the trust file"));
    }
    let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .context("build the certificate verifier")?;
    let certs = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("read the client certificate")?;
    if certs.is_empty() {
        return Err(anyhow!("no certificate in the client certificate file"));
    }
    let key = PrivateKeyDer::from_pem_slice(key_pem).context("read the client key")?;
    let cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .context("TLS protocol versions")?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(ChainThenLeaf {
            inner,
            check: LeafCheckDebug(check),
        }))
        .with_client_auth_cert(certs, key)
        .context("use the client certificate and key")?;
    Ok(cfg)
}
