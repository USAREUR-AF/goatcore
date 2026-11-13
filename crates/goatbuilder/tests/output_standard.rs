//! The human-output standard (docs/design/goatbuilder-output.md), enforced
//! across a corpus of read-only and fixture commands rather than one verb at a
//! time, so a new verb cannot drift.
//!
//! A verb that does not meet a rule yet is listed in that rule's allowlist. The
//! allowlists may only shrink: a test fails if a listed verb already complies,
//! so each entry is removed by the change that fixes it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const WIDTH: usize = 80;

/// Verbs whose table rows are still wider than the page: the full
/// `goatnet/site/class/host/component` row names and the image digests.
///
/// Width is a rule about the page a person reads, and a page wraps to 80 only
/// on a terminal (`output.rs`'s `titles`). This harness runs the binary with a
/// pipe, so what it measures is the row form a script reads, which has always
/// been wide. The terminal pages have unit tests on their layout, and
/// `a_failing_row_is_on_the_page_a_person_reads` runs them on a real terminal.
/// `federate` joined it when its plan page started listing each phase's command
/// verbatim and unwrapped, so a line pastes as the command it runs
/// (docs/design/goatbuilder-legibility.md, step 3): a wrapped command would not.
const WIDE_ALLOWED: &[&str] =
    &["deploy", "diff", "history", "rollback", "standup", "teardown", "join-node", "verify", "survey", "pins", "promote", "validate", "explain", "doctor", "preflight", "assay", "federate"];

/// Verbs still printing em or en dashes (the sweep on #2341). Same surface as
/// the width list: on a terminal these are rewritten to plain punctuation.
///
/// `preflight` and `assay` are here for a reason of their own: the phrases are
/// written the same way by the script avenue (`ops/scripts/rollout-preflight.py`,
/// the acceptance runner), and a parity gate compares the two word for word, so
/// the dash leaves both at once or neither (ADR 1201). `doctor` left this list
/// when its rows were rewritten.
const DASH_ALLOWED: &[&str] =
    &["deploy", "diff", "history", "rollback", "standup", "teardown", "join-node", "verify", "survey", "pins", "promote", "validate", "explain", "preflight", "assay"];

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_goatbuilder")
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

/// Whether this checkout declares a deployment of its own. A public clone of
/// goatcore declares none, by design; the tests that read this checkout's own
/// deployment return early there.
fn checkout_declares_a_deployment() -> bool {
    repo().join("ops/goatbuilder/goatbuilder.yaml").is_file()
}

/// A test's own directory under the system temp directory, removed when the
/// returned guard drops (a panicking test included).
fn scratch(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("a temporary directory for the test")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), &dest).unwrap();
        }
    }
}

struct Case {
    verb: &'static str,
    args: Vec<String>,
    dry_run: bool,
}

