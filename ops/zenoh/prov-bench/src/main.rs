//! Per-sample cost of the `prov` v1 attachment in goatcore design pod-carriage-and-provenance §7.2.
//!
//! For each payload size it measures, per sample:
//! - sign: SHA-256 of the payload, build `prov` without `sig`, canonical JSON (sorted keys, no
//!   insignificant whitespace), prefix `goat-prov-v1\n`, ECDSA P-256 sign (SHA-256), DER, base64,
//!   and the attachment JSON `{"prov": {...}}`.
//! - verify: parse the attachment, remove `sig`, canonical JSON, prefix, ECDSA verify, SHA-256 of
//!   the payload and compare. Certificate fetch and chain check are cached work and not included.
//!
//! It also prints the attachment size for both grades.
//!
//! Usage: cargo run --release -- [iterations]   (default 2000 per size)

use base64::Engine;
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{DerSignature, SigningKey, VerifyingKey};
use p256::elliptic_curve::Generate;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::time::Instant;

const SLOT: &str = "7f3c2a100000400080000000000000d1";
const PREFIX: &[u8] = b"goat-prov-v1\n";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// serde_json without `preserve_order` keeps object keys in a BTreeMap, so `to_vec` is the
/// canonical form: sorted keys, no insignificant whitespace.
fn canonical(v: &Value) -> Vec<u8> {
    let mut out = PREFIX.to_vec();
    out.extend(serde_json::to_vec(v).expect("serialize"));
    out
}

fn prov_without_sig(payload: &[u8], adapter: bool) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("v".into(), json!(1));
    m.insert(
        "grade".into(),
        json!(if adapter {
            "adapter-attested"
        } else {
            "device-signed"
        }),
    );
    m.insert("src".into(), json!(SLOT));
    m.insert("signer".into(), json!(format!("{SLOT}.g1")));
    m.insert("ts".into(), json!("2026-11-19T10:00:00.123456789Z"));
    m.insert(
        "key".into(),
        json!(format!("{SLOT}/stream/asterix/48/raw/v1")),
    );
    m.insert("sha256".into(), json!(hex(&Sha256::digest(payload))));
    if adapter {
        m.insert(
            "via".into(),
            json!({"iface": "udp", "port": 10009, "from": "192.0.2.10:5000"}),
        );
    }
    m
}

fn sign(key: &SigningKey, payload: &[u8], adapter: bool) -> Vec<u8> {
    let mut prov = prov_without_sig(payload, adapter);
    let sig: DerSignature = key.sign(&canonical(&Value::Object(prov.clone())));
    prov.insert(
        "sig".into(),
        json!(base64::engine::general_purpose::STANDARD.encode(sig.as_bytes())),
    );
    serde_json::to_vec(&json!({ "prov": prov })).expect("serialize")
}

fn verify(vk: &VerifyingKey, payload: &[u8], attachment: &[u8]) -> bool {
    let mut v: Value = serde_json::from_slice(attachment).expect("parse");
    let prov = v
        .get_mut("prov")
        .and_then(Value::as_object_mut)
        .expect("prov");
    let sig_b64 = prov
        .remove("sig")
        .and_then(|s| s.as_str().map(str::to_owned))
        .expect("sig");
    let der = base64::engine::general_purpose::STANDARD
        .decode(sig_b64)
        .expect("b64");
    let sig = DerSignature::try_from(der.as_slice()).expect("der");
    let digest_ok =
        prov.get("sha256").and_then(Value::as_str) == Some(&hex(&Sha256::digest(payload)));
    let msg = canonical(&Value::Object(prov.clone()));
    digest_ok && vk.verify(&msg, &sig).is_ok()
}

fn per_op_us<F: FnMut()>(iters: usize, mut f: F) -> (f64, f64) {
    // Median and p99 of per-iteration time in microseconds.
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t = Instant::now();
        f();
        samples.push(t.elapsed().as_nanos() as f64 / 1000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (samples[iters / 2], samples[(iters * 99) / 100])
}

fn main() {
    let iters: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);
    let key = SigningKey::generate();
    let vk = *key.verifying_key();
    println!("iterations per cell: {iters}");
    println!(
        "{:>8} {:>18} {:>18} {:>18} {:>18}",
        "payload", "sign median us", "sign p99 us", "verify median us", "verify p99 us"
    );
    for size in [64usize, 1024, 65536] {
        let payload: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        for _ in 0..200 {
            let a = sign(&key, &payload, false);
            assert!(verify(&vk, &payload, &a));
        }
        let (sm, sp) = per_op_us(iters, || {
            std::hint::black_box(sign(&key, &payload, false));
        });
        let att = sign(&key, &payload, false);
        let (vm, vp) = per_op_us(iters, || {
            assert!(std::hint::black_box(verify(&vk, &payload, &att)));
        });
        let (hm, _) = per_op_us(iters, || {
            std::hint::black_box(Sha256::digest(&payload));
        });
        println!("{size:>8} {sm:>18.1} {sp:>18.1} {vm:>18.1} {vp:>18.1}   (sha256 alone median {hm:.1} us)");
    }
    let mut tampered = sign(&key, b"payload", false);
    assert!(verify(&vk, b"payload", &tampered));
    assert!(
        !verify(&vk, b"payloaX", &tampered),
        "payload tamper must fail"
    );
    let text = String::from_utf8(tampered.clone()).expect("utf8");
    assert!(text.contains("\"v\":1"));
    tampered = text.replacen("\"v\":1", "\"v\":2", 1).into_bytes();
    assert!(
        !verify(&vk, b"payload", &tampered),
        "field tamper must fail"
    );
    for adapter in [false, true] {
        let att = sign(&key, b"x", adapter);
        println!(
            "attachment size, {}: {} bytes",
            if adapter {
                "adapter-attested"
            } else {
                "device-signed"
            },
            att.len()
        );
    }
}
