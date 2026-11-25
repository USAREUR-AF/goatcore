//! The network tap type (docs/design/goatbuilder-build-targets.md section
//! 3.8), through the built binary: a machine states its mode, listen or
//! inline, with no default; inline is shown "inline, fail-closed"; the type
//! requires an encrypted root; and nothing installs the tap itself yet.
//! Nothing here reaches a host: every run is a dry run or reads the checkout.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// Goatnet `zagros`, one founder site with three taps, in a checkout with no
/// type files of its own.
fn checkout() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        "ops/goatbuilder/goatbuilder.yaml",
        "schema: goatbuilder/v1\n\
         repo: {source: example-org/example-repo, image_namespace: ghcr.io/example-org}\n\
         components:\n  netbird-mgmt: {declared_in: ops/pins/fleet-pins.yaml}\n\
         environments:\n  - name: zagros\n    converger: push-ansible\n\
         promotion:\n  path:\n    - {name: zagros, kind: rung, environment: zagros}\n\
         deploy: {ssh_user: operator}\n",
    );
    write(
        t.path(),
        "ops/ansible/group_vars/zagros_hosts/vars.yml",
        "goatnet_founder: true\n",
    );
    write(
        t.path(),
        "ops/ansible/inventory/mgmt-hosts.ini",
        "[zagros_hosts]\nmgmt-zagros-01-a\n[zagros_network_tap_hosts]\ntap-1\ntap-2\ntap-3\n",
    );
    write(t.path(), "ops/ansible/host_vars/tap-1.yml", "platform_family: small-computer\ntap:\n  mode: inline\n  ports: [eth1, eth2]\n  management: eth0\n");
    write(
        t.path(),
        "ops/ansible/host_vars/tap-2.yml",
        "tap:\n  mode: listen\n  source: copper-tap\n  ports: [eth1, eth2]\n  management: eth0\n",
    );
    let scripts = t.path().join("ops/scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts/inventory-hosts.py"),
        scripts.join("inventory-hosts.py"),
    )
    .unwrap();
    t
}

/// The exit code, stdout and stderr of one run.
fn run(root: &Path, args: &[&str]) -> (i32, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["-o", "json", "--yes", args[0], "-C", root.to_str().unwrap()])
        .args(&args[1..])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", root.join("home"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("GOAT_MESH_PROBE", "0")
        .output()
        .expect("run goatbuilder");
    (
        o.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
    )
}

fn join(node: &str) -> Vec<&str> {
    vec![
        "join-node",
        "zagros",
        "--site",
        "zagros",
        "--node",
        node,
        "--class",
        "network-tap",
        "--address",
        "192.0.2.9",
        "--dry-run",
    ]
}

fn rows(out: &str) -> Vec<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(out).unwrap_or_else(|e| panic!("{e}: {out}"));
    v["checks"].as_array().unwrap().clone()
}

fn row<'a>(rows: &'a [serde_json::Value], name: &str) -> &'a serde_json::Value {
    rows.iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("no row {name}: {rows:?}"))
}

#[test]
fn join_node_shows_the_mode_and_installs_nothing_of_the_tap() {
    let t = checkout();
    let (rc, out) = run(t.path(), &join("tap-1"));
    assert_eq!(rc, 0, "{out}");
    assert!(out.contains("tap-1: inline, fail-closed: the machine bridges the link, so the link is down whenever the machine is off, restarting or broken (ports eth1, eth2; management eth0) (ops/ansible/host_vars/tap-1.yml)"), "{out}");
    assert!(out.contains("goatrope has no release yet: nothing is installed, and this machine is not a working tap"), "phase 13 says the tap is not installed: {out}");
    let (rc, out) = run(t.path(), &join("tap-2"));
    assert!(
        rc == 0 && out.contains("tap-2: listen from a copper-tap, fail-open: "),
        "{out}"
    );
}

#[test]
fn a_tap_with_no_mode_is_refused_before_any_phase() {
    let t = checkout();
    let (rc, out) = run(t.path(), &join("tap-3"));
    assert!(
        rc != 0 && out.contains("GB-380") && out.contains("there is no default"),
        "{out}"
    );
    assert!(!out.contains("Phase 1"), "refused before any phase: {out}");
    write(
        t.path(),
        "ops/ansible/host_vars/tap-3.yml",
        "tap:\n  mode: inline\n  source: mirror-port\n  ports: [eth1, eth2]\n  management: eth0\n",
    );
    let (rc, out) = run(t.path(), &join("tap-3"));
    assert!(
        rc != 0
            && out.contains("GB-380")
            && out.contains("an inline tap carries the link itself, so it has no source"),
        "{out}"
    );
}

#[test]
fn doctor_target_reads_each_taps_mode_and_names_the_encryption_it_cannot_see() {
    let t = checkout();
    let (rc, out) = run(
        t.path(),
        &["doctor", "--target", "network-tap", "--site", "zagros"],
    );
    let r = rows(&out);
    assert_eq!(rc, 1, "tap-3 declares no mode, so a row fails: {out}");
    assert!(
        row(&r, "tap tap-1")["detail"]
            .as_str()
            .unwrap()
            .starts_with("inline, fail-closed: "),
        "{out}"
    );
    assert!(
        row(&r, "tap tap-2")["detail"]
            .as_str()
            .unwrap()
            .starts_with("listen from a copper-tap, fail-open: "),
        "{out}"
    );
    let bad = row(&r, "tap tap-3");
    assert!(
        bad["ok"] == false
            && bad["detail"]
                .as_str()
                .unwrap()
                .contains("there is no default"),
        "{bad}"
    );
    // The encryption the type requires is not read here: UNKNOWN, never a pass.
    let enc = row(&r, "encryption");
    assert!(
        enc["ok"] == false
            && enc.get("skipped").is_none()
            && enc["detail"]
                .as_str()
                .unwrap()
                .starts_with("UNKNOWN — network-tap requires an encrypted root"),
        "{enc}"
    );
    assert!(
        row(&r, "plays")["ok"] == true,
        "the type has no play to be missing: {out}"
    );
}