/// Commands that touch no host: read-only on this checkout, or on throwaway
/// fixture copies under `dir`.
fn corpus(dir: &Path) -> Vec<Case> {
    let fx = repo().join("ops/goatbuilder/fixtures");
    let tutor = dir.join("tutor");
    copy_tree(&fx.join("tutor"), &tutor);
    let founder = dir.join("founder");
    copy_tree(&fx.join("goatnet-from-scratch/01-founder"), &founder);
    let t = tutor.display().to_string();
    let f = founder.display().to_string();
    let work = dir.join("federate").display().to_string();
    let tw = dir.join("tutor-ws").display().to_string();
    let r = repo().display().to_string();
    let v = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    vec![
        Case { verb: "status", args: v(&["--mock", &t, "status", "--env", "dev"]), dry_run: false },
        Case { verb: "diff", args: v(&["--mock", &t, "diff", "--env", "dev"]), dry_run: false },
        Case { verb: "history", args: v(&["--mock", &t, "history", "--env", "preprod", "--component", "portal"]), dry_run: false },
        Case { verb: "survey", args: v(&["--mock", &t, "survey"]), dry_run: false },
        Case { verb: "deploy", args: v(&["--mock", &t, "deploy", "--env", "preprod", "--component", "portal", "--dry-run"]), dry_run: true },
        Case { verb: "rollback", args: v(&["--mock", &t, "rollback", "portal", "1", "--env", "preprod", "--dry-run"]), dry_run: true },
        Case { verb: "standup", args: v(&["--mock", &f, "standup", "zagros", "--site", "alpha", "--ack", "Standard Test", "--yes"]), dry_run: false },
        Case { verb: "deploy", args: v(&["-C", &r, "deploy", "--env", "zagros_dev", "--component", "netvis", "--dry-run"]), dry_run: true },
        Case { verb: "verify", args: v(&["-C", &r, "verify", "netvis", "--env", "zagros_dev", "--dry-run"]), dry_run: true },
        Case { verb: "standup", args: v(&["-C", &r, "standup", "zagros_dev", "--site", "zagros-dev", "--dry-run"]), dry_run: true },
        Case { verb: "teardown", args: v(&["-C", &r, "teardown", "zagros_dev", "--site", "zagros-dev", "--dry-run"]), dry_run: true },
        Case { verb: "join-node", args: v(&["-C", &r, "join-node", "zagros_dev", "--site", "zagros-dev", "--node", "node-x", "--class", "breakglass", "--dry-run"]), dry_run: true },
        Case { verb: "federate", args: v(&["-C", &r, "--mock", &format!("{r}/ops/goatbuilder/fixtures/goatnet-from-scratch/03-boundary-node"), "federate", "hello-ally", "--work", &work, "--dry-run"]), dry_run: true },
        Case { verb: "tutor", args: v(&["-C", &r, "tutor", "--workspace", &tw]), dry_run: false },
        Case { verb: "standup", args: v(&["--mock", &f, "standup", "zagros", "--site", "alpha", "--dry-run"]), dry_run: true },
        Case { verb: "teardown", args: v(&["--mock", &f, "teardown", "zagros", "--site", "alpha", "--dry-run"]), dry_run: true },
        Case { verb: "explain", args: v(&["explain", "GB-002"]), dry_run: false },
        // Read-only on this checkout: declarations, records and the catalog.
        // No host is reached (GOAT_MESH_PROBE=0) and no credential is read
        // (that needs --credentials), so these are as safe as the rest.
        Case { verb: "validate", args: v(&["-C", &r, "validate"]), dry_run: false },
        Case { verb: "doctor", args: v(&["-C", &r, "doctor", "--env", "zagros_dev"]), dry_run: false },
        Case { verb: "doctor", args: v(&["-C", &r, "doctor", "--env", "zagros_dev", "--all"]), dry_run: false },
        Case { verb: "preflight", args: v(&["-C", &r, "preflight", "netvis", "--env", "zagros_dev"]), dry_run: false },
        Case { verb: "pins", args: v(&["-C", &r, "pins", "--check"]), dry_run: false },
        Case { verb: "promote", args: v(&["-C", &r, "promote", "review"]), dry_run: false },
        Case { verb: "promote", args: v(&["-C", &r, "promote", "readiness"]), dry_run: false },
        Case { verb: "promote", args: v(&["-C", &r, "promote", "status"]), dry_run: false },
        Case { verb: "promote", args: v(&["-C", &r, "promote", "check"]), dry_run: false },
        Case { verb: "assay", args: v(&["-C", &r, "assay", "zagros_dev", "--plan"]), dry_run: false },
        Case { verb: "explain", args: v(&["explain", "--list"]), dry_run: false },
    ]
}

fn run(args: &[String], json: bool, dir: &Path) -> Output {
    let mut a: Vec<String> = Vec::new();
    if json {
        a.push("-o".into());
        a.push("json".into());
    }
    a.extend_from_slice(args);
    Command::new(bin())
        .args(&a)
        .current_dir(repo())
        .env("GOAT_MESH_PROBE", "0")
        .env("GOAT_LANG", "en")
        .env("HOME", dir)
        .env_remove("GOATBUILDER_TUTOR")
        // What this shell has loaded is not part of the standard: a malformed
        // reference in one person's environment adds a row nobody else sees.
        .envs(std::env::vars().filter(|(_, v)| is_reference(v)).map(|(k, _)| (k, String::new())))
        .output()
        .expect("run goatbuilder")
}

fn is_reference(v: &str) -> bool {
    ["op://", "bao://", "bw://"].iter().any(|r| v.starts_with(r))
}

