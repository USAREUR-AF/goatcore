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
/// refusal says why. `transport` names that the partner cannot yet receive
/// our new package, so wiring it before that is built fails this test.
#[test]
fn a_key_this_build_does_not_rotate_is_refused_before_anything_runs() {
    let w = world();
    for (key, code, why) in [
        (
            "transport",
            "GB-189",
            "the partner cannot yet receive our new package",
        ),
        ("signing", "GB-189", "previous seed"),
        ("ca", "GB-282", "§5.1"),
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
