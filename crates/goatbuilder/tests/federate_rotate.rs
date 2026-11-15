//! `goatbuilder federate rotate <partner> <key>`, through the real binary
//! against a stand-in repository (federate_world).

mod federate_world;

use federate_world::{exe, says, world};

fn text(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// `rotate <partner> leaves` runs phases 8 to 14 with phase 8 replacing every
/// leaf: the leaf helper gets --rotate, and nothing before phase 8 runs.
#[test]
fn rotating_the_leaves_starts_at_phase_8_and_asks_the_helper_to_rotate() {
    let w = world();
    let log = w.root.path().join("mint.log");
    exe(
        &w.repo.join("ops/federation/standup/mint-peering-leaves.sh"),
        &format!("#!/bin/sh\necho \"mint $*\" >> '{}'\n", log.display()),
    );
    let (out, _) = w.run(&["rotate", "hello-ally", "leaves", "--dry-run"]);
    assert!(out.status.success(), "{}", says(&out));
    let (out, _) = w.run(&["rotate", "hello-ally", "leaves"]);
    let mint = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(mint.contains("--rotate"), "{mint}\n{}", says(&out));
    assert!(
        !w.log().contains("federate init"),
        "a phase before 8 ran: {}",
        w.log()
    );
}

/// A key this build does not rotate is refused before anything runs, and the
/// refusal says why.
#[test]
fn a_key_this_build_does_not_rotate_is_refused_before_anything_runs() {
    let w = world();
    for (key, code, why) in [
        ("signing", "GB-189", "previous seed"),
        ("ca", "GB-282", "§5.1"),
        ("everything", "GB-189", "takes one of"),
    ] {
        let (out, _) = w.run(&["rotate", "hello-ally", key]);
        assert!(!out.status.success(), "{key}: {}", says(&out));
        let t = text(&out);
        assert!(t.contains(code) && t.contains(why), "{key}: {t}");
        assert!(w.log().is_empty(), "{key}: something ran: {}", w.log());
        assert!(
            !w.work.join("state.json").exists(),
            "{key}: a run was journaled"
        );
    }
}

/// `rotate <partner> carrier` is not refused: it runs from phase 2, and
/// phase 2's plays get the rotation's id. Nothing before phase 2 runs, and
/// once phase 2 has passed the id is no longer kept.
#[test]
fn rotating_the_carrier_starts_at_phase_2_and_passes_the_rotation_id_to_its_plays() {
    let w = world();
    let plays = w.root.path().join("plays.log");
    exe(
        &w.repo.join("ops/scripts/ansible-preflight.sh"),
        &format!(
            "#!/bin/sh\necho \"play $*\" >> '{}'\nexit 0\n",
            plays.display()
        ),
    );
    let (out, _) = w.run(&["rotate", "hello-ally", "carrier"]);
    let t = text(&out);
    assert!(!t.contains("GB-189"), "carrier was refused: {t}");
    let played = std::fs::read_to_string(&plays).unwrap_or_default();
    let first = played.lines().next().unwrap_or_default();
    assert!(
        first.contains("peering_wg_carrier_rotation=carrier-")
            && first.contains("peering_forwarder_carrier_rotation_name=hello-ally"),
        "phase 2's first play has no rotation id: {played}\n{t}"
    );
    assert!(!w.log().contains("preflight "), "phase 1 ran: {}", w.log());
    assert!(
        !w.work.join(".rotation-carrier").exists(),
        "the id is kept after phase 2 passed: {t}"
    );
}

/// `rotate <partner> transport` runs from phase 2 with the rotation's id on
/// phase 2's plays, and writes the record phase 3 measures into our new
/// package (ADR 1229 lets the partner's phase 5 take it). Nothing before
/// phase 2 runs, and once phase 2 has passed the id is no longer kept.
#[test]
fn rotating_the_transport_key_starts_at_phase_2_and_records_the_rotation_for_phase_3() {
    let w = world();
    let plays = w.root.path().join("plays.log");
    exe(
        &w.repo.join("ops/scripts/ansible-preflight.sh"),
        &format!(
            "#!/bin/sh\necho \"play $*\" >> '{}'\nexit 0\n",
            plays.display()
        ),
    );
    let (out, _) = w.run(&["rotate", "hello-ally", "transport"]);
    let t = text(&out);
    assert!(!t.contains("GB-189"), "transport was refused: {t}");
    let played = std::fs::read_to_string(&plays).unwrap_or_default();
    let first = played.lines().next().unwrap_or_default();
    assert!(
        first.contains("peering_wg_transport_rotation=transport-"),
        "phase 2's first play has no rotation id: {played}\n{t}"
    );
    assert!(!w.log().contains("preflight "), "phase 1 ran: {}", w.log());
    let record = std::fs::read_to_string(w.work.join("rotation.json")).unwrap_or_default();
    assert!(record.contains("\"key\":\"transport\""), "no record for phase 3: {record}\n{t}");
    assert!(
        !w.work.join(".rotation-transport").exists(),
        "the id is kept after phase 2 passed: {t}"
    );
}

/// The rotating side's phase 5 after `rotate <partner> transport` (ADR 1229): a
/// world where both confirmations were journaled through the command.
fn confirmed_world() -> federate_world::World {
    use federate_world::{public_b64, sha256_hex};
    let w = world();
    let fp = sha256_hex(b"fake-ca-der");
    let key = public_b64("xab");
    let (out, _) = w.run(&["hello-ally", "--partner-key", &key, "--partner-ca-fingerprint", &fp]);
    assert!(out.status.success(), "{}", says(&out));
    let digest = sha256_hex(&std::fs::read(w.work.join("agreement.json")).unwrap());
    let (out, _) = w.run(&["hello-ally", "--resume", "--agreement-digest", &digest]);
    assert!(out.status.success(), "{}", says(&out));
    w
}

/// Change the journal: `edit` gets the confirmations array.
fn edit_journal(w: &federate_world::World, edit: impl FnOnce(&mut Vec<serde_json::Value>)) {
    let path = w.work.join("state.json");
    let mut st = w.state();
    edit(st["confirmations"].as_array_mut().unwrap());
    std::fs::write(&path, serde_json::to_vec_pretty(&st).unwrap()).unwrap();
}

fn set_keys_value(w: &federate_world::World, key: &str, value: &str) {
    edit_journal(w, |cs| {
        let c = cs.iter_mut().find(|c| c["kind"] == "keys").expect("a keys confirmation");
        c["values"][key] = serde_json::Value::String(value.into());
    });
}

/// Runs the rotation and requires it to stop at phase 5: phase 5 ran, phase 6
/// did not, and phase 5 did not carry on.
fn assert_stops_at_phase_5(w: &federate_world::World) {
    use federate_world::phases_run;
    let (out, v) = w.run(&["rotate", "hello-ally", "transport"]);
    let ran = phases_run(&v);
    assert!(ran.contains(&5) && !ran.contains(&6), "{ran:?}: {}", says(&out));
    let st = w.state();
    assert!(!st["phases"]["5"]["detail"].as_str().unwrap_or("").contains("unchanged"), "{}", st["phases"]["5"]);
}

/// (a) Our own verifying key changed since the keys were confirmed: stops.
#[test]
fn rotating_side_phase_5_stops_a_when_our_verifying_key_changed() {
    let w = confirmed_world();
    set_keys_value(&w, "our_key", &federate_world::public_b64("xab"));
    assert_stops_at_phase_5(&w);
}

/// (b) Our CA fingerprint changed: stops.
#[test]
fn rotating_side_phase_5_stops_b_when_our_ca_fingerprint_changed() {
    let w = confirmed_world();
    set_keys_value(&w, "our_ca_fingerprint", &format!("sha256:{}", "cd".repeat(32)));
    assert_stops_at_phase_5(&w);
}

/// (c) The partner's key, or their CA fingerprint, changed: stops (each).
#[test]
fn rotating_side_phase_5_stops_c_when_the_partners_key_or_fingerprint_changed() {
    let w = confirmed_world();
    set_keys_value(&w, "partner_key", &federate_world::public_b64("xaa"));
    assert_stops_at_phase_5(&w);
    let w = confirmed_world();
    set_keys_value(&w, "partner_ca_fingerprint", &format!("sha256:{}", "ef".repeat(32)));
    assert_stops_at_phase_5(&w);
}

/// (d) The agreement on disk is not the one whose digest was confirmed: stops
/// at phase 5, before compose.
#[test]
fn rotating_side_phase_5_stops_d_when_the_agreement_digest_differs_from_the_confirmed_one() {
    let w = confirmed_world();
    edit_journal(&w, |cs| {
        let c = cs.iter_mut().find(|c| c["kind"] == "agreement").expect("an agreement confirmation");
        c["bound_to"] = serde_json::Value::String(format!("sha256:{}", "12".repeat(32)));
    });
    assert_stops_at_phase_5(&w);
}

/// (e) No keys confirmation journaled (the keys bound by hand): stops.
#[test]
fn rotating_side_phase_5_stops_e_when_no_keys_confirmation_is_journaled() {
    let w = confirmed_world();
    edit_journal(&w, |cs| cs.retain(|c| c["kind"] != "keys"));
    assert_stops_at_phase_5(&w);
}

/// (f) A pure transport rotation: phase 5 carries on, saying why, and phase 6
/// does not ask the agreement digest again.
#[test]
fn rotating_side_phase_5_continues_f_for_a_pure_transport_rotation() {
    use federate_world::phases_run;
    let w = confirmed_world();
    let (out, v) = w.run(&["rotate", "hello-ally", "transport"]);
    let t = text(&out);
    let ran = phases_run(&v);
    assert!(ran.starts_with(&[2, 3, 4, 5, 6]), "{ran:?}: {t}");
    let st = w.state();
    assert_eq!(st["phases"]["5"]["outcome"], "passed", "{}", st["phases"]["5"]);
    assert!(st["phases"]["5"]["detail"].as_str().unwrap_or("").contains("unchanged"), "{}", st["phases"]["5"]);
    assert_eq!(st["phases"]["6"]["outcome"], "passed", "{}", st["phases"]["6"]);
    assert!(!t.contains("waiting for the agreement digest"), "{t}");
}
