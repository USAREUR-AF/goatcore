//! The tutor, used the way a learner uses it: open the current lesson, type
//! the commands it prints exactly as printed, ask the tutor again, and expect
//! the lesson done and the next one open — from the first lesson to the last.
//!
//! Two lessons need a checkout of main and the network (release, fastlane);
//! their runs are written to the tutor's log as a learner's would be, and the
//! unit tests in src/tutor.rs cover how those runs are judged. Every other
//! lesson's commands run for real, on the tutor's own fixture copies or
//! read-only on this checkout.


#[path = "common/test_home.rs"]
mod test_home;
use std::path::{Path, PathBuf};
use std::process::Command;


/// A test's own directory under the system temp directory, removed when the
/// returned guard drops (a panicking test included).
fn scratch(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("a temporary directory for the test")
}
fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_goatbuilder"))
}

fn repo() -> PathBuf {
    std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap()
}

fn tutor(ws: &Path) -> serde_json::Value {
    let out = test_home::isolate(&mut Command::new(bin()))
        .args(["-C", repo().to_str().unwrap(), "-o", "json", "tutor", "--workspace", ws.to_str().unwrap()])
        .env("GOATBUILDER_TUTOR", ws)
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("tutor -o json: {e}: {}", String::from_utf8_lossy(&out.stdout)))
}

fn rows(v: &serde_json::Value) -> Vec<(String, bool, String)> {
    v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["name"].as_str().unwrap().to_string(), c["ok"].as_bool().unwrap(), c["detail"].as_str().unwrap().to_string()))
        .collect()
}

fn append_log(ws: &Path, line: &str) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).create(true).open(ws.join("log.jsonl")).unwrap();
    writeln!(f, "{line}").unwrap();
}


/// Whether this checkout declares a deployment of its own. Lesson 1 runs
/// `goatbuilder validate` and `doctor` against it; a public clone of goatcore
/// declares none, by design, so these tests return early there.
fn checkout_declares_a_deployment() -> bool {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ops/goatbuilder/goatbuilder.yaml")
        .is_file()
}

#[test]
fn a_learner_completes_every_available_lesson_by_typing_what_it_prints() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let held = scratch("goatbuilder-tutor-e2e-");
    let ws = held.path().join("ws");
    let path = format!("{}:{}", bin().parent().unwrap().display(), std::env::var("PATH").unwrap_or_default());
    let mut finished = Vec::new();
    for _ in 0..20 {
        let v = tutor(&ws);
        let r = rows(&v);
        if r.iter().any(|(n, _, d)| n == "tutor" && d.starts_with("all ")) {
            break;
        }
        let lesson: String = r.iter().find_map(|(n, _, _)| n.strip_prefix("lesson ").map(|s| s.split('/').next().unwrap().to_string())).expect("a lesson is open");
        let typed: Vec<String> = r.iter().filter(|(n, _, _)| n.contains("/type ")).map(|(_, _, d)| d.clone()).collect();
        assert!(!typed.is_empty(), "lesson {lesson} printed no commands: {r:?}");
        if typed.iter().any(|c| c.contains(" release ") || c.contains(" fastlane ")) {
            // The network-bound lessons: the runs a learner on main would log.
            let lines: &[&str] = if typed.iter().any(|c| c.contains(" release ")) {
                &[
                    r#"{"args":["release","microscope","--tag","microscope/v0.1.0-rc.2","--from","dev","--ack","CI","--dry-run"],"exit":0,"rows":["step ==> component microscope"]}"#,
                    r#"{"args":["release","microscope","--tag","microscope/v0.1.0-rc.2","--from","prod","--ack","CI","--dry-run"],"exit":0,"rows":["step ==> prod"]}"#,
                ]
            } else {
                &[r#"{"args":["fastlane","netvis","--ref","sha-e4201ea","--reason","x","--ack","CI","--dry-run"],"exit":1,"rows":["refused REFUSE — tag not found"]}"#]
            };
            // `promote status` is offline: run it for real.
            for c in typed.iter().filter(|c| c.starts_with("goatbuilder promote")) {
                run(c, &ws, &path);
            }
            for l in lines {
                append_log(&ws, l);
            }
        } else {
            for c in &typed {
                if c.starts_with("export ") {
                    continue;
                }
                run(c, &ws, &path);
            }
        }
        let after = rows(&tutor(&ws));
        let done = after.iter().any(|(n, ok, d)| n == &format!("lesson {lesson}") && *ok && d == "done");
        assert!(done, "lesson {lesson} not done after typing {typed:?}: {after:?}");
        finished.push(lesson);
    }
    // 18 lessons, minus those needing a verb this build lacks (diff/history, the
    // explain-backed lessons, the four verb lessons); this build has every verb.
    assert!(finished.len() >= 14, "finished only {finished:?}");
    assert!(finished.contains(&"11".to_string()), "never reached tutorial 2: {finished:?}");
    assert!(finished.contains(&"18".to_string()), "never reached the bonus lesson: {finished:?}");
    let _ = std::fs::remove_dir_all(&ws);
}

