//! Gate row E2, first half (docs/design/goatbuilder-v1-gate.md, "Upgrading is
//! tested"): a peering taken to a middle phase with the last released 0.x
//! binary is finished with the build under test.
//!
//! The peering is the stand-in world of the federate tests
//! (tests/federate_world, used as it is): a `goat` stub, preflight, leaf
//! minting, plays and acceptance that record what they were asked to do, and
//! no host. The released binary runs exactly the command line the world builds
//! for the build under test, with the same environment and directory.
//!
//! The same three runs are made twice, in two fresh worlds:
//!   - mixed: 0.5.1 to the phase 5 stop, 0.5.1 `--from 6` to the phase 10
//!     stop, then the build under test `--from 11` to the end;
//!   - all new: the same three runs, each by the build under test.
//!
//! The mixed peering must finish, read the journal 0.5.1 wrote without a
//! refusal, and end in the same state as the all-new one: the same phases run
//! by the last run, the same outcome per phase, and the same calls to the
//! stubs.
//!
//! The released binary comes from `$GOATBUILDER_UPGRADE_DIR`, as for E1
//! (tests/upgrade.rs, tests/upgrade/fetch.sh).

mod federate_world;

#[path = "upgrade/released.rs"]
mod released;

use std::path::Path;
use std::process::{Command, Output};

use federate_world::{phases_run, says, world, World};

/// Run the world's command line with `bin` instead of the build under test.
fn run_as(bin: &Path, w: &World, extra: &[&str]) -> (Output, serde_json::Value) {
    let c = w.command(extra);
    let mut o = Command::new(bin);
    o.args(c.get_args()).env_clear();
    for (k, v) in c.get_envs() {
        if let Some(v) = v {
            o.env(k, v);
        }
    }
    if let Some(d) = c.get_current_dir() {
        o.current_dir(d);
    }
    let out = o.stdin(std::process::Stdio::null()).output().unwrap();
    let v = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out, v)
}

struct Ended {
    last_phases: Vec<u32>,
    outcomes: Vec<(String, String)>,
    log: String,
}

/// The three runs, the first two by `early` and the last by the build under
/// test; returns what the last run did and the journal it left.
fn three_runs(early: &Path) -> Ended {
    let w = world();
    let new = Path::new(federate_world::bin());
    let (out, v) = run_as(early, &w, &["hello-ally"]);
    assert!(
        out.status.success(),
        "{}: first run: {}",
        early.display(),
        says(&out)
    );
    assert_eq!(phases_run(&v), [1, 2, 3, 4, 5], "{}", says(&out));
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    let (out, v) = run_as(early, &w, &["hello-ally", "--from", "6"]);
    assert!(
        out.status.success(),
        "{}: --from 6: {}",
        early.display(),
        says(&out)
    );
    assert_eq!(phases_run(&v), [6, 7, 8, 9, 10], "{}", says(&out));
    w.clear_log();
    let (out, v) = run_as(new, &w, &["hello-ally", "--from", "11"]);
    assert!(
        out.status.success(),
        "the build under test, --from 11: {}",
        says(&out)
    );
    let st = w.state();
    assert_eq!(st["schema"], "goatbuilder.federate.state/v1");
    let mut outcomes: Vec<(String, String)> = st["phases"]
        .as_object()
        .expect("phases in state.json")
        .iter()
        .map(|(n, p)| (n.clone(), p["outcome"].as_str().unwrap_or("").to_string()))
        .collect();
    outcomes.sort();
    Ended {
        last_phases: phases_run(&v),
        outcomes,
        log: w
            .log()
            .replace(&w.root.path().display().to_string(), "ROOT"),
    }
}

#[test]
fn e2_a_peering_taken_to_phase_10_by_0_5_1_is_finished_by_this_build() {
    let Some(old) = released::binary("v0.5.1") else {
        return;
    };
    let mixed = three_runs(&old);
    let all_new = three_runs(Path::new(federate_world::bin()));
    assert_eq!(mixed.last_phases, [11, 12, 13, 14]);
    assert_eq!(mixed.last_phases, all_new.last_phases);
    assert_eq!(
        mixed.outcomes, all_new.outcomes,
        "the journal 0.5.1 started ends unlike one this build started"
    );
    assert!(
        mixed.outcomes.iter().all(|(_, o)| o == "passed"),
        "{:?}",
        mixed.outcomes
    );
    assert_eq!(
        mixed.log, all_new.log,
        "phases 11 to 14 asked the stubs for something else after 0.5.1"
    );
}
