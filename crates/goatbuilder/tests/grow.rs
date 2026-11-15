//! Gate row G1 through the binary (docs/design/goatlite-growth.md §8): on the lite
//! fixture `ops/goatbuilder/fixtures/goatnet-from-scratch/00-goatlite`,
//! `goatbuilder grow --dry-run` produces a full configuration that passes
//! validation, shows its difference from goatlite.yaml, and writes nothing.
//!
//! The rest of the row is proven next to the code: the standup plan from the grown
//! configuration changes nothing that exists
//! (`config::tests::a_standup_plan_from_the_grown_config_changes_nothing_that_exists`),
//! and a real run's hand-over and refusals (`grow::tests`).
//!
//! C10 at a terminal is proven here, through the real command line on a
//! pseudo-terminal: `grow` asks only when it can be answered (stdin and stderr both
//! a terminal), and without --yes it never writes when it cannot ask.

#[path = "common/test_home.rs"]
mod test_home;
use std::path::Path;
use std::process::Command;

fn lite_fixture() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../ops/goatbuilder/fixtures/goatnet-from-scratch/00-goatlite/goatbuilder.yaml",
        ),
    )
    .unwrap()
}

#[test]
fn grow_dry_run_on_the_lite_fixture_shows_a_valid_full_config_and_writes_nothing() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("goatlite.yaml"), lite_fixture()).unwrap();
    // The dry run asks the checkout's own renderer which files the hand-over
    // takes (`--hand-over --check`, which writes nothing); nothing is rendered
    // here, so it names none.
    let renderer = t.path().join("ops/scripts/goatlite-render.py");
    std::fs::create_dir_all(renderer.parent().unwrap()).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts/goatlite-render.py"),
        &renderer,
    )
    .unwrap();
    let out = test_home::isolate(&mut Command::new(env!("CARGO_BIN_EXE_goatbuilder")))
        .args(["-C"])
        .arg(t.path())
        .args(["grow", "--dry-run", "-o", "json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "exit {:?}\nstdout:\n{stdout}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["verb"], "grow");
    assert_eq!(report["ok"], true);
    let details: Vec<String> = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            format!(
                "{} {}",
                c["name"].as_str().unwrap(),
                c["detail"].as_str().unwrap()
            )
        })
        .collect();
    let all = details.join("\n");
    assert!(all.contains("config goatlite.yaml grows into a full configuration that validates against goatbuilder/v1"), "{all}");
    assert!(
        all.contains("difference ~ preset: \"lite\" -> \"full\""),
        "{all}"
    );
    assert!(
        all.contains("plan would write ops/goatbuilder/goatbuilder.yaml"),
        "{all}"
    );
    assert!(
        all.contains(&format!(
            "plan would hand the rendered files over: python3 {} --site pasture --config {} --hand-over",
            renderer.display(),
            t.path().join("goatlite.yaml").display()
        )),
        "the dry run shows the command the real run executes: {all}"
    );
    let mut left: Vec<String> = std::fs::read_dir(t.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        vec!["goatlite.yaml".to_string(), "ops".to_string()],
        "a dry run writes nothing into the checkout"
    );
    assert!(
        !t.path().join("ops/goatbuilder").exists(),
        "a dry run writes no config"
    );
}

/// A checkout holding the lite fixture and a stand-in renderer that records each
/// run in `renderer-ran` and lists nothing to hand over.
fn checkout_with_stand_in_renderer() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("goatlite.yaml"), lite_fixture()).unwrap();
    let renderer = t.path().join("ops/scripts/goatlite-render.py");
    std::fs::create_dir_all(renderer.parent().unwrap()).unwrap();
    std::fs::write(
        &renderer,
        "import pathlib, sys\n\
         root = pathlib.Path(__file__).resolve().parents[2]\n\
         with open(root / 'renderer-ran', 'a') as f: f.write(' '.join(sys.argv[1:]) + '\\n')\n\
         print('goatlite-render: pasture -- nothing to hand over')\n",
    )
    .unwrap();
    t
}

