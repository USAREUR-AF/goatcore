// Live behavioral probe for the deployed goat-relay instances.
// Drives the real WS/Auth handshake (reusing goat-relay-protocol's Auth frame)
// against a live relay, keyed with the relay's shared secret, and classifies
// each case as ACCEPT (server returns a valid AuthResponse) or REJECT
// (connection closed / error / timeout). Self-signed first-light cert → the
// TLS verifier accepts any server cert (this is a probe, not a peer).
//
// Usage: RELAY_SECRET=<hex/str> cargo run -- <host> [<host> ...]

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use goat_relay_protocol::messages::auth::{Auth, AuthResponse};
use goat_relay_protocol::messages::{PeerId, PEER_ID_SIZE};
use tokio_tungstenite::tungstenite::Message;

const RELAY_PORT: u16 = 33073;

fn peer_id(byte: u8) -> [u8; PEER_ID_SIZE] {
    let mut b = [0u8; PEER_ID_SIZE];
    b[..4].copy_from_slice(b"sha-");
    b[4..].fill(byte);
    b
}

/// Build a relay auth token. algo 1 = unbound HMAC(secret, payload);
/// algo 2 = peer-bound HMAC(secret, peer_id || payload). Mirrors
/// goat-relay's generate_test_token / generate_test_token_bound.
fn make_token(secret: &[u8], algo: u8, bind_peer: Option<&[u8; PEER_ID_SIZE]>, expiry: i64) -> Vec<u8> {
    use ring::{digest, hmac};
    let hashed = digest::digest(&digest::SHA256, secret);
    let key = hmac::Key::new(hmac::HMAC_SHA256, hashed.as_ref());
    let payload = expiry.to_string().into_bytes();
    let sig = if let Some(pid) = bind_peer {
        let mut msg = Vec::with_capacity(PEER_ID_SIZE + payload.len());
        msg.extend_from_slice(pid);
        msg.extend_from_slice(&payload);
        hmac::sign(&key, &msg)
    } else {
        hmac::sign(&key, &payload)
    };
    let mut t = Vec::with_capacity(1 + 32 + payload.len());
    t.push(algo);
    t.extend_from_slice(sig.as_ref());
    t.extend_from_slice(&payload);
    t
}

#[derive(Debug)]
struct NoVerify(Arc<rustls::crypto::CryptoProvider>);
impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _e: &rustls::pki_types::CertificateDer<'_>,
        _i: &[rustls::pki_types::CertificateDer<'_>],
        _s: &rustls::pki_types::ServerName<'_>,
        _o: &[u8],
        _n: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &rustls::pki_types::CertificateDer<'_>,
        d: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &rustls::pki_types::CertificateDer<'_>,
        d: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Returns Ok(true) if the relay ACCEPTED (valid AuthResponse), Ok(false) if it
/// REJECTED (close/error/timeout/no valid response).
async fn probe(host: &str, frame: Vec<u8>) -> Result<bool, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("tls versions: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
        .with_no_client_auth();

    let tcp = tokio::net::TcpStream::connect((host, RELAY_PORT))
        .await
        .map_err(|e| format!("tcp: {e}"))?;
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|e| format!("server name: {e}"))?;
    let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(server_name, tcp)
        .await
        .map_err(|e| format!("tls: {e}"))?;
    let url = format!("wss://{host}:{RELAY_PORT}/relay");
    let (mut ws, _) = tokio_tungstenite::client_async(url, tls)
        .await
        .map_err(|e| format!("ws upgrade: {e}"))?;

    ws.send(Message::Binary(frame))
        .await
        .map_err(|e| format!("send: {e}"))?;

    match tokio::time::timeout(Duration::from_secs(6), ws.next()).await {
        Ok(Some(Ok(Message::Binary(b)))) => Ok(AuthResponse::parse(&b).is_ok()),
        Ok(Some(Ok(Message::Close(_)))) => Ok(false),
        Ok(Some(Ok(_other))) => Ok(false),
        Ok(Some(Err(_))) => Ok(false), // server closed the connection (reject)
        Ok(None) => Ok(false),
        Err(_) => Ok(false), // timeout → no AuthResponse → reject
    }
}

#[tokio::main]
async fn main() {
    let secret = std::env::var("RELAY_SECRET").expect("set RELAY_SECRET");
    let secret = secret.as_bytes();
    let hosts: Vec<String> = std::env::args().skip(1).collect();
    if hosts.is_empty() {
        eprintln!("usage: RELAY_SECRET=<secret> relay-auth-probe <relay-host> [<relay-host> ...]");
        std::process::exit(2);
    }

    let expiry = (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64) + 3600;
    let peer_a = peer_id(0xAA);
    let peer_b = peer_id(0xBB);

    // (label, frame, expected_accept)
    let cases: Vec<(&str, Vec<u8>, bool)> = vec![
        (
            "valid algo-1 (unbound)            -> ACCEPT",
            Auth::new(PeerId(peer_a), Bytes::from(make_token(secret, 1, None, expiry))).unwrap().marshal(),
            true,
        ),
        (
            "valid algo-2 bound to self        -> ACCEPT",
            Auth::new(PeerId(peer_a), Bytes::from(make_token(secret, 2, Some(&peer_a), expiry))).unwrap().marshal(),
            true,
        ),
        (
            "algo-2 bound to A, claims B (imp.)-> REJECT",
            Auth::new(PeerId(peer_b), Bytes::from(make_token(secret, 2, Some(&peer_a), expiry))).unwrap().marshal(),
            false,
        ),
        (
            "algo-2 wrong secret               -> REJECT",
            Auth::new(PeerId(peer_a), Bytes::from(make_token(b"wrong-secret-not-this-relay", 2, Some(&peer_a), expiry))).unwrap().marshal(),
            false,
        ),
        (
            "garbage token                     -> REJECT",
            Auth::new(PeerId(peer_a), Bytes::from(vec![2u8, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 9, 9, 9])).unwrap().marshal(),
            false,
        ),
    ];

    let mut all_pass = true;
    for host in &hosts {
        println!("\n=== relay {host}:{RELAY_PORT} ===");
        for (label, frame, expect_accept) in &cases {
            let got = probe(host, frame.clone()).await;
            let (accepted, note) = match got {
                Ok(a) => (a, String::new()),
                Err(e) => (false, format!(" (probe error: {e})")),
            };
            let pass = accepted == *expect_accept;
            if !pass { all_pass = false; }
            println!(
                "  [{}] {label}  (got {}){note}",
                if pass { "PASS" } else { "FAIL" },
                if accepted { "ACCEPT" } else { "REJECT" },
            );
        }
    }
    println!("\n{}", if all_pass { "ALL PASS" } else { "SOME FAILED" });
    if !all_pass { std::process::exit(1); }
}
