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
//! Both halves run from every 0.x release pinned in
//! tests/upgrade/releases.sha256 from 0.5.1 on, so a release pinned later is
//! covered without a change here.
//!
//! The second half: a peering the released binary takes through phase 14 is
//! accepted by the build under test. The build runs phase 14 alone on the
//! released binary's working directory; it must pass and leave the journal's
//! records for phases 1 to 13 as they were. The control: a journal of a
//! schema the build does not read is refused (GB-182).
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

/// The first half from `version`: taken to phase 10 by it, finished by the
/// build under test, ending as a peering the build ran throughout.
fn finished_by_this_build(version: &str) {
    let Some(old) = released::binary(version) else {
        return;
    };
    let mixed = three_runs(&old);
    let all_new = three_runs(Path::new(federate_world::bin()));
    assert_eq!(mixed.last_phases, [11, 12, 13, 14]);
    assert_eq!(mixed.last_phases, all_new.last_phases);
    assert_eq!(
        mixed.outcomes, all_new.outcomes,
        "the journal {version} started ends unlike one this build started"
    );
    assert!(
        mixed.outcomes.iter().all(|(_, o)| o == "passed"),
        "{:?}",
        mixed.outcomes
    );
    assert_eq!(
        mixed.log, all_new.log,
        "phases 11 to 14 asked the stubs for something else after {version}"
    );
}

/// The second half from `version`: the peering it takes through phase 14 is
/// accepted by the build under test, which runs phase 14 alone and leaves the
/// records of phases 1 to 13 as they were.
fn accepted_by_this_build(version: &str) {
    let Some(old) = released::binary(version) else {
        return;
    };
    let w = world();
    let (out, v) = run_as(&old, &w, &["hello-ally"]);
    assert!(out.status.success(), "{version}: first run: {}", says(&out));
    assert_eq!(phases_run(&v), [1, 2, 3, 4, 5], "{}", says(&out));
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    let (out, v) = run_as(&old, &w, &["hello-ally", "--from", "6"]);
    assert!(out.status.success(), "{version}: --from 6: {}", says(&out));
    assert_eq!(phases_run(&v), [6, 7, 8, 9, 10], "{}", says(&out));
    let (out, v) = run_as(&old, &w, &["hello-ally", "--from", "11"]);
    assert!(out.status.success(), "{version}: --from 11: {}", says(&out));
    assert_eq!(phases_run(&v), [11, 12, 13, 14], "{}", says(&out));
    let before = w.state();
    w.clear_log();
    let (out, v) = w.run(&["hello-ally", "--from", "14", "--to", "14"]);
    assert!(
        out.status.success(),
        "the build under test, phase 14 on {version}'s peering: {}",
        says(&out)
    );
    assert_eq!(phases_run(&v), [14], "{}", says(&out));
    assert!(
        w.log().lines().any(|l| l.starts_with("acceptance")),
        "phase 14 ran no acceptance on {version}'s peering: {}",
        w.log()
    );
    let after = w.state();
    assert_eq!(
        after["phases"]["14"]["outcome"], "passed",
        "{}",
        after["phases"]["14"]
    );
    for n in 1..=13 {
        let k = n.to_string();
        assert_eq!(
            before["phases"][&k], after["phases"][&k],
            "phase {n}'s record changed when the build ran phase 14 on {version}'s peering"
        );
    }
}

/// Every 0.x release pinned in tests/upgrade/releases.sha256 from 0.5.1 on,
/// oldest first: a release pinned later is tested without a change here.
fn pinned_0x_from_0_5_1() -> Vec<String> {
    let mut v: Vec<(u32, u32, u32)> = include_str!("upgrade/releases.sha256")
        .lines()
        .filter_map(|l| l.split_once("  ")?.1.split('/').next())
        .filter_map(|tag| {
            let mut n = tag
                .strip_prefix('v')?
                .split('.')
                .map(|x| x.parse::<u32>().ok());
            Some((n.next()??, n.next()??, n.next()??))
        })
        .filter(|&(major, minor, _)| major == 0 && minor >= 5)
        .collect();
    v.sort();
    v.dedup();
    let v: Vec<String> = v.iter().map(|(a, b, c)| format!("v{a}.{b}.{c}")).collect();
    assert!(
        v.first().map(String::as_str) == Some("v0.5.1"),
        "pinned: {v:?}"
    );
    v
}

#[test]
fn e2_a_peering_taken_to_phase_10_by_each_pinned_0x_is_finished_by_this_build() {
    for version in pinned_0x_from_0_5_1() {
        finished_by_this_build(&version);
    }
}

#[test]
fn e2_a_peering_each_pinned_0x_finished_is_accepted_by_this_build() {
    for version in pinned_0x_from_0_5_1() {
        accepted_by_this_build(&version);
    }
}

/// The control: a journal of a schema this build does not read is refused,
/// and left as it was.
#[test]
fn e2_a_journal_of_a_schema_this_build_does_not_read_is_refused() {
    let w = world();
    std::fs::create_dir_all(&w.work).unwrap();
    let path = w.work.join("state.json");
    let newer = r#"{"schema":"goatbuilder.federate.state/v9","peering":"hello-ally","updated":""}"#;
    std::fs::write(&path, newer).unwrap();
    let (out, _) = w.run(&["hello-ally", "--from", "14", "--to", "14"]);
    assert!(!out.status.success(), "{}", says(&out));
    assert!(says(&out).contains("GB-182"), "{}", says(&out));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        newer,
        "the journal was changed"
    );
}
