//! The network tap type (docs/design/goatbuilder-build-targets.md section
//! 3.8), through the built binary: a machine states its mode, listen or
//! inline, with no default; inline is shown "inline, fail-closed"; the type
//! requires an encrypted root; and phase 13 installs goatrope at its release pin,
//! or is a skip that says why there is none. The release is a synthetic one, and
//! nothing here reaches a host: the one real run's play is a stand-in.

#[path = "support/sigstore.rs"]
mod sigstore;

#[path = "../src/test_support.rs"]
mod test_support;

#[path = "common/signed_intent.rs"]
mod signed_intent;

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
    assert!(out.contains("it has no pin here: nothing is installed, and this machine is not a working tap: this checkout has no ops/pins/fleet-pins.yaml, so goatrope has no pin"), "phase 13 says the tap is not installed: {out}");
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
    // Each family model's entry says whether its preparation gives that root.
    assert!(
        enc["detail"].as_str().unwrap().ends_with(
            "Of its families' models, these prepare an encrypted root: rpi5, karbon-801, karbon-802; these do not: orin (data), thor (data)"
        ),
        "{enc}"
    );
    assert!(
        row(&r, "plays")["ok"] == true,
        "the type has no play to be missing: {out}"
    );
}

const REPO: &str = "example-org/example-tap";
const TAG: &str = "v0.1.0";
const GROUP: &str = "zagros_network_tap_hosts";

fn sha256(b: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(b))
}

/// A synthetic release of goatrope in a mirror directory, signed by a stand-in
/// Sigstore whose root is beside it; its pin.
fn release(root: &Path) -> String {
    let at = root.join("mirror").join(REPO).join(TAG);
    std::fs::create_dir_all(&at).unwrap();
    let sums = format!("{}  goatrope-linux-arm64\n", sha256(b"a synthetic build\n"));
    std::fs::write(at.join("goatrope-linux-arm64"), b"a synthetic build\n").unwrap();
    std::fs::write(at.join("SHA256SUMS"), &sums).unwrap();
    let s = sigstore::Sigstore::new();
    let w = sigstore::Workflow { repo: REPO.into(), workflow: "release.yml".into(), tag: TAG.into() };
    std::fs::write(at.join("SHA256SUMS.cosign-bundle"), s.bundle(sums.as_bytes(), &w)).unwrap();
    std::fs::write(root.join("trusted_root.json"), s.trusted_root()).unwrap();
    format!("{REPO}@{TAG}@sha256:{}", sha256(sums.as_bytes()))
}

/// goatrope in the pins, for the goatnet's members, rendered for the tap's group.
fn declare(root: &Path, members: &str, extra: &str, pin: &str, rendered: bool) {
    write(root, "ops/pins/fleet-pins.yaml", &format!("goatnets:\n  zagros:\n    members: [{members}]\ncomponents:\n  goatrope:\n    var: goatrope_release\n    artifact_type: release\n    fleet: \"{pin}\"\n    release:\n      checksums: SHA256SUMS\n      files: {{arm64: goatrope-linux-arm64}}\n      signer: https://github.com/{REPO}/.github/workflows/_release-binary.yml\n{extra}"));
    write(root, "ops/ansible/release-file-deploy.yml", "- name: release\n  hosts: \"{{ target | mandatory }}\"\n  tasks: []\n");
    if rendered {
        write(root, &format!("ops/ansible/group_vars/{GROUP}/pins.yml"), &format!("goatrope_release: \"{pin}\"\n"));
    }
}

/// Every phase's printed command.
fn commands(out: &str) -> Vec<String> {
    rows(out).iter().filter(|r| r["name"] == "cmd").map(|r| r["detail"].as_str().unwrap().to_string()).collect()
}

#[test]
fn with_a_pin_phase_3_verifies_goatrope_and_phase_13_installs_it() {
    let t = checkout();
    let pin = release(t.path());
    declare(t.path(), &format!("zagros_hosts, {GROUP}"), "", &pin, true);
    let (rc, out) = run(t.path(), &join("tap-1"));
    assert_eq!(rc, 0, "{out}");
    assert!(!out.contains("is not a working tap"), "{out}");
    let r = rows(&out);
    let phases: Vec<&str> = r.iter().filter(|x| x["name"] == "phase").map(|x| x["detail"].as_str().unwrap()).collect();
    let gate = phases.iter().position(|p| p.starts_with("=== Phase 3 [GATE] goatrope's release") && p.contains(&pin)).unwrap_or_else(|| panic!("{phases:?}"));
    let four = phases.iter().position(|p| p.starts_with("=== Phase 4")).unwrap();
    assert!(gate < four, "before anything on the node changes: {phases:?}");
    assert!(phases.iter().any(|p| p.starts_with("=== Phase 13 [A] [network-tap]") && p.contains(&pin)), "{phases:?}");
    let cmds = commands(&out);
    let install = cmds.iter().find(|c| c.contains("release-file-deploy.yml")).unwrap();
    assert!(install.contains(&format!("release-file-deploy.yml -e target={GROUP} -e @\"$vars\" -l tap-1")), "{install}");
    // The gate is deploy's dry run, and the checkout path is quoted as a word.
    let gate = cmds.iter().find(|c| c.ends_with(&format!(" deploy --env zagros --component goatrope --group {GROUP} --dry-run"))).unwrap_or_else(|| panic!("{cmds:?}"));
    assert!(gate.contains(&format!("-C '{}' deploy", t.path().display())), "{gate}");
    // doctor names the pin, and does not fetch it.
    let (_, out) = run(t.path(), &["doctor", "--target", "network-tap", "--site", "zagros"]);
    assert!(row(&rows(&out), "component goatrope")["detail"].as_str().unwrap().starts_with(&format!("pinned at {pin}")), "{out}");
}