fn run(cmd: &str, ws: &Path, path: &str) {
    let cmd = cmd.to_string();
    // A command that refuses is still a command a learner types; the tutor
    // judges its exit. Only a failure to start is a test failure.
    let st = Command::new("bash")
        .args(["-c", &cmd])
        .current_dir(repo())
        .env("PATH", path)
        .env("GOATBUILDER_TUTOR", ws)
        .env("GOAT_MESH_PROBE", "0")
        // The lessons fill --ack from `git config user.name`; a CI runner may
        // have none, so give this learner one (git reads GIT_CONFIG_* first).
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "user.name")
        .env("GIT_CONFIG_VALUE_0", "CI Learner")
        .output()
        .unwrap_or_else(|e| panic!("{cmd}: {e}"));
    let _ = st;
}

/// The wrong move a learner makes most: running a lesson's command against the
/// real checkout instead of the tutor's copy. It is logged, it does not count,
/// and the tutor says where it should have run.
#[test]
fn a_command_run_outside_the_tutors_copy_does_not_count() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let held = scratch("goatbuilder-tutor-wrong-");
    let ws = held.path().join("ws");
    let _ = tutor(&ws);
    // Lesson 1 done through its two commands: validate on the skeleton, doctor on the checkout.
    let path = format!("{}:{}", bin().parent().unwrap().display(), std::env::var("PATH").unwrap_or_default());
    run(&format!("goatbuilder validate -C {}/repo", ws.display()), &ws, &path);
    run("goatbuilder doctor --env zagros_dev", &ws, &path);
    assert!(rows(&tutor(&ws)).iter().any(|(n, _, d)| n == "lesson 1" && d == "done"));
    // Lesson 2 on the basic fixture in the checkout, not the tutor's copy.
    let basic = repo().join("ops/goatbuilder/fixtures/basic");
    let held_copy = scratch("goatbuilder-tutor-basic-");
    let copy = held_copy.path().to_path_buf();
    Command::new("cp").args(["-R", &format!("{}/.", basic.display()), copy.to_str().unwrap()]).status().unwrap();
    run(&format!("goatbuilder --mock {} status --env dev", copy.display()), &ws, &path);
    let r = rows(&tutor(&ws));
    assert!(r.iter().any(|(n, ok, _)| n == "lesson 2/1" && !ok), "{r:?}");
    let why = r.iter().find(|(n, _, _)| n == "lesson 2/not yet").map(|(_, _, d)| d.clone()).unwrap_or_default();
    assert!(why.contains("did not run on the tutor's copy"), "{why}");
    let _ = std::fs::remove_dir_all(&ws);
    let _ = std::fs::remove_dir_all(&copy);
}

/// A finished lesson can be read again, by number or with --lesson, and doing
/// so leaves the learner where they were.
#[test]
fn a_finished_lesson_opens_to_read_and_changes_nothing() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let held = scratch("goatbuilder-tutor-review-");
    let ws = held.path().join("ws");
    let _ = tutor(&ws);
    let path = format!("{}:{}", bin().parent().unwrap().display(), std::env::var("PATH").unwrap_or_default());
    run(&format!("goatbuilder validate -C {}/repo", ws.display()), &ws, &path);
    run("goatbuilder doctor --env zagros_dev", &ws, &path);
    let _ = tutor(&ws); // marks lesson 1 done
    let page = |args: &[&str]| {
        let mut a = vec!["-C", repo().to_str().unwrap().to_owned().leak(), "tutor", "--workspace", ws.to_str().unwrap().to_owned().leak()];
        a.extend_from_slice(args);
        let o = test_home::isolate(&mut Command::new(bin())).args(&a).env("GOATBUILDER_TUTOR", &ws).env("GOAT_LANG", "en").output().unwrap();
        String::from_utf8_lossy(&o.stdout).to_string()
    };
    for args in [&["--lesson", "1"][..], &["1"][..]] {
        let p = page(args);
        assert!(p.contains("Lesson 1 of 18"), "{args:?}: {p}");
        assert!(!p.contains("Lesson 2 of 18"), "{args:?} moved on: {p}");
        assert!(p.contains("only to read"), "{args:?}: {p}");
        assert!(!p.contains('\u{2717}'), "{args:?}: a finished lesson has no open steps: {p}");
    }
    assert!(page(&[]).contains("Lesson 2 of 18"), "reading lesson 1 left the learner on lesson 2");
    let list = page(&["--list"]);
    assert!(list.lines().any(|l| l.contains("Check your setup") && l.trim_end().ends_with("done")), "{list}");
    assert!(list.lines().any(|l| l.trim_end().ends_with("next")), "{list}");
    assert!(!list.contains("\nok "), "a list is not a check: {list}");
    let _ = std::fs::remove_dir_all(&ws);
}
