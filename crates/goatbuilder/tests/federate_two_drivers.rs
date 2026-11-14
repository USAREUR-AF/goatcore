//! Gate row B10, its first piece: a peering declared `drivers: two` stops the
//! controller at phase 1 with the hand-off object of
//! docs/design/federation-two-drivers.md ("The hand-off object"), through the
//! real binary against a stand-in repository (federate_world).

mod federate_world;

use federate_world::{phases_run, public_b64, rows, says, sha256_hex, world, World};

fn git(w: &World, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.org",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(&w.repo)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The stand-in repository as a clean git checkout whose `hello-ally` entry
/// declares two drivers. Returns the commit.
fn two_drivers(w: &World) -> String {
    let cfg = w.repo.join("ops/goatbuilder/goatbuilder.yaml");
    let text = std::fs::read_to_string(&cfg).unwrap();
    let declared = text.replace(
        "    edge_class: edge\n",
        "    edge_class: edge\n    drivers: two\n",
    );
    assert_ne!(declared, text, "the fixture's hello-ally entry moved");
    std::fs::write(&cfg, declared).unwrap();
    git(w, &["init", "-q"]);
    git(w, &["add", "-A"]);
    git(w, &["commit", "-q", "-m", "fixture"]);
    git(w, &["rev-parse", "HEAD"])
}

fn strings(v: &[&str]) -> serde_json::Value {
    serde_json::Value::from(v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}

/// The phase 1 object, exactly as the contract writes it for this peering.
#[test]
fn phase_1_stops_with_the_contracts_hand_off_object() {
    let w = world();
    let commit = two_drivers(&w);
    let (out, v) = w.run(&["hello-ally", "--from", "1", "--to", "1"]);
    assert!(
        out.status.success(),
        "a hand-off stop exits 0, as a by-hand stop does: {}",
        says(&out)
    );
    assert_eq!(v["exit"], 0, "{}", says(&out));
    let h = &v["handoff"];
    assert_eq!(h["phase"], 1, "{}", says(&out));
    assert_eq!(h["code"], "GB-265");
    assert_eq!(h["run_on"], "edge");
    assert_eq!(
        h["command"],
        strings(&[
            "goatbuilder",
            "federate",
            "hello-ally",
            "--side",
            "edge",
            "--from",
            "1",
            "--to",
            "1",
            "--expect-commit",
            &commit
        ])
    );
    assert_eq!(h["commit"], commit.as_str());
    assert_eq!(h["carry"], serde_json::json!([]));
    assert_eq!(h["result"], "handoff/1/edge.json");
    assert_eq!(
        h["import"],
        strings(&[
            "goatbuilder",
            "federate",
            "import",
            "hello-ally",
            "1",
            "{result}",
            "--sha256",
            "{code}"
        ])
    );
    assert_eq!(
        h["then"],
        strings(&["goatbuilder", "federate", "hello-ally", "--from", "1"])
    );
    // Nothing of phase 1 ran here: the stop comes before acting.
    assert!(
        !w.log().contains("preflight"),
        "phase 1 acted before the hand-off: {}",
        w.log()
    );
    assert_eq!(w.state()["phases"]["1"]["outcome"], "stopped");
    assert_eq!(phases_run(&v), [1]);
}

/// The text a person reads names every argv of the object, quoted for a POSIX
/// shell, each on its own line.
#[test]
fn the_text_names_every_argv_of_the_object() {
    let w = world();
    two_drivers(&w);
    let (out, v) = w.run(&["hello-ally", "--from", "1", "--to", "1"]);
    assert!(out.status.success(), "{}", says(&out));
    let text: Vec<&str> = rows(&v, "handoff");
    assert!(!text.is_empty(), "no hand-off row: {}", says(&out));
    for member in ["command", "import", "then"] {
        let argv: Vec<String> = v["handoff"][member]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_string())
            .collect();
        let line = shell_line(&argv);
        assert!(
            text.iter().any(|t| t.lines().any(|l| l.trim() == line)),
            "{member}: no line `{line}` in {text:?}"
        );
    }
    assert!(text.iter().any(|t| t.contains("GB-265")), "{text:?}");
}

/// The test's own POSIX quoting: a word made only of safe characters as it
/// is, any other in single quotes.
fn shell_line(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if !a.is_empty()
                && a.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c))
            {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A controller whose checkout has changes emits no hand-off object: the edge
/// could not be pinned to what the controller holds. It names each file.
#[test]
fn a_dirty_checkout_names_each_file_and_hands_off_nothing() {
    let w = world();
    two_drivers(&w);
    std::fs::write(w.repo.join("uncommitted.txt"), "x").unwrap();
    std::fs::write(
        w.repo.join("ops/goatbuilder/goatbuilder.yaml"),
        std::fs::read_to_string(w.repo.join("ops/goatbuilder/goatbuilder.yaml")).unwrap() + "\n",
    )
    .unwrap();
    let (out, v) = w.run(&["hello-ally", "--from", "1", "--to", "1"]);
    assert!(
        !out.status.success(),
        "a dirty checkout is refused: {}",
        says(&out)
    );
    assert!(
        v["handoff"].is_null(),
        "a hand-off object from a dirty checkout: {}",
        says(&out)
    );
    let all = says(&out);
    assert!(all.contains("GB-266"), "{all}");
    assert!(
        all.contains("uncommitted.txt") && all.contains("ops/goatbuilder/goatbuilder.yaml"),
        "{all}"
    );
    assert!(!w.log().contains("preflight"), "{}", w.log());
}

/// One driver, the default: phase 1 runs as it always has, with no hand-off.
#[test]
fn one_driver_is_the_default_and_hands_off_nothing() {
    let w = world();
    let (out, v) = w.run(&["hello-ally", "--from", "1", "--to", "1"]);
    assert!(out.status.success(), "{}", says(&out));
    assert!(v["handoff"].is_null(), "{}", says(&out));
    assert!(
        w.log().contains("preflight"),
        "phase 1 did not run: {}",
        w.log()
    );
}

/// The edge driver's mode is offered, and refused until it is built.
#[test]
fn side_edge_is_offered_and_refused_until_built() {
    let w = world();
    let help = std::process::Command::new(federate_world::bin())
        .args(["federate", "--help"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&help.stdout).contains("--side"),
        "{}",
        String::from_utf8_lossy(&help.stdout)
    );
    two_drivers(&w);
    let (out, _) = w.run(&["hello-ally", "--side", "edge", "--from", "1", "--to", "1"]);
    assert!(!out.status.success(), "{}", says(&out));
    assert!(says(&out).contains("not built"), "{}", says(&out));
    assert!(says(&out).contains("GB-267"), "{}", says(&out));
    assert!(!says(&out).contains("GB-189"), "{}", says(&out));
    assert!(!w.log().contains("preflight"), "{}", w.log());
}

/// Phase 2's object is the contract's too: the interface and its key, no
/// file carried, its result and its `then` at phase 2.
#[test]
fn phase_2_stops_with_the_contracts_hand_off_object() {
    let w = world();
    let commit = two_drivers(&w);
    let (out, v) = w.run(&["hello-ally", "--from", "2", "--to", "2"]);
    assert!(out.status.success(), "{}", says(&out));
    let h = &v["handoff"];
    assert_eq!(h["phase"], 2, "{}", says(&out));
    assert_eq!(h["code"], "GB-265");
    assert_eq!(h["run_on"], "edge");
    assert_eq!(h["commit"], commit.as_str());
    assert_eq!(
        h["import"],
        strings(&[
            "goatbuilder",
            "federate",
            "import",
            "hello-ally",
            "2",
            "{result}",
            "--sha256",
            "{code}"
        ])
    );
    assert_eq!(
        h["command"],
        strings(&[
            "goatbuilder",
            "federate",
            "hello-ally",
            "--side",
            "edge",
            "--from",
            "2",
            "--to",
            "2",
            "--expect-commit",
            &commit
        ])
    );
    assert_eq!(h["carry"], serde_json::json!([]));
    assert_eq!(h["result"], "handoff/2/edge.json");
    assert_eq!(
        h["then"],
        strings(&["goatbuilder", "federate", "hello-ally", "--from", "2"])
    );
    assert!(
        !w.log().contains("play peering-prepare.yml"),
        "phase 2 acted: {}",
        w.log()
    );
}

/// From phase 3 on, the generic object is not the contract's (phase 3's
/// result is the edge half, phases 11 to 13 carry files, phase 14 binds by
/// id), so a two-driver run there is refused before acting, and no object is
/// printed.
#[test]
fn a_two_driver_run_after_phase_2_is_refused_with_no_object() {
    for (from, acted) in [
        ("3", "goat init"),
        ("11", "play peering-wg.yml"),
        ("12", "play peering-zenoh-edge.yml"),
        ("14", "acceptance"),
    ] {
        let w = world();
        two_drivers(&w);
        let (out, v) = w.run(&["hello-ally", "--from", from, "--to", from]);
        let all = says(&out);
        assert!(!out.status.success(), "--from {from}: {all}");
        assert!(
            v["handoff"].is_null(),
            "--from {from} printed an object: {all}"
        );
        assert!(
            all.contains("GB-267") && all.contains("not built"),
            "--from {from}: {all}"
        );
        assert!(
            all.contains(&format!("nothing of phase {from} ran")),
            "--from {from}: {all}"
        );
        assert!(!all.contains("Nothing ran"), "--from {from}: {all}");
        assert!(!w.log().contains(acted), "--from {from} acted: {}", w.log());
        // Refused before the run began: no journal was written.
        assert!(
            !w.work.join("state.json").exists(),
            "--from {from}: a journal was written"
        );
    }
}

/// A run from 4 with no --to would run 4 to 10, which need no edge, then
/// meet 11, which does. It is refused before phase 4 acts, and says so.
#[test]
fn a_two_driver_run_from_4_to_the_end_runs_nothing() {
    let w = world();
    two_drivers(&w);
    let (out, v) = w.run(&["hello-ally", "--from", "4"]);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert!(v["handoff"].is_null(), "{all}");
    assert!(all.contains("GB-267"), "{all}");
    assert!(all.contains("nothing of phase 11 ran"), "{all}");
    assert!(!all.contains("Nothing ran"), "{all}");
    assert_eq!(w.log(), "", "the run acted");
    assert!(!w.work.join("state.json").exists(), "a journal was written");
}

/// Phases 1 to 9 with one driver (confirmation 1 by flag), then the peering
/// declared with two drivers, and a plan for what is declared now, so the plan
/// holds. Returns its hash.
fn planned_with_two_drivers(w: &World) -> String {
    let fp = sha256_hex(b"fake-ca-der");
    let key = public_b64("xab");
    let digest = sha256_hex(&std::fs::read(w.root.path().join("fake/agreement.json")).unwrap());
    let (out, _) = w.run(&[
        "hello-ally",
        "--to",
        "9",
        "--partner-key",
        &key,
        "--partner-ca-fingerprint",
        &fp,
        "--agreement-digest",
        &digest,
    ]);
    assert!(out.status.success(), "{}", says(&out));
    two_drivers(w);
    let (out, _) = w.run(&["plan", "hello-ally"]);
    assert!(out.status.success(), "{}", says(&out));
    let plan: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(w.work.join("plan.json")).unwrap()).unwrap();
    assert_eq!(plan["verdict"], "READY", "{plan}");
    plan["hash"].as_str().unwrap().to_string()
}