/// Run `goatbuilder <args>` with its stdin and stdout on a pseudo-terminal, and its
/// stderr on that terminal too, or in `stderr_file` when one is given. When the run
/// asks ("Type yes"), `answer` is typed. Returns the exit code and what the
/// terminal showed. Python's pty module opens the terminal on Linux and macOS.
fn at_a_terminal(dir: &Path, args: &[&str], stderr_file: Option<&Path>, answer: Option<&str>) -> (Option<i32>, String) {
    let driver = dir.join("pty-driver.py");
    std::fs::write(
        &driver,
        r#"import os, pty, select, subprocess, sys, time
err = os.environ.pop("PTY_STDERR", "")
answer = os.environ.pop("PTY_ANSWER", None)
m, s = pty.openpty()
ef = open(err, "w") if err else s
p = subprocess.Popen(sys.argv[1:], stdin=s, stdout=s, stderr=ef, start_new_session=True)
os.close(s)
buf, sent, deadline = b"", False, time.time() + 120
while time.time() < deadline:
    r, _, _ = select.select([m], [], [], 0.2)
    if r:
        try:
            d = os.read(m, 4096)
        except OSError:
            d = b""
        if not d:
            break
        buf += d
        if answer is not None and not sent and b"Type yes" in buf:
            os.write(m, (answer + "\n").encode())
            sent = True
    elif p.poll() is not None:
        break
p.wait(timeout=30)
sys.stdout.write(buf.decode("utf-8", "replace"))
sys.exit(p.returncode)
"#,
    )
    .unwrap();
    let mut cmd = Command::new("python3");
    test_home::isolate(&mut cmd)
        .arg(&driver)
        .arg(env!("CARGO_BIN_EXE_goatbuilder"))
        .arg("-C")
        .arg(dir)
        .args(args);
    if let Some(f) = stderr_file {
        cmd.env("PTY_STDERR", f);
    }
    if let Some(a) = answer {
        cmd.env("PTY_ANSWER", a);
    }
    let out = cmd.output().unwrap();
    (out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned())
}

/// C10 through main.rs: stdin is a terminal but stderr is a file, so a question
/// could not be seen. Without --yes, grow refuses (GB-008) and changes nothing:
/// no config, goatlite.yaml kept, the renderer never run.
#[test]
fn at_a_terminal_with_stderr_redirected_grow_without_yes_changes_nothing() {
    let t = checkout_with_stand_in_renderer();
    let err = t.path().join("grow-stderr");
    let (code, shown) = at_a_terminal(t.path(), &["grow"], Some(&err), None);
    let stderr = std::fs::read_to_string(&err).unwrap_or_default();
    assert_eq!(code, Some(2), "terminal:\n{shown}\nstderr:\n{stderr}");
    assert!(stderr.contains("GB-008"), "stderr:\n{stderr}");
    assert!(
        t.path().join("goatlite.yaml").exists()
            && !t.path().join("ops/goatbuilder/goatbuilder.yaml").exists()
            && !t.path().join("renderer-ran").exists(),
        "nothing changed: terminal:\n{shown}\nstderr:\n{stderr}"
    );
}

/// C10 through main.rs: stdin and stderr both a terminal, no --yes. grow shows
/// every write, asks, and on Enter stops (exit 1) with nothing changed.
#[test]
fn at_a_terminal_grow_without_yes_asks_and_enter_changes_nothing() {
    let t = checkout_with_stand_in_renderer();
    let (code, shown) = at_a_terminal(t.path(), &["grow"], None, Some(""));
    assert!(shown.contains("would write ops/goatbuilder/goatbuilder.yaml") && shown.contains("Type yes"), "{shown}");
    assert_eq!(code, Some(1), "{shown}");
    assert!(
        t.path().join("goatlite.yaml").exists() && !t.path().join("ops/goatbuilder/goatbuilder.yaml").exists(),
        "{shown}"
    );
    let ran = std::fs::read_to_string(t.path().join("renderer-ran")).unwrap_or_default();
    assert!(ran.lines().all(|l| l.ends_with("--check")), "only the check ran before the answer: {ran}");
}
