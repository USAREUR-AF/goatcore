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
        ("ca", "GB-282", "is decided, not built: rotating our CA"),
        ("everything", "GB-189", "takes one of"),
    ] {
        let (out, _) = w.run(&["rotate", "hello-ally", key]);
        assert_eq!(out.status.code(), Some(1), "{key}: {}", says(&out));
        let t = text(&out);
        assert!(t.contains(code) && t.contains(why), "{key}: {t}");
        assert!(w.log().is_empty(), "{key}: something ran: {}", w.log());
        assert!(
            !w.work.join("state.json").exists(),
            "{key}: a run was journaled"
        );
    }
}

/// `rotate <partner> carrier` on a peering no forwarder relays is refused
/// before anything runs: its edge has no carrier, so a run would report a
/// rotation that changed no key. It is a configuration precondition, so
/// GB-317 with exit 1, not GB-189 (a stage asked the wrong way). The id is not kept. Phase 2 passing the id
/// to its plays, for a relayed peering, is held by `rotation::tests`.
#[test]
fn rotating_the_carrier_of_a_peering_with_no_forwarder_is_refused_before_anything_runs() {
    let w = world();
    let (out, _) = w.run(&["rotate", "hello-ally", "carrier"]);
    assert_eq!(out.status.code(), Some(1), "{}", says(&out));
    let t = text(&out);
    assert!(t.contains("GB-317") && t.contains("no forwarder"), "{t}");
    assert!(!t.contains("GB-189"), "{t}");
    assert!(w.log().is_empty(), "something ran: {}", w.log());
    assert!(
        !w.work.join(".rotation-carrier").exists(),
        "a rotation id was kept for a carrier that does not exist: {t}"
    );
    assert!(!w.work.join("state.json").exists(), "a run was journaled");
}

/// A hand-run deployment declares its forwarder in the edge group's own
/// switch, `peering_wg_via_forwarder: true` (what makes goat-peering-wg build
/// the carrier), with the forwarder's entry, and the edge's carrier public key
/// in it, in hand-written group_vars. A rotation would make the edge's new key
/// in phase 2 and could not change the forwarder's entry, so the carrier would
/// stay down. It stops before anything runs (GB-312), naming the value the
/// forwarder holds, the play that applies it, and the way to rotate: declare
/// the forwarder in goatbuilder.yaml. No id is kept and nothing is journaled.
#[test]
fn a_hand_declared_forwarder_stops_the_carrier_rotation_before_anything_runs() {
    let w = world();
    federate_world::write(
        &w.repo.join("ops/ansible/group_vars/hello-ally_edge/forwarder.yml"),
        "peering_wg_via_forwarder: true\npeering_wg_forwarder_endpoint: 192.0.2.20:51900\n",
    );
    let (out, _) = w.run(&["rotate", "hello-ally", "carrier"]);
    assert_eq!(out.status.code(), Some(1), "a configuration precondition: {}", says(&out));
    let t = text(&out);
    assert!(t.contains("GB-312"), "{t}");
    assert!(!t.contains("no forwarder"), "{t}");
    for name in [
        "peering_forwarder_peerings[name=hello-ally].edge_carrier_public_key",
        "peering-forwarder.yml",
        "forwarder:",
        "goatbuilder federate allocate",
    ] {
        assert!(t.contains(name), "{name}: {t}");
    }
    // In that order: the value, then the play.
    assert!(
        t.find("edge_carrier_public_key").unwrap() < t.find("peering-forwarder.yml").unwrap(),
        "{t}"
    );
    assert!(w.log().is_empty(), "something ran: {}", w.log());
    assert!(!w.work.join(".rotation-carrier").exists(), "a rotation id was kept: {t}");
    assert!(!w.work.join("state.json").exists(), "a run was journaled");
}