/// The approved-plan path (`deploy`) admits the edge on our core at phase 10
/// and carries on into 11, which needs the edge. For a two-driver peering that
/// would leave the admission half done. A two-driver deploy is refused before
/// it acts, in either form: no live recheck, no claim, no admission, no play.
#[test]
fn a_two_driver_deploy_runs_nothing() {
    let w = world();
    let plan = planned_with_two_drivers(&w);
    let journal = w.state();
    for extra in [
        &[][..],
        &["--review-in-pr"][..],
        &["--approve-admission", "sha256:00"][..],
    ] {
        w.clear_log();
        let mut args = vec!["deploy", "hello-ally", "--approve-plan", &plan];
        args.extend(extra);
        let (out, v) = w.run(&args);
        let all = says(&out);
        assert!(!out.status.success(), "{extra:?}: {all}");
        assert!(all.contains("GB-267"), "{extra:?}: {all}");
        assert!(
            !all.contains("GB-188"),
            "{extra:?}: the plan should hold: {all}"
        );
        assert!(v["handoff"].is_null(), "{extra:?}: {all}");
        assert_eq!(w.log(), "", "{extra:?}: the deploy acted");
        assert_eq!(w.state(), journal, "{extra:?}: the journal changed");
        assert!(
            !w.root.path().join("core/config.json5").exists(),
            "{extra:?}: our core admitted the edge"
        );
        assert!(
            !w.work.join("admission").exists(),
            "{extra:?}: an admission review was written"
        );
    }
}

