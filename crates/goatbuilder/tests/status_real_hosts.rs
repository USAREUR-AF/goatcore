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

#[path = "common/signed_intent.rs"]
mod signed_intent;

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
    if st.get("state") == "failed":
        print(h + ' | FAILED! => {"changed": false, "msg": "the read failed on the host"}')
        rc = 2
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
    world_with(serde_json::json!({
        "core-a": {"ledgers": {"portal": row(PORTAL), "herd": row(HERD)}},
        "core-b": {"ledgers": {"portal": row(PORTAL_OLD), "herd": row(HERD)}},
        "core-c": {"state": "unreachable"},
        "core-f": {"state": "silent"},
    }))
}

/// The same goatnet with every machine answering: converged, one behind on
/// portal, and two more converged. The three verbs reach all four.
fn reached_world() -> tempfile::TempDir {
    world_with(serde_json::json!({
        "core-a": {"ledgers": {"portal": row(PORTAL), "herd": row(HERD)}},
        "core-b": {"ledgers": {"portal": row(PORTAL_OLD), "herd": row(HERD)}},
        "core-c": {"ledgers": {"portal": row(PORTAL), "herd": row(HERD)}},
        "core-f": {"ledgers": {"portal": row(PORTAL), "herd": row(HERD)}},
    }))
}

