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

/// `rotate <partner> carrier` on a peering no forwarder relays is refused
/// before anything runs: its edge has no carrier, so a run would report a
/// rotation that changed no key. The id is not kept. Phase 2 passing the id
/// to its plays, for a relayed peering, is held by `rotation::tests`.
#[test]
fn rotating_the_carrier_of_a_peering_with_no_forwarder_is_refused_before_anything_runs() {
    let w = world();
    let (out, _) = w.run(&["rotate", "hello-ally", "carrier"]);
    assert!(!out.status.success(), "{}", says(&out));
    let t = text(&out);
    assert!(t.contains("GB-189") && t.contains("no forwarder"), "{t}");
    assert!(w.log().is_empty(), "something ran: {}", w.log());
    assert!(
        !w.work.join(".rotation-carrier").exists(),
        "a rotation id was kept for a carrier that does not exist: {t}"
    );
    assert!(!w.work.join("state.json").exists(), "a run was journaled");
}

/// A hand-run deployment declares its forwarder in the edge group's own
/// switch, `peering_wg_via_forwarder: true` (what makes goat-peering-wg build
/// the carrier), with the forwarder's values in group_vars, and nothing in
/// goatbuilder.yaml. Its carrier rotation is not refused for "no forwarder":
/// it starts, from phase 2, with the rotation's id kept.
#[test]
fn a_forwarder_declared_in_the_edge_groups_vars_is_a_carrier_to_rotate() {
    let w = world();
    federate_world::write(
        &w.repo.join("ops/ansible/group_vars/hello-ally_edge/forwarder.yml"),
        "peering_wg_via_forwarder: true\npeering_wg_forwarder_endpoint: 192.0.2.20:51900\n",
    );
    let (out, _) = w.run(&["rotate", "hello-ally", "carrier"]);
    let t = text(&out);
    assert!(!t.contains("no forwarder"), "{t}");
    assert!(!w.log().is_empty(), "nothing ran: {t}");
}

/// The same peering with its forwarder declared in goatbuilder.yaml instead.
#[test]
fn a_forwarder_declared_in_goatbuilder_yaml_is_a_carrier_to_rotate() {
    let w = world();
    let cfg = w.repo.join("ops/goatbuilder/goatbuilder.yaml");
    let text_before = std::fs::read_to_string(&cfg).unwrap();
    let declared = text_before.replacen(
        "    edge_class: edge\n",
        "    edge_class: edge\n    forwarder:\n      endpoint: 192.0.2.20:51900\n",
        1,
    );
    assert_ne!(declared, text_before, "the fixture's peering entry moved");
    federate_world::write(&cfg, &declared);
    let (out, _) = w.run(&["rotate", "hello-ally", "carrier"]);
    let t = text(&out);
    assert!(!t.contains("no forwarder"), "{t}");
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
    assert!(!out.status.success(), "{}", says(&out));
    let t = text(&out);
    assert!(t.contains("GB-189") && t.contains("no forwarder"), "{t}");
    assert!(w.log().is_empty(), "something ran: {}", w.log());
}