/// `--side` is one flag, shared with `init --over-ssh`'s phase 3 halves, so
/// GB-267 answers only `--side edge` on its own on a peering run. A
/// misplaced flag for the built halves, or `--side edge` beside other
/// measure flags or on `status`, gets the message it got before, never
/// GB-267's advice to remove `drivers: two`.
#[test]
fn side_edge_is_gb_267_only_alone_on_a_peering_run() {
    let w = world();
    two_drivers(&w);
    let (out, _) = w.run(&[
        "--side",
        "edge",
        "init",
        "--over-ssh",
        "xab",
        "--half-out",
        "f",
    ]);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert!(all.contains("after the word init"), "{all}");
    assert!(!all.contains("GB-267"), "{all}");

    let (out, _) = w.run(&[
        "hello-ally",
        "--over-ssh",
        "xab",
        "--side",
        "edge",
        "--half-out",
        "f",
    ]);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert!(all.contains("after the word init"), "{all}");
    assert!(!all.contains("GB-267"), "{all}");

    let (out, _) = w.run(&["status", "--side", "edge"]);
    assert!(!says(&out).contains("GB-267"), "{}", says(&out));

    let (out, _) = w.run(&["hello-ally", "--side", "edge"]);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert!(all.contains("GB-267"), "{all}");
    assert!(!w.log().contains("preflight"), "{}", w.log());
}