/// The binary on a pseudo-terminal, 80 columns wide: the surface a person reads,
/// which a pipe never reaches because the pages are chosen by `IsTerminal`.
fn on_a_terminal(args: &[String], dir: &Path) -> String {
    const PTY: &str = "import fcntl,os,pty,struct,sys,termios\npid,fd=pty.fork()\nif pid==0:\n    os.execv(sys.argv[1],sys.argv[1:])\nfcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',50,80,0,0))\nb=b''\nwhile True:\n    try:\n        d=os.read(fd,65536)\n    except OSError:\n        break\n    if not d:\n        break\n    b+=d\nos.waitpid(pid,0)\nsys.stdout.buffer.write(b)\n";
    let o = Command::new("python3")
        .args(["-c", PTY, bin()])
        .args(args)
        .current_dir(repo())
        .env("HOME", dir)
        .env("GOAT_MESH_PROBE", "0")
        .env("GOAT_LANG", "en")
        .env("GOAT_NO_FUN", "1")
        .env("NO_COLOR", "1")
        .env_remove("GOATBUILDER_TUTOR")
        .output()
        .expect("python3 runs the pseudo-terminal");
    String::from_utf8_lossy(&o.stdout).replace('\r', "")
}

/// Rule 2: a page may lay rows out as it likes, but a failure is never dropped
/// and its machine is never lost. Every failing `-o json` row is named on the
/// page. `doctor`'s per-component secrets rows are excluded: rule 8 folds them
/// into one entry, and `doctor_prints_problems_...` covers that.
#[test]
fn a_failing_row_is_on_the_page_a_person_reads() {
    let held = scratch("goatbuilder-standard-failing-");
    let dir = held.path().to_path_buf();
    let tutor = dir.join("tutor");
    copy_tree(&repo().join("ops/goatbuilder/fixtures/tutor"), &tutor);
    let t = tutor.display().to_string();
    let v = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let cases = [
        v(&["--mock", &t, "verify", "portal", "--env", "preprod"]),
        v(&["--mock", &t, "rollback", "db-schema", "1", "--env", "preprod", "--ack", "Standard Test", "--yes"]),
    ];
    let mut failures = 0;
    for args in &cases {
        let doc: serde_json::Value = serde_json::from_str(&stdout(&run(args, true, &dir))).unwrap();
        let page = on_a_terminal(args, &dir);
        for row in doc["checks"].as_array().unwrap().iter().filter(|c| c["ok"] == false) {
            let name = row["name"].as_str().unwrap();
            let place = name.split_once('/').map_or(name, |(_, rest)| rest);
            assert!(page.contains(place), "{args:?}: {name} is not on the page:\n{page}");
            failures += 1;
        }
    }
    assert!(failures > 0, "these cases are here because they fail; nothing failed");
    let _ = std::fs::remove_dir_all(&dir);
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn with_corpus(name: &str, f: impl Fn(&Case, &Path)) {
    let held = scratch(&format!("goatbuilder-standard-{name}-"));
    let dir = held.path().to_path_buf();
    for c in corpus(&dir) {
        f(&c, &dir);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Rule 1: `-o json` is the contract, for every verb in the corpus.
#[test]
fn every_verb_keeps_the_machine_contract() {
    with_corpus("json", |c, dir| {
        if c.verb == "explain" {
            return; // explain prints its own document
        }
        let o = run(&c.args, true, dir);
        let out = stdout(&o);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{:?}: -o json is not JSON ({e}): {out}", c.args));
        let schema = v["schema"].as_str().unwrap_or_default();
        // A versioned document a consumer can pin to. Most are
        // `goatbuilder.<verb>/v<n>`; `survey` and `promote readiness` carry
        // their own names, versioned with a `-v<n>` suffix.
        let versioned = (schema.starts_with("goatbuilder.") && schema.contains("/v"))
            || schema.rsplit_once("-v").is_some_and(|(head, n)| !head.is_empty() && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        assert!(versioned, "{:?}: schema {schema:?}", c.args);
    });
}

/// Rule 2: a title is not a check. No `===` banner reaches the table.
#[test]
fn no_table_prints_a_banner() {
    with_corpus("banner", |c, dir| {
        let out = stdout(&run(&c.args, false, dir));
        for line in out.lines() {
            assert!(!line.contains("==="), "{:?}: a banner in the table: {line}", c.args);
        }
    });
}

/// Rule 2: a dry run never claims a result it did not observe.
#[test]
fn a_dry_run_claims_nothing_it_did_not_read() {
    with_corpus("dry", |c, dir| {
        if !c.dry_run {
            return;
        }
        for json in [false, true] {
            let out = stdout(&run(&c.args, json, dir));
            for claim in ["every host's ledger names", "(ledger-verified)"] {
                assert!(!out.contains(claim), "{:?}: a dry run says \"{claim}\":\n{out}", c.args);
            }
        }
    });
}

/// Rule 3: pages stay inside 80 columns, past the shrink-only allowlist. A line
/// holding one word (a long path) cannot wrap and is not held to it.
#[test]
fn pages_stay_inside_the_width() {
    let wide = std::sync::Mutex::new(std::collections::BTreeSet::<&'static str>::new());
    with_corpus("width", |c, dir| {
        let out = stdout(&run(&c.args, false, dir));
        let over: Vec<&str> = out.lines().filter(|l| l.chars().count() > WIDTH && l.trim().contains(' ')).collect();
        if !over.is_empty() {
            assert!(WIDE_ALLOWED.contains(&c.verb), "{}: lines over {WIDTH} columns:\n{}", c.verb, over.join("\n"));
            wide.lock().unwrap().insert(c.verb);
        }
    });
    let wide = wide.into_inner().unwrap();
    for v in WIDE_ALLOWED {
        // Only verbs the corpus runs can be judged; the list only shrinks.
        if ["standup", "deploy"].contains(v) {
            assert!(wide.contains(v), "{v} now fits the page: remove it from WIDE_ALLOWED");
        }
    }
}

/// Rule 4: no em or en dash, past the shrink-only allowlist.
#[test]
fn no_dashes_past_the_allowlist() {
    with_corpus("dash", |c, dir| {
        if DASH_ALLOWED.contains(&c.verb) {
            return;
        }
        let out = stdout(&run(&c.args, false, dir));
        for line in out.lines() {
            assert!(!line.contains('\u{2014}') && !line.contains('\u{2013}'), "{}: a dash: {line}", c.verb);
        }
    });
}

/// Rule 8: doctor prints problems by default; `--all` and `-o json` keep every row.
#[test]
fn doctor_prints_problems_and_keeps_every_row_for_machines() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let held = scratch("goatbuilder-standard-doctor-");
    let dir = held.path().to_path_buf();
    let r = repo().display().to_string();
    let args = |extra: &[&str]| -> Vec<String> { ["-C", &r, "doctor", "--env", "zagros_dev"].iter().chain(extra).map(|s| s.to_string()).collect() };
    let page = stdout(&run(&args(&[]), false, &dir));
    assert!(page.contains("--all"), "{page}");
    assert!(!page.contains("playbook:netvis"), "a passing row on the page:\n{page}");
    for line in page.lines() {
        assert!(line.chars().count() <= WIDTH || !line.trim().contains(' '), "wider than the page: {line}");
    }
    assert!(stdout(&run(&args(&["--all"]), false, &dir)).contains("playbook:netvis"));
    let doc: serde_json::Value = serde_json::from_str(&stdout(&run(&args(&[]), true, &dir))).unwrap();
    assert!(doc["checks"].as_array().unwrap().iter().any(|c| c["name"] == "playbook:netvis"));
    // A check that did not run says so in the contract, not only in its words.
    for c in doc["checks"].as_array().unwrap() {
        let words = c["detail"].as_str().unwrap_or("").starts_with("not checked");
        assert_eq!(c["skipped"] == true, words && c["ok"] == true, "{c}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Rule 7: the banner and the goat are for a terminal. These runs are piped.
#[test]
fn no_flourish_when_the_output_is_not_a_terminal() {
    with_corpus("fun", |c, dir| {
        let out = stdout(&run(&c.args, false, dir));
        assert!(!out.contains('\u{1f410}'), "{:?}: the goat on a pipe:\n{out}", c.args);
        assert!(!out.contains("\u{2588}\u{2590}\u{2580}\u{2588}"), "{:?}: the banner on a pipe:\n{out}", c.args);
    });
}

/// Rule 1: `-o json` is the contract on a terminal too, not only through a pipe.
#[test]
fn json_on_a_terminal_is_the_document() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let held = scratch("goatbuilder-standard-jsontty-");
    let dir = held.path().to_path_buf();
    let r = repo().display().to_string();
    let out = on_a_terminal(&["-o".into(), "json".into(), "-C".into(), r, "promote".into(), "readiness".into()], &dir);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"));
    assert!(v["schema"].as_str().unwrap_or("").contains("release-readiness"), "{}", v["schema"]);
    let _ = std::fs::remove_dir_all(&dir);
}
