//! The simple-boundary type (gate J10, ADR 1253) through the built binary:
//! `join-node` plans the machine and runs `simple-bn.yml` on it in phase 13,
//! the families it accepts are held, `doctor --target` reads it, and `standup`
//! builds none. Every run is a dry run or reads the checkout only.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn checkout(family: &str) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    write(t.path(), "ops/goatbuilder/goatbuilder.yaml", "schema: goatbuilder/v1\nrepo: {source: example-org/example-repo, image_namespace: ghcr.io/example-org}\ncomponents:\n  netbird-mgmt: {declared_in: ops/pins/fleet-pins.yaml}\nenvironments:\n  - name: zagros\n    converger: push-ansible\npromotion:\n  path:\n    - {name: zagros, kind: rung, environment: zagros}\ndeploy: {ssh_user: goat}\n");
    write(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\n");
    write(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nmgmt-zagros-01-a\n[zagros_simple_boundary_hosts]\nsbn-1\n");
    write(t.path(), "ops/ansible/host_vars/sbn-1.yml", &format!("platform_family: {family}\n"));
    let s = t.path().join("ops/scripts");
    std::fs::create_dir_all(&s).unwrap();
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts/inventory-hosts.py"), s.join("inventory-hosts.py")).unwrap();
    t
}

fn run(t: &tempfile::TempDir, args: &[&str]) -> (i32, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["-o", "json", "--yes", args[0], "-C", t.path().to_str().unwrap()])
        .args(&args[1..])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", t.path().join("home"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("GOAT_MESH_PROBE", "0")
        .output()
        .unwrap();
    (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

fn join(t: &tempfile::TempDir) -> (i32, String) {
    run(t, &["join-node", "zagros", "--site", "zagros", "--node", "sbn-1", "--class", "simple-boundary", "--address", "192.0.2.9", "--dry-run"])
}

#[test]
fn join_node_plans_the_machine_and_runs_the_simple_bn_play_on_it() {
    for family in ["small-computer", "host-class"] {
        let (rc, out) = join(&checkout(family));
        assert_eq!(rc, 0, "{out}");
        assert!(out.contains("mgmt-host-baseline.yml -e target=zagros_simple_boundary_hosts"), "the machine's own group throughout: {out}");
        assert!(out.contains("=== Phase 13 [A] [simple-boundary]"), "{out}");
        assert!(out.contains("simple-bn.yml -e target=zagros_simple_boundary_hosts -l sbn-1"), "the play, on this machine alone: {out}");
    }
}

#[test]
fn a_family_the_type_does_not_accept_is_refused() {
    let (rc, out) = join(&checkout("cloud"));
    assert!(rc != 0 && out.contains("GB-376") && !out.contains("=== Phase"), "{out}");
}

#[test]
fn doctor_reads_the_type_and_finds_its_play() {
    let t = checkout("host-class");
    // A checkout without the play: the row fails and names it.
    let (rc, out) = run(&t, &["doctor", "--target", "simple-boundary", "--site", "zagros"]);
    assert!(rc == 1 && out.contains("simple-bn.yml"), "{out}");
    write(t.path(), "ops/ansible/simple-bn.yml", "---\n");
    let (rc, out) = run(&t, &["doctor", "--target", "simple-boundary", "--site", "zagros"]);
    assert_eq!(rc, 0, "{out}");
    assert!(out.contains("simple-boundary at zagros: inventory group [zagros_simple_boundary_hosts], families small-computer, host-class"), "{out}");
    assert!(out.contains("sbn-1 is host-class") && out.contains("all 1 of the type's plays are in this tree"), "{out}");
}

#[test]
fn standup_builds_none() {
    let t = checkout("host-class");
    let (rc, out) = run(&t, &["standup", "zagros", "--site", "zagros", "--profile", "lan", "--dry-run"]);
    assert_eq!(rc, 0, "{out}");
    assert!(!out.contains("simple-bn.yml") && !out.contains("simple-boundary"), "no site declares one: {out}");
}
