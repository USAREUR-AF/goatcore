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

/// `--retire <id>` passes the retire step for that rotation and needs no read-back.
#[test]
fn retiring_passes_the_rotation_id_and_the_retire_step() {
    let (w, plays) = ipsec_world("ipsec-fips");
    let (out, _) = w.run(&["rotate", "hello-ally", "ipsec-ca", "--retire", "ipsec-20261114T200000Z"]);
    let edge = edge_play(&plays);
    assert!(
        edge.contains("peering_edge_rotation=ipsec-20261114T200000Z") && edge.contains("peering_edge_rotation_retire=true"),
        "{edge}\n{}",
        text(&out)
    );
}
