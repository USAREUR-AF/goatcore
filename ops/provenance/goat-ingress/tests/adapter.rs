//! The adapter end to end on loopback: UDP frames from the declared sender
//! and from another sender go in; a local zenoh session receives what comes
//! out, and every sample is graded by the same verifier `goat sub
//! --provenance` uses. Keys and certificates are generated per run.

use goat_ingress::{run_source, Source, SourceConfig, Stats};
use goat_provenance::{cert_key, Grade, Signer, TrustRoots, Verifier, Via};
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

const ADAPTER: &str = "fedcba9876543210fedcba9876543210";
const KEYLESS: &str = "00112233445566778899aabbccddeeff";

fn block(cat: u8, body: &[u8]) -> Vec<u8> {
    let len = (body.len() + 3) as u16;
    let mut b = vec![cat];
    b.extend_from_slice(&len.to_be_bytes());
    b.extend_from_slice(body);
    b
}

fn pki() -> (Signer, TrustRoots) {
    let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.distinguished_name
        .push(DnType::CommonName, "zagros test CA");
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = p.self_signed(&ca_key).unwrap();
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut lp = CertificateParams::new(Vec::<String>::new()).unwrap();
    lp.distinguished_name
        .push(DnType::CommonName, format!("{ADAPTER}.g1"));
    let leaf = lp.signed_by(&key, &ca, &ca_key).unwrap();
    (
        Signer::from_pem(&leaf.pem(), &key.serialize_pem()).unwrap(),
        TrustRoots::from_pem(ca.pem().as_bytes()).unwrap(),
    )
}

async fn local_session() -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"peer\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("listen/endpoints", "[]").unwrap();
    zenoh::open(c).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn frames_from_the_declared_sender_are_published_attested_and_others_are_dropped() {
    let (signer, roots) = pki();
    let session = local_session().await;

    // Stand in for the router's storage of the adapter's published certificate.
    let pem = signer.cert_pem.clone();
    let ck = cert_key(ADAPTER);
    let q = session.declare_queryable(ck.clone()).await.unwrap();
    tokio::spawn(async move {
        while let Ok(query) = q.recv_async().await {
            let _ = query.reply(ck.clone(), pem.clone().into_bytes()).await;
        }
    });

    let sender = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let stranger = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let adapter_sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let listen = adapter_sock.local_addr().unwrap();

    let src = Source::from_config(&SourceConfig {
        slot: KEYLESS.into(),
        listen: listen.to_string(),
        interface: "udp".into(),
        // The declared sender is one address AND port, so a second socket on
        // the same loopback address is a different sender.
        sender: sender.local_addr().unwrap().to_string(),
        format: "asterix:034,048".into(),
        volatile: false,
    })
    .unwrap();

    let sub = session
        .declare_subscriber(format!("{KEYLESS}/stream/asterix/**"))
        .await
        .unwrap();

    let stats = Arc::new(Stats::default());
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(run_source(
        session.clone(),
        Arc::new(signer),
        src,
        adapter_sock,
        stats.clone(),
        stop_rx,
    ));

    let cat48 = block(48, &[0xfd, 0x01, 0x02]);
    let mixed = [block(34, &[0x01]), block(48, &[0x02, 0x03])].concat();
    sender.send_to(&cat48, listen).await.unwrap();
    stranger.send_to(&block(48, &[0x66]), listen).await.unwrap();
    sender.send_to(&[48, 0, 9, 1], listen).await.unwrap(); // bad framing
    sender.send_to(&block(62, &[0x01]), listen).await.unwrap(); // undeclared category
    sender.send_to(&mixed, listen).await.unwrap();

    let mut got = Vec::new();
    while got.len() < 3 {
        let s = tokio::time::timeout(Duration::from_secs(5), sub.recv_async())
            .await
            .expect("timed out waiting for samples")
            .unwrap();
        got.push((
            s.key_expr().as_str().to_string(),
            s.payload().to_bytes().to_vec(),
            s.attachment().map(|a| a.to_bytes().to_vec()),
        ));
    }
    // Nothing else arrives: the stranger's frame and the two bad frames were dropped.
    assert!(
        tokio::time::timeout(Duration::from_millis(500), sub.recv_async())
            .await
            .is_err()
    );

    let k48 = format!("{KEYLESS}/stream/asterix/048/raw/v1");
    let k34 = format!("{KEYLESS}/stream/asterix/034/raw/v1");
    assert_eq!(got[0].0, k48);
    assert_eq!(got[0].1, cat48);
    assert_eq!(got[1].0, k34);
    assert_eq!(got[1].1, block(34, &[0x01]));
    assert_eq!(got[2].0, k48);
    assert_eq!(got[2].1, block(48, &[0x02, 0x03]));

    let mut verifier = Verifier::new(session.clone(), roots);
    let via = Via {
        iface: "udp".into(),
        port: listen.port(),
        from: sender.local_addr().unwrap().to_string(),
    };
    for (key, payload, att) in &got {
        let g = verifier.grade(key, payload, att.as_deref()).await;
        assert_eq!(
            g,
            Grade::AdapterAttested {
                signer: format!("{ADAPTER}.g1"),
                via: via.clone()
            },
            "{key}"
        );
    }
    // A frame altered in transit grades INVALID.
    let (key, mut payload, att) = got[0].clone();
    payload[3] ^= 0xff;
    assert!(matches!(
        verifier.grade(&key, &payload, att.as_deref()).await,
        Grade::Invalid(_)
    ));

    assert_eq!(stats.published.load(Ordering::Relaxed), 3);
    assert_eq!(stats.wrong_sender.load(Ordering::Relaxed), 1);
    assert_eq!(stats.lint_dropped.load(Ordering::Relaxed), 2);

    stop_tx.send(true).unwrap();
    task.await.unwrap().unwrap();
    session.close().await.unwrap();
}
