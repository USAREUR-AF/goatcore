//! Whole-flow integration tests: the built `goatbuilder` binary driven through
//! multi-step flows on throwaway copies, asserting on exit codes, the records
//! the flows leave behind, and what the repository looks like afterwards.
//!
//! Defect classes these flows guard (grouped from the drivers' fix history and
//! from what the parity gates and the e2e harness found; each class is covered
//! through a verb's contract, not one case per past fix):
//!   - silent exit: a refusal always carries a reason on stderr/stdout and a
//!     non-zero exit; nothing ends with no output
//!   - target and scope resolution: a verb acts only on the declared
//!     environment, group and component
//!   - checkout freshness and repository root: `-C` decides the tree read
//!   - dry runs that hide a step: every step a run would take is printed
//!   - tool errors masked as data errors: a refusal names the real cause
//!   - stale or superseded release records: `promote check` refuses them and
//!     `abandon` closes them
//!   - non-idempotent re-runs: a second run on an unchanged tree changes nothing
//!   - provenance of done: pending and unknown are never counted converged
//!   - topology versus skew: an absent deviation is not drift
//!
//! Flows that need hosts, GitHub or a registry are not here: the deployment
//! e2e harness (ops/e2e) runs those against container hosts.


#[path = "common/test_home.rs"]
mod test_home;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_goatbuilder")
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

fn run(args: &[&str]) -> Output {
    test_home::isolate(&mut Command::new(bin()))
        .args(args)
        .env("GOAT_MESH_PROBE", "0")
        .output()
        .expect("run goatbuilder")
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}

/// Copy a directory tree (the fixtures are small, plain files).
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

fn fixture_copy(name: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join(name);
    copy_tree(&repo().join("ops/goatbuilder/fixtures").join(name), &dest);
    (tmp, dest)
}

/// Every file under `dir`, with its bytes, in a stable order: a snapshot of
/// what a flow left on disk.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    fn walk(d: &Path, root: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries: Vec<_> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, root, out);
            } else {
                out.push((p.strip_prefix(root).unwrap().to_path_buf(), std::fs::read(&p).unwrap()));
            }
        }
    }
    walk(dir, dir, &mut out);
    out
}

/// A detached git worktree of HEAD, removed on drop: the repository's real
/// pins, ladder, records and intents, safe to mutate.
struct Worktree {
    path: PathBuf,
    _tmp: tempfile::TempDir,
}

impl Worktree {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        // A unique basename: git names the worktree's admin directory after it,
        // and parallel tests adding "wt" at once race for .git/worktrees/wt.
        let unique = tmp.path().file_name().unwrap().to_string_lossy().trim_start_matches('.').to_string();
        let path = tmp.path().join(format!("wt-{unique}"));
        let st = Command::new("git")
            .arg("-C")
            .arg(repo())
            .args(["worktree", "add", "-q", "--detach"])
            .arg(&path)
            .arg("HEAD")
            .status()
            .expect("git worktree add");
        assert!(st.success(), "git worktree add failed");
        Worktree { path, _tmp: tmp }
    }

    fn s(&self) -> &str {
        self.path.to_str().unwrap()
    }

    fn porcelain(&self) -> String {
        let o = Command::new("git").arg("-C").arg(&self.path).args(["status", "--porcelain"]).output().unwrap();
        String::from_utf8_lossy(&o.stdout).to_string()
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        let _ = Command::new("git")
            .arg("-C")
            .arg(repo())
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .status();
    }
}

