//! The run journal, the per-peering lock and `--resume`, through the real
//! binary against a stand-in repository (federate_world).

mod federate_world;

use federate_world::{phases_run, rows, says, world};

/// Kill the whole process group the child leads (it was spawned with
/// `process_group(0)`): goatbuilder, its bash and the sleeping stub. Asserts
/// the kill was delivered and the child died of it, so a kill that did not
/// land fails here instead of letting the run finish.
fn kill_group(child: &mut std::process::Child) {
    use std::os::unix::process::ExitStatusExt;
    let pgid = child.id();
    let sent = std::process::Command::new("kill")
        .args(["-s", "KILL", "--", &format!("-{pgid}")])
        .status()
        .expect("run kill");
    assert!(sent.success(), "kill -s KILL -- -{pgid} failed: {sent}");
    let status = child.wait().expect("wait for the killed run");
    assert_eq!(status.signal(), Some(9), "the run was not killed: {status}");
}

/// Without `--resume` a real run behaves as it always has, and is recorded:
/// the stop at phase 5, `--from 6` declaring it done, the stop at phase 10,
/// `--from 11` to the end.
#[test]
fn a_run_is_recorded_and_behaves_as_before() {
    let w = world();
    let (out, v) = w.run(&["hello-ally"]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [1, 2, 3, 4, 5], "{}", says(&out));
    assert!(rows(&v, "note").iter().any(|d| d.contains("OPERATOR STEP")));
    let st = w.state();
    assert_eq!(st["schema"], "goatbuilder.federate.state/v1");
    assert_eq!(st["peering"], "hello-ally");
    assert_eq!(st["runs"][0]["outcome"], "stopped");
    assert_eq!(st["phases"]["5"]["outcome"], "stopped");
    assert_eq!(st["phases"]["3"]["outcome"], "passed");

    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    let (out, v) = w.run(&["hello-ally", "--from", "6"]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [6, 7, 8, 9, 10]);
    let st = w.state();
    assert_eq!(
        st["phases"]["5"]["outcome"], "passed",
        "--from 6 declares phase 5 done"
    );
    assert!(st["confirmations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["kind"] == "operator-step" && c["phase"] == 5));

    let (out, v) = w.run(&["hello-ally", "--from", "11"]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [11, 12, 13, 14]);
    let st = w.state();
    assert_eq!(st["runs"].as_array().unwrap().len(), 3);
    assert_eq!(st["runs"][2]["outcome"], "passed");
    assert_eq!(st["phases"]["14"]["outcome"], "passed");
}

/// A dry run and a mock execute nothing, so they write no journal and take
/// no lock.
#[test]
fn a_plan_or_a_mock_writes_no_journal() {
    let w = world();
    for args in [
        vec!["hello-ally", "--dry-run"],
        vec!["hello-ally", "--dry-run", "--resume"],
    ] {
        let (out, _) = w.run(&args);
        assert!(out.status.success(), "{}", says(&out));
    }
    let mock = w.repo.join("ops/goatbuilder").display().to_string();
    let (out, v) = w.run_with(&["--mock", &mock], &["hello-ally", "--resume"]);
    assert!(out.status.success(), "{}", says(&out));
    assert!(!rows(&v, "mock").is_empty(), "{}", says(&out));
    assert!(!w.work.join("state.json").exists());
    assert!(!w.work.join(".lock").exists());
}

/// A phase that fails is recorded as failed with GB-183; `--resume` then
/// starts again where the files it can see stop matching, skips what is
/// unchanged, and runs every check and every play again.
#[test]
fn resume_after_a_failure_skips_only_what_it_can_see() {
    let w = world();
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    w.run(&["hello-ally"]);
    w.run(&["hello-ally", "--from", "6"]);
    std::fs::write(&w.fail_12, "").unwrap();
    let (out, _) = w.run(&["hello-ally", "--from", "11"]);
    assert!(!out.status.success(), "phase 12 fails: {}", says(&out));
    let st = w.state();
    assert_eq!(st["phases"]["12"]["outcome"], "failed");
    assert!(st["phases"]["12"]["detail"]
        .as_str()
        .unwrap()
        .starts_with("GB-183: phase 12 failed with exit 2"));
    assert_eq!(
        st["runs"].as_array().unwrap().last().unwrap()["outcome"],
        "failed"
    );

    std::fs::remove_file(&w.fail_12).unwrap();
    w.clear_log();
    let (out, v) = w.run(&["hello-ally", "--resume"]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(
        rows(&v, "skipped").len(),
        5,
        "3, 5, 6, 9, 10: {}",
        says(&out)
    );
    assert_eq!(phases_run(&v), [1, 2, 4, 7, 8, 11, 12, 13, 14]);
    let log = w.log();
    assert!(
        !log.contains("goat federate init")
            && !log.contains("goat federate compose")
            && !log.contains("goat federate apply"),
        "{log}"
    );
    assert!(
        log.contains("goat federate lint")
            && log.contains("goat federate verify")
            && log.contains("mint"),
        "checks and minting run again: {log}"
    );
    for play in [
        "peering-prepare.yml",
        "peering-wg.yml",
        "peering-zenoh-edge.yml",
        "peering-spire.yml",
    ] {
        assert!(log.contains(play), "plays run again: {play}: {log}");
    }

    // Without --resume, nothing is skipped: the default is unchanged.
    w.clear_log();
    let (out, v) = w.run(&["hello-ally", "--to", "4"]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases_run(&v), [1, 2, 3, 4]);
    assert!(rows(&v, "skipped").is_empty());
}

/// A run killed in the middle of phase 6 (compose, a phase resume could
/// skip) leaves a journal that parses, shows the phase it was in, and a lock
/// that died with it. `--resume` runs phase 6 again, because nothing records
/// it passed.
#[test]
fn a_killed_run_resumes() {
    use std::os::unix::process::CommandExt;
    let w = world();
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    w.run(&["hello-ally"]);
    std::fs::write(&w.hang_6, "").unwrap();
    let mut child = w
        .command(&["hello-ally", "--from", "6"])
        .process_group(0)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !w.log().contains("goat federate compose") {
        assert!(
            std::time::Instant::now() < deadline,
            "phase 6 never started: {}",
            w.log()
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    kill_group(&mut child);
    std::fs::remove_file(&w.hang_6).unwrap();

    let st = w.state();
    let last = st["runs"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        last["ended"],
        serde_json::Value::Null,
        "a killed run has no end: {last}"
    );
    let a = st["attempts"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        (a["phase"].as_u64(), a["event"].as_str()),
        (Some(6), Some("start")),
        "the journal shows the phase it was in: {a}"
    );
    assert!(
        st["phases"].get("6").is_none(),
        "phase 6 never recorded an outcome"
    );
    assert_eq!(
        st["phases"]["5"]["outcome"], "passed",
        "declared done before the kill"
    );

    w.clear_log();
    let (out, v) = w.run(&["hello-ally", "--resume", "--to", "9"]);
    assert!(
        out.status.success(),
        "the lock died with the killed run: {}",
        says(&out)
    );
    assert_eq!(
        phases_run(&v),
        [1, 2, 4, 6, 7, 8, 9],
        "3 and 5 skipped, 6 runs again: {}",
        says(&out)
    );
    assert!(w.log().contains("goat federate compose"));
    assert_eq!(w.state()["phases"]["6"]["outcome"], "passed");
}

/// Two runs of one peering at once: the second is refused with GB-180 and
/// changes nothing.
#[test]
fn a_second_run_of_the_same_peering_is_refused() {
    use std::os::unix::process::CommandExt;
    let w = world();
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    w.run(&["hello-ally"]);
    w.run(&["hello-ally", "--from", "6"]);
    std::fs::write(&w.hang_12, "").unwrap();
    let mut first = w
        .command(&["hello-ally", "--from", "11"])
        .process_group(0)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !w.log().contains("play peering-zenoh-edge.yml") {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let before = std::fs::read_to_string(w.work.join("state.json")).unwrap();
    let (out, _) = w.run(&["hello-ally", "--from", "11"]);
    let text = says(&out);
    kill_group(&mut first);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("GB-180") && text.contains("another run of peering 'hello-ally' holds"),
        "{text}"
    );
    assert_eq!(
        before,
        std::fs::read_to_string(w.work.join("state.json")).unwrap(),
        "the refused run wrote nothing"
    );
}
