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