/// The same peering with its forwarder declared in goatbuilder.yaml instead,
/// adopted as it runs: its values pinned, the hand-written lines that allocate
/// now owns removed, and `goatbuilder federate allocate` run.
fn declare_the_forwarder_in_goatbuilder_yaml(w: &federate_world::World) {
    let cfg = w.repo.join("ops/goatbuilder/goatbuilder.yaml");
    let text_before = std::fs::read_to_string(&cfg).unwrap();
    let declared = text_before.replacen(
        "    edge_class: edge\n",
        &format!(
            "    edge_class: edge\n    forwarder:\n      endpoint: 192.0.2.20\n    values:\n      wg_interface: wg-fed1\n      wg_listen_port: 51821\n      edge_listen_port: 7448\n      republisher_metrics_port: 9464\n      cert_dir: {}\n",
            w.work.join("pki").display()
        ),
        1,
    );
    assert_ne!(declared, text_before, "the fixture's peering entry moved");
    federate_world::write(&cfg, &declared);
    let gv = w.repo.join("ops/ansible/group_vars/hello-ally_edge/peering.yml");
    let kept: String = std::fs::read_to_string(&gv)
        .unwrap()
        .lines()
        .filter(|l| !["peering_wg_interface:", "peering_wg_listen_port:", "goat_zenoh_edge_listen_port:"].iter().any(|k| l.starts_with(k)))
        .map(|l| format!("{l}\n"))
        .collect();
    federate_world::write(&gv, &kept);
    federate_world::write(&w.repo.join("ops/ansible/group_vars/hello-ally_core/peering.yml"), "");
    let (a, _) = w.run(&["allocate", "hello-ally"]);
    assert!(a.status.success(), "allocate: {}", says(&a));
}

#[test]
fn a_forwarder_declared_in_goatbuilder_yaml_is_a_carrier_to_rotate() {
    let w = world();
    declare_the_forwarder_in_goatbuilder_yaml(&w);
    rotate_the_carrier_through_both_plays(&w);
}

/// A forwarder declared in goatbuilder.yaml, with a hand switch
/// (`peering_wg_via_forwarder: true`) put back in the edge group's vars after
/// allocate: the declaration wins, so goatbuilder applies the forwarder and
/// the rotation is neither stopped for a hand-declared forwarder (GB-312) nor
/// refused for having none.
#[test]
fn a_forwarder_in_goatbuilder_yaml_wins_over_a_leftover_hand_switch() {
    let w = world();
    declare_the_forwarder_in_goatbuilder_yaml(&w);
    federate_world::write(
        &w.repo.join("ops/ansible/group_vars/hello-ally_edge/forwarder.yml"),
        "peering_wg_via_forwarder: true\n",
    );
    rotate_the_carrier_through_both_plays(&w);
}

/// A carrier rotation of a forwarder goatbuilder applies: the run passes,
/// phase 2 runs the edge's play and the forwarder's, and its command hands
/// both one rotation id, for this peering; the id is dropped once phase 2
/// passes. The stand-in plays copy no key, so the forwarder's carrier public
/// key that phase 2 waits for is put where the copy would leave it.
fn rotate_the_carrier_through_both_plays(w: &federate_world::World) {
    federate_world::write(&w.work.join("pki/forwarder-carrier.pub"), "stand-in-forwarder-carrier-public-key\n");
    let (out, v) = w.run(&["rotate", "hello-ally", "carrier"]);
    let t = text(&out);
    assert!(!t.contains("GB-312") && !t.contains("no forwarder"), "{t}");
    assert_eq!(out.status.code(), Some(0), "{}\n{}", says(&out), w.log());
    let log = w.log();
    for play in ["play peering-prepare.yml", "play peering-carrier-keys.yml", "play peering-forwarder.yml"] {
        assert!(log.lines().any(|l| l == play), "{play} did not run:\n{log}");
    }
    let phase_2 = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] == "cmd")
        .filter_map(|c| c["detail"].as_str())
        .find(|d| d.contains("peering-prepare.yml"))
        .unwrap_or_else(|| panic!("no phase 2 command: {}", says(&out)));
    let parts: Vec<&str> = phase_2.split(") && (").collect();
    let edge = parts.iter().find(|p| p.contains(" peering-prepare.yml")).unwrap();
    let fwd = parts
        .iter()
        .find(|p| p.contains(" peering-forwarder.yml"))
        .unwrap_or_else(|| panic!("phase 2 runs no forwarder play: {phase_2}"));
    let id = |p: &str| {
        p.split("-e peering_wg_carrier_rotation='")
            .nth(1)
            .and_then(|r| r.split('\'').next())
            .map(String::from)
    };
    let edge_id = id(edge).unwrap_or_else(|| panic!("the edge's play has no rotation id: {edge}"));
    assert!(edge_id.starts_with("carrier-"), "{edge_id}");
    assert_eq!(id(fwd).as_deref(), Some(edge_id.as_str()), "one id for both plays: {fwd}");
    for p in [edge, fwd] {
        assert!(p.contains(&format!("-e peering_forwarder_carrier_rotation='{edge_id}' -e peering_forwarder_carrier_rotation_name='hello-ally'")), "{p}");
    }
    assert!(!w.work.join(".rotation-carrier").exists(), "the id is kept after phase 2 passed");
}