/// The checkout's git status, to prove a read-only verb left it untouched.
fn repo_porcelain() -> String {
    let o = Command::new("git").arg("-C").arg(repo()).args(["status", "--porcelain"]).output().unwrap();
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn python_deps() -> bool {
    Command::new("python3")
        .args(["-c", "import yaml, jsonschema"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// --- deploy and status on the conformance fixture -----------------------------


/// Whether this checkout declares a deployment of its own. A public clone of
/// goatcore declares none, by design; the flows that run against this
/// checkout's own deployment return early there.
fn checkout_declares_a_deployment() -> bool {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ops/goatbuilder/goatbuilder.yaml")
        .is_file()
}

#[test]
fn deploy_refuses_a_mutable_tag_declaration_c1() {
    let (_t, fx) = fixture_copy("basic");
    let r = repo();
    // The fixture declares netvis at `:latest`: a deploy declares a digest (C1).
    let o = run(&["deploy", "-C", r.to_str().unwrap(), "--mock", fx.to_str().unwrap(), "--env", "dev", "--component", "netvis", "--ack", "Flow Test", "--yes"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("mutable tag") && text(&o).contains("C1"), "{}", text(&o));
}

#[test]
fn deploy_converges_resumes_and_a_rerun_changes_nothing() {
    let (_t, fx) = fixture_copy("basic");
    let r = repo();
    let r = r.to_str().unwrap();
    let f = fx.to_str().unwrap();
    let args = ["deploy", "-C", r, "--mock", f, "--env", "dev", "--component", "portal", "--ack", "Flow Test", "--yes"];

    // First run: `after: 2` is still pending, `never` and `unreadable` stay
    // unconverged, so the verb exits 3 (C7), with every machine named.
    let one = run(&args);
    assert_eq!(code(&one), 3, "{}", text(&one));
    let t1 = text(&one);
    assert!(t1.lines().any(|l| l.contains("dev/dev-obs-a/portal") && l.contains("pending")), "{t1}");

    // Second run resumes: the `after: 2` machine converges now (C5).
    let two = run(&args);
    assert_eq!(code(&two), 3);
    let t2 = text(&two);
    assert!(t2.lines().any(|l| l.contains("dev/dev-obs-a/portal") && l.contains("converged")), "{t2}");
    let records_after_two = snapshot(&fx.join("records"));

    // Third run on an unchanged declaration writes no new record (C5).
    let three = run(&args);
    assert_eq!(code(&three), 3);
    assert_eq!(snapshot(&fx.join("records")), records_after_two, "a re-run must not append records");
}

#[test]
fn status_never_counts_pending_or_unknown_as_done() {
    let (_t, fx) = fixture_copy("basic");
    let r = repo();
    let before = snapshot(&fx);
    let o = run(&["-o", "json", "status", "-C", r.to_str().unwrap(), "--mock", fx.to_str().unwrap(), "--env", "dev"]);
    assert_eq!(code(&o), 3);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("json");
    assert_eq!(v["ok"], false);
    assert_eq!(v["exit"], 3);
    let checks = v["checks"].as_array().unwrap();
    let unknown = checks.iter().filter(|c| c["detail"].as_str().unwrap_or("").starts_with("unknown")).count();
    let pending = checks.iter().filter(|c| c["detail"].as_str().unwrap_or("").starts_with("pending")).count();
    assert!(unknown >= 1 && pending >= 1, "{v}");
    for c in checks {
        let d = c["detail"].as_str().unwrap_or("");
        if d.starts_with("unknown") || d.starts_with("pending") {
            assert_eq!(c["ok"], false, "pending/unknown counted done: {d}");
        }
    }
    // status reads only (C8).
    assert_eq!(snapshot(&fx), before, "status modified the fixture");
}

#[test]
fn deploy_then_rollback_then_status_names_the_previous_ref_by_a_new_declaration_c9() {
    let (_t, fx) = fixture_copy("basic");
    let r = repo();
    let (r, f) = (r.to_str().unwrap(), fx.to_str().unwrap());
    let deploy = ["deploy", "-C", r, "--mock", f, "--env", "dev", "--component", "portal", "--ack", "Flow Test", "--yes"];
    run(&deploy);
    run(&deploy);
    let pubs = || std::fs::read_to_string(fx.join("declared/publications.jsonl")).unwrap_or_default();
    let rows = || std::fs::read_to_string(fx.join("records/dev/dev-core-a.jsonl")).unwrap();
    let before_pubs = pubs().lines().count();
    let before_rows = rows();
    assert!(before_rows.lines().last().unwrap().contains("portal:v1.4.0"), "{before_rows}");

    // A rollback without --env on a fixture is refused, naming what it needs.
    let no_env = run(&["rollback", "-C", r, "--mock", f, "portal", "1", "--ack", "Flow Test", "--yes"]);
    assert_ne!(code(&no_env), 0);
    assert!(text(&no_env).contains("--env"), "{}", text(&no_env));

    let rb = run(&["rollback", "-C", r, "--mock", f, "portal", "1", "--env", "dev", "--ack", "Flow Test", "--yes"]);
    assert_eq!(code(&rb), 3, "never/unreadable machines keep it not converged: {}", text(&rb));
    assert!(text(&rb).lines().any(|l| l.contains("dev/dev-core-a/portal") && l.contains("portal:v1.3.2")), "{}", text(&rb));

    // C9: the revert is a NEW declaration at the old digest, appended; history is kept.
    assert_eq!(pubs().lines().count(), before_pubs + 1, "a rollback publishes one new declaration");
    assert!(pubs().lines().last().unwrap().contains("portal:v1.3.2"));
    assert!(rows().starts_with(&before_rows), "records are appended, never rewritten");
    assert!(rows().lines().last().unwrap().contains("portal:v1.3.2"));

    let st = status_rows(r, f);
    assert!(st.iter().any(|c| c["name"] == "dev/dev-core-a/portal" && c["ok"] == true && c["detail"].as_str().unwrap().contains("portal:v1.3.2")), "{st:?}");
}

#[test]
fn status_after_deploy_reports_the_converged_machines() {
    let (_t, fx) = fixture_copy("basic");
    let r = repo();
    let (r, f) = (r.to_str().unwrap(), fx.to_str().unwrap());
    run(&["deploy", "-C", r, "--mock", f, "--env", "dev", "--component", "portal", "--ack", "Flow Test", "--yes"]);
    let st = status_rows(r, f);
    assert!(st.iter().any(|c| c["name"] == "dev/dev-core-a/portal" && c["ok"] == true), "{st:?}");
    assert!(st.iter().any(|c| c["name"].as_str().unwrap().contains("dev-edge-a") && c["detail"].as_str().unwrap().starts_with("unknown")), "{st:?}");
    // The page a person reads carries the same answer, in columns.
    let page = text(&run(&["status", "-C", r, "--mock", f, "--env", "dev"]));
    assert!(page.contains("COMPONENT") && page.lines().any(|l| l.contains("dev-core-a") && l.contains("converged")), "{page}");
}

/// `status` as a program reads it: the rows of `-o json` (output standard, rule 1).
fn status_rows(r: &str, f: &str) -> Vec<serde_json::Value> {
    let o = run(&["-o", "json", "status", "-C", r, "--mock", f, "--env", "dev"]);
    let doc: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&o.stdout)).unwrap();
    doc["checks"].as_array().unwrap().clone()
}

#[test]
fn deploy_on_an_undeclared_environment_is_refused_with_a_reason() {
    let (_t, fx) = fixture_copy("basic");
    let r = repo();
    let o = run(&["deploy", "-C", r.to_str().unwrap(), "--mock", fx.to_str().unwrap(), "--env", "staging", "--ack", "Flow Test", "--yes"]);
    assert_ne!(code(&o), 0);
    assert!(!text(&o).trim().is_empty(), "a refusal must say why");
}

#[test]
fn a_mutation_without_yes_and_without_a_terminal_touches_nothing() {
    let (_t, fx) = fixture_copy("basic");
    let r = repo();
    let before = snapshot(&fx);
    let o = run(&["deploy", "-C", r.to_str().unwrap(), "--mock", fx.to_str().unwrap(), "--env", "dev"]);
    assert_ne!(code(&o), 0);
    assert!(text(&o).contains("C10"), "{}", text(&o));
    assert_eq!(snapshot(&fx), before, "a refused mutation changed the fixture");
}

// --- standup on the fixture ---------------------------------------------------

#[test]
fn standup_on_the_fixture_converges_then_status_agrees_and_a_rerun_is_idempotent() {
    let (_t, fx) = fixture_copy("standup");
    let r = repo();
    let (r, f) = (r.to_str().unwrap(), fx.to_str().unwrap());
    let one = run(&["standup", "-C", r, "--mock", f, "dev", "--site", "alpha", "--ack", "Flow Test", "--yes"]);
    assert_eq!(code(&one), 0, "{}", text(&one));
    assert!(text(&one).to_lowercase().contains("verified"), "{}", text(&one));
    let st = run(&["status", "-C", r, "--mock", f, "--env", "dev"]);
    assert_eq!(code(&st), 0, "{}", text(&st));
    let records = snapshot(&fx.join("records"));
    let two = run(&["standup", "-C", r, "--mock", f, "dev", "--site", "alpha", "--ack", "Flow Test", "--yes"]);
    assert_eq!(code(&two), 0, "{}", text(&two));
    assert_eq!(snapshot(&fx.join("records")), records, "a second standup must not append records");
}

#[test]
fn standup_on_an_undeclared_site_is_refused() {
    let (_t, fx) = fixture_copy("standup");
    let r = repo();
    let o = run(&["standup", "-C", r.to_str().unwrap(), "--mock", fx.to_str().unwrap(), "dev", "--site", "nowhere", "--yes"]);
    assert_ne!(code(&o), 0);
    assert!(text(&o).contains("nowhere"), "{}", text(&o));
}

// --- validate every fixture the repository ships ------------------------------

#[test]
fn every_shipped_fixture_config_validates() {
    let root = repo().join("ops/goatbuilder/fixtures");
    let mut configs = Vec::new();
    fn find(d: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                find(&p, out);
            } else if p.file_name().map(|n| n == "goatbuilder.yaml").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    find(&root, &mut configs);
    assert!(configs.len() >= 2, "expected the shipped fixtures, found {configs:?}");
    let r = repo();
    for c in configs {
        let o = run(&["validate", "-C", r.to_str().unwrap(), c.to_str().unwrap()]);
        assert_eq!(code(&o), 0, "{}: {}", c.display(), text(&o));
    }
}

#[test]
fn the_repository_config_validates() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let r = repo();
    let o = run(&["validate", "-C", r.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}", text(&o));
}

// --- pins on a worktree of the real tree ----------------------------------------

#[test]
fn pins_rerun_on_an_unchanged_tree_changes_nothing() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    if !python_deps() {
        eprintln!("skipped: PyYAML + jsonschema not importable (pins renders through the same schema)");
        return;
    }
    let wt = Worktree::new();
    let o = run(&["pins", "-C", wt.s(), "--yes"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert_eq!(wt.porcelain(), "", "pins on an unchanged tree must change nothing");
}

#[test]
fn pins_check_names_drift_and_pins_repairs_it_without_resigning() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    if !python_deps() {
        eprintln!("skipped: PyYAML + jsonschema not importable");
        return;
    }
    let wt = Worktree::new();
    let rel = "ops/ansible/group_vars/zagros_dev_hosts/pins.yml";
    let p = wt.path.join(rel);
    let s = std::fs::read_to_string(&p).unwrap();
    let re = regex::Regex::new(r"sha256:[0-9a-f]{8}").unwrap();
    let drifted = re.replace(&s, "sha256:00000000").to_string();
    assert_ne!(drifted, s);
    std::fs::write(&p, drifted).unwrap();

    let check = run(&["pins", "-C", wt.s(), "--check"]);
    assert_eq!(code(&check), 1, "{}", text(&check));
    assert!(text(&check).contains(rel), "the refusal must name the drifted file: {}", text(&check));

    let fix = run(&["pins", "-C", wt.s(), "--yes"]);
    assert_eq!(code(&fix), 0, "{}", text(&fix));
    // Regenerating restores the file; no intent changed, so nothing is re-signed
    // and the tree is back to HEAD.
    assert_eq!(wt.porcelain(), "", "repair must restore the tree exactly");
}

// --- promote: a stale open record is refused, then closed ---------------------

#[test]
fn promote_open_then_check_refuses_the_stale_record_then_abandon_closes_it() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    if !python_deps() {
        eprintln!("skipped: PyYAML + jsonschema not importable");
        return;
    }
    let wt = Worktree::new();
    let ok = run(&["promote", "-C", wt.s(), "check"]);
    assert_eq!(code(&ok), 0, "main must pass its own check: {}", text(&ok));

    let digest = "a".repeat(64);
    let reference = format!("ghcr.io/{{{{ ghcr_org }}}}/netvis:sha-1234567@sha256:{digest}");
    let open = run(&["promote", "-C", wt.s(), "open", "netvis", "--ref", &reference, "--reason", "flow test", "--yes"]);
    assert_eq!(code(&open), 0, "{}", text(&open));
    let status = wt.porcelain();
    let record = status
        .lines()
        .find_map(|l| l.strip_prefix("?? "))
        .expect("open wrote a record")
        .trim()
        .to_string();
    assert!(record.starts_with("ops/release/records/netvis/"), "{record}");

    // The record is open, but no rung runs its ref: C6 refuses and names it.
    let refused = run(&["promote", "-C", wt.s(), "check"]);
    assert_eq!(code(&refused), 1, "{}", text(&refused));
    assert!(text(&refused).contains("C6") && text(&refused).contains(&record), "{}", text(&refused));

    // Abandon without --yes and without a terminal is refused (C10).
    let nope = run(&["promote", "-C", wt.s(), "abandon", &record, "--reason", "flow test"]);
    assert_ne!(code(&nope), 0);
    assert!(text(&nope).contains("C10"));

    let abandon = run(&["promote", "-C", wt.s(), "abandon", &record, "--reason", "flow test", "--yes"]);
    assert_eq!(code(&abandon), 0, "{}", text(&abandon));
    let after = run(&["promote", "-C", wt.s(), "check"]);
    assert_eq!(code(&after), 0, "{}", text(&after));
}

#[test]
fn promote_status_and_readiness_read_only() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    if !python_deps() {
        eprintln!("skipped: PyYAML + jsonschema not importable");
        return;
    }
    let wt = Worktree::new();
    for args in [vec!["promote", "-C", wt.s(), "status"], vec!["-o", "json", "promote", "-C", wt.s(), "readiness"]] {
        let o = run(&args);
        assert_eq!(code(&o), 0, "{:?}: {}", args, text(&o));
    }
    assert_eq!(wt.porcelain(), "", "read-only verbs changed the tree");
}

// --- dry runs on the real tree: every decision printed, nothing touched -------

#[test]
fn deploy_dry_run_prints_every_decision_and_touches_nothing() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let r = repo();
    let rs = r.to_str().unwrap();
    let before = repo_porcelain();
    let o = run(&["deploy", "-C", rs, "--env", "zagros_dev", "--component", "netvis", "--dry-run"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let t = text(&o);
    for want in ["/transport", "/playbook", "DRY: deploying", "DRY: would read /var/lib/goat/netvis/deploy-history.jsonl"] {
        assert!(t.contains(want), "dry run hid `{want}`:\n{t}");
    }
    assert_eq!(repo_porcelain(), before, "a read-only run changed the checkout");
}

#[test]
fn deploy_dry_run_names_secret_variables_never_their_values() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let r = repo();
    let rs = r.to_str().unwrap();
    let before = repo_porcelain();
    let secret = "s3cr3t-value-that-must-not-print";
    let o = test_home::isolate(&mut Command::new(bin()))
        .args(["deploy", "-C", rs, "--env", "zagros_dev", "--component", "netvis", "--dry-run"])
        .env("GOAT_MESH_PROBE", "0")
        .env("ZAGROS_DEV_NETVIS_OAUTH2_PROXY_CLIENT_SECRET", secret)
        .output()
        .unwrap();
    let t = text(&o);
    assert!(t.contains("secrets"), "{t}");
    assert!(!t.contains(secret), "a secret's value reached the output");
    assert_eq!(repo_porcelain(), before, "a read-only run changed the checkout");
}


#[test]
fn deploy_dry_run_skips_an_absent_deviation_as_topology_not_drift() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let r = repo();
    let rs = r.to_str().unwrap();
    let before = repo_porcelain();
    // microscope is single-homed: its joiner groups declare `absent: true`.
    let o = run(&["deploy", "-C", rs, "--env", "zagros", "--component", "microscope", "--dry-run"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let t = text(&o);
    assert!(t.lines().any(|l| l.contains("/skipped") && l.contains("absent: true")), "{t}");
    assert_eq!(repo_porcelain(), before, "a read-only run changed the checkout");
}


#[test]
fn deploy_dry_run_on_a_group_outside_the_environment_is_refused() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let r = repo();
    let rs = r.to_str().unwrap();
    let before = repo_porcelain();
    let o = run(&["deploy", "-C", rs, "--env", "zagros_dev", "--component", "netvis", "--group", "zagros_hosts", "--dry-run"]);
    assert_ne!(code(&o), 0, "{}", text(&o));
    assert!(text(&o).contains("zagros_hosts"), "{}", text(&o));
    assert_eq!(repo_porcelain(), before, "a read-only run changed the checkout");
}


#[test]
fn standup_dry_run_prints_every_phase_in_order() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let r = repo();
    let rs = r.to_str().unwrap();
    let before = repo_porcelain();
    let o = run(&["standup", "-C", rs, "zagros_dev", "--site", "zagros-dev", "--dry-run"]);
    let t = text(&o);
    let phases: Vec<u32> = t
        .lines()
        // The table prints each phase as a heading: "Phase 5 · core · …".
        .filter_map(|l| l.strip_prefix("Phase "))
        .filter_map(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .filter_map(|n| n.parse().ok())
        .collect();
    assert!(phases.len() >= 10, "standup dry run printed too few phases:\n{t}");
    let mut sorted = phases.clone();
    sorted.sort_unstable();
    assert_eq!(phases, sorted, "phases out of order:\n{t}");
    assert_eq!(repo_porcelain(), before, "a read-only run changed the checkout");
}


#[test]
fn join_node_dry_run_plans_every_phase_without_an_address() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let r = repo();
    let rs = r.to_str().unwrap();
    let before = repo_porcelain();
    let o = run(&["join-node", "-C", rs, "zagros", "--site", "zagros", "--node", "flow-test-node", "--class", "core", "--dry-run"]);
    let t = text(&o);
    assert!(t.contains("flow-test-node"), "{t}");
    assert!(t.matches("Phase").count() >= 10, "{t}");
    assert_eq!(repo_porcelain(), before, "a read-only run changed the checkout");
}

