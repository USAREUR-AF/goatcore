//! Gate row C3 on the command line: `goatbuilder status` without `--mock` reads
//! real hosts, and agrees with `verify` and `diff` for every machine.
//!
//! The library's own agreement test (`status_agreement_tests` in
//! `src/inspect.rs`) calls `inspect::status` directly. This one runs the built
//! binary, so it also proves the `status` arm in `main.rs` reaches it: a
//! checkout whose stand-in `ops/scripts/with-aws` answers `ansible` and
//! `ansible-inventory` from one table of machines, and `status`, `diff` and
//! `verify` each run with `-o json`, as an operator scripts them.

#[path = "common/test_home.rs"]
mod test_home;
#[path = "../src/test_support.rs"]
mod test_support;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

const PORTAL: &str = "ghcr.io/zagros/portal:v1.3.2@sha256:9f8e00000000000000000000000000000000000000000000000000000000aa01";
const PORTAL_OLD: &str = "ghcr.io/zagros/portal:v1.3.1@sha256:1d2c00000000000000000000000000000000000000000000000000000000bb02";
const HERD: &str = "ghcr.io/zagros/herd:v0.2.1@sha256:4b5a00000000000000000000000000000000000000000000000000000000cc03";

/// Answers the reads the verbs make: a READ_HOST shell per group (diff, status),
/// a `tail -n 1` of one ledger per group (verify), and `ansible-inventory --list`.
const STUB: &str = r#"#!/usr/bin/env python3
import json, os, sys
root = os.path.abspath(os.path.join(os.getcwd(), "..", ".."))
argv = sys.argv[1:]
tool, args = argv[2], argv[3:]
world = json.load(open(os.path.join(root, "hosts.json")))
if tool == "ansible-inventory":
    print(json.dumps(world["inventory"]))
    sys.exit(0)
if tool != "ansible":
    sys.exit(9)
i, group, cmd = 0, None, ""
while i < len(args):
    a = args[i]
    if a in ("-i", "-m", "-a", "-e", "-l"):
        if a == "-a":
            cmd = args[i + 1]
        i += 2
        continue
    if a.startswith("-"):
        i += 1
        continue
    if group is None:
        group = a
    i += 1
rc = 0
for h in world["inventory"].get(group, {}).get("hosts", []):
    st = world["hosts"].get(h, {})
    if st.get("state") == "silent":
        continue
    if st.get("state") == "unreachable":
        print(h + ' | UNREACHABLE! => {"changed": false, "unreachable": true}')
        rc = 4
        continue
    ledgers = st.get("ledgers", {})
    if cmd.startswith("tail -n 1"):
        ledger = cmd.split("/var/lib/goat/")[1].split("/")[0]
        row = ledgers.get(ledger)
        out = json.dumps(row, separators=(",", ":")) if row is not None else "NO_LEDGER"
        print(h + " | CHANGED | rc=0 | (stdout) " + out)
    else:
        print(h + " | CHANGED | rc=0 >>")
        for l, row in ledgers.items():
            print("GBROW " + l + " " + json.dumps(row))
        print("GBEND")
sys.exit(rc)
"#;

fn row(r: &str) -> serde_json::Value {
    serde_json::json!({"ref": r, "action": "deploy", "ts": "2026-11-01T00:00:00Z", "prior_ref": ""})
}