/// The gate and the install as they run: the fetch is real, against the mirror;
/// the play's driver is a stand-in that records its words.
#[test]
fn the_gate_and_the_install_run_what_they_print() {
    let t = checkout();
    let pin = release(t.path());
    declare(t.path(), &format!("zagros_hosts, {GROUP}"), "", &pin, true);
    test_support::executable(&t.path().join("ops/scripts/with-aws"), "#!/bin/sh\nshift 2\nexec \"$@\"\n");
    for s in ["with-secrets", "ansible-preflight.sh"] {
        test_support::executable(&t.path().join("ops/scripts").join(s), "#!/bin/sh\necho \"$0 $*\" >> \"$GB_CALLS\"\ncase \"$0\" in *with-secrets) exec \"$@\" ;; esac\n");
    }
    let (_, out) = run(t.path(), &join("tap-1"));
    let cmds = commands(&out);
    let gate = cmds.iter().find(|c| c.contains(&format!(" deploy --env zagros --component goatrope --group {GROUP} --dry-run"))).unwrap();
    let install = cmds.iter().find(|c| c.contains("release-file-deploy.yml")).unwrap();
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
    // The gate is deploy's own dry run: in this checkout, which has no trunk,
    // no ladder gate and no signed intent, it refuses for each, though the
    // release itself verifies, and the install is not reached.
    let (rc, o) = sh(gate);
    assert!(rc == 1 && o.contains("release verified: goatrope-linux-arm64 (arm64)") && o.contains("a real run would refuse before any host (ladder fresh-source intent)"), "{rc} {o}");
    let (rc, o) = sh(install);
    assert_eq!(rc, 0, "{o}");
    let calls = std::fs::read_to_string(t.path().join("calls")).unwrap();
    let vars = t.path().join("home/.goat/state/releases/goatrope").join(pin.rsplit(':').next().unwrap()).join("deploy-vars.json");
    assert!(vars.is_file(), "{o}");
    assert!(calls.contains(&format!("release-file-deploy.yml -e target={GROUP} -e @{} -l tap-1", vars.display())), "{calls}");
    // A release that does not verify: the gate stops, and the play is not run.
    std::fs::remove_dir_all(t.path().join("home/.goat/state/releases")).unwrap();
    std::fs::write(t.path().join("mirror").join(REPO).join(TAG).join("goatrope-linux-arm64"), b"another build\n").unwrap();
    std::fs::remove_file(t.path().join("calls")).unwrap();
    for c in [gate, install] {
        let (rc, o) = sh(c);
        assert!(rc == 1 && o.contains("GB-381 refused before any host: goatrope's release") && o.contains("(check: listed)"), "{rc} {o}");
    }
    assert!(!std::fs::read_to_string(t.path().join("calls")).unwrap_or_default().contains("ansible-preflight.sh"), "no play");
}