#[test]
fn join_node_real_run_without_an_address_is_refused() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let r = repo();
    let rs = r.to_str().unwrap();
    let before = repo_porcelain();
    let o = run(&["join-node", "-C", rs, "zagros", "--site", "zagros", "--node", "n", "--class", "core", "--yes"]);
    assert_ne!(code(&o), 0);
    assert!(text(&o).contains("--address"), "{}", text(&o));
    assert_eq!(repo_porcelain(), before, "a refused run changed the checkout");
}

// --- rollback: refusals that fire before any ledger is read --------------------

#[test]
fn rollback_refuses_out_of_range_steps_before_reading_anything() {
    for n in ["0", "8"] {
        let o = run(&["rollback", "-C", repo().to_str().unwrap(), "netvis", n, "--dry-run"]);
        assert_ne!(code(&o), 0);
        assert!(text(&o).contains("1..7"), "{}", text(&o));
    }
}

#[test]
fn rollback_names_a_component_without_a_ledger_role() {
    let o = run(&["rollback", "-C", repo().to_str().unwrap(), "no-such-component", "--status"]);
    assert_ne!(code(&o), 0);
    assert!(text(&o).contains("no-such-component"), "{}", text(&o));
}

// --- explain: a flag after the positional, and --list under -o json (#2131) ---