fn world_with(hosts: serde_json::Value) -> tempfile::TempDir {
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
        format!("goatnets:\n  zagros:\n    members: [zagros_hosts]\ncomponents:\n  portal:\n    var: portal_image\n    fleet: \"{PORTAL}\"\n  herd:\n    var: herd_image\n    fleet: \"{HERD}\"\n"),
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
    // Deploy verifies the goatnet's signed intent before any host (GB-383).
    signed_intent::prepare(p, &["zagros_hosts"]);
    signed_intent::sign(p);
    // A dry run runs the fresh-source check (it touches no host); here it passes.
    test_support::executable(&p.join("ops/scripts/fresh-source.sh"), "#!/bin/sh\nexit 0\n");
    // The release-ladder gate deploy runs first; with no ladder.yaml, zagros is
    // not a live rung, and the dry run's document carries that pass row.
    std::fs::create_dir_all(p.join("ops/release")).unwrap();
    test_support::copied_executable(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/release/ladder_gate.py"),
        &p.join("ops/release/ladder_gate.py"),
    );
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

/// Gate D on real hosts: the `machines` list `status` and `verify` carry beside
/// their rows, graded the same way (tests/status_machines.rs holds its fields).
#[path = "common/machines.rs"]
mod shape;

#[test]
fn status_and_verify_on_real_hosts_list_each_machine_s_state_and_reason() {
    let w = world();
    let repo = w.path();
    let (code, doc, text) = run(repo, &["status", "--env", "zagros"]);
    assert_eq!(code, 3, "{text}");
    let want = |rows: &[(&str, &str, &str, &str)]| -> BTreeMap<(String, String), (String, String)> {
        rows.iter()
            .map(|(h, c, s, r)| {
                (
                    (h.to_string(), c.to_string()),
                    (s.to_string(), r.to_string()),
                )
            })
            .collect()
    };
    assert_eq!(
        shape::machines(&doc, &text),
        want(&[
            ("core-a", "portal", "converged", "declared"),
            ("core-a", "herd", "converged", "declared"),
            ("core-b", "portal", "pending", "other-ref"),
            ("core-b", "herd", "converged", "declared"),
            ("core-c", "portal", "unknown", "unreachable"),
            ("core-c", "herd", "unknown", "unreachable"),
            ("core-f", "portal", "unknown", "no-answer"),
            ("core-f", "herd", "unknown", "no-answer"),
        ]),
        "{text}"
    );
    let (code, doc, text) = run(repo, &["verify", "portal", "--env", "zagros"]);
    assert_eq!(code, 3, "{text}");
    assert_eq!(
        shape::machines(&doc, &text),
        want(&[
            ("core-a", "portal", "converged", "declared"),
            ("core-b", "portal", "pending", "other-ref"),
            ("core-c", "portal", "unknown", "unreachable"),
            ("core-f", "portal", "unknown", "no-answer"),
        ]),
        "{text}"
    );
}

#[test]
fn a_verify_that_reads_nothing_carries_an_empty_machines_list() {
    let w = world();
    let (_, doc, text) = run(
        w.path(),
        &["verify", "portal", "--env", "zagros", "--dry-run"],
    );
    assert_eq!(doc["verb"], "verify", "{text}");
    assert_eq!(
        doc["machines"],
        serde_json::json!([]),
        "a dry run reads no ledger, and says so with an empty list:\n{text}"
    );
}

/// The same condition gives the same reason on both verbs: a host that answered
/// and whose read FAILED is `unknown/unreadable` in `status` as in `verify`,
/// and a host that did not answer at all is `unknown/unreachable` in both.
#[test]
fn a_read_that_failed_is_unreadable_on_status_and_verify_alike() {
    let w = world();
    let repo = w.path();
    let path = repo.join("hosts.json");
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    doc["hosts"]["core-x"] = serde_json::json!({"state": "failed"});
    doc["inventory"]["zagros_hosts"]["hosts"]
        .as_array_mut()
        .unwrap()
        .push("core-x".into());
    std::fs::write(&path, doc.to_string()).unwrap();
    let (_, status, stext) = run(repo, &["status", "--env", "zagros"]);
    let (_, verify, vtext) = run(repo, &["verify", "portal", "--env", "zagros"]);
    let by_status = shape::machines(&status, &stext);
    let by_verify = shape::machines(&verify, &vtext);
    for (host, want) in [("core-x", "unreadable"), ("core-c", "unreachable")] {
        let k = (host.to_string(), "portal".to_string());
        assert_eq!(
            by_status[&k],
            ("unknown".to_string(), want.to_string()),
            "status, {host}:\n{stext}"
        );
        assert_eq!(
            by_verify[&k], by_status[&k],
            "verify and status disagree on {host}:\n{vtext}"
        );
    }
}

/// The real-host documents, held byte for byte across the move of the push
/// converger behind the converger trait (gate D): `status`, `verify` (a read and
/// a dry run) and `deploy --dry-run` on the stand-in world must print exactly
/// what they printed before it. `GOATBUILDER_GOLDEN=overwrite` records them.
#[test]
fn the_real_host_documents_are_unchanged_by_the_converger_trait() {
    let w = world();
    let repo = w.path();
    let mut got = serde_json::Map::new();
    let stamp = regex::Regex::new(r"\d{8}T\d{6}Z").unwrap();
    let openssl = regex::Regex::new(r#""[^"]*/openssl dgst"#).unwrap();
    for (name, args) in [
        ("status", vec!["status", "--env", "zagros"]),
        ("verify-portal", vec!["verify", "portal", "--env", "zagros"]),
        (
            "verify-portal-dry-run",
            vec!["verify", "portal", "--env", "zagros", "--dry-run"],
        ),
        (
            "deploy-portal-dry-run",
            vec![
                "deploy",
                "--env",
                "zagros",
                "--component",
                "portal",
                "--dry-run",
            ],
        ),
    ] {
        let (code, doc, _) = run(repo, &args);
        // Paths and times differ per run; the document's shape and words do not.
        let text = doc.to_string().replace(repo.to_str().unwrap(), "<repo>");
        let text = stamp.replace_all(&text, "<stamp>").into_owned();
        // openssl is found on PATH, in another directory on each system.
        let text = openssl.replace_all(&text, "\"openssl dgst").into_owned();
        // The deploy log goes to $TMPDIR when it is set (push.rs), which macOS sets
        // to a per-user directory; the document records it as /tmp.
        let text = match std::env::var("TMPDIR") {
            Ok(t) if !t.is_empty() => text.replace(&format!("{t}/goat-deploy-"), "/tmp/goat-deploy-"),
            _ => text,
        };
        got.insert(name.into(), serde_json::json!({"exit": code, "document": serde_json::from_str::<serde_json::Value>(&text).unwrap()}));
    }
    let got = serde_json::to_string_pretty(&serde_json::Value::Object(got)).unwrap() + "\n";
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/real-host-documents.json");
    if std::env::var("GOATBUILDER_GOLDEN").as_deref() == Ok("overwrite") {
        std::fs::write(&path, &got).unwrap();
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        got, want,
        "the real-host documents changed; if on purpose, GOATBUILDER_GOLDEN=overwrite"
    );
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
if mode == "door" and os.environ.get("C3_DOOR_STATE"):
    # The verbs' transport opens the goatnet's door, as push::transport does
    # when the laptop is not on the mesh.
    open(os.environ["C3_DOOR_STATE"], "w").write("open")
if mode == "crash" and verb == "diff":
    # A row c3-agreement.py cannot read at all: it fails with a traceback.
    doc = json.loads(out)
    doc["checks"].append({"name": "zagros/core-a/portal"})
    out = json.dumps(doc)
if mode == "hang" and verb == "verify" and "portal" in args:
    import time
    time.sleep(120)
if mode == "secret" and verb == "doctor":
    # A live secret with no known shape, from the operator's environment, in a
    # row and on stderr.
    doc = json.loads(out) if out.strip().startswith("{") else {"verb": "doctor", "checks": []}
    doc.setdefault("checks", []).append(
        {"name": "env:zagros:transport", "detail": "door key " + os.environ["C3_PORTAL_KEY"]})
    out = json.dumps(doc)
    p = subprocess.CompletedProcess(p.args, p.returncode, p.stdout, p.stderr + "auth with " + os.environ["C3_PORTAL_KEY"] + "\n")
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
    measure_with(repo, wrap, out, tmp, &[], false)
}

/// [`measure`] with more environment, and with the door left to the script
/// (`door`: GOAT_DOOR_OPEN unset) instead of pinned shut.
fn measure_with(
    repo: &Path,
    wrap: &str,
    out: Option<&Path>,
    tmp: &Path,
    env: &[(&str, &str)],
    door: bool,
) -> Measured {
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
        .env("GOAT_EXPLAIN", "0");
    if door {
        cmd.env_remove("GOAT_DOOR_OPEN");
    } else {
        cmd.env("GOAT_DOOR_OPEN", "0");
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
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
    let w = reached_world();
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("rec");
    let m = measure(w.path(), "", Some(&out), tmp.path());
    assert_eq!(m.code, 0, "the three verbs agree on the fixture:\n{}\n{}", m.stdout, m.stderr);
    let s = summary(&m);
    assert!(s.contains("AGREE"), "{s}");
    assert!(s.contains("reached 4 of 4 machine(s)"), "{s}");
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
    assert_eq!(rec["reach"], "reached 4 of 4 machine(s)", "{rec}");
    for v in ["status", "diff", "verify:portal", "verify:herd"] {
        assert!(rec["times_s"][v].is_number(), "the wall time of {v}: {rec}");
    }
}

#[test]
fn c3_measure_without_out_makes_a_private_directory_of_its_own() {
    let w = reached_world();
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
    let w = reached_world();
    let tmp = tempfile::tempdir().unwrap();
    let m = measure(w.path(), "disagree", Some(&tmp.path().join("rec")), tmp.path());
    assert_eq!(m.code, 3, "a disagreement fails C3:\n{}\n{}", m.stdout, m.stderr);
    let s = summary(&m);
    assert!(s.contains("DISAGREE"), "{s}");
}

#[test]
fn c3_measure_says_a_failed_read_is_not_a_measurement() {
    let w = reached_world();
    let tmp = tempfile::tempdir().unwrap();
    let m = measure(w.path(), "unread", Some(&tmp.path().join("rec")), tmp.path());
    assert_eq!(m.code, 2, "a verify that read no machine is not a measurement:\n{}\n{}", m.stdout, m.stderr);
    let s = summary(&m);
    assert!(s.contains("NOT A MEASUREMENT"), "{s}");
}

#[test]
fn c3_measure_keeps_no_token_shaped_value_in_the_record() {
    let w = reached_world();
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

#[test]
fn c3_measure_says_a_partial_reach_is_not_a_measurement() {
    // core-c is unreachable and core-f silent under all three verbs: they agree,
    // but C3 is about every machine, so two of four reached is not a measurement.
    let w = world();
    let tmp = tempfile::tempdir().unwrap();
    let m = measure(w.path(), "", Some(&tmp.path().join("rec")), tmp.path());
    assert_eq!(m.code, 2, "a partial reach is not a measurement:\n{}\n{}", m.stdout, m.stderr);
    let s = summary(&m);
    assert!(s.contains("NOT A MEASUREMENT: reached 2 of 4 machine(s)"), "{s}");
    assert!(!s.contains("AGREE"), "{s}");
}

/// A stand-in `ops/scripts/door-open.sh`: the door's state in `$C3_DOOR_STATE`,
/// each call appended to `$C3_DOOR_STATE.log`.
const DOOR: &str = r#"#!/bin/sh
s="$C3_DOOR_STATE"
echo "$*" >> "$s.log"
case "$2" in
  --status) if [ "$(cat "$s" 2>/dev/null)" = open ]; then echo "door $1: OPEN"; exit 0; fi
            echo "door $1: closed"; exit 1 ;;
  --close) echo closed > "$s"; echo "door $1: closed"; exit 0 ;;
  *) echo open > "$s"; exit 0 ;;
esac
"#;

fn door_run(before: &str) -> (Measured, String, String) {
    let w = reached_world();
    test_support::executable(&w.path().join("ops/scripts/door-open.sh"), DOOR);
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("door-state");
    std::fs::write(&state, before).unwrap();
    let st = state.to_str().unwrap().to_string();
    let m = measure_with(w.path(), "door", Some(&tmp.path().join("rec")), tmp.path(), &[("C3_DOOR_STATE", &st)], true);
    let after = std::fs::read_to_string(&state).unwrap();
    let log = std::fs::read_to_string(format!("{st}.log")).unwrap_or_default();
    (m, after.trim().to_string(), log)
}

#[test]
fn c3_measure_closes_a_door_its_own_run_opened() {
    let (m, after, log) = door_run("closed");
    assert_eq!(m.code, 0, "{}\n{}", m.stdout, m.stderr);
    assert_eq!(after, "closed", "the door the run opened is closed again:\n{log}");
    assert!(log.lines().any(|l| l == "zagros --close"), "{log}");
    assert!(log.lines().next() == Some("zagros --status"), "the state is read before the verbs run: {log}");
}

#[test]
fn c3_measure_leaves_a_door_that_was_open_before_it() {
    let (m, after, log) = door_run("open");
    assert_eq!(m.code, 0, "{}\n{}", m.stdout, m.stderr);
    assert_eq!(after, "open", "a door the operator opened stays open:\n{log}");
    assert!(!log.contains("--close"), "{log}");
}

#[test]
fn c3_measure_says_a_failed_agreement_script_is_not_a_measurement() {
    let w = reached_world();
    let tmp = tempfile::tempdir().unwrap();
    let m = measure(w.path(), "crash", Some(&tmp.path().join("rec")), tmp.path());
    assert_eq!(m.code, 2, "a crash of c3-agreement.py is not a measurement, never exit 1:\n{}\n{}", m.stdout, m.stderr);
    let s = summary(&m);
    assert!(s.contains("NOT A MEASUREMENT"), "{s}");
}

#[test]
fn c3_measure_bounds_each_verb_and_a_timeout_is_not_a_measurement() {
    let w = reached_world();
    let tmp = tempfile::tempdir().unwrap();
    let t0 = std::time::Instant::now();
    let m = measure_with(w.path(), "hang", Some(&tmp.path().join("rec")), tmp.path(), &[("C3_VERB_TIMEOUT", "3")], false);
    assert!(t0.elapsed().as_secs() < 60, "the hung verb was not stopped: {:?}", t0.elapsed());
    assert_eq!(m.code, 2, "a verb that timed out is not a measurement:\n{}\n{}", m.stdout, m.stderr);
    let s = summary(&m);
    assert!(s.contains("NOT A MEASUREMENT"), "{s}");
    assert!(m.stderr.contains("timed out after 3s"), "{}", m.stderr);
}

#[test]
fn c3_measure_keeps_no_live_secret_in_the_record_or_the_kept_stderr() {
    // A value of a secret-named variable in the operator's environment, with no
    // shape a pattern knows, built at run time so no file holds it.
    let secret: String = "Qz7".repeat(14);
    let w = reached_world();
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("rec");
    let m = measure_with(w.path(), "secret", Some(&out), tmp.path(), &[("C3_PORTAL_KEY", &secret)], false);
    assert_eq!(m.code, 0, "{}\n{}", m.stdout, m.stderr);
    summary(&m);
    let rec = std::fs::read_to_string(out.join("record.json")).unwrap();
    assert!(rec.contains("door key"), "the doctor row reached the record (redacted): {rec}");
    assert!(!rec.contains(&secret), "record.json holds the live secret: {rec}");
    assert!(!m.stdout.contains(&secret) && !m.stderr.contains(&secret), "the run printed the live secret");
    for e in std::fs::read_dir(&out).unwrap().flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("stderr") {
            let t = std::fs::read_to_string(&p).unwrap();
            assert!(!t.contains(&secret), "{} keeps the live secret", p.display());
        }
    }
}
