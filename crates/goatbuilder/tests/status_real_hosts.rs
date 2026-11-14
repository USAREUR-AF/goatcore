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
    for dir in ["ops/pins", "ops/scripts", "ops/ansible/inventory", "ops/ansible/group_vars/zagros_hosts"] {
        std::fs::create_dir_all(p.join(dir)).unwrap();
    }
    std::fs::write(
        p.join("ops/pins/fleet-pins.yaml"),
        "goatnets:\n  zagros:\n    members: [zagros_hosts]\ncomponents:\n  portal:\n    var: portal_image\n  herd:\n    var: herd_image\n",
    )
    .unwrap();
    std::fs::write(p.join("ops/ansible/group_vars/zagros_hosts/pins.yml"), format!("portal_image: \"{PORTAL}\"\nherd_image: \"{HERD}\"\n")).unwrap();
    for c in ["portal", "herd"] {
        std::fs::write(p.join(format!("ops/ansible/{c}-deploy.yml")), "- name: deploy\n  hosts: \"{{ target }}\"\n").unwrap();
    }
    std::fs::write(p.join("ops/ansible/inventory/mgmt-hosts.ini"), "[zagros_hosts]\n").unwrap();
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
        let st = Command::new("git").args(args).current_dir(p).env("GIT_CONFIG_GLOBAL", "/dev/null").status().unwrap();
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
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let doc = serde_json::from_slice(&o.stdout).unwrap_or(serde_json::Value::Null);
    (o.status.code().unwrap_or(-1), doc, text)
}

fn checks(doc: &serde_json::Value) -> Vec<(String, String)> {
    doc["checks"]
        .as_array()
        .map(|a| a.iter().map(|c| (c["name"].as_str().unwrap_or("").to_string(), c["detail"].as_str().unwrap_or("").to_string())).collect())
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
        } else if d.ends_with(": host unreachable") || d.contains(": no answer from this host") || d.starts_with("unknown: host unreachable") || d.starts_with("unknown: no answer") {
            V::Unread
        } else if d.starts_with("DRIFT") || d.starts_with("pending: ") || d.starts_with("unknown: ") {
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
        let Some(host) = name.strip_prefix("zagros/").filter(|h| h.starts_with("core-")) else { continue };
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
    assert!(!text.contains("GB-026"), "status without --mock still refuses with GB-026:\n{text}");
    assert_eq!(code, 3, "status exits 3 while a machine is behind or unread:\n{text}");
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