/// A flag written after the positional code must parse as a flag, not be
/// swallowed into the code/text positional (#2131, bug 1). Before the fix,
/// `trailing_var_arg` on the `explain` positional made clap treat every token
/// after the first one, including `-o json`, as more of that positional's
/// text, so this printed the garbled table header "GB-017 -o json" instead
/// of parsing `-o` as the output-format flag.
#[test]
fn explain_a_flag_after_the_positional_code_parses_as_a_flag_not_text() {
    let o = run(&["explain", "GB-017", "-o", "json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("explain GB-017 -o json did not print JSON: {e}\n{stdout}")
    });
    assert_eq!(v["code"], "GB-017", "{stdout}");
    // The defect printed the table form, headed by the echoed text
    // "GB-017 -o json"; prove that string never appears.
    assert!(!stdout.contains("GB-017 -o json"), "{stdout}");

    // A flag anywhere else on the line parses the same way.
    let o2 = run(&["explain", "--lang", "en", "GB-017", "-o", "json"]);
    assert_eq!(code(&o2), 0, "{}", text(&o2));
    let stdout2 = String::from_utf8_lossy(&o2.stdout).to_string();
    let v2: serde_json::Value = serde_json::from_str(&stdout2).unwrap();
    assert_eq!(v2["code"], "GB-017", "{stdout2}");
}

