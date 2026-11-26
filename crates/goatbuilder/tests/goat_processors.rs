//! The goat-processors type (gate J11; docs/design/goatbuilder-build-targets.md
//! sections 3.2 and 4.4), through the built binary: `join-node` builds a
//! host-class machine by push, other families are refused, and phase 13 names
//! the release component goat-processors-runner: a declared skip with no pin, its play
//! with one. Every run is a dry run, and nothing reaches a host.

use std::path::Path;
use std::process::Command;

const GROUP: &str = "zagros_goat_processors_hosts";

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn checkout(family: &str) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    write(t.path(), "ops/goatbuilder/goatbuilder.yaml", "schema: goatbuilder/v1\nrepo: {source: example-org/example-repo, image_namespace: ghcr.io/example-org}\ncomponents:\n  netbird-mgmt: {declared_in: ops/pins/fleet-pins.yaml}\nenvironments:\n  - name: zagros\n    converger: push-ansible\npromotion:\n  path:\n    - {name: zagros, kind: rung, environment: zagros}\ndeploy: {ssh_user: operator}\n");
    write(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\n");
    write(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", &format!("[zagros_hosts]\nmgmt-zagros-01-a\n[{GROUP}]\nproc-1\n"));
    write(t.path(), "ops/ansible/host_vars/proc-1.yml", &format!("platform_family: {family}\n"));
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

const JOIN: [&str; 11] = ["join-node", "zagros", "--site", "zagros", "--node", "proc-1", "--class", "goat-processors", "--address", "192.0.2.9", "--dry-run"];

fn rows(out: &str) -> Vec<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(out).unwrap_or_else(|e| panic!("{e}: {out}"));
    v["checks"].as_array().unwrap().clone()
}

#[test]
fn with_no_pin_join_node_builds_the_machine_and_says_it_runs_no_engine() {
    let t = checkout("host-class");
    let (rc, out) = run(t.path(), &JOIN);
    assert_eq!(rc, 0, "{out}");
    assert!(out.contains(&format!("-e target={GROUP}")), "{out}");
    assert!(out.contains("=== Phase 13 [SKIP] [goat-processors]"), "{out}");
    assert!(out.contains("nothing is installed, and this machine runs no engine: this checkout has no ops/pins/fleet-pins.yaml, so goat-processors-runner has no pin"), "{out}");
    let (_, out) = run(t.path(), &["doctor", "--target", "goat-processors", "--site", "zagros"]);
    let c = rows(&out).into_iter().find(|x| x["name"] == "component goat-processors-runner").unwrap_or_else(|| panic!("{out}"));
    assert!(c["detail"].as_str().unwrap().contains("goat-processors-runner has no pin"), "{c}");
}

#[test]
fn only_a_host_class_machine_is_accepted() {
    for family in ["small-computer", "cloud"] {
        let (rc, out) = run(checkout(family).path(), &JOIN);
        assert!(rc != 0 && out.contains("GB-376") && !out.contains("=== Phase"), "{family}: {out}");
    }
}

#[test]
fn with_a_pin_phase_13_runs_the_components_play() {
    let t = checkout("host-class");
    let pin = format!("example-org/example-processors@runner-v0.1.0@sha256:{}", "a".repeat(64));
    write(t.path(), "ops/pins/fleet-pins.yaml", &format!("goatnets:\n  zagros:\n    members: [zagros_hosts, {GROUP}]\ncomponents:\n  goat-processors-runner:\n    var: goat_processors_runner_release\n    deploy_playbook: goat-processors-runner-deploy.yml\n    artifact_type: release\n    fleet: \"{pin}\"\n    release:\n      checksums: SHA256SUMS\n      files: {{amd64: goat-processors_0.1.0_amd64.deb, arm64: goat-processors_0.1.0_arm64.deb}}\n      signer: https://github.com/example-org/example-processors/.github/workflows/runner-release.yml\n"));
    write(t.path(), &format!("ops/ansible/group_vars/{GROUP}/pins.yml"), &format!("goat_processors_runner_release: \"{pin}\"\n"));
    write(t.path(), "ops/ansible/goat-processors-runner-deploy.yml", "- name: the runner\n  hosts: \"{{ target | mandatory }}\"\n  tasks: []\n");
    let (rc, out) = run(t.path(), &JOIN);
    assert_eq!(rc, 0, "{out}");
    assert!(!out.contains("runs no engine"), "{out}");
    let r = rows(&out);
    assert!(r.iter().any(|x| x["name"] == "phase" && x["detail"].as_str().unwrap().starts_with("=== Phase 3 [GATE] goat-processors-runner's release")), "{out}");
    assert!(r.iter().any(|x| x["name"] == "cmd" && x["detail"].as_str().unwrap().contains(&format!("goat-processors-runner-deploy.yml -e target={GROUP} -e @\"$vars\" -l proc-1"))), "{out}");
}
