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

/// The run record keeps the rows that stopped a run that ended without an
/// error: a phase that failed returns its failure as rows, and those rows,
/// with their code, are why it stopped (docs/design/goatbuilder-legibility.md,
/// step 4).
#[test]
fn the_run_record_keeps_the_rows_that_stopped_the_run() {
    let w = world();
    w.run(&["hello-ally"]);
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    w.run(&["hello-ally", "--from", "6"]);
    std::fs::write(&w.fail_12, "").unwrap();
    let (out, v) = w.run(&["hello-ally", "--from", "11"]);
    assert!(!out.status.success(), "phase 12 fails: {}", says(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let path = stderr
        .lines()
        .find_map(|l| l.strip_prefix("run record: "))
        .unwrap_or_else(|| panic!("no run record: {}", says(&out)));
    let rec: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let stop = rec["stop"]
        .as_array()
        .unwrap_or_else(|| panic!("no stop in the run record: {rec}"));
    let failed = rows(&v, "failed");
    assert!(!failed.is_empty(), "{}", says(&out));
    for d in failed {
        assert!(
            stop.iter()
                .any(|s| s["name"] == "failed" && s["detail"] == d),
            "the failure row is kept: {d}\n{rec}"
        );
    }
    assert!(stop.iter().all(|s| s["ok"] == false), "{rec}");
}
