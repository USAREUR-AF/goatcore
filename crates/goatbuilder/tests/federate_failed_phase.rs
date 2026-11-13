//! Gate row F5: a federate phase made to fail with a known error carries that
//! error's code and fix, the log of its output, and the command that runs the
//! phase by hand (docs/design/goatbuilder-legibility.md, step 7). Through the
//! real binary, on the federate tests' stand-in world (tests/federate_world,
//! used as it is): phase 12's play is made to fail the way an edge host that
//! refuses our key fails.

mod federate_world;

use federate_world::{exe, rows, says, world};

#[test]
fn f5_a_failed_phase_carries_its_known_error_s_code_and_fix() {
    let w = world();
    w.run(&["hello-ally"]);
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    w.run(&["hello-ally", "--from", "6"]);
    // The edge play fails as ansible does when ssh refuses the key.
    exe(
        &w.repo.join("ops/scripts/ansible-preflight.sh"),
        "#!/bin/sh\nfor a; do last=$a; done\nif [ \"$last\" = peering-zenoh-edge.yml ]; then\n  echo 'fatal: [edge-1]: UNREACHABLE! => {\"msg\": \"Failed to connect to the host via ssh: goat@192.0.2.10: Permission denied (publickey).\"}'\n  exit 4\nfi\nexit 0\n",
    );
    let (out, v) = w.run(&["hello-ally", "--from", "11"]);
    assert!(!out.status.success(), "phase 12 fails: {}", says(&out));
    let interpreted = rows(&v, "interpreted");
    assert_eq!(interpreted.len(), 1, "{}", says(&out));
    let d = interpreted[0];
    let e = goat_errors::explain("X-SSH-001").unwrap();
    assert!(d.starts_with(&format!("X-SSH-001: {}", e.title)), "{d}");
    assert!(d.contains(&e.fixes[0]), "the fix: {d}");
    assert!(d.contains("goatbuilder explain X-SSH-001"), "{d}");
    // The log of phase 12's output, which holds the error.
    let log = d
        .split(". Its output: ")
        .nth(1)
        .and_then(|r| r.split(". By hand: ").next())
        .unwrap_or_else(|| panic!("no log path: {d}"));
    let text = std::fs::read_to_string(log).unwrap_or_else(|e| panic!("{log}: {e}"));
    assert!(text.contains("Permission denied (publickey)"), "{log}: {text}");
    // The command that runs phase 12 by hand.
    let p = goat_errors::phases::explain_phase("federate", "12").unwrap();
    assert!(d.contains(&format!("By hand: {}", p.by_hand)), "{d}");
}
