use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CODE: &str = "E_MEMBERSHIP_VERSION_UNSUPPORTED";
const SCOPE: &str = "zagros-dev";

fn goat() -> Command {
    Command::new(env!("CARGO_BIN_EXE_goat"))
}

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

fn declaration(root: &Path, case: &str) -> PathBuf {
    let dir = root.join(case);
    match case {
        "scope" => write(
            &dir.join("_scope.yaml"),
            "version: 2\nscope: zagros-dev\nconverge: report\n",
        ),
        "baseline" => {
            write(
                &dir.join("_scope.yaml"),
                "version: 1\nscope: zagros-dev\nconverge: report\n",
            );
            write(
                &dir.join("_baseline.generated.yaml"),
                "version: 2\nscope: zagros-dev\npeople: {}\n",
            );
        }
        "fragment" => {
            write(
                &dir.join("_scope.yaml"),
                "version: 1\nscope: zagros-dev\nconverge: report\n",
            );
            write(
                &dir.join("people/operators.yaml"),
                "version: 2\nentries: []\n",
            );
        }
        _ => unreachable!(),
    }
    dir
}

fn run_apply(root: &Path, dir: &Path, explain: &str) -> (Output, TcpListener, PathBuf) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let metrics = root.join(format!("metrics-{explain}"));
    std::fs::create_dir_all(&metrics).unwrap();
    let output = goat()
        .env("GOAT_EXPLAIN", explain)
        .args([
            "--profile-dir",
            root.join("profile").to_str().unwrap(),
            "warrant",
            "apply",
            "--membership",
            "--scope",
            SCOPE,
            "--file",
            dir.to_str().unwrap(),
            "--mode",
            "enforce",
            "--mgmt-url",
            &format!("http://{}", listener.local_addr().unwrap()),
            "--textfile-dir",
            metrics.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    (output, listener, metrics)
}

#[test]
fn compiled_apply_renders_typed_file_error_before_any_side_effect() {
    let temp = tempfile::tempdir().unwrap();
    for (case, relative, label) in [
        ("scope", "_scope.yaml", "_scope.yaml"),
        (
            "baseline",
            "_baseline.generated.yaml",
            "_baseline.generated.yaml",
        ),
        ("fragment", "people/operators.yaml", "fragment"),
    ] {
        let dir = declaration(temp.path(), case);
        let source = dir.join(relative);
        let mut disabled_stderr = None;
        for explain in ["0", "1"] {
            let (output, listener, metrics) = run_apply(temp.path(), &dir, explain);
            assert_eq!(output.status.code(), Some(1), "{case}/{explain}");
            assert!(output.stdout.is_empty(), "{case}/{explain}");
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(
                stderr.lines().any(|line| line
                    == format!(
                        "error: {}: {label} version 2 — this goat reads version 1",
                        source.display()
                    )),
                "{stderr}"
            );
            assert!(stderr.contains(&format!("code: {CODE}")), "{stderr}");
            assert!(stderr.contains("Nothing was changed"), "{stderr}");
            if explain == "0" {
                disabled_stderr = Some(stderr);
            } else {
                assert_eq!(
                    stderr,
                    disabled_stderr.take().unwrap(),
                    "explanation mode must not repeat the typed error's own catalog entry"
                );
            }

            // A malformed declaration must stop before the NetBird client or
            // metrics sink is touched, even when enforce mode was requested.
            assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
            assert!(std::fs::read_dir(&metrics).unwrap().next().is_none());
        }
    }
}

#[test]
fn compiled_explain_supports_code_legacy_text_human_and_json() {
    let by_code = goat().args(["explain", CODE]).output().unwrap();
    assert!(by_code.status.success());
    let human = String::from_utf8(by_code.stdout).unwrap();
    assert!(human.contains(CODE), "{human}");
    assert!(human.contains("strict schema"), "{human}");

    let legacy = "_scope.yaml version 2 — this goat reads version 1";
    let by_text = goat().args(["explain", legacy]).output().unwrap();
    assert!(by_text.status.success());
    let human = String::from_utf8(by_text.stdout).unwrap();
    assert!(human.contains(CODE), "{human}");

    let json = goat().args(["explain", "--json", legacy]).output().unwrap();
    assert!(json.status.success());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["schema"], "goat-errors.interpretation/v1");
    assert_eq!(value["unrecognised"], false);
    assert!(value["hits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|hit| { hit["code"] == CODE && hit["layer"] == "pattern" }));
}