/// The switch set false in the edge group's vars is no forwarder: still refused.
#[test]
fn an_edge_group_that_switches_the_forwarder_off_is_still_refused() {
    let w = world();
    federate_world::write(
        &w.repo.join("ops/ansible/group_vars/hello-ally_edge/forwarder.yml"),
        "peering_wg_via_forwarder: false\n",
    );
    let (out, _) = w.run(&["rotate", "hello-ally", "carrier"]);
    assert_eq!(out.status.code(), Some(1), "{}", says(&out));
    let t = text(&out);
    assert!(t.contains("GB-317") && t.contains("no forwarder"), "{t}");
    assert!(w.log().is_empty(), "something ran: {}", w.log());
}

/// The check reads the edge group the run deploys to: `--edge-group`, else
/// `<partner>_edge`, never a goatbuilder.yaml `values.edge_group` pin that the
/// run itself does not read. With such a pin and no allocation record, the
/// switch in the pinned group alone is no carrier the run would touch
/// (refused), and the switch in `<partner>_edge` alone is (not refused).
#[test]
fn the_forwarder_switch_is_read_from_the_group_the_run_deploys_to() {
    let pinned = |switch_group: &str| {
        let w = world();
        let cfg = w.repo.join("ops/goatbuilder/goatbuilder.yaml");
        let text_before = std::fs::read_to_string(&cfg).unwrap();
        let declared = text_before.replacen(
            "    edge_class: edge\n",
            "    edge_class: edge\n    values:\n      edge_group: ally_pinned_edge\n",
            1,
        );
        assert_ne!(declared, text_before, "the fixture's peering entry moved");
        federate_world::write(&cfg, &declared);
        federate_world::write(
            &w.repo
                .join(format!("ops/ansible/group_vars/{switch_group}/forwarder.yml")),
            "peering_wg_via_forwarder: true\n",
        );
        let (out, _) = w.run(&["rotate", "hello-ally", "carrier"]);
        (text(&out), w.log(), out.status.code())
    };
    let (t, log, rc) = pinned("ally_pinned_edge");
    assert_eq!(rc, Some(1), "{t}");
    assert!(t.contains("GB-317") && t.contains("no forwarder"), "{t}");
    assert!(log.is_empty(), "something ran: {log}");
    let (t, _, _) = pinned("hello-ally_edge");
    assert!(!t.contains("no forwarder"), "{t}");
}

/// Phase 6 keeps a copy of the package the agreement was composed from,
/// `<work>/ours.composed.json`, beside `agreement.json`: phase 3 rewrites
/// ours.canonical.json on every run, so a later phase 3 cross-checks a
/// re-declared MTU against the offer the applied agreement holds, read from
/// that copy.
#[test]
fn phase_6_keeps_the_package_the_agreement_was_composed_from() {
    let w = world();
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    w.run(&["hello-ally"]);
    assert!(!w.work.join("ours.composed.json").exists(), "a copy before phase 6");
    let (out, _) = w.run(&["hello-ally", "--from", "6", "--to", "6"]);
    assert!(out.status.success(), "{}", says(&out));
    let composed = std::fs::read(w.work.join("ours.composed.json")).expect("phase 6 wrote no ours.composed.json");
    assert_eq!(composed, std::fs::read(w.work.join("ours.canonical.json")).unwrap());
}

/// Phase 6 that cannot keep the copy (here a folder stands where the copy
/// goes) fails with GB-184, the working folder that cannot be written, and
/// exit 1; `explain` reads the row's words as GB-184.
#[test]
fn phase_6_that_cannot_keep_the_copy_is_gb_184() {
    let w = world();
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    w.run(&["hello-ally"]);
    federate_world::write(&w.work.join("ours.composed.json/in-the-way"), "");
    let (out, _) = w.run(&["hello-ally", "--from", "6", "--to", "6"]);
    assert_eq!(out.status.code(), Some(1), "{}", says(&out));
    let t = text(&out);
    let row = t
        .lines()
        .find(|l| l.contains("could not keep") && l.contains("ours.composed.json"))
        .unwrap_or_else(|| panic!("no row names the copy: {t}"));
    assert!(row.contains("(GB-184)"), "{row}");
    let detail = row.trim().trim_start_matches("\"detail\": \"").trim_end_matches(['"', ',']);
    let hits = goat_errors::interpret(detail).hits;
    assert!(hits.first().is_some_and(|h| h.code == "GB-184"), "explain does not read it as GB-184: {detail}");
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
    // No directory step here (the hand exchange): nobody has sent our new
    // package, so the row says to send it now.
    assert!(t.contains("send our new package") && t.contains("ours.canonical.json"), "{t}");
}

