//! The octo-pod type (gate J18; docs/design/goatbuilder-build-targets.md
//! section 3.2), through the built binary: `join-node` builds the machine by
//! push, the families it accepts are held, and phase 13 installs octo-pod at
//! its release pin with `octo-pod-deploy.yml`, or is a skip that says why there
//! is none. The release is a synthetic one, signed by a stand-in Sigstore, and
//! nothing here reaches a host: the one real run's play driver is a stand-in.

#[path = "support/sigstore.rs"]
mod sigstore;

#[path = "../src/test_support.rs"]
mod test_support;

use std::path::Path;
use std::process::Command;

const REPO: &str = "example-org/example-pod";
const TAG: &str = "octo-pod-v0.1.0";
const DEB: &str = "octo-pod_0.1.0_all.deb";
const GROUP: &str = "zagros_octo_pod_hosts";

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// Goatnet `zagros`, one founder site with one pod of the family given.
fn checkout(family: &str) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    write(t.path(), "ops/goatbuilder/goatbuilder.yaml", "schema: goatbuilder/v1\nrepo: {source: example-org/example-repo, image_namespace: ghcr.io/example-org}\ncomponents:\n  netbird-mgmt: {declared_in: ops/pins/fleet-pins.yaml}\nenvironments:\n  - name: zagros\n    converger: push-ansible\npromotion:\n  path:\n    - {name: zagros, kind: rung, environment: zagros}\ndeploy: {ssh_user: operator}\n");
    write(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\n");
    write(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", &format!("[zagros_hosts]\nmgmt-zagros-01-a\n[{GROUP}]\npod-1\n"));
    write(t.path(), "ops/ansible/host_vars/pod-1.yml", &format!("platform_family: {family}\n"));
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

const JOIN: [&str; 11] = ["join-node", "zagros", "--site", "zagros", "--node", "pod-1", "--class", "octo-pod", "--address", "192.0.2.9", "--dry-run"];

fn rows(out: &str) -> Vec<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(out).unwrap_or_else(|e| panic!("{e}: {out}"));
    v["checks"].as_array().unwrap().clone()
}

fn commands(out: &str) -> Vec<String> {
    rows(out).iter().filter(|r| r["name"] == "cmd").map(|r| r["detail"].as_str().unwrap().to_string()).collect()
}

fn phases(out: &str) -> Vec<String> {
    rows(out).iter().filter(|r| r["name"] == "phase").map(|r| r["detail"].as_str().unwrap().to_string()).collect()
}

fn sha256(b: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(b))
}

/// A synthetic octo-pod release in a mirror directory: one package for every
/// architecture, as octo-pod publishes it, and its checksums file signed by a
/// stand-in Sigstore whose root is beside it (the stand-in's certificate names
/// the workflow `_release-binary.yml`). Its pin.
fn release(root: &Path) -> String {
    let at = root.join("mirror").join(REPO).join(TAG);
    std::fs::create_dir_all(&at).unwrap();
    std::fs::write(at.join(DEB), b"a synthetic package\n").unwrap();
    let sums = format!("{}  {DEB}\n", sha256(b"a synthetic package\n"));
    std::fs::write(at.join("SHA256SUMS"), &sums).unwrap();
    let s = sigstore::Sigstore::new();
    let w = sigstore::Workflow { repo: REPO.into(), workflow: "_release-binary.yml".into(), tag: TAG.into() };
    std::fs::write(at.join("SHA256SUMS.cosign-bundle"), s.bundle(sums.as_bytes(), &w)).unwrap();
    std::fs::write(root.join("trusted_root.json"), s.trusted_root()).unwrap();
    format!("{REPO}@{TAG}@sha256:{}", sha256(sums.as_bytes()))
}

/// octo-pod in the pins for the pod's group, with the play this repository
/// ships for it.
fn declare(root: &Path, pin: &str) {
    write(root, "ops/pins/fleet-pins.yaml", &format!("goatnets:\n  zagros:\n    members: [zagros_hosts, {GROUP}]\ncomponents:\n  octo-pod:\n    var: octo_pod_release\n    artifact_type: release\n    fleet: \"{pin}\"\n    release:\n      checksums: SHA256SUMS\n      files: {{amd64: {DEB}, arm64: {DEB}}}\n      signer: https://github.com/{REPO}/.github/workflows/_release-binary.yml\n"));
    write(root, &format!("ops/ansible/group_vars/{GROUP}/pins.yml"), &format!("octo_pod_release: \"{pin}\"\n"));
    let play = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/ansible/octo-pod-deploy.yml")).unwrap();
    write(root, "ops/ansible/octo-pod-deploy.yml", &play);
    write(root, "ops/ansible/release-file-deploy.yml", "- name: the play of a release with none of its own\n  hosts: \"{{ target | mandatory }}\"\n  tasks: []\n");
}

#[test]
fn with_no_pin_join_node_builds_the_machine_and_says_it_is_not_a_pod() {
    for family in ["small-computer", "host-class"] {
        let t = checkout(family);
        let (rc, out) = run(t.path(), &JOIN);
        assert_eq!(rc, 0, "{out}");
        assert!(out.contains(&format!("-e target={GROUP}")), "{out}");
        assert!(phases(&out).iter().any(|p| p.starts_with("=== Phase 13 [SKIP] [octo-pod]")), "{out}");
        assert!(out.contains("nothing is installed, and this machine is not a pod: this checkout has no ops/pins/fleet-pins.yaml, so octo-pod has no pin"), "{out}");
        assert!(!out.contains("octo-pod-deploy.yml"), "{out}");
    }
}

