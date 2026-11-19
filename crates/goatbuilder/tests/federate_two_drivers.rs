//! Gate row B10, its first piece: a peering declared `drivers: two` stops the
//! controller at phase 1 with the hand-off object of
//! docs/design/federation-two-drivers.md ("The hand-off object"), through the
//! real binary against a stand-in repository (federate_world).

mod federate_world;

use federate_world::{
    phases_run, public_b64, rows, says, sha256_hex, world, World, CORE_ADDRESS, EDGE_ADDRESS,
};

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

/// The edge driver's mode is offered; after phase 1 it is refused until built.
#[test]
fn side_edge_is_offered_and_refused_after_phase_1() {
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
    // Phase 1's edge run is built; phase 2's is not.
    let (out, _) = w.run(&["hello-ally", "--side", "edge", "--from", "2", "--to", "2"]);
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

/// From phase 11 on, the hand-off is not built (phases 11 to 13 carry files,
/// phase 14 binds by id), so a two-driver run there is refused before acting,
/// and no object is printed. Phase 3's hand-off is built (below).
#[test]
fn a_two_driver_run_at_11_or_later_is_refused_with_no_object() {
    for (from, acted) in [
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
/// only `--side edge` on its own on a peering run is the edge driver's run. A
/// misplaced flag for the built halves, or `--side edge` beside other
/// measure flags or on `status`, gets the message it got before.
#[test]
fn side_edge_alone_on_a_peering_run_is_the_edge_drivers_run() {
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

    // Alone on a peering run, it is the edge driver's run, which runs one
    // phase, as a hand-off names it, and never answers GB-267 here.
    let (out, _) = w.run(&["hello-ally", "--side", "edge"]);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert!(
        all.contains("one phase") && !all.contains("GB-267"),
        "{all}"
    );
    assert!(!w.log().contains("preflight"), "{}", w.log());
}

/// Every command a hand-off object names parses with the binary that printed
/// it: run as written (placeholders filled in), none is a usage error.
/// Phase 1's edge run and import are built: the run ends with the way back,
/// and the import refuses a file that is not there (GB-277). Phase 2's are
/// not built, so each refuses with GB-267. `then` is an ordinary run, which
/// stops at the same hand-off again while nothing is imported.
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
            } else if member == "command" && phase == "1" {
                // Phase 1's edge run is built: it ends with the way back.
                assert!(out.status.success(), "phase {phase} {member}: {all}");
                assert_eq!(
                    v["handoff"]["run_on"], "controller",
                    "phase {phase} {member}: {all}"
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

/// The edge driver's phase 1, as its controller's hand-off names it.
fn edge_run(
    w: &World,
    commit: &str,
    edge_work: &std::path::Path,
    extra: &[&str],
) -> (std::process::Output, serde_json::Value) {
    let mut args = vec![
        "hello-ally",
        "--side",
        "edge",
        "--from",
        "1",
        "--to",
        "1",
        "--expect-commit",
        commit,
        "--work",
        edge_work.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    w.run(&args)
}

/// The edge driver's run of phase 1 runs preflight's edge checks alone,
/// writes their result under its own work, and prints the way-back object
/// with the confirm code. The controller imports the file with that code and
/// goes on to phase 2.
#[test]
fn an_edge_drivers_phase_1_writes_its_result_and_prints_the_way_back() {
    let w = world();
    let commit = two_drivers(&w);
    let (out, _) = w.run(&["hello-ally", "--from", "1", "--to", "1"]);
    assert!(out.status.success(), "{}", says(&out));
    w.clear_log();

    let edge_work = w.root.path().join("edge-work");
    let (out, v) = edge_run(&w, &commit, &edge_work, &[]);
    assert!(out.status.success(), "{}", says(&out));
    let written = edge_work.join("handoff/1/edge.json");
    let log = w.log();
    assert!(
        log.lines().any(|l| l.starts_with("preflight hello-ally")
            && l.contains("--side edge")
            && l.contains(&format!("--result {}", written.display()))),
        "{log}"
    );
    let bytes = std::fs::read(&written).expect("the edge driver wrote its result");
    let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(doc["side"], "edge", "{doc}");
    assert_eq!(doc["peering"], "hello-ally", "{doc}");
    let code = confirm_code(&bytes);
    let h = &v["handoff"];
    assert_eq!(h["phase"], 1, "{}", says(&out));
    assert_eq!(h["code"], "GB-265");
    assert_eq!(h["run_on"], "controller");
    assert!(h["command"].is_null(), "{h}");
    assert_eq!(h["commit"], commit.as_str());
    assert_eq!(h["carry"], serde_json::json!([]));
    assert_eq!(h["result"], "handoff/1/edge.json");
    assert_eq!(h["result_path"], written.to_str().unwrap());
    assert_eq!(h["result_sha256"], sha256_hex(&bytes));
    assert_eq!(h["confirm_code"], code.as_str());
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
            &code
        ])
    );
    assert_eq!(
        h["then"],
        strings(&["goatbuilder", "federate", "hello-ally", "--from", "1"])
    );
    let text = rows(&v, "handoff").join("\n");
    assert!(text.contains(&code), "{text}");
    assert!(
        text.contains(&format!(
            "goatbuilder federate import hello-ally 1 '{{result}}' --sha256 '{code}'"
        )),
        "{text}"
    );
    // The code is never in the file it confirms.
    let held = String::from_utf8_lossy(&bytes).to_string();
    assert!(
        !held.contains(&code.replace(' ', "")),
        "the result holds its own code: {held}"
    );

    // The way back, as the object names it, on the controller.
    let carried = w.root.path().join("carried.json");
    std::fs::copy(&written, &carried).unwrap();
    let (out, _) = w.run(&[
        "import",
        "hello-ally",
        "1",
        carried.to_str().unwrap(),
        "--sha256",
        &code,
    ]);
    assert!(out.status.success(), "{}", says(&out));
    let (out, v) = w.run(&["hello-ally", "--from", "1"]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(v["handoff"]["phase"], 2, "{}", says(&out));
}

/// The edge driver runs nothing unless its checkout is the controller's
/// commit with nothing changed, and every carried file has the digest the
/// hand-off names: each refusal is GB-278, and preflight never runs.
#[test]
fn the_edge_driver_refuses_a_checkout_or_carried_file_not_the_controllers() {
    let w = world();
    let commit = two_drivers(&w);
    let edge_work = w.root.path().join("edge-work");
    std::fs::create_dir_all(edge_work.join("pki")).unwrap();
    std::fs::write(edge_work.join("pki/carried.pem"), "carried\n").unwrap();
    let good = format!(
        "pki/carried.pem={}",
        sha256_hex(b"carried\n").trim_start_matches("sha256:")
    );
    let wrong = format!(
        "pki/carried.pem={}",
        sha256_hex(b"other\n").trim_start_matches("sha256:")
    );
    let missing = format!(
        "pki/absent.pem={}",
        sha256_hex(b"carried\n").trim_start_matches("sha256:")
    );
    let other = "0".repeat(40);
    let refused = |what: &str, out: &std::process::Output| {
        let all = says(out);
        assert!(!out.status.success(), "{what}: {all}");
        assert!(all.contains("GB-278"), "{what}: {all}");
        assert!(
            !w.log().contains("preflight"),
            "{what}: preflight ran: {}",
            w.log()
        );
        assert!(
            !edge_work.join("handoff/1/edge.json").exists(),
            "{what}: a result was written"
        );
    };
    let (out, _) = w.run(&[
        "hello-ally",
        "--side",
        "edge",
        "--from",
        "1",
        "--to",
        "1",
        "--work",
        edge_work.to_str().unwrap(),
    ]);
    refused("no --expect-commit", &out);
    let (out, _) = edge_run(&w, &other, &edge_work, &[]);
    refused("another commit", &out);
    std::fs::write(w.repo.join("changed.txt"), "x\n").unwrap();
    let (out, _) = edge_run(&w, &commit, &edge_work, &[]);
    refused("a checkout with changes", &out);
    assert!(says(&out).contains("changed.txt"), "{}", says(&out));
    std::fs::remove_file(w.repo.join("changed.txt")).unwrap();
    let (out, _) = edge_run(&w, &commit, &edge_work, &["--expect", &wrong]);
    refused("a carried file with another digest", &out);
    let (out, _) = edge_run(&w, &commit, &edge_work, &["--expect", &missing]);
    refused("a carried file that is not there", &out);

    let (out, _) = edge_run(&w, &commit, &edge_work, &["--expect", &good]);
    assert!(
        out.status.success(),
        "the carried file as named: {}",
        says(&out)
    );
    assert!(edge_work.join("handoff/1/edge.json").exists());
}

/// The edge driver's run is built for phase 1 only, one phase at a time, and
/// only on a peering declared with two drivers.
#[test]
fn the_edge_driver_runs_phase_1_alone_and_only_for_two_drivers() {
    let w = world();
    let commit = two_drivers(&w);
    let edge_work = w.root.path().join("edge-work");
    let e = edge_work.to_str().unwrap();
    let (out, _) = w.run(&[
        "hello-ally",
        "--side",
        "edge",
        "--from",
        "2",
        "--to",
        "2",
        "--expect-commit",
        &commit,
        "--work",
        e,
    ]);
    assert!(
        !out.status.success() && says(&out).contains("GB-267"),
        "{}",
        says(&out)
    );
    let (out, _) = w.run(&[
        "hello-ally",
        "--side",
        "edge",
        "--from",
        "1",
        "--to",
        "2",
        "--expect-commit",
        &commit,
        "--work",
        e,
    ]);
    assert!(
        !out.status.success() && says(&out).contains("one phase"),
        "{}",
        says(&out)
    );
    assert!(!w.log().contains("preflight"), "{}", w.log());

    let w = world();
    git(&w, &["init", "-q"]);
    git(&w, &["add", "-A"]);
    git(&w, &["commit", "-q", "-m", "fixture"]);
    let commit = git(&w, &["rev-parse", "HEAD"]);
    let (out, _) = w.run(&[
        "hello-ally",
        "--side",
        "edge",
        "--from",
        "1",
        "--to",
        "1",
        "--expect-commit",
        &commit,
        "--work",
        e,
    ]);
    let all = says(&out);
    assert!(
        !out.status.success() && all.contains("drivers: two"),
        "{all}"
    );
    assert!(!w.log().contains("preflight"), "{}", w.log());
}

// --- phase 3: the hand-off around the two halves ----------------------------

/// The phase 3 object: the edge half's command, its file, and the core half
/// after the import. Nothing is measured on either host at the stop.
#[test]
fn phase_3_stops_with_the_edge_half_hand_off_object() {
    let w = world();
    let commit = two_drivers(&w);
    let ssh = w.over_ssh();
    let (out, v) = w.run_over_ssh(&ssh, &["hello-ally", "--from", "3", "--to", "3"]);
    assert!(out.status.success(), "{}", says(&out));
    let h = &v["handoff"];
    assert_eq!(h["phase"], 3, "{}", says(&out));
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
            "3",
            "--to",
            "3",
            "--expect-commit",
            &commit
        ])
    );
    assert_eq!(h["commit"], commit.as_str());
    assert_eq!(h["carry"], serde_json::json!([]));
    assert_eq!(h["result"], "handoff/3/edge-half.json");
    assert_eq!(
        h["import"],
        strings(&[
            "goatbuilder",
            "federate",
            "import",
            "hello-ally",
            "3",
            "{result}",
            "--sha256",
            "{code}"
        ])
    );
    assert_eq!(
        h["then"],
        strings(&["goatbuilder", "federate", "hello-ally", "--from", "3"])
    );
    let text = rows(&v, "handoff").join("\n");
    for member in ["command", "import", "then"] {
        let argv: Vec<String> = h[member]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_string())
            .collect();
        let line = shell_line(&argv);
        assert!(
            text.lines().any(|l| l.trim() == line),
            "{member}: no line `{line}` in {text}"
        );
    }
    assert_eq!(ssh.calls(), "", "the stop measured a host");
    assert!(!w.log().contains("goat init"), "{}", w.log());
    assert_eq!(w.state()["phases"]["3"]["outcome"], "stopped");
    assert_eq!(phases_run(&v), [3]);
}

