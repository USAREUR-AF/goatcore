//! Phase 6 composes the agreement only with the peering's declared partner
//! (FED_COUNTERPARTY_ID), through the real binary against the stand-in
//! repository (federate_world), with the verbs run in process.
//!
//! A keyring shared by several peerings holds the keys of more parties than
//! the two of this one. A package at `theirs.canonical.json` that is another
//! party's, signed by the key bound for that party, is refused at phase 6:
//! nothing is composed.

mod federate_world;

use federate_world::{says, seed_hex, world};

/// `party`'s conformance package with `sovereign_id` set to `as_id`, issued
/// yesterday and expiring in thirty days (so it composes on any day the test
/// runs), signed with `party`'s committed test seed.
fn package(party: &str, as_id: &str) -> Vec<u8> {
    use base64::Engine;
    use goat_federation::json::{canonical, parse_document, JVal};
    let vectors = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/federation/vectors");
    let bytes = std::fs::read(vectors.join(format!("compose-full/package-{party}.canonical.json"))).unwrap();
    let JVal::Obj(mut top) = parse_document(&bytes).unwrap().without_member("signature") else {
        panic!()
    };
    let today = chrono::Utc::now().date_naive();
    for (k, v) in top.iter_mut() {
        match k.as_str() {
            "sovereign_id" => *v = JVal::Str(as_id.into()),
            "issued" => *v = JVal::Str((today - chrono::Duration::days(1)).to_string()),
            "expires" => *v = JVal::Str((today + chrono::Duration::days(30)).to_string()),
            _ => {}
        }
    }
    let hex = seed_hex(party);
    let seed: [u8; 32] = (0..32)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
        .collect::<Vec<u8>>()
        .try_into()
        .unwrap();
    let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
    let sig = ed25519_dalek::Signer::sign(&sk, canonical(&JVal::Obj(top.clone())).as_bytes());
    top.push((
        "signature".into(),
        JVal::Str(base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())),
    ));
    canonical(&JVal::Obj(top)).into_bytes()
}

fn public(party: &str) -> String {
    federate_world::public_b64(party)
}

/// Our package, a counterparty package of `theirs_id`, and a keyring that also
/// binds a third party (xac) to the key that signs its package.
fn run_phase_6(theirs_id: &str) -> (std::process::Output, federate_world::World) {
    let w = world();
    std::fs::write(w.work.join("ours.canonical.json"), package("xaa", "xaa")).unwrap();
    std::fs::write(w.work.join("theirs.canonical.json"), package("xab", theirs_id)).unwrap();
    std::fs::write(
        w.work.join("keys.txt"),
        format!("xaa={}\nxab={}\nxac={}\n", public("xaa"), public("xab"), public("xab")),
    )
    .unwrap();
    let _ = std::fs::remove_file(w.work.join("agreement.json"));
    let out = w
        .command(&["hello-ally", "--from", "6", "--to", "6"])
        .env("FED_VERBS", "goatbuilder")
        .output()
        .unwrap();
    (out, w)
}

/// The declared partner's package composes.
#[test]
fn phase_6_composes_with_the_declared_partner() {
    let (out, w) = run_phase_6("xab");
    assert!(out.status.success(), "{}", says(&out));
    assert!(w.work.join("agreement.json").exists(), "no agreement: {}", says(&out));
}

/// Another party's genuine package, its key in a shared keyring, is refused
/// at phase 6 (GB-191), and nothing is composed.
#[test]
fn phase_6_refuses_a_package_of_a_party_other_than_the_declared_partner() {
    let (out, w) = run_phase_6("xac");
    let t = says(&out);
    assert!(!out.status.success(), "{t}");
    assert!(t.contains("GB-191") && t.contains("xac"), "{t}");
    assert!(!w.work.join("agreement.json").exists(), "an agreement was composed: {t}");
}
