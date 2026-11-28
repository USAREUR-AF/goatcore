//! The first-bite type (gate J20; docs/design/goatbuilder-build-targets.md
//! section 3.2), through the built binary: `join-node` builds the machine by
//! push, the families it accepts are held, and phase 13 names the release
//! component first-bite: a declared skip with no pin, `first-bite-deploy.yml`
//! (found by the component's name) with one. Every run is a dry run, and
//! nothing reaches a host.

use std::path::Path;
use std::process::Command;

const GROUP: &str = "zagros_first_bite_hosts";

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn checkout(family: &str) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    write(t.path(), "ops/goatbuilder/goatbuilder.yaml", "schema: goatbuilder/v1\nrepo: {source: example-org/example-repo, image_namespace: ghcr.io/example-org}\ncomponents:\n  netbird-mgmt: {declared_in: ops/pins/fleet-pins.yaml}\nenvironments:\n  - name: zagros\n    converger: push-ansible\npromotion:\n  path:\n    - {name: zagros, kind: rung, environment: zagros}\ndeploy: {ssh_user: operator}\n");
    write(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\n");
    write(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", &format!("[zagros_hosts]\nmgmt-zagros-01-a\n[{GROUP}]\ndecoder-1\n"));
    write(t.path(), "ops/ansible/host_vars/decoder-1.yml", &format!("platform_family: {family}\nfirst_bite_interface: eth1\n"));
    let s = t.path().join("ops/scripts");
    std::fs::create_dir_all(&s).unwrap();
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts/inventory-hosts.py"), s.join("inventory-hosts.py")).unwrap();
    t
}

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
        .unwrap();
    (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

const JOIN: [&str; 11] = ["join-node", "zagros", "--site", "zagros", "--node", "decoder-1", "--class", "first-bite", "--address", "192.0.2.9", "--dry-run"];

fn rows(out: &str) -> Vec<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(out).unwrap_or_else(|e| panic!("{e}: {out}"));
    v["checks"].as_array().unwrap().clone()
}

#[test]
fn with_no_pin_join_node_builds_the_machine_and_says_it_decodes_nothing() {
    for family in ["small-computer", "host-class"] {
        let t = checkout(family);
        let (rc, out) = run(t.path(), &JOIN);
        assert_eq!(rc, 0, "{out}");
        assert!(out.contains(&format!("-e target={GROUP}")), "{out}");
        assert!(out.contains("=== Phase 13 [SKIP] [first-bite]"), "{out}");
        assert!(out.contains("nothing is installed, and this machine decodes nothing: this checkout has no ops/pins/fleet-pins.yaml, so first-bite has no pin"), "{out}");
    }
}

#[test]
fn a_family_the_type_does_not_accept_is_refused() {
    let (rc, out) = run(checkout("cloud").path(), &JOIN);
    assert!(rc != 0 && out.contains("GB-376") && !out.contains("=== Phase"), "{out}");
}

#[test]
fn with_an_attested_pin_phase_13_runs_first_bites_play() {
    let t = checkout("host-class");
    let pin = format!("example-org/example-decoder@v0.1.0@sha256:{}", "a".repeat(64));
    write(t.path(), "ops/pins/fleet-pins.yaml", &format!("goatnets:\n  zagros:\n    members: [zagros_hosts, {GROUP}]\ncomponents:\n  first-bite:\n    var: first_bite_release\n    artifact_type: release\n    owns_state: true\n    rollback: pin-revert\n    fleet: \"{pin}\"\n    release:\n      checksums: SHA256SUMS\n      bundle: attestation\n      files: {{amd64: first-bite-0.1.0-x86_64-linux-gnu.tar.gz, arm64: first-bite-0.1.0-aarch64-linux-gnu.tar.gz}}\n      signer: https://github.com/example-org/example-decoder/.github/workflows/release.yml\n"));
    write(t.path(), &format!("ops/ansible/group_vars/{GROUP}/pins.yml"), &format!("first_bite_release: \"{pin}\"\n"));
    let play = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/ansible/first-bite-deploy.yml")).unwrap();
    write(t.path(), "ops/ansible/first-bite-deploy.yml", &play);
    let (rc, out) = run(t.path(), &JOIN);
    assert_eq!(rc, 0, "{out}");
    assert!(!out.contains("decodes nothing"), "{out}");
    let r = rows(&out);
    assert!(r.iter().any(|x| x["name"] == "phase" && x["detail"].as_str().unwrap().starts_with("=== Phase 3 [GATE] first-bite's release")), "{out}");
    assert!(r.iter().any(|x| x["name"] == "cmd" && x["detail"].as_str().unwrap().contains(&format!("first-bite-deploy.yml -e target={GROUP} -e @\"$vars\" -l decoder-1"))), "{out}");
    let (_, out) = run(t.path(), &["doctor", "--target", "first-bite", "--site", "zagros"]);
    let c = rows(&out).into_iter().find(|x| x["name"] == "component first-bite").unwrap_or_else(|| panic!("{out}"));
    assert!(c["detail"].as_str().unwrap().starts_with(&format!("pinned at {pin}")), "{c}");
}
