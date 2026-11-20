//! Sign/verify round trip, tamper detection and certificate checks for
//! `prov` v1. Keys and certificates are generated per test run.

use goat_provenance::prov::{self, sign_at};
use goat_provenance::*;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256};
use serde_json::Value;

const DEV: &str = "0123456789abcdef0123456789abcdef";
const ADAPTER: &str = "fedcba9876543210fedcba9876543210";
const KEYLESS: &str = "00112233445566778899aabbccddeeff";
const TS: &str = "2026-11-19T10:00:00.123456789Z";

struct Ca {
    cert: rcgen::Certificate,
    key: KeyPair,
}

fn ca() -> Ca {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.distinguished_name
        .push(DnType::CommonName, "zagros test CA");
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let cert = p.self_signed(&key).unwrap();
    Ca { cert, key }
}

fn leaf(ca: &Ca, cn: &str) -> (String, String) {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.distinguished_name.push(DnType::CommonName, cn);
    p.not_before = rcgen::date_time_ymd(2025, 1, 1);
    p.not_after = rcgen::date_time_ymd(2030, 1, 1);
    let cert = p.signed_by(&key, &ca.cert, &ca.key).unwrap();
    (cert.pem(), key.serialize_pem())
}

fn setup(cn: &str) -> (Ca, Signer, SignerCert, TrustRoots) {
    let ca = ca();
    let (cert_pem, key_pem) = leaf(&ca, cn);
    let signer = Signer::from_pem(&cert_pem, &key_pem).unwrap();
    let roots = TrustRoots::from_pem(ca.cert.pem().as_bytes()).unwrap();
    let checked = check_signer_cert(&cert_pem, &roots, Some(cn)).unwrap();
    (ca, signer, checked, roots)
}

type Edit = Box<dyn Fn(&mut Value)>;

fn raw(p: &Prov) -> Value {
    serde_json::to_value(p).unwrap()
}

fn via() -> Via {
    Via {
        iface: "udp".into(),
        port: 30048,
        from: "192.0.2.10:5000".into(),
    }
}

#[test]
fn device_signed_round_trip() {
    let cn = format!("{DEV}.g1");
    let (_ca, signer, cert, _) = setup(&cn);
    let key = format!("{DEV}/observations/v1");
    let p = sign_at(
        &signer,
        SignedGrade::DeviceSigned,
        DEV,
        &key,
        b"hello",
        None,
        TS,
    )
    .unwrap();
    let got = verify(&raw(&p), &key, b"hello", &cert).unwrap();
    assert_eq!(Grade::of_verified(&got), Grade::DeviceSigned { signer: cn });
}

#[test]
fn adapter_attested_round_trip_names_the_source_and_how_it_arrived() {
    let cn = format!("{ADAPTER}.g2");
    let (_ca, signer, cert, _) = setup(&cn);
    let key = format!("{KEYLESS}/stream/asterix/048/raw/v1");
    let p = sign_at(
        &signer,
        SignedGrade::AdapterAttested,
        KEYLESS,
        &key,
        b"\x30\x00\x03",
        Some(via()),
        TS,
    )
    .unwrap();
    let got = verify(&raw(&p), &key, b"\x30\x00\x03", &cert).unwrap();
    assert_eq!(got.src, KEYLESS);
    assert_eq!(
        Grade::of_verified(&got),
        Grade::AdapterAttested {
            signer: cn,
            via: via()
        }
    );
}