/// The edge driver's phase 3: the stop, then the edge half measured there.
/// Returns the edge half's bytes and the way-back object.
fn phase_3_edge_half(
    w: &World,
    ssh: &federate_world::OverSsh,
    commit: &str,
    edge_work: &std::path::Path,
) -> (Vec<u8>, serde_json::Value) {
    let (out, _) = w.run_over_ssh(ssh, &["hello-ally", "--from", "3", "--to", "3"]);
    assert!(out.status.success(), "{}", says(&out));
    ssh.reach_only(EDGE_ADDRESS);
    ssh.clear();
    let (out, v) = w.run_over_ssh(
        ssh,
        &[
            "hello-ally",
            "--side",
            "edge",
            "--from",
            "3",
            "--to",
            "3",
            "--expect-commit",
            commit,
            "--work",
            edge_work.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "{}", says(&out));
    let all = says(&out);
    // Inside the one command the edge half names the one command's import
    // and then, not init's own core-half line.
    assert!(!all.contains("--side core --edge-half"), "{all}");
    let bytes = std::fs::read(edge_work.join("handoff/3/edge-half.json"))
        .unwrap_or_else(|e| panic!("the edge driver wrote no edge half ({e}): {all}"));
    (bytes, v["handoff"].clone())
}

/// B10, phase 3: the edge driver measures the edge half with only the edge
/// reachable, and prints the way back with its confirm code; the controller
/// imports the file with that code, journals its sha256, and `--from 3` runs
/// the core half against it with only the core reachable. The edge is never
/// reached from the controller, nor the core from the edge driver.
#[test]
fn the_edge_driver_measures_phase_3s_edge_half_and_the_controller_joins_it() {
    let w = world();
    let commit = two_drivers(&w);
    let ssh = w.over_ssh();
    let edge_work = w.root.path().join("edge-work");
    let (bytes, h) = phase_3_edge_half(&w, &ssh, &commit, &edge_work);

    let calls = ssh.calls();
    assert!(!calls.is_empty(), "the edge was never reached");
    assert!(
        calls
            .lines()
            .all(|l| l.starts_with(&format!("ssh {EDGE_ADDRESS} "))),
        "the edge driver reached another host: {calls}"
    );
    let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(doc["schema"], "goatbuilder.phase3-edge-half/v1", "{doc}");
    assert_eq!(doc["peering"], "hello-ally", "{doc}");
    assert_eq!(doc["edge_host"], "edge-1", "{doc}");
    assert_eq!(doc["interface"], "wg-fed1", "{doc}");

    let code = confirm_code(&bytes);
    let written = edge_work.join("handoff/3/edge-half.json");
    assert_eq!(h["phase"], 3, "{h}");
    assert_eq!(h["code"], "GB-265");
    assert_eq!(h["run_on"], "controller");
    assert!(h["command"].is_null(), "{h}");
    assert_eq!(h["commit"], commit.as_str());
    assert_eq!(h["result"], "handoff/3/edge-half.json");
    assert_eq!(h["result_path"], written.to_str().unwrap());
    assert_eq!(h["result_sha256"], sha256_hex(&bytes));
    assert_eq!(h["confirm_code"], code.as_str());
    assert_eq!(
        h["import"],
        strings(&[
            "goatbuilder",
            "federate",
            "import",
            "hello-ally",
            "3",
            "{result}",
            "--sha256",
            &code
        ])
    );
    assert_eq!(
        h["then"],
        strings(&["goatbuilder", "federate", "hello-ally", "--from", "3"])
    );
    let held = String::from_utf8_lossy(&bytes).to_string();
    assert!(
        !held.contains(&code.replace(' ', "")),
        "the edge half holds its own code: {held}"
    );

    // The import, on the controller, with the code read from the edge driver.
    let carried = w.root.path().join("carried-edge-half.json");
    std::fs::copy(&written, &carried).unwrap();
    let (out, _) = w.run_over_ssh(
        &ssh,
        &[
            "import",
            "hello-ally",
            "3",
            carried.to_str().unwrap(),
            "--sha256",
            &code,
        ],
    );
    assert!(out.status.success(), "{}", says(&out));
    let placed = w.work.join("handoff/3/edge-half.json");
    assert_eq!(std::fs::read(&placed).unwrap(), bytes);
    let st = w.state();
    let bound = st["confirmations"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|c| c["kind"] == "operator-step" && c["phase"] == 3)
        .unwrap_or_else(|| panic!("no operator-step on phase 3: {st}"));
    assert_eq!(bound["bound_to"], sha256_hex(&bytes), "{bound}");

    // Then the core half, where only the core answers.
    ssh.reach_only(CORE_ADDRESS);
    ssh.clear();
    let (out, v) = w.run_over_ssh(&ssh, &["hello-ally", "--from", "3", "--to", "3"]);
    let all = says(&out);
    let calls = ssh.calls();
    assert!(
        calls
            .lines()
            .any(|l| l.starts_with(&format!("ssh {CORE_ADDRESS} "))),
        "the core half never reached the core: {calls}\n{all}"
    );
    assert!(
        !calls.contains(EDGE_ADDRESS),
        "the controller reached the edge: {calls}"
    );
    assert!(
        all.contains("the edge half of peering hello-ally from")
            && all.contains(placed.to_str().unwrap()),
        "the core half did not join the imported edge half: {all}"
    );
    assert!(v["handoff"].is_null(), "it handed off again: {all}");
    assert!(
        !all.contains("GB-114"),
        "the imported half was refused: {all}"
    );
    // No router answers on the stand-in core, so the core half's own
    // refusal on core-1 (GB-113, no core router to measure) is what stops
    // it, after the join.
    assert!(all.contains("GB-113") && all.contains("core-1"), "{all}");
}

/// The import runs the core half's file checks: the confirm code, the
/// schema and peering, the 24-hour bound, and the edge host this checkout's
/// inventory names. Each failure is GB-114, and nothing is placed or
/// recorded.
#[test]
fn a_phase_3_edge_half_that_does_not_hold_is_not_imported() {
    let w = world();
    let commit = two_drivers(&w);
    let ssh = w.over_ssh();
    let edge_work = w.root.path().join("edge-work");
    let (bytes, _) = phase_3_edge_half(&w, &ssh, &commit, &edge_work);
    let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let changed = |k: &str, v: serde_json::Value| -> Vec<u8> {
        let mut d = doc.clone();
        d[k] = v;
        serde_json::to_vec_pretty(&d).unwrap()
    };
    let stale = (chrono::Utc::now() - chrono::Duration::hours(30))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let cases: Vec<(&str, Vec<u8>, Option<String>, &str)> = vec![
        (
            "another peering",
            changed("peering", "xac".into()),
            None,
            "xac",
        ),
        (
            "measured too long ago",
            changed("measured_at", stale.into()),
            None,
            "hours ago",
        ),
        (
            "another edge host",
            changed("edge_host", "edge-9".into()),
            None,
            "edge-9",
        ),
        (
            "another file",
            bytes.clone(),
            Some(confirm_code(b"other")),
            "not the one",
        ),
    ];
    for (what, file, code, says_why) in cases {
        let path = w.root.path().join("carried.json");
        std::fs::write(&path, &file).unwrap();
        let code = code.unwrap_or_else(|| confirm_code(&file));
        let (out, _) = w.run_over_ssh(
            &ssh,
            &[
                "import",
                "hello-ally",
                "3",
                path.to_str().unwrap(),
                "--sha256",
                &code,
            ],
        );
        let all = says(&out);
        assert!(!out.status.success(), "{what}: {all}");
        assert!(all.contains("GB-114"), "{what}: {all}");
        assert!(all.contains(says_why), "{what}: {all}");
        assert!(
            !w.work.join("handoff/3/edge-half.json").exists(),
            "{what}: placed"
        );
        let st = w.state();
        assert!(
            !st["confirmations"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|c| c["kind"] == "operator-step" && c["phase"] == 3),
            "{what}: recorded: {st}"
        );
    }
}

/// Phase 3 of a two-driver peering measures in halves over SSH. Measured
/// here (FED_MEASURE=here) it has no halves, so a run that holds phase 3 is
/// refused before it acts, naming FED_MEASURE.
#[test]
fn a_two_driver_phase_3_measured_here_is_refused() {
    let w = world();
    two_drivers(&w);
    let (out, v) = w.run(&["hello-ally", "--from", "3", "--to", "3"]);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert!(
        all.contains("GB-267") && all.contains("FED_MEASURE"),
        "{all}"
    );
    assert!(v["handoff"].is_null(), "{all}");
    assert!(!w.log().contains("goat init"), "{}", w.log());
    assert!(!w.work.join("state.json").exists(), "a journal was written");
}
