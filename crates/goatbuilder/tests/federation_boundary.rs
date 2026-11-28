//! The federation-boundary type (gate J8) through the built binary: `join-node`
//! plans the machine and says that a peering is `federate`'s, the families it
//! accepts are held, and `standup` builds none. Every run is a dry run.

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
    write(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nmgmt-zagros-01-a\n[zagros_federation_boundary_hosts]\nedge-1\n");
    write(t.path(), "ops/ansible/host_vars/edge-1.yml", &format!("platform_family: {family}\n"));
    let s = t.path().join("ops/scripts");
    std::fs::create_dir_all(&s).unwrap();
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts/inventory-hosts.py"), s.join("inventory-hosts.py")).unwrap();
    t
}

fn join(t: &tempfile::TempDir) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["-o", "json", "--yes", "join-node", "-C", t.path().to_str().unwrap(), "zagros", "--site", "zagros", "--node", "edge-1", "--class", "federation-boundary", "--address", "192.0.2.9", "--dry-run"])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", t.path().join("home"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("GOAT_MESH_PROBE", "0")
        .output()
        .unwrap();
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

#[test]
fn join_node_plans_the_machine_and_leaves_its_peerings_to_federate() {
    for family in ["small-computer", "host-class"] {
        let (ok, out) = join(&checkout(family));
        assert!(ok, "{out}");
        assert!(out.contains("-e target=zagros_federation_boundary_hosts"), "{out}");
        assert!(out.contains("=== Phase 13 [SKIP] [federation-boundary]") && out.contains("this machine peers with nobody until it has run"), "{out}");
        assert!(!out.contains("peering-"), "no peering play is run without an agreement: {out}");
    }
}

#[test]
fn a_family_the_type_does_not_accept_is_refused() {
    let (ok, out) = join(&checkout("cloud"));
    assert!(!ok && out.contains("GB-376") && !out.contains("=== Phase"), "{out}");
}