/// (g) A new package of the partner's is waiting (theirs.incoming.json): the
/// rotation does not skip phase 5, which asks about it.
#[test]
fn rotating_side_phase_5_stops_g_when_a_partner_package_is_waiting() {
    let w = confirmed_world();
    let fp = federate_world::sha256_hex(b"fake-ca-der");
    std::fs::write(
        w.work.join("theirs.incoming.json"),
        federate_world::resigned_with_transport("xab", &fp, Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")),
    )
    .unwrap();
    assert_stops_at_phase_5(&w);
}

// ---------------------------------------------------------------- K6: the IPsec leaf
// docs/design/federation-key-rotation.md §8. goat-peering-edge does the edge's
// checks, set-aside, switch and roll-back; the command picks the run, passes the
// rotation to phase 11's play, and keeps the rotation's record until phase 11 passed.

const READBACK: &str = "sha256:5f1c0e4b8d6a2f90a1b3c5d7e9f00112233445566778899aabbccddeeff00112";

/// A world whose agreement chose ipsec-fips, with phase 11's play recorded.
fn ipsec_world(kind: &str) -> (federate_world::World, std::path::PathBuf) {
    let w = world();
    std::fs::create_dir_all(&w.work).unwrap();
    std::fs::write(
        w.work.join("agreement.json"),
        format!(r#"{{"spec":"federation-agreement/v1","transport":{{"kind":"{kind}"}}}}"#),
    )
    .unwrap();
    let plays = w.root.path().join("plays.log");
    exe(
        &w.repo.join("ops/scripts/ansible-preflight.sh"),
        &format!("#!/bin/sh\necho \"play $*\" >> '{}'\nexit 0\n", plays.display()),
    );
    (w, plays)
}

fn edge_play(plays: &std::path::Path) -> String {
    std::fs::read_to_string(plays)
        .unwrap_or_default()
        .lines()
        .find(|l| l.contains("peering-edge.yml"))
        .unwrap_or_default()
        .to_string()
}

/// `ipsec-leaf` runs phases 11 to 14 and passes the rotation to peering-edge.yml;
/// nothing before phase 11 runs, and the record is gone once phase 11 passed.
#[test]
fn rotating_the_ipsec_leaf_runs_from_phase_11_and_passes_the_rotation_to_the_edge() {
    let (w, plays) = ipsec_world("ipsec-fips");
    let (out, _) = w.run(&["rotate", "hello-ally", "ipsec-leaf"]);
    let t = text(&out);
    let edge = edge_play(&plays);
    assert!(
        edge.contains("peering_edge_rotation=ipsec-") && edge.contains("peering_edge_rotation_kind=leaf"),
        "phase 11 has no rotation: {edge}\n{t}"
    );
    assert!(!edge.contains("peering_edge_rotation_ca_readback"), "{edge}");
    assert!(!w.log().contains("federate init"), "a phase before 11 ran: {}", w.log());
    assert!(
        !std::fs::read_to_string(&plays).unwrap_or_default().contains("peering-prepare.yml"),
        "phase 2 ran"
    );
    assert!(!w.work.join(".rotation-ipsec").exists(), "the record is kept after phase 11 passed: {t}");
}

/// `ipsec-ca` needs the partner's read-back of our new CA, well formed, before anything runs.
#[test]
fn rotating_the_ipsec_ca_needs_the_partners_read_back_before_anything_runs() {
    for extra in [vec![], vec!["--partner-holds-ipsec-ca", "not-a-fingerprint"]] {
        let (w, plays) = ipsec_world("ipsec-fips");
        let mut a = vec!["rotate", "hello-ally", "ipsec-ca"];
        a.extend(extra.iter().copied());
        let (out, _) = w.run(&a);
        assert!(!out.status.success(), "{extra:?}: {}", says(&out));
        let t = text(&out);
        assert!(t.contains("GB-303") && t.contains("--partner-holds-ipsec-ca"), "{extra:?}: {t}");
        assert!(!plays.exists(), "{extra:?}: a play ran");
        assert!(!w.work.join(".rotation-ipsec").exists(), "{extra:?}: a record was written");
    }
    let (w, plays) = ipsec_world("ipsec-fips");
    let (out, _) = w.run(&["rotate", "hello-ally", "ipsec-ca", "--partner-holds-ipsec-ca", READBACK]);
    let edge = edge_play(&plays);
    assert!(
        edge.contains("peering_edge_rotation_kind=ca")
            && edge.contains(&format!("peering_edge_rotation_ca_readback={READBACK}")),
        "{edge}\n{}",
        text(&out)
    );
}

/// The read-back belongs to `ipsec-ca`; on any other key it is refused.
#[test]
fn the_read_back_is_refused_on_any_other_key() {
    let (w, plays) = ipsec_world("ipsec-fips");
    let (out, _) = w.run(&["rotate", "hello-ally", "ipsec-leaf", "--partner-holds-ipsec-ca", READBACK]);
    assert!(!out.status.success(), "{}", says(&out));
    assert!(text(&out).contains("GB-189"), "{}", text(&out));
    assert!(!plays.exists(), "a play ran");
}

/// A WireGuard peering has no IPsec leaf: refused, naming the transport, before anything runs.
#[test]
fn an_ipsec_rotation_on_a_wireguard_peering_is_refused() {
    let (w, plays) = ipsec_world("wireguard");
    let (out, _) = w.run(&["rotate", "hello-ally", "ipsec-leaf"]);
    assert!(!out.status.success(), "{}", says(&out));
    let t = text(&out);
    assert!(t.contains("GB-189") && t.contains("ipsec-fips") && t.contains("wireguard"), "{t}");
    assert!(!plays.exists(), "a play ran");
}

/// A rotation under way is resumed with the same id; one of another kind, or with
/// another read-back, is refused (GB-298) and nothing runs.
#[test]
fn a_rotation_under_way_resumes_with_its_id_and_another_kind_is_refused() {
    let (w, plays) = ipsec_world("ipsec-fips");
    std::fs::write(w.work.join(".rotation-ipsec"), "{\"kind\":\"leaf\",\"id\":\"ipsec-kept-1\"}\n").unwrap();
    let (out, _) = w.run(&["rotate", "hello-ally", "ipsec-ca", "--partner-holds-ipsec-ca", READBACK]);
    assert!(!out.status.success(), "{}", says(&out));
    let t = text(&out);
    assert!(t.contains("GB-298") && t.contains("ipsec-kept-1"), "{t}");
    assert!(!plays.exists(), "a play ran");
    let (out, _) = w.run(&["rotate", "hello-ally", "ipsec-leaf"]);
    assert!(edge_play(&plays).contains("peering_edge_rotation=ipsec-kept-1"), "{}", text(&out));
}

/// `--retire <id>` passes the retire step for a rotation this side saw switch
/// and pass phase 14 (its ledger), and needs no read-back.
#[test]
fn retiring_passes_the_rotation_id_and_the_retire_step() {
    let (w, plays) = ipsec_world("ipsec-fips");
    std::fs::write(
        w.work.join("ipsec-rotations.json"),
        "{\"rotations\":[{\"id\":\"ipsec-20261114T200000Z\",\"kind\":\"ca\",\"started\":\"2026-11-14T20:00:00Z\",\"switched\":\"2026-11-14T20:05:00Z\",\"accepted\":\"2026-11-14T20:20:00Z\"}]}\n",
    )
    .unwrap();
    let (out, _) = w.run(&["rotate", "hello-ally", "ipsec-ca", "--retire", "ipsec-20261114T200000Z"]);
    let edge = edge_play(&plays);
    assert!(
        edge.contains("peering_edge_rotation=ipsec-20261114T200000Z") && edge.contains("peering_edge_rotation_retire=true"),
        "{edge}\n{}",
        text(&out)
    );
}

/// A retire of an id this side never saw pass phase 14 is refused before anything runs.
#[test]
fn retiring_an_unknown_rotation_is_refused_before_anything_runs() {
    let (w, plays) = ipsec_world("ipsec-fips");
    let (out, _) = w.run(&["rotate", "hello-ally", "ipsec-leaf", "--retire", "ipsec-typo"]);
    assert!(!out.status.success(), "{}", says(&out));
    assert!(text(&out).contains("GB-309"), "{}", text(&out));
    assert!(!plays.exists(), "a play ran");
}

/// A run that leaves phase 11 out rotates nothing and records nothing.
#[test]
fn a_rotation_from_after_phase_11_records_nothing() {
    let (w, _plays) = ipsec_world("ipsec-fips");
    let _ = w.run(&["rotate", "hello-ally", "ipsec-leaf", "--from", "13"]);
    assert!(!w.work.join(".rotation-ipsec").exists());
    assert!(!w.work.join("ipsec-rotations.json").exists());
}