#[test]
fn the_attachment_keeps_other_keys_and_round_trips() {
    let (_ca, signer, cert, _) = setup(DEV);
    let key = format!("volatile/{DEV}/x");
    let p = sign_at(
        &signer,
        SignedGrade::DeviceSigned,
        DEV,
        &key,
        b"p",
        None,
        TS,
    )
    .unwrap();
    let att = attach(Some(br#"{"other":{"n":1}}"#), &p).unwrap();
    let v: Value = serde_json::from_slice(&att).unwrap();
    assert_eq!(v["other"]["n"], 1);
    match extract(Some(&att)) {
        Extracted::Present(raw) => {
            verify(&raw, &key, b"p", &cert).unwrap();
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(extract(None), Extracted::Absent);
    assert_eq!(extract(Some(b"not json")), Extracted::Absent);
    assert!(matches!(
        extract(Some(br#"{"prov":3}"#)),
        Extracted::Malformed(_)
    ));
}

#[test]
fn a_changed_payload_is_refused() {
    let (_ca, signer, cert, _) = setup(DEV);
    let key = format!("{DEV}/k");
    let p = sign_at(
        &signer,
        SignedGrade::DeviceSigned,
        DEV,
        &key,
        b"one",
        None,
        TS,
    )
    .unwrap();
    assert_eq!(
        verify(&raw(&p), &key, b"two", &cert),
        Err(ProvError::DigestMismatch)
    );
}

#[test]
fn a_sample_moved_to_another_key_is_refused() {
    let (_ca, signer, cert, _) = setup(DEV);
    let key = format!("{DEV}/k");
    let p = sign_at(
        &signer,
        SignedGrade::DeviceSigned,
        DEV,
        &key,
        b"x",
        None,
        TS,
    )
    .unwrap();
    let other = format!("{DEV}/elsewhere");
    assert!(matches!(
        verify(&raw(&p), &other, b"x", &cert),
        Err(ProvError::KeyMismatch { .. })
    ));
}

#[test]
fn every_signed_field_is_covered() {
    let cn = format!("{ADAPTER}.g2");
    let (_ca, signer, cert, _) = setup(&cn);
    let key = format!("{KEYLESS}/stream/asterix/048/raw/v1");
    let p = sign_at(
        &signer,
        SignedGrade::AdapterAttested,
        KEYLESS,
        &key,
        b"f",
        Some(via()),
        TS,
    )
    .unwrap();
    // Each edit keeps the other checks satisfied so only the signature can catch it.
    let edits: Vec<(&str, Edit)> = vec![
        ("ts", Box::new(|v| v["ts"] = "2026-11-19T10:00:01Z".into())),
        (
            "via.from",
            Box::new(|v| v["via"]["from"] = "198.51.100.7:5000".into()),
        ),
        ("via.port", Box::new(|v| v["via"]["port"] = 30049.into())),
        ("via.iface", Box::new(|v| v["via"]["iface"] = "eth1".into())),
        ("added field", Box::new(|v| v["note"] = "x".into())),
    ];
    for (what, edit) in edits {
        let mut v = raw(&p);
        edit(&mut v);
        assert_eq!(
            verify(&v, &key, b"f", &cert),
            Err(ProvError::BadSignature),
            "{what}"
        );
    }
    // Swapping the source slot breaks the signature too (src is under the key's slot rule,
    // so move both).
    let other_src = "ffffffffffffffffffffffffffffffff";
    let mut v = raw(&p);
    v["src"] = other_src.into();
    let moved = format!("{other_src}/stream/asterix/048/raw/v1");
    v["key"] = moved.clone().into();
    assert_eq!(
        verify(&v, &moved, b"f", &cert),
        Err(ProvError::BadSignature)
    );
}

#[test]
fn a_signature_from_another_key_is_refused() {
    let (ca, signer, _, roots) = setup(DEV);
    let key = format!("{DEV}/k");
    let p = sign_at(
        &signer,
        SignedGrade::DeviceSigned,
        DEV,
        &key,
        b"x",
        None,
        TS,
    )
    .unwrap();
    // Same CN, same CA, different key pair.
    let (impostor_pem, _) = leaf(&ca, DEV);
    let impostor = check_signer_cert(&impostor_pem, &roots, Some(DEV)).unwrap();
    assert_eq!(
        verify(&raw(&p), &key, b"x", &impostor),
        Err(ProvError::BadSignature)
    );
}

#[test]
fn a_certificate_with_the_wrong_cn_is_refused() {
    let (ca, signer, _, roots) = setup(DEV);
    let other_cn = format!("{ADAPTER}.g1");
    let (other_pem, _) = leaf(&ca, &other_cn);
    // At fetch time: the certificate on the anchor's key is not the signer's.
    assert!(matches!(
        check_signer_cert(&other_pem, &roots, Some(DEV)),
        Err(CertError::WrongCn { .. })
    ));
    // At verify time: prov.signer must equal the certificate CN.
    let key = format!("{DEV}/k");
    let p = sign_at(
        &signer,
        SignedGrade::DeviceSigned,
        DEV,
        &key,
        b"x",
        None,
        TS,
    )
    .unwrap();
    let other = check_signer_cert(&other_pem, &roots, None).unwrap();
    assert!(matches!(
        verify(&raw(&p), &key, b"x", &other),
        Err(ProvError::SignerCnMismatch { .. })
    ));
}

#[test]
fn a_certificate_from_another_ca_is_refused() {
    let (_ca, _, _, roots) = setup(DEV);
    let stranger = ca();
    let (pem, _) = leaf(&stranger, DEV);
    assert_eq!(
        check_signer_cert(&pem, &roots, Some(DEV)).unwrap_err(),
        CertError::Unchained
    );
}

#[test]
fn device_signed_means_the_signer_is_the_source() {
    let (_ca, signer, _, _) = setup(DEV);
    let key = format!("{KEYLESS}/k");
    assert!(matches!(
        sign_at(
            &signer,
            SignedGrade::DeviceSigned,
            KEYLESS,
            &key,
            b"x",
            None,
            TS
        ),
        Err(ProvError::SignerNotSource { .. })
    ));
    // And a key outside the signer's own slot is refused before signing.
    let foreign = format!("{KEYLESS}/k");
    assert!(matches!(
        sign_at(
            &signer,
            SignedGrade::DeviceSigned,
            DEV,
            &foreign,
            b"x",
            None,
            TS
        ),
        Err(ProvError::KeyNotUnderSource { .. })
    ));
}

#[test]
fn grade_rules_for_via() {
    let (_ca, signer, _, _) = setup(DEV);
    let key = format!("{DEV}/k");
    assert_eq!(
        sign_at(
            &signer,
            SignedGrade::DeviceSigned,
            DEV,
            &key,
            b"x",
            Some(via()),
            TS
        )
        .unwrap_err(),
        ProvError::ViaUnexpected
    );
    assert!(matches!(
        sign_at(
            &signer,
            SignedGrade::AdapterAttested,
            DEV,
            &key,
            b"x",
            None,
            TS
        ),
        Err(ProvError::ViaMissing(_))
    ));
}

#[test]
fn signed_outside_the_certificate_validity_is_refused() {
    let (_ca, signer, cert, _) = setup(DEV);
    let key = format!("{DEV}/k");
    let p = sign_at(
        &signer,
        SignedGrade::DeviceSigned,
        DEV,
        &key,
        b"x",
        None,
        "1999-01-01T00:00:00Z",
    )
    .unwrap();
    assert!(matches!(
        verify(&raw(&p), &key, b"x", &cert),
        Err(ProvError::OutsideValidity(_))
    ));
}

#[test]
fn a_key_that_is_not_the_certificates_is_refused_at_load() {
    let ca = ca();
    let (cert_pem, _) = leaf(&ca, DEV);
    let (_, other_key) = leaf(&ca, DEV);
    assert!(matches!(
        Signer::from_pem(&cert_pem, &other_key),
        Err(CertError::KeyMismatch)
    ));
}

#[test]
fn samples_without_prov_grade_by_path() {
    let routed = format!("{ADAPTER}/routed/{KEYLESS}/feed");
    assert_eq!(Grade::without_prov(&routed, false), Grade::PodVouchedOnly);
    assert_eq!(
        Grade::without_prov(&format!("{DEV}/k"), false),
        Grade::Unsigned
    );
    assert_eq!(
        Grade::without_prov(&format!("{DEV}/k"), true),
        Grade::PodVouchedOnly
    );
}

#[test]
fn signed_bytes_are_prefixed_canonical_json() {
    let v = serde_json::json!({"v": 1, "a": "b"});
    assert_eq!(
        prov::signed_bytes(&v),
        b"goat-prov-v1\n{\"a\":\"b\",\"v\":1}".to_vec()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cache_fetches_once_and_remembers_a_miss() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let cn = format!("{DEV}.g1");
    let (_ca, signer, _, roots) = setup(&cn);
    let mut cache = CertCache::new(roots);
    let calls = AtomicUsize::new(0);
    for _ in 0..3 {
        let pem = signer.cert_pem.clone();
        let c = cache
            .get(&cn, |anchor| {
                calls.fetch_add(1, Ordering::SeqCst);
                assert_eq!(anchor, DEV);
                async move { Some(pem) }
            })
            .await
            .unwrap();
        assert_eq!(c.cn, cn);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let absent = format!("{ADAPTER}.g1");
    for _ in 0..3 {
        let err = cache
            .get(&absent, |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                async { None }
            })
            .await
            .unwrap_err();
        assert!(err.contains("_goat/prov/cert"), "{err}");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_verifier_fetches_the_published_certificate_over_zenoh() {
    let cn = format!("{DEV}.g1");
    let (_ca, signer, _, roots) = setup(&cn);
    let session = local_session().await;
    // Stand in for the router's storage: answer queries on the cert key.
    let pem = signer.cert_pem.clone();
    let q = session.declare_queryable(cert_key(DEV)).await.unwrap();
    let qkey = cert_key(DEV);
    tokio::spawn(async move {
        while let Ok(query) = q.recv_async().await {
            let _ = query.reply(qkey.clone(), pem.clone().into_bytes()).await;
        }
    });
    let key = format!("{DEV}/obs");
    let p = sign_at(
        &signer,
        SignedGrade::DeviceSigned,
        DEV,
        &key,
        b"x",
        None,
        TS,
    )
    .unwrap();
    let att = attach(None, &p).unwrap();
    let mut v = Verifier::new(session.clone(), roots);
    assert_eq!(
        v.grade(&key, b"x", Some(&att)).await,
        Grade::DeviceSigned { signer: cn.clone() }
    );
    assert!(matches!(
        v.grade(&key, b"y", Some(&att)).await,
        Grade::Invalid(_)
    ));
    assert_eq!(v.grade(&key, b"x", None).await, Grade::Unsigned);
    session.close().await.unwrap();
}

async fn local_session() -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"peer\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("listen/endpoints", "[]").unwrap();
    zenoh::open(c).await.unwrap()
}