/// Free text with no flags still collects every word into the one
/// code/text positional (the behavior `trailing_var_arg` existed for, which
/// the fix must not regress).
#[test]
fn explain_unflagged_free_text_still_collects_every_word() {
    let o = run(&["explain", "zxqv", "frobnicated", "the", "flux", "capacitor"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("zxqv frobnicated the flux capacitor"), "{}", text(&o));
}

/// A hyphen-leading literal still reads as text after an explicit `--`
/// separator (clap's standard convention; the fix relies on it).
#[test]
fn explain_a_hyphen_leading_literal_reads_as_text_after_a_separator() {
    let o = run(&["explain", "--", "-o", "json"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("-o json"), "{}", text(&o));
}

/// `--list` under `-o json`/`-o yaml` must render the catalog as structured
/// rows, not the fixed human table (#2131, bug 2). Before the fix, `--list`
/// ignored `format` entirely and always printed the table.
#[test]
fn explain_list_honors_the_output_format_c10() {
    let table = run(&["explain", "--list"]);
    assert_eq!(code(&table), 0, "{}", text(&table));
    let table_stdout = String::from_utf8_lossy(&table.stdout).to_string();
    // The table form has no braces; a defect regressing to it would fail the
    // JSON parse below anyway, but this pins the two shapes as distinct.
    assert!(!table_stdout.trim_start().starts_with('['), "{table_stdout}");

    for flag in ["json", "yaml"] {
        let o = run(&["explain", "-o", flag, "--list"]);
        assert_eq!(code(&o), 0, "{}", text(&o));
        let stdout = String::from_utf8_lossy(&o.stdout).to_string();
        assert_ne!(stdout, table_stdout, "{flag}: {stdout}");
        let rows: Vec<serde_json::Value> = if flag == "json" {
            serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("not a JSON array: {e}\n{stdout}"))
        } else {
            let v: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap_or_else(|e| panic!("not YAML: {e}\n{stdout}"));
            serde_json::from_value(serde_json::to_value(v).unwrap()).unwrap()
        };
        assert!(rows.len() > 1, "{flag}: {stdout}");
        for row in &rows {
            assert!(row.get("code").is_some(), "{flag}: {row}");
            assert!(row.get("tool").is_some(), "{flag}: {row}");
            assert!(row.get("title").is_some(), "{flag}: {row}");
        }
        // The known codes from the table form all show up as rows.
        assert!(rows.iter().any(|r| r["code"] == "GB-017"), "{flag}: {stdout}");
    }
}

/// The rollout preflight (ADR 1211) grades the four cases on the fixture: a
/// clean stateless pin-revert, a stateful restore-required with its window, an
/// undeclared component that fails closed, and a declared coupling — honesty-
/// graded, the three facts always printed. GOAT_PREFLIGHT_PULL=0 keeps the
/// candidate-ref probe off; GOAT_PREFLIGHT_TODAY pins the retention window.
#[test]
fn preflight_mock_grades_the_four_rollout_cases() {
    let (_tmp, fx) = fixture_copy("preflight");
    let r = repo();
    let preflight = |component: &str| -> Output {
        test_home::isolate(&mut Command::new(bin()))
            .args(["-o", "json", "preflight", "-C", r.to_str().unwrap(), "--mock", fx.to_str().unwrap(), component, "--env", "zagros-dev"])
            .env("GOAT_MESH_PROBE", "0")
            .env("GOAT_PREFLIGHT_PULL", "0")
            .env("GOAT_PREFLIGHT_TODAY", "2026-11-05")
            .output()
            .expect("run goatbuilder preflight")
    };
    let facts = |o: &Output| -> std::collections::BTreeMap<String, String> {
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap_or_else(|_| panic!("json: {}", text(o)));
        v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c["name"].as_str().unwrap().to_string(), c["detail"].as_str().unwrap().to_string()))
            .collect()
    };

    // Stateless / pin-revert / single: clean, and the pin flip is validated.
    let o = preflight("netvis");
    assert_eq!(code(&o), 0, "{}", text(&o));
    let f = facts(&o);
    assert!(f["preflight/state"].contains("stateless"), "{:?}", f);
    assert_eq!(f["preflight/rollback"], "Rollback: pin-revert");
    assert!(f["preflight/scope"].contains("single"));
    assert!(f["preflight/rollback-validated"].contains("confirm"), "{:?}", f);

    // Stateful / restore-required: clean, with the restore pointer and the days
    // left in the window (as_of 28 days, today > 21 left).
    let o = preflight("id-console");
    assert_eq!(code(&o), 0, "{}", text(&o));
    let f = facts(&o);
    assert!(f["preflight/state"].contains("stateful"));
    assert!(f["preflight/rollback"].contains("restore-required") && f["preflight/rollback"].contains("28-day window"));
    assert!(f["preflight/restore-window"].contains("21 day(s) left"), "{:?}", f);

    // Undeclared: fails closed to stateful / none, graded not clean (GB-076).
    let o = preflight("mystery");
    assert_eq!(code(&o), 3, "{}", text(&o));
    let f = facts(&o);
    assert!(f["preflight/state"].contains("stateful") && f["preflight/state"].contains("undeclared"));
    assert!(f["preflight/rollback"].contains("none") && f["preflight/rollback"].contains("undeclared"));
    assert!(text(&o).contains("GB-076"), "{}", text(&o));

    // Declared coupling: scope reads coupled and names the other member.
    let o = preflight("console");
    assert_eq!(code(&o), 0, "{}", text(&o));
    let f = facts(&o);
    assert!(f["preflight/scope"].contains("coupled") && f["preflight/scope"].contains("health"), "{:?}", f);
}

