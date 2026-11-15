//! Exit statuses, one row per command line, run with a clean environment
//! (`env -i` with PATH, HOME and TMPDIR only). Each row is a place whose exit
//! a change has moved or decided, so a later change to any of them shows here
//! as a diff to this table, not only in a changelog line.
//!
//! The contract's exit codes (docs/reference/goatbuilder/06-contract.md): 1,
//! refused: a precondition failed; 2, usage: the command line was wrong, or
//! names something the config does not declare.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A temporary checkout copied from a fixture repository under the crate.
fn checkout(fixture: &str) -> tempfile::TempDir {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(fixture);
    let d = tempfile::tempdir().unwrap();
    copy(&src, d.path());
    d
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst: PathBuf = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), &dst).unwrap();
        }
    }
}

/// The exit status and stderr of `goatbuilder <args>` in a clean environment.
fn run(args: &[&str]) -> (i32, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
    c.env_clear();
    for k in ["PATH", "HOME", "TMPDIR"] {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    let out = c.args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// The fixture repository that declares one peering, hello-ally on zagros.
const JOURNAL_REPO: &str = "ops/goatbuilder/compat/documents/fixtures/journal/repo";

#[test]
fn each_command_line_exits_as_the_table_says() {
    let repo = checkout(JOURNAL_REPO);
    let r = repo.path().to_str().unwrap();
    // (what, args, exit, a word stderr must hold)
    let table: Vec<(&str, Vec<&str>, i32, &str)> = vec![
        // The contract's exit 2: the command line names a peering the config does
        // not declare. Left uncoded until GB-149's exit is decided (it exits 1).
        (
            "allocate names an undeclared peering",
            vec!["-C", r, "--yes", "federate", "allocate", "no-such-peer"],
            2,
            "is not declared",
        ),
    ];
    let mut problems = Vec::new();
    for (what, args, want, word) in &table {
        let (got, err) = run(args);
        if got != *want || !err.contains(word) {
            problems.push(format!("{what}: `goatbuilder {}` exited {got} (want {want}): {err}", args.join(" ")));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