/// One goatnet, one member group, two components, four machines: converged,
/// one release behind on portal, unreachable, and silent.
fn world() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    for dir in [
        "ops/pins",
        "ops/scripts",
        "ops/ansible/inventory",
        "ops/ansible/group_vars/zagros_hosts",
    ] {
        std::fs::create_dir_all(p.join(dir)).unwrap();
    }
    std::fs::write(
        p.join("ops/pins/fleet-pins.yaml"),
        "goatnets:\n  zagros:\n    members: [zagros_hosts]\ncomponents:\n  portal:\n    var: portal_image\n  herd:\n    var: herd_image\n",
    )
    .unwrap();
    std::fs::write(
        p.join("ops/ansible/group_vars/zagros_hosts/pins.yml"),
        format!("portal_image: \"{PORTAL}\"\nherd_image: \"{HERD}\"\n"),
    )
    .unwrap();
    for c in ["portal", "herd"] {
        std::fs::write(
            p.join(format!("ops/ansible/{c}-deploy.yml")),
            "- name: deploy\n  hosts: \"{{ target }}\"\n",
        )
        .unwrap();
    }
    std::fs::write(
        p.join("ops/ansible/inventory/mgmt-hosts.ini"),
        "[zagros_hosts]\n",
    )
    .unwrap();
    test_support::copied_executable(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts/served-payloads.py"),
        &p.join("ops/scripts/served-payloads.py"),
    );
    test_support::executable(&p.join("ops/scripts/with-aws"), STUB);
    let hosts = serde_json::json!({
        "core-a": {"ledgers": {"portal": row(PORTAL), "herd": row(HERD)}},
        "core-b": {"ledgers": {"portal": row(PORTAL_OLD), "herd": row(HERD)}},
        "core-c": {"state": "unreachable"},
        "core-f": {"state": "silent"},
    });
    let all = ["core-a", "core-b", "core-c", "core-f"];
    let w = serde_json::json!({"inventory": {"zagros_hosts": {"hosts": all}, "_meta": {"hostvars": {}}}, "hosts": hosts});
    std::fs::write(p.join("hosts.json"), w.to_string()).unwrap();
    let git = |args: &[&str]| {
        let st = Command::new("git")
            .args(args)
            .current_dir(p)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    d
}

/// A machine's verdict for one component: at the declared ref, read and not at
/// it, or not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum V {
    Healthy,
    Drifted,
    Unread,
}

fn run(repo: &Path, args: &[&str]) -> (i32, serde_json::Value, String) {
    let o = test_home::isolate(&mut Command::new(env!("CARGO_BIN_EXE_goatbuilder")))
        .args(["-o", "json", "-C", repo.to_str().unwrap()])
        .args(args)
        .env("GOAT_MESH_PROBE", "0")
        .env("GOAT_DOOR_OPEN", "0")
        .env("GOAT_EXPLAIN", "0")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let doc = serde_json::from_slice(&o.stdout).unwrap_or(serde_json::Value::Null);
    (o.status.code().unwrap_or(-1), doc, text)
}