/// The deploy prep phase (ADR 1211) gates a declared restore-required rollout on
/// --ack and names the risk; with --ack it gets past the gate (and only then
/// fails further along, on this minimal tree's missing goatnet). Built on a
/// temp tree whose fleet-pins declares one restore-required component, so the
/// gate is exercised without touching a real goatnet or any committed pins.
#[test]
fn deploy_prep_phase_gates_a_declared_restore_required_rollout_on_ack() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("ops/pins")).unwrap();
    std::fs::write(
        root.join("ops/pins/fleet-pins.yaml"),
        "goatnets:\n  demo:\n    members: [demo_hosts]\ncomponents:\n  cutover:\n    var: cutover_image\n    owns_state: true\n    rollback: restore-required\n    restore_from:\n      pointer: \"pre-cutover SQLite archive\"\n      retention_days: 28\n      as_of: \"2026-10-29\"\n",
    )
    .unwrap();
    let rs = root.to_str().unwrap();

    // No --ack: refused, the risk named, --ack named as the way through.
    let o = run(&["deploy", "-C", rs, "--env", "demo", "--component", "cutover", "--yes"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("restore-required") && t.contains("--ack"), "{t}");
    assert!(t.contains("not cleanly reversible"), "{t}");

    // With --ack: past the gate (no ack refusal); it fails later on this tree.
    let o = run(&["deploy", "-C", rs, "--env", "demo", "--component", "cutover", "--ack", "A Operator", "--yes"]);
    assert!(!text(&o).contains("re-run with --ack"), "ack given: not gated — {}", text(&o));
}