#[test]
fn a_family_the_type_does_not_accept_is_refused() {
    let (rc, out) = run(checkout("cloud").path(), &JOIN);
    assert!(rc != 0 && out.contains("GB-376") && !out.contains("=== Phase"), "{out}");
}

#[test]
fn with_a_pin_phase_3_verifies_the_release_and_phase_13_runs_the_pods_play() {
    let t = checkout("small-computer");
    let pin = release(t.path());
    declare(t.path(), &pin);
    let (rc, out) = run(t.path(), &JOIN);
    assert_eq!(rc, 0, "{out}");
    assert!(!out.contains("is not a pod"), "{out}");
    let p = phases(&out);
    let gate = p.iter().position(|x| x.starts_with("=== Phase 3 [GATE] octo-pod's release") && x.contains(&pin)).unwrap_or_else(|| panic!("{p:?}"));
    let four = p.iter().position(|x| x.starts_with("=== Phase 4")).unwrap();
    assert!(gate < four, "before anything on the node changes: {p:?}");
    assert!(p.iter().any(|x| x.starts_with("=== Phase 13 [A] [octo-pod]") && x.contains(&pin)), "{p:?}");
    let cmds = commands(&out);
    let install = cmds.iter().find(|c| c.contains("octo-pod-deploy.yml")).unwrap_or_else(|| panic!("the pod's own play, found by its name: {cmds:?}"));
    assert!(install.contains(&format!("octo-pod-deploy.yml -e target={GROUP} -e @\"$vars\" -l pod-1")), "{install}");
    assert!(!cmds.iter().any(|c| c.contains("release-file-deploy.yml")), "{cmds:?}");

    // The install as it runs: the fetch and verification are real, against the
    // mirror; the play's driver is a stand-in that records its words.
    test_support::executable(&t.path().join("ops/scripts/with-aws"), "#!/bin/sh\nshift 2\nexec \"$@\"\n");
    for s in ["with-secrets", "ansible-preflight.sh"] {
        test_support::executable(&t.path().join("ops/scripts").join(s), "#!/bin/sh\necho \"$0 $*\" >> \"$GB_CALLS\"\ncase \"$0\" in *with-secrets) exec \"$@\" ;; esac\n");
    }
    let sh = |cmd: &str| {
        let o = Command::new("bash")
            .args(["-euo", "pipefail", "-c", cmd])
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", t.path().join("home"))
            .env("GB_CALLS", t.path().join("calls"))
            .env("GOATBUILDER_BIN", env!("CARGO_BIN_EXE_goatbuilder"))
            .env("GOATBUILDER_RELEASE_MIRROR", t.path().join("mirror"))
            .env("GOATBUILDER_UPDATE_TRUSTED_ROOT", t.path().join("trusted_root.json"))
            .output()
            .unwrap();
        (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
    };
    let (rc, o) = sh(install);
    assert_eq!(rc, 0, "{o}");
    let vars_file = t.path().join("home/.goat/state/releases/octo-pod").join(pin.rsplit(':').next().unwrap()).join("deploy-vars.json");
    let vars: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&vars_file).unwrap_or_else(|e| panic!("{e}: {o}"))).unwrap();
    for arch in ["amd64", "arm64"] {
        assert_eq!(vars["release_file_files"][arch]["name"], DEB, "one package for every architecture: {vars}");
    }
    let calls = std::fs::read_to_string(t.path().join("calls")).unwrap();
    assert!(calls.contains(&format!("octo-pod-deploy.yml -e target={GROUP} -e @{} -l pod-1", vars_file.display())), "{calls}");

    // A package that is not the one the signed checksums list: refused before
    // the play.
    std::fs::remove_dir_all(t.path().join("home/.goat/state/releases")).unwrap();
    std::fs::remove_file(t.path().join("calls")).unwrap();
    std::fs::write(t.path().join("mirror").join(REPO).join(TAG).join(DEB), b"another package\n").unwrap();
    let (rc, o) = sh(install);
    assert!(rc == 1 && o.contains("GB-381 refused before any host: octo-pod's release") && o.contains("(check: listed)"), "{rc} {o}");
    assert!(!std::fs::read_to_string(t.path().join("calls")).unwrap_or_default().contains("ansible-preflight.sh"), "no play");
}

#[test]
fn doctor_target_names_the_pin_or_what_is_missing() {
    let t = checkout("host-class");
    let (_, out) = run(t.path(), &["doctor", "--target", "octo-pod", "--site", "zagros"]);
    let r = rows(&out);
    let c = r.iter().find(|x| x["name"] == "component octo-pod").unwrap_or_else(|| panic!("{out}"));
    assert!(c["detail"].as_str().unwrap().contains("octo-pod has no pin"), "{c}");
    let u = r.iter().find(|x| x["name"] == "automatic security updates").unwrap_or_else(|| panic!("{out}"));
    assert!(u["detail"].as_str().unwrap().contains("--live reads each machine"), "{u}");
    let pin = release(t.path());
    declare(t.path(), &pin);
    let (_, out) = run(t.path(), &["doctor", "--target", "octo-pod", "--site", "zagros"]);
    let r = rows(&out);
    let c = r.iter().find(|x| x["name"] == "component octo-pod").unwrap_or_else(|| panic!("{out}"));
    assert!(c["detail"].as_str().unwrap().starts_with(&format!("pinned at {pin}")), "{c}");
}