fn git(dir: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.org", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

/// Phase 3's gate is deploy's dry run, so join-node installs nothing deploy
/// would refuse: in a checkout current with its trunk and with a signed intent
/// the gate passes; a pins.yml its intent does not sign is refused (GB-383)
/// though the release verifies.
#[test]
fn the_gate_refuses_what_deploy_refuses_before_the_node_changes() {
    let t = checkout();
    let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for d in ["ops/scripts", "ops/secrets", "ops/consistency/python"] {
        let o = Command::new("cp").arg("-R").arg(real.join(d)).arg(t.path().join(d).parent().unwrap()).output().unwrap();
        assert!(o.status.success());
    }
    for f in ["ops/release/ladder_gate.py", "ops/pins/generate-pins.py"] {
        write(t.path(), f, &std::fs::read_to_string(real.join(f)).unwrap());
    }
    let pin = release(t.path());
    declare(t.path(), &format!("zagros_hosts, {GROUP}"), "", &pin, true);
    signed_intent::prepare(t.path(), &["zagros_hosts", GROUP]);
    signed_intent::sign(t.path());
    let bare = t.path().join("../").join(format!("{}-origin.git", t.path().file_name().unwrap().to_string_lossy()));
    git(t.path(), &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()]);
    git(t.path(), &["init", "-q", "-b", "main"]);
    git(t.path(), &["remote", "add", "origin", bare.to_str().unwrap()]);
    let publish = |why: &str| {
        git(t.path(), &["add", "-A"]);
        git(t.path(), &["commit", "-q", "-m", why]);
        git(t.path(), &["push", "-q", "origin", "HEAD:main"]);
    };
    publish("a tap goatnet");
    let (_, out) = run(t.path(), &join("tap-1"));
    let cmds = commands(&out);
    let gate = cmds.iter().find(|c| c.contains(" deploy --env zagros --component goatrope")).unwrap().clone();
    let sh = |cmd: &str| {
        let o = Command::new("bash")
            .args(["-euo", "pipefail", "-c", cmd])
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", t.path().join("home"))
            .env("GOAT_MESH_PROBE", "0")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GOATBUILDER_BIN", env!("CARGO_BIN_EXE_goatbuilder"))
            .env("GOATBUILDER_RELEASE_MIRROR", t.path().join("mirror"))
            .env("GOATBUILDER_UPDATE_TRUSTED_ROOT", t.path().join("trusted_root.json"))
            .output()
            .unwrap();
        (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
    };
    let (rc, o) = sh(&gate);
    assert!(rc == 0 && o.contains("signed intent verified") && o.contains("release verified: goatrope-linux-arm64 (arm64)"), "{rc} {o}");
    // The rendered pins.yml edited by hand: the release still verifies, but
    // the goatnet's CA did not sign it, and deploy refuses it, so the gate does.
    write(t.path(), &format!("ops/ansible/group_vars/{GROUP}/pins.yml"), &format!("goatrope_release: \"{pin}\"  # edited\n"));
    publish("an edit the intent does not sign");
    let (rc, o) = sh(&gate);
    assert!(rc == 1 && o.contains("GB-383 REFUSE — the signed intent of goatnet zagros does not verify") && o.contains("a real run would refuse before any host (intent)"), "{rc} {o}");
    let _ = std::fs::remove_dir_all(&bare);
}

#[test]
fn with_no_pin_phase_13_is_a_skip_that_says_which_pin_is_missing() {
    let pin = format!("{REPO}@{TAG}@sha256:{}", "a".repeat(64));
    let both = format!("zagros_hosts, {GROUP}");
    let absent = format!("    deviations:\n      {GROUP}: {{absent: true, reason: no tap here}}\n");
    for (members, extra, rendered, why) in [
        (both.as_str(), "", false, format!("ops/ansible/group_vars/{GROUP}/pins.yml has no goatrope_release (declare goatrope")),
        ("zagros_hosts", "", true, format!("{GROUP} is not a member group of the goatnet zagros")),
        (both.as_str(), absent.as_str(), true, format!("ops/pins/fleet-pins.yaml declares goatrope absent on {GROUP}")),
    ] {
        let t = checkout();
        declare(t.path(), members, extra, &pin, rendered);
        let (rc, out) = run(t.path(), &join("tap-1"));
        assert!(rc == 0 && out.contains("this machine is not a working tap: ") && out.contains(&why), "{why}: {out}");
        assert!(!out.contains("release-file vars"), "{out}");
    }
    let t = checkout();
    write(t.path(), "ops/pins/fleet-pins.yaml", "goatnets:\n  zagros:\n    members: [zagros_hosts]\ncomponents:\n  other:\n    var: other_image\n    fleet: \"a@sha256:1\"\n");
    let (rc, out) = run(t.path(), &join("tap-1"));
    assert!(rc == 0 && out.contains("ops/pins/fleet-pins.yaml declares no component goatrope"), "{out}");
    let (_, out) = run(t.path(), &["doctor", "--target", "network-tap", "--site", "zagros"]);
    let r = rows(&out);
    let c = row(&r, "component goatrope");
    assert!(c["detail"].as_str().unwrap().contains("declares no component goatrope"), "{c}");
}

#[test]
fn a_goatrope_that_is_not_a_release_or_whose_pin_is_not_one_is_refused_before_any_phase() {
    let t = checkout();
    declare(t.path(), &format!("zagros_hosts, {GROUP}"), "", "example-org/example-tap@v1", true);
    let (rc, out) = run(t.path(), &join("tap-1"));
    assert!(rc == 1 && out.contains("GB-381") && out.contains("(check: pin)") && !out.contains("=== Phase"), "{rc} {out}");
    write(t.path(), "ops/pins/fleet-pins.yaml", &format!("goatnets:\n  zagros:\n    members: [{GROUP}]\ncomponents:\n  goatrope:\n    var: goatrope_release\n    fleet: \"a@sha256:1\"\n"));
    let (rc, out) = run(t.path(), &join("tap-1"));
    assert!(rc == 1 && out.contains("is not a release component") && !out.contains("=== Phase"), "{rc} {out}");
}