fn founder_copy(name: &str) -> (tempfile::TempDir, PathBuf) {
    let held = scratch(&format!("gb-mock-{name}-"));
    let dir = held.path().to_path_buf();
    copy_tree(&repo().join("ops/goatbuilder/fixtures/goatnet-from-scratch/01-founder"), &dir);
    (held, dir)
}

/// A dry run on a fixture acts on nothing and needs no actor, for standup and
/// for teardown (the walkthrough found both ignoring --dry-run).
#[test]
fn a_mock_dry_run_changes_nothing() {
    let (_held, dir) = founder_copy("dry");
    let d = dir.to_str().unwrap();
    let before = snapshot(&dir);
    let o = run(&["--mock", d, "standup", "zagros", "--site", "alpha", "--dry-run"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(text(&o).contains("DRY: would publish"), "{}", text(&o));
    assert!(!text(&o).contains("converged on") && !text(&o).contains("every record names"), "{}", text(&o));
    assert_eq!(snapshot(&dir), before, "a standup dry run changed the fixture");

    let up = run(&["--mock", d, "standup", "zagros", "--site", "alpha", "--ack", "A Person", "--yes"]);
    assert_eq!(code(&up), 0, "{}", text(&up));
    let built = snapshot(&dir);
    let o = run(&["--mock", d, "teardown", "zagros", "--site", "alpha", "--dry-run"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(text(&o).contains("DRY: would tear down"), "{}", text(&o));
    assert!(!text(&o).contains("torn down;"), "{}", text(&o));
    assert_eq!(snapshot(&dir), built, "a teardown dry run changed the fixture");
    let _ = std::fs::remove_dir_all(&dir);
}

/// --from and --to bound a rehearsal as they bound a real run.
#[test]
fn a_mock_standup_honours_its_phase_range() {
    let (_held, dir) = founder_copy("range");
    let d = dir.to_str().unwrap();
    let o = run(&["--mock", d, "standup", "zagros", "--site", "alpha", "--from", "9", "--to", "9", "--dry-run"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let t = text(&o);
    let phases: Vec<&str> = t.lines().filter(|l| l.starts_with("Phase ")).collect();
    assert!(!phases.is_empty() && phases.iter().all(|p| p.starts_with("Phase 9 ")), "{t}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// assay reads only: it leaves the checkout as it found it, whichever way the
/// site is spelled (the hyphen form names a tracked values file).
#[test]
fn assay_plan_writes_nothing_in_the_checkout() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    if !python_deps() {
        return;
    }
    let r = repo();
    let rs = r.to_str().unwrap();
    // Both spellings of the site are tracked values files now (ADR 1213 added
    // the underscore one), so the claim is that a plan leaves every one of them
    // byte for byte as it found it -- not that a path is absent.
    let values: Vec<PathBuf> = ["values-zagros-dev.yaml", "values-zagros_dev.yaml"]
        .iter()
        .map(|n| r.join("ops/validation").join(n))
        .filter(|p| p.is_file())
        .collect();
    assert!(!values.is_empty(), "the checkout tracks a values file to compare against");
    let tracked: Vec<Vec<u8>> = values.iter().map(|p| std::fs::read(p).unwrap()).collect();
    for site in ["zagros_dev", "zagros-dev"] {
        let o = run(&["-C", rs, "assay", site, "--plan"]);
        assert_eq!(code(&o), 0, "assay {site} --plan: {}", text(&o));
        for (p, before) in values.iter().zip(&tracked) {
            assert_eq!(&std::fs::read(p).unwrap(), before, "assay {site} --plan rewrote {}", p.display());
        }
    }
}

/// A throwaway repo and a `gh` that logs its arguments and knows one tag.
struct FastlaneRepo {
    dir: PathBuf,
    gh_log: PathBuf,
    path: String,
    _held: tempfile::TempDir,
}

fn fastlane_repo(name: &str, component: &str) -> FastlaneRepo {
    let held = scratch(&format!("goatbuilder-fastlane-{name}-"));
    let dir = held.path().to_path_buf();
    for d in ["ops/pins", "ops/ansible/roles/app", "ops/release/sets", "ops/goatbuilder", "bin"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    std::fs::write(dir.join("ops/goatbuilder/goatbuilder.yaml"), "repo:\n  source: USAREUR-AF/goatcore\n  image_namespace: ghcr.io/usareur-af\n").unwrap();
    std::fs::write(dir.join("ops/pins/fleet-pins.yaml"), format!("components:\n  app:\n    fleet: \"ghcr.io/{{{{ ghcr_org }}}}/app:sha-old@sha256:{}\"\n{component}goatnets: {{}}\n", "1".repeat(64))).unwrap();
    std::fs::write(dir.join("ops/ansible/odd-name.yml"), "- hosts: all\n").unwrap();
    let gh_log = dir.join("gh.log");
    let gh = dir.join("bin/gh");
    std::fs::write(&gh, format!("#!/bin/sh\necho \"$*\" >> '{}'\necho sha256:{}\n", gh_log.display(), "a".repeat(64))).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", dir.join("bin").display(), std::env::var("PATH").unwrap_or_default());
    FastlaneRepo { dir, gh_log, path, _held: held }
}

fn fastlane_dry(r: &FastlaneRepo, reference: &str) -> serde_json::Value {
    fastlane_dry_on(r, reference, &r.path)
}

/// The same dry run with a PATH of the caller's choosing, so a test can take a
/// tool away (a PATH of the fixture's bin alone has no `gh` once it is removed).
fn fastlane_dry_on(r: &FastlaneRepo, reference: &str, path: &str) -> serde_json::Value {
    let o = test_home::isolate(&mut Command::new(env!("CARGO_BIN_EXE_goatbuilder")))
        .args(["-o", "json", "-C", r.dir.to_str().unwrap(), "fastlane", "app", "--ref", reference, "--reason", "test", "--ack", "Flow Test", "--dry-run"])
        .env("PATH", path)
        .env("GOAT_MESH_PROBE", "0")
        .env_remove("GOATBUILDER_TUTOR")
        .output()
        .unwrap();
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)))
}

fn details(v: &serde_json::Value) -> String {
    v["checks"].as_array().unwrap().iter().map(|c| c["detail"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>().join("\n")
}

#[test]
fn fastlane_reads_every_page_of_the_one_org_and_honours_the_declared_play() {
    if !checkout_declares_a_deployment() {
        return; // a public clone of goatcore ships no deployment
    }
    let r = fastlane_repo("play", "    deploy_playbook: odd-name.yml\n");
    let v = fastlane_dry(&r, "sha-new");
    let log = std::fs::read_to_string(&r.gh_log).unwrap();
    let text = details(&v);
    let _ = std::fs::remove_dir_all(&r.dir);
    assert_eq!(v["exit"], 0, "{text}");
    // One registry lookup, whatever else `gh` was asked (it is probed for
    // presence first, so that a missing gh is not reported as a missing image).
    assert_eq!(log.lines().filter(|l| l.contains("api")).count(), 1, "one org, one lookup: {log}");
    assert!(log.contains("--paginate") && log.contains("/orgs/usareur-af/"), "{log}");
    assert!(!log.contains("goat-maintainer"), "{log}");
    assert!(text.contains("would deploy via odd-name.yml"), "{text}");
}

#[test]
fn fastlane_refuses_a_rollback_that_is_not_a_pin_revert() {
    for (decl, why) in [("    artifact_type: data\n", "artifact_type is data"), ("    reversible: false\n", "reversible is false"), ("    rollback: restore-required\n", "rollback is restore-required"), ("    rollback: false\n", "rollback is false")] {
        let r = fastlane_repo("revert", &format!("    deploy_playbook: odd-name.yml\n{decl}"));
        let v = fastlane_dry(&r, "sha-new");
        let text = details(&v);
        let _ = std::fs::remove_dir_all(&r.dir);
        assert_eq!(v["exit"], 1, "{decl}: {text}");
        assert!(text.contains(why) && text.contains("ADR 1127"), "{decl}: {text}");
    }
}

/// A missing `gh` is a refusal that names `gh`, not a missing image: a reader
/// without access to this project's packages must not be sent to look at CI.
#[test]
fn fastlane_without_gh_says_so_rather_than_blaming_the_image() {
    let r = fastlane_repo("nogh", "    deploy_playbook: odd-name.yml\n");
    std::fs::remove_file(r.dir.join("bin/gh")).unwrap();
    let only_bin = r.dir.join("bin").display().to_string();
    let v = fastlane_dry_on(&r, "sha-new", &only_bin);
    let text = details(&v);
    let _ = std::fs::remove_dir_all(&r.dir);
    assert_eq!(v["exit"], 1, "{text}");
    assert!(text.contains("the GitHub CLI (gh) is not on PATH"), "{text}");
    assert!(!text.contains("has CI built it"), "the image is not the cause: {text}");
}

#[test]
fn fastlane_treats_an_empty_artifact_type_as_image() {
    let r = fastlane_repo("empty-at", "    deploy_playbook: odd-name.yml\n    artifact_type: \"\"\n");
    let v = fastlane_dry(&r, "sha-new");
    let text = details(&v);
    let _ = std::fs::remove_dir_all(&r.dir);
    assert_eq!(v["exit"], 0, "an empty artifact_type is the default, as the script reads it: {text}");
}

#[test]
fn fastlane_refuses_a_digest_the_registry_does_not_hold_for_the_tag() {
    let r = fastlane_repo("digest", "    deploy_playbook: odd-name.yml\n");
    let v = fastlane_dry(&r, &format!("ghcr.io/x/app:sha-new@sha256:{}", "b".repeat(64)));
    let text = details(&v);
    let _ = std::fs::remove_dir_all(&r.dir);
    assert_eq!(v["exit"], 1, "{text}");
    assert!(text.contains("not sha256:bbbb"), "{text}");
}

#[test]
fn fastlane_refuses_a_component_in_an_open_set() {
    let r = fastlane_repo("set", "    deploy_playbook: odd-name.yml\n");
    std::fs::write(r.dir.join("ops/release/sets/s.yaml"), "name: together\nstate: open\nmembers:\n  - component: app\n").unwrap();
    let v = fastlane_dry(&r, "sha-new");
    let text = details(&v);
    let _ = std::fs::remove_dir_all(&r.dir);
    assert_eq!(v["exit"], 1, "{text}");
    assert!(text.contains("OPEN graduation set 'together'") && text.contains("graduate the set normally"), "{text}");
}

/// `completion zsh | head` closes the pipe early; that is the reader's right,
/// so the writer exits 0 with no panic (the 2341 walkthrough hit this).
#[test]
fn completion_survives_a_closed_pipe() {
    use std::process::Stdio;
    let mut child = test_home::isolate(&mut Command::new(bin()))
        .args(["completion", "zsh"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take()); // the reader goes away before the writer is done
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "exit {:?}: {err}", out.status);
    assert!(!err.contains("panicked"), "{err}");
}
