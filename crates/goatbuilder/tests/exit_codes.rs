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

/// The exit status and stderr of `goatbuilder <args>` in a clean environment:
/// PATH and TMPDIR from the test, and an empty HOME of its own, so no run
/// record lands in the caller's home.
fn run(args: &[&str]) -> (i32, String) {
    let home = tempfile::tempdir().unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
    c.env_clear();
    for k in ["PATH", "TMPDIR"] {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    c.env("HOME", home.path());
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
        // not declare. GB-359, not GB-149: GB-149 exits 1 for a peering run, and
        // whether it moves to 2 is its own question.
        (
            "allocate names an undeclared peering",
            vec!["-C", r, "--yes", "federate", "allocate", "no-such-peer"],
            2,
            "GB-359",
        ),
        // The six verbs' context flags, given to a plan or to a peering run: GB-239,
        // exit 2, as for any flag given where it does not apply.
        ("--context-dir given to a plan", vec!["federate", "plan", "hello-ally", "--context-dir", "/dev/null"], 2, "GB-239"),
        ("--context-dir given to a peering run", vec!["-C", r, "--yes", "federate", "hello-ally", "--context-dir", "/dev/null"], 2, "GB-239"),
        ("--over-ssh given to a plan", vec!["federate", "plan", "hello-ally", "--over-ssh", "hello-ally"], 2, "GB-239"),
        ("--over-ssh given to a peering run", vec!["-C", r, "--yes", "federate", "hello-ally", "--over-ssh", "hello-ally"], 2, "GB-239"),
        // The federation verbs: an uncoded fault exits 1 there (verbs::fault_exit),
        // so each command-line error below exited 1 before it was coded; it exits 2
        // now, the contract's usage error.
        // A flag for another verb.
        ("--over-ssh given to lint", vec!["federate", "lint", "/dev/null", "--over-ssh", "xab"], 2, "GB-239"),
        // A flag without the one it needs.
        ("a host flag without --over-ssh", vec!["federate", "init", "--edge-host", "h"], 2, "GB-325"),
        ("--side without --over-ssh", vec!["federate", "init", "--side", "edge"], 2, "GB-325"),
        (
            "--side edge without --half-out",
            vec!["-C", r, "federate", "init", "--over-ssh", "hello-ally", "--side", "edge"],
            2,
            "GB-325",
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

/// The exit each code this work uses gives, by the contract's meanings: a failure
/// that is not the command line's (a program, a file, goatbuilder's own state, a
/// terminal) is 1, refused; a command line that is wrong is 2, usage.
#[test]
fn each_code_exits_by_the_contracts_meaning() {
    let want = [
        ("GB-184", 1),
        ("GB-315", 1),
        ("GB-316", 1),
        ("GB-320", 1),
        ("GB-321", 1),
        ("GB-322", 1),
        ("GB-323", 1),
        ("GB-239", 2),
        ("GB-242", 2),
        ("GB-324", 2),
        ("GB-325", 2),
        // The rest of goatbuilder (gate row C5's last batch): each was an uncoded
        // error, exit 2, on goatbuilder's own error path.
        ("GB-020", 1),
        ("GB-025", 1),
        ("GB-258", 1),
        ("GB-005", 2),
        ("GB-008", 2),
        ("GB-024", 2),
        ("GB-052", 2),
        ("GB-254", 2),
        ("GB-257", 2),
        ("GB-259", 2),
        ("GB-326", 2),
        ("GB-277", 1),
    ];
    let got: Vec<(&str, Option<i32>)> = want
        .iter()
        .map(|(c, _)| (*c, goat_errors::explain(c).and_then(|e| e.exit)))
        .collect();
    let want: Vec<(&str, Option<i32>)> = want.iter().map(|(c, e)| (*c, Some(*e))).collect();
    assert_eq!(got, want);
}
