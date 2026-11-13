//! Gate row B10, its first piece: a peering declared `drivers: two` stops the
//! controller at phase 1 with the hand-off object of
//! docs/design/federation-two-drivers.md ("The hand-off object"), through the
//! real binary against a stand-in repository (federate_world).

mod federate_world;

use federate_world::{phases_run, rows, says, world, World};

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
    assert!(!w.log().contains("preflight"), "{}", w.log());
}