/// Every command a hand-off object names parses with the binary that printed
/// it: run as written (placeholders filled in), none is a usage error. The
/// edge driver's run and `import` are not built, so each refuses with GB-267;
/// `then` is an ordinary run, which stops at the same hand-off again.
#[test]
fn every_command_a_hand_off_names_parses_and_refuses_as_not_built() {
    for phase in ["1", "2"] {
        let w = world();
        two_drivers(&w);
        let (out, v) = w.run(&["hello-ally", "--from", phase, "--to", phase]);
        assert!(out.status.success(), "{}", says(&out));
        let h = v["handoff"].clone();
        assert_eq!(
            h["phase"].as_u64().map(|n| n.to_string()).as_deref(),
            Some(phase)
        );
        let argv = |member: &str| -> Vec<String> {
            h[member]
                .as_array()
                .unwrap_or_else(|| panic!("{member}: {h}"))
                .iter()
                .map(|s| {
                    s.as_str()
                        .unwrap()
                        .replace("{result}", h["result"].as_str().unwrap())
                        .replace("{code}", "0123456789abcdef0123456789abcdef")
                })
                .collect()
        };
        for member in ["command", "import", "then"] {
            let a = argv(member);
            assert_eq!(&a[..2], ["goatbuilder", "federate"], "{member}: {a:?}");
            let rest: Vec<&str> = a[2..].iter().map(String::as_str).collect();
            let (out, v) = w.run(&rest);
            let all = says(&out);
            assert_ne!(
                out.status.code(),
                Some(2),
                "phase {phase} {member} {a:?}: {all}"
            );
            assert!(
                !all.contains("unexpected argument") && !all.contains("Usage:"),
                "phase {phase} {member} {a:?}: a usage error: {all}"
            );
            if member == "then" {
                assert!(out.status.success(), "phase {phase} then: {all}");
                assert_eq!(
                    v["handoff"]["phase"], h["phase"],
                    "phase {phase} then: {all}"
                );
            } else if member == "import" && phase == "1" {
                // Phase 1's import is built: a result file that is not there
                // is refused as an edge result that cannot be imported.
                assert!(!out.status.success(), "phase {phase} {member}: {all}");
                assert!(all.contains("GB-277"), "phase {phase} {member}: {all}");
            } else {
                assert!(!out.status.success(), "phase {phase} {member}: {all}");
                assert!(
                    all.contains("GB-267") && all.contains("not built"),
                    "phase {phase} {member}: {all}"
                );
            }
        }
    }
}