fn checks(doc: &serde_json::Value) -> Vec<(String, String)> {
    doc["checks"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| {
                    (
                        c["name"].as_str().unwrap_or("").to_string(),
                        c["detail"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `diff`'s or `status`'s rows, `zagros/<host>/<component>`.
fn from_rows(doc: &serde_json::Value) -> BTreeMap<(String, String), V> {
    let mut out = BTreeMap::new();
    for (name, d) in checks(doc) {
        let parts: Vec<&str> = name.split('/').collect();
        if parts.len() != 3 || parts[2] == "inventory" {
            continue;
        }
        let v = if d.starts_with("same: ") || d.starts_with("converged on ") {
            V::Healthy
        } else if d.ends_with(": host unreachable")
            || d.contains(": no answer from this host")
            || d.starts_with("unknown: host unreachable")
            || d.starts_with("unknown: no answer")
        {
            V::Unread
        } else if d.starts_with("DRIFT") || d.starts_with("pending: ") || d.starts_with("unknown: ")
        {
            V::Drifted
        } else {
            panic!("row with no verdict: {name}: {d}")
        };
        out.insert((parts[1].to_string(), parts[2].to_string()), v);
    }
    out
}

/// `verify`'s per-host rows, `zagros/<host>`, for one component.
fn from_verify(doc: &serde_json::Value, component: &str, into: &mut BTreeMap<(String, String), V>) {
    for (name, d) in checks(doc) {
        let Some(host) = name
            .strip_prefix("zagros/")
            .filter(|h| h.starts_with("core-"))
        else {
            continue;
        };
        let v = if d.starts_with("converged on") {
            V::Healthy
        } else if d.starts_with("unknown — ") || d.contains("no ledger row came back") {
            V::Unread
        } else if d.starts_with("NOT converged") {
            V::Drifted
        } else {
            panic!("verify row with no verdict: {name}: {d}")
        };
        into.insert((host.to_string(), component.to_string()), v);
    }
}

#[test]
fn status_on_real_hosts_agrees_with_verify_and_diff_for_every_machine() {
    let w = world();
    let repo = w.path();
    let (code, status, text) = run(repo, &["status", "--env", "zagros"]);
    assert!(
        !text.contains("GB-026"),
        "status without --mock still refuses with GB-026:\n{text}"
    );
    assert_eq!(
        code, 3,
        "status exits 3 while a machine is behind or unread:\n{text}"
    );
    assert_eq!(status["verb"], "status", "{text}");
    let by_status = from_rows(&status);

    let (_, diff, dtext) = run(repo, &["diff", "--env", "zagros"]);
    let by_diff = from_rows(&diff);
    let mut by_verify = BTreeMap::new();
    for c in ["portal", "herd"] {
        let (_, v, _) = run(repo, &["verify", c, "--env", "zagros"]);
        from_verify(&v, c, &mut by_verify);
    }

    let want: BTreeMap<(String, String), V> = [
        ("core-a", "portal", V::Healthy),
        ("core-a", "herd", V::Healthy),
        ("core-b", "portal", V::Drifted),
        ("core-b", "herd", V::Healthy),
        ("core-c", "portal", V::Unread),
        ("core-c", "herd", V::Unread),
        ("core-f", "portal", V::Unread),
        ("core-f", "herd", V::Unread),
    ]
    .into_iter()
    .map(|(h, c, v)| ((h.to_string(), c.to_string()), v))
    .collect();
    assert_eq!(by_status, want, "status, every machine:\n{text}");
    assert_eq!(by_diff, want, "diff, every machine:\n{dtext}");
    assert_eq!(by_verify, want, "verify, every machine");
}

// --- The operator's C3 run as one command: ops/goatbuilder/c3-measure.sh ---
//
// The script runs doctor, status, diff and one verify per component, then
// ops/goatbuilder/c3-agreement.py, times each verb, keeps the reports and a
// record.json in a private directory, and prints one summary line that is safe
// for the gate row. These tests run it on the same fixture, with the built
// binary, and a wrapper that can change one verb's report where a case needs it.

const WRAPPER: &str = r#"#!/usr/bin/env python3
import json, os, subprocess, sys
args = sys.argv[1:]
mode = os.environ.get("C3_WRAP", "")
p = subprocess.run([os.environ["C3_REAL_GOATBUILDER"]] + args, capture_output=True, text=True)
out = p.stdout
verb = next((a for a in args if a in ("doctor", "status", "diff", "verify")), "")
if mode == "disagree" and verb == "verify" and "portal" in args:
    doc = json.loads(out)
    for c in doc["checks"]:
        if c["name"].endswith("/core-a") and c["detail"].startswith("converged on"):
            c["detail"] = "NOT converged: changed by the test"
    out = json.dumps(doc)
if mode == "unread" and verb == "verify":
    out = json.dumps({"verb": "verify", "checks": [{"name": "zagros/transport", "detail": "read failed"}]})
if mode == "token" and verb == "doctor":
    doc = json.loads(out) if out.strip().startswith("{") else {"verb": "doctor", "checks": []}
    doc.setdefault("checks", []).append(
        {"name": "env:zagros:transport", "detail": "door with " + os.environ["C3_TOKEN"]})
    out = json.dumps(doc)
sys.stdout.write(out)
sys.stderr.write(p.stderr)
sys.exit(p.returncode)
"#;

/// A token-shaped value, built at run time so no file holds one.
fn token() -> String {
    format!("gh{}_{}", "p", "A1b2C3d4".repeat(5).chars().take(36).collect::<String>())
}

fn token_shaped(text: &str) -> bool {
    regex::Regex::new(r"(gh[opsu]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{20,}|AKIA[0-9A-Z]{16})")
        .unwrap()
        .is_match(text)
}

struct Measured {
    code: i32,
    stdout: String,
    stderr: String,
}

fn measure(repo: &Path, wrap: &str, out: Option<&Path>, tmp: &Path) -> Measured {
    let wrapper = repo.join("c3-test-goatbuilder");
    test_support::executable(&wrapper, WRAPPER);
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/c3-measure.sh");
    let mut cmd = Command::new("bash");
    test_home::isolate(&mut cmd)
        .arg(&script)
        .args(["--env", "zagros", "-C", repo.to_str().unwrap()])
        .current_dir(repo)
        .env("GOATBUILDER", &wrapper)
        .env("C3_REAL_GOATBUILDER", env!("CARGO_BIN_EXE_goatbuilder"))
        .env("C3_WRAP", wrap)
        .env("C3_TOKEN", token())
        .env("GITHUB_TOKEN", token())
        .env("TMPDIR", tmp)
        .env("GOAT_MESH_PROBE", "0")
        .env("GOAT_DOOR_OPEN", "0")
        .env("GOAT_EXPLAIN", "0");
    if let Some(o) = out {
        cmd.args(["--out", o.to_str().unwrap()]);
    }
    let o = cmd.output().unwrap();
    Measured {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

/// The one summary line, with every name of the fixture's goatnet checked absent.
fn summary(m: &Measured) -> String {
    let lines: Vec<&str> = m.stdout.lines().filter(|l| l.starts_with("c3 <env>: ")).collect();
    assert_eq!(lines.len(), 1, "exactly one summary line:\n{}\n{}", m.stdout, m.stderr);
    let s = lines[0].to_string();
    for name in ["zagros", "core-a", "core-b", "core-c", "core-f", "zagros_hosts"] {
        assert!(!s.contains(name), "the summary line names {name:?}: {s}");
    }
    assert!(!token_shaped(&s), "a token-shaped value in the summary line: {s}");
    eprintln!("summary: {s}");
    s
}

#[test]
fn c3_measure_keeps_a_private_record_and_prints_one_line_with_no_names() {
    let w = world();
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("rec");
    let m = measure(w.path(), "", Some(&out), tmp.path());
    assert_eq!(m.code, 0, "the three verbs agree on the fixture:\n{}\n{}", m.stdout, m.stderr);
    let s = summary(&m);
    assert!(s.contains("AGREE"), "{s}");
    assert!(s.contains("reached 2 of 4 machine(s)"), "{s}");
    assert!(s.contains("2 component(s)"), "{s}");
    assert!(s.contains("goatbuilder 0."), "the version is in the line: {s}");
    for f in ["status.json", "diff.json", "verify-portal.json", "verify-herd.json", "record.json"] {
        assert!(out.join(f).is_file(), "{f} kept in the private directory");
    }
    let rec: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("record.json")).unwrap()).unwrap();
    for k in ["build", "transport", "reach", "result", "times_s", "reports", "summary"] {
        assert!(!rec[k].is_null(), "record.json keeps {k}: {rec}");
    }
    assert_eq!(rec["result"]["exit"], 0, "{rec}");
    assert_eq!(rec["reach"], "reached 2 of 4 machine(s)", "{rec}");
    for v in ["status", "diff", "verify:portal", "verify:herd"] {
        assert!(rec["times_s"][v].is_number(), "the wall time of {v}: {rec}");
    }
}

#[test]
fn c3_measure_without_out_makes_a_private_directory_of_its_own() {
    let w = world();
    let tmp = tempfile::tempdir().unwrap();
    let m = measure(w.path(), "", None, tmp.path());
    assert_eq!(m.code, 0, "{}\n{}", m.stdout, m.stderr);
    summary(&m);
    let made: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("record.json").is_file())
        .collect();
    assert_eq!(made.len(), 1, "one directory under TMPDIR holds the record");
    let dir = made[0].path();
    assert!(m.stderr.contains(dir.to_str().unwrap()), "the script names where it kept the record:\n{}", m.stderr);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "the private directory is the operator's alone");
    }
}

#[test]
fn c3_measure_fails_on_a_disagreement() {
    let w = world();
    let tmp = tempfile::tempdir().unwrap();
    let m = measure(w.path(), "disagree", Some(&tmp.path().join("rec")), tmp.path());
    assert_eq!(m.code, 3, "a disagreement fails C3:\n{}\n{}", m.stdout, m.stderr);
    let s = summary(&m);
    assert!(s.contains("DISAGREE"), "{s}");
}

#[test]
fn c3_measure_says_a_failed_read_is_not_a_measurement() {
    let w = world();
    let tmp = tempfile::tempdir().unwrap();
    let m = measure(w.path(), "unread", Some(&tmp.path().join("rec")), tmp.path());
    assert_eq!(m.code, 2, "a verify that read no machine is not a measurement:\n{}\n{}", m.stdout, m.stderr);
    let s = summary(&m);
    assert!(s.contains("NOT A MEASUREMENT"), "{s}");
}

#[test]
fn c3_measure_keeps_no_token_shaped_value_in_the_record() {
    let w = world();
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("rec");
    let m = measure(w.path(), "token", Some(&out), tmp.path());
    assert_eq!(m.code, 0, "{}\n{}", m.stdout, m.stderr);
    summary(&m);
    let rec = std::fs::read_to_string(out.join("record.json")).unwrap();
    assert!(rec.contains("door with"), "the doctor row reached the record (redacted): {rec}");
    assert!(!token_shaped(&rec), "record.json holds a token-shaped value: {rec}");
    assert!(!token_shaped(&m.stdout) && !token_shaped(&m.stderr), "the run printed a token-shaped value");
}