/// A phase 1 edge result as the edge driver's run writes it: preflight's
/// result for the edge side's checks.
fn edge_result(peering: &str, side: &str, ready: bool) -> String {
    let outcome = if ready { "PASS" } else { "FAIL" };
    serde_json::json!({
        "schema": "goat-federation-preflight/v1",
        "peering": peering,
        "only": [],
        "side": side,
        "ready": ready,
        "results": [
            {"check": "inputs", "outcome": "PASS", "code": "", "detail": "edge inputs"},
            {"check": "edge-ports", "outcome": outcome, "code": if ready { "" } else { "GB-131" }, "detail": "edge ports"}
        ]
    })
    .to_string()
}

/// The confirm code an edge driver prints: the first 32 hex digits of the
/// result's sha256, in groups of four.
fn confirm_code(bytes: &[u8]) -> String {
    let hex = sha256_hex(bytes);
    let hex = hex.trim_start_matches("sha256:");
    hex.as_bytes()[..32]
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Gate row B10, slice 1: the controller imports phase 1's edge result with
/// the confirm code, journals its sha256, and `--from 1` runs phase 1's core
/// half against it, joins the two, and goes on to phase 2's hand-off.
#[test]
fn an_imported_phase_1_edge_result_carries_the_controller_to_phase_2() {
    let w = world();
    two_drivers(&w);
    let (out, v) = w.run(&["hello-ally", "--from", "1", "--to", "1"]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(v["handoff"]["phase"], 1);
    let carried = w.root.path().join("from-the-edge.json");
    let text = edge_result("hello-ally", "edge", true);
    std::fs::write(&carried, &text).unwrap();
    let code = confirm_code(text.as_bytes());
    let (out, v) = w.run(&[
        "import",
        "hello-ally",
        "1",
        carried.to_str().unwrap(),
        "--sha256",
        &code,
    ]);
    assert!(out.status.success(), "the import: {}", says(&out));
    assert_eq!(v["exit"], 0, "{}", says(&out));
    let placed = w.work.join("handoff/1/edge.json");
    assert_eq!(
        std::fs::read_to_string(&placed).unwrap(),
        text,
        "the import places the file as it was"
    );
    let full = sha256_hex(text.as_bytes());
    let journal = w.state();
    let confirmed = journal["confirmations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "operator-step" && c["phase"] == 1)
        .cloned()
        .unwrap_or_else(|| panic!("no import in the journal: {journal}"));
    assert_eq!(confirmed["bound_to"], full.as_str(), "{confirmed}");
    w.clear_log();
    let (out, v) = w.run(&["hello-ally", "--from", "1"]);
    assert!(out.status.success(), "then: {}", says(&out));
    assert_eq!(
        v["handoff"]["phase"],
        2,
        "then goes on to phase 2's hand-off: {}",
        says(&out)
    );
    assert!(
        w.log().contains("preflight") && w.log().contains("--side core"),
        "phase 1's core half did not run: {}",
        w.log()
    );
    assert_eq!(
        w.state()["phases"]["1"]["outcome"],
        "passed",
        "{}",
        w.state()
    );
    // The phase 1 result is the two sides joined: ready, and holding both.
    let joined: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(w.work.join("preflight/result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(joined["side"], "both", "{joined}");
    assert_eq!(joined["ready"], true, "{joined}");
    let checks: Vec<&str> = joined["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["check"].as_str())
        .collect();
    assert!(
        checks.contains(&"edge-ports") && checks.contains(&"core-ports"),
        "{joined}"
    );
}

/// What the import refuses, each with GB-277 and nothing recorded: a code
/// that is not the file's, a code too short to bind it, another peering's
/// result, a result of the core side, and a peering not waiting on phase 1.
#[test]
fn an_edge_result_that_is_not_this_phases_is_not_imported() {
    let w = world();
    two_drivers(&w);
    let carried = w.root.path().join("from-the-edge.json");
    let good = edge_result("hello-ally", "edge", true);
    std::fs::write(&carried, &good).unwrap();
    let code = confirm_code(good.as_bytes());
    let path = carried.to_str().unwrap().to_string();
    // Nothing is waiting on phase 1 yet.
    let (out, _) = w.run(&["import", "hello-ally", "1", &path, "--sha256", &code]);
    assert!(
        !out.status.success() && says(&out).contains("GB-277"),
        "{}",
        says(&out)
    );
    let (out, _) = w.run(&["hello-ally", "--from", "1", "--to", "1"]);
    assert!(out.status.success(), "{}", says(&out));
    for (what, text, code) in [
        (
            "another file's code",
            good.clone(),
            confirm_code(b"another file"),
        ),
        (
            "a code too short to bind the file",
            good.clone(),
            sha256_hex(good.as_bytes()).trim_start_matches("sha256:")[..8].to_string(),
        ),
        (
            "another peering's result",
            edge_result("xab", "edge", true),
            String::new(),
        ),
        (
            "the core side's result",
            edge_result("hello-ally", "core", true),
            String::new(),
        ),
        (
            "not a phase 1 result",
            "{\"schema\":\"something-else/v1\"}".to_string(),
            String::new(),
        ),
    ] {
        std::fs::write(&carried, &text).unwrap();
        let code = if code.is_empty() {
            confirm_code(text.as_bytes())
        } else {
            code
        };
        let (out, _) = w.run(&["import", "hello-ally", "1", &path, "--sha256", &code]);
        assert!(!out.status.success(), "{what}: imported: {}", says(&out));
        assert!(says(&out).contains("GB-277"), "{what}: {}", says(&out));
        assert!(
            !w.work.join("handoff/1/edge.json").exists(),
            "{what}: placed"
        );
        assert!(
            !w.state()["confirmations"]
                .as_array()
                .map(|a| a
                    .iter()
                    .any(|c| c["kind"] == "operator-step" && c["phase"] == 1))
                .unwrap_or(false),
            "{what}: journaled"
        );
    }
}

/// `--from 1` hashes the imported file again before reading it: a file
/// changed since its import stops the run with the hand-off code, before
/// phase 1's core half runs. An edge result that is not ready fails phase 1
/// before its core half runs.
#[test]
fn a_changed_or_unready_edge_result_stops_before_the_core_half() {
    for ready in [true, false] {
        let w = world();
        two_drivers(&w);
        let (out, _) = w.run(&["hello-ally", "--from", "1", "--to", "1"]);
        assert!(out.status.success(), "{}", says(&out));
        let carried = w.root.path().join("from-the-edge.json");
        let text = edge_result("hello-ally", "edge", ready);
        std::fs::write(&carried, &text).unwrap();
        let (out, _) = w.run(&[
            "import",
            "hello-ally",
            "1",
            carried.to_str().unwrap(),
            "--sha256",
            &confirm_code(text.as_bytes()),
        ]);
        assert!(out.status.success(), "{}", says(&out));
        if ready {
            std::fs::write(
                w.work.join("handoff/1/edge.json"),
                edge_result("hello-ally", "edge", false),
            )
            .unwrap();
        }
        w.clear_log();
        let (out, v) = w.run(&["hello-ally", "--from", "1"]);
        assert!(
            !w.log().contains("preflight"),
            "ready={ready}: the core half ran: {}",
            w.log()
        );
        if ready {
            assert!(
                says(&out).contains("GB-265"),
                "a changed file: {}",
                says(&out)
            );
            assert_eq!(
                w.state()["phases"]["1"]["outcome"],
                "stopped",
                "{}",
                w.state()
            );
        } else {
            assert!(
                !out.status.success(),
                "an unready edge passed phase 1: {}",
                says(&out)
            );
            assert!(
                says(&out).contains("edge-ports") && says(&out).contains("GB-131"),
                "{}",
                says(&out)
            );
            assert_eq!(
                w.state()["phases"]["1"]["outcome"],
                "failed",
                "{}",
                w.state()
            );
        }
        let _ = v;
    }
}
