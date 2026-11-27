//! A target type's `services:` (docs/design/goatbuilder-build-targets.md section
//! 3.9; gate J27), with a synthetic overlay type that lists one image component
//! and one release component: `join-node` plans a deploy of each to the one
//! machine, `deploy --node` touches and reads back that machine alone on both
//! avenues, and `doctor --target` holds the machine to them. `with-aws`, which
//! starts every ansible tool, is a stand-in: it records its calls, and host-b,
//! the machine's neighbour, is down.

#[path = "support/sigstore.rs"]
mod sigstore;

#[path = "../src/test_support.rs"]
mod test_support;

#[path = "common/signed_intent.rs"]
mod signed_intent;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const REPO: &str = "example-org/example-tool";
const TAG: &str = "v1.2.0";
const GROUP: &str = "zagros_example_role_hosts";
const IMAGE: &str = "ghcr.io/example-org/svc:v1@sha256:3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c";

fn sha256(b: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(b))
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let (f, t) = (e.path(), to.join(e.file_name()));
        match e.file_type().unwrap() {
            k if k.is_dir() => copy_tree(&f, &t),
            k if k.is_file() => test_support::copied_executable(&f, &t),
            _ => {}
        }
    }
}

struct World {
    root: tempfile::TempDir,
    repo: PathBuf,
    pin: String,
}

impl World {
    fn new() -> World {
        let root = tempfile::tempdir().unwrap();
        let r = &root.path().to_path_buf();
        let repo = r.join("repo");
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        copy_tree(&real.join("ops/scripts"), &repo.join("ops/scripts"));
        let at = r.join("mirror").join(REPO).join(TAG);
        std::fs::create_dir_all(&at).unwrap();
        let sums = format!("{}  tool-linux-amd64\n", sha256(b"a build\n"));
        std::fs::write(at.join("tool-linux-amd64"), b"a build\n").unwrap();
        std::fs::write(at.join("SHA256SUMS"), &sums).unwrap();
        let s = sigstore::Sigstore::new();
        let w = sigstore::Workflow { repo: REPO.into(), workflow: "release.yml".into(), tag: TAG.into() };
        std::fs::write(at.join("SHA256SUMS.cosign-bundle"), s.bundle(sums.as_bytes(), &w)).unwrap();
        std::fs::write(r.join("trusted_root.json"), s.trusted_root()).unwrap();
        let w = World { pin: format!("{REPO}@{TAG}@sha256:{}", sha256(sums.as_bytes())), repo, root };
        let real_text = |p: &str| std::fs::read_to_string(real.join(p)).unwrap();
        w.put("ops/release/ladder_gate.py", &real_text("ops/release/ladder_gate.py"));
        // Deploy verifies the goatnet's signed intent before any host (GB-383);
        // the script avenue with the real generate-pins.py.
        copy_tree(&real.join("ops/consistency/python"), &w.repo.join("ops/consistency/python"));
        w.put("ops/pins/generate-pins.py", &real_text("ops/pins/generate-pins.py"));
        w.put("ops/secrets/posture.py", "import sys\nsys.exit(0 if sys.argv[1:2] == [\"available\"] else 2)\n");
        w.put("ops/ansible/release-file-deploy.yml", &real_text("ops/ansible/release-file-deploy.yml"));
        w.put("ops/ansible/svc-deploy.yml", "- name: svc\n  hosts: \"{{ target }}\"\n  tasks: []\n");
        w.put("ops/goatbuilder/goatbuilder.yaml", "schema: goatbuilder/v1\nrepo: {source: example-org/example-repo, image_namespace: ghcr.io/example-org}\ncomponents:\n  svc: {declared_in: ops/pins/fleet-pins.yaml}\nenvironments:\n  - name: zagros\n    converger: push-ansible\npromotion:\n  path:\n    - {name: zagros, kind: rung, environment: zagros}\ndeploy: {ssh_user: goat}\n");
        w.put("ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\n");
        w.put("ops/ansible/inventory/mgmt-hosts.ini", &format!("[zagros_hosts]\nmgmt-zagros-01-a\n[{GROUP}]\nhost-a ansible_host=192.0.2.11\nhost-b ansible_host=192.0.2.12\n"));
        w.put("ops/goatbuilder/overlay/targets/example-role.yaml", "schema: goatbuilder-target/v1\ntype: example-role\nfamilies: [host-class]\nservices: [svc, tool]\nphases:\n  - label: the machine itself\n    skip: nothing beside its services\n");
        w.pins("");
        test_support::executable(&w.repo.join("ops/scripts/fresh-source.sh"), "#!/bin/sh\necho 1111111111111111111111111111111111111111\n");
        test_support::executable(
            &w.repo.join("ops/scripts/with-aws"),
            r#"#!/bin/sh
d="${GB_STUB_DIR:?}"
printf '%s\n' "$*" >> "$d/calls"
shift; [ "$1" = "--" ] && shift
case "$1" in
  ansible-inventory) printf '%s\n' '{"_meta":{"hostvars":{}},"GROUP":{"hosts":["host-a","host-b"]}}' ;;
  ../scripts/ansible-preflight.sh) printf 'PLAY RECAP ***\nhost-a : ok=3 changed=1 unreachable=0 failed=0\n' ;;
  ansible)
    case "$*" in
      *GBFILE*) line="GBFILE x86_64 SHA  /f" ;;
      */svc/*) line='{"ts": "t", "action": "deploy", "ref": "IMAGE"}' ;;
      *) line='{"ts": "t", "action": "deploy", "ref": "PIN"}' ;;
    esac
    printf 'host-a | CHANGED | rc=0 | (stdout) %s\n' "$line"
    case "$*" in *" host-a "*) ;; *) printf 'host-b | UNREACHABLE!: no route\n'; exit 4 ;; esac ;;
esac
"#
            .replace("GROUP", GROUP)
            .replace("IMAGE", IMAGE)
            .replace("PIN", &w.pin)
            .replace("SHA ", &format!("{} ", sha256(b"a build\n")))
            .as_str(),
        );
        for d in ["home", "tmp", "stub"] {
            std::fs::create_dir_all(r.join(d)).unwrap();
        }
        test_support::executable(&r.join("bin/netbird"), "#!/bin/sh\nexit 0\n");
        w
    }

    fn put(&self, p: &str, t: &str) {
        let p = self.repo.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    }

    fn pins(&self, svc_extra: &str) {
        self.put(
            "ops/pins/fleet-pins.yaml",
            &format!("goatnets:\n  zagros:\n    members: [zagros_hosts, {GROUP}]\ncomponents:\n  svc:\n    var: svc_image\n    fleet: \"{IMAGE}\"\n    owns_state: false\n    rollback: pin-revert\n{svc_extra}  tool:\n    var: tool_file\n    artifact_type: release\n    owns_state: false\n    rollback: pin-revert\n    fleet: \"{0}\"\n    release:\n      checksums: SHA256SUMS\n      files: {{amd64: tool-linux-amd64}}\n      signer: https://github.com/{REPO}/.github/workflows/_release-binary.yml\n", self.pin),
        );
        self.put(&format!("ops/ansible/group_vars/{GROUP}/pins.yml"), &format!("svc_image: \"{IMAGE}\"\ntool_file: \"{}\"\n", self.pin));
        signed_intent::prepare(&self.repo, &["zagros_hosts", GROUP]);
        signed_intent::sign(&self.repo);
    }

    fn run(&self, mut c: Command) -> Output {
        let r = self.root.path();
        let mut path = std::ffi::OsString::from(r.join("bin"));
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());
        c.env_clear()
            .env("PATH", path)
            .env("HOME", r.join("home"))
            .env("TMPDIR", r.join("tmp"))
            .env("GB_STUB_DIR", r.join("stub"))
            .env("GOATBUILDER_BIN", env!("CARGO_BIN_EXE_goatbuilder"))
            .env("GOATBUILDER_RELEASE_MIRROR", r.join("mirror"))
            .env("GOATBUILDER_UPDATE_TRUSTED_ROOT", r.join("trusted_root.json"))
            .env("GOAT_MESH_PROBE", "0")
            .env("GOAT_DOOR_OPEN", "0")
            .env("GOAT_NO_FUN", "1")
            .env("GOAT_EXPLAIN", "0")
            .env("GOAT_LANG", "en")
            .env("NO_COLOR", "1")
            .current_dir(&self.repo);
        c.output().unwrap()
    }

    fn gb(&self, args: &[&str]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
        c.args(["-o", "json", "--yes", "-C"]).arg(&self.repo).args(args);
        self.run(c)
    }

    fn join(&self) -> Output {
        self.gb(&["join-node", "zagros", "--site", "zagros", "--node", "host-a", "--class", "example-role", "--address", "192.0.2.11", "--dry-run"])
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.root.path().join("stub/calls")).unwrap_or_default()
    }
}

fn text_of(o: &Output) -> String {
    text(o)
}

fn text(o: &Output) -> String {
    let out = String::from_utf8_lossy(&o.stdout);
    match serde_json::from_str::<serde_json::Value>(&out) {
        Ok(v) if v["checks"].is_array() => v["checks"].as_array().unwrap().iter().map(|c| format!("{} {} {}\n", c["name"].as_str().unwrap_or(""), c["ok"], c["detail"].as_str().unwrap_or(""))).collect(),
        _ => format!("{out}{}", String::from_utf8_lossy(&o.stderr)),
    }
}

#[test]
fn join_node_deploys_each_listed_service_to_the_one_machine() {
    let w = World::new();
    let o = w.join();
    let t = text(&o);
    assert_eq!(o.status.code(), Some(0), "{t}");
    let cmds: Vec<&str> = t.lines().filter(|l| l.starts_with("cmd ") && l.contains(" deploy --env zagros")).collect();
    assert_eq!(cmds.len(), 2, "{t}");
    for (cmd, c) in cmds.iter().zip(["svc", "tool"]) {
        assert!(cmd.ends_with(&format!("deploy --env zagros --component {c} --group {GROUP} --node host-a")), "{cmd}");
    }
    assert!(t.contains(&format!("[example-role] service svc: {IMAGE}")) && t.contains(&format!("[example-role] service tool: {}", w.pin)), "{t}");
    // The commands as printed, run: each touches and reads back host-a alone,
    // though its neighbour is down.
    for cmd in cmds {
        let line = cmd.strip_prefix("cmd true ").unwrap();
        let mut c = Command::new("bash");
        c.args(["-euo", "pipefail", "-c", line]);
        let o = w.run(c);
        assert_eq!(o.status.code(), Some(0), "{line}\n{}", text(&o));
    }
    let calls = w.calls();
    let plays: Vec<&str> = calls.lines().filter(|l| l.contains("ansible-preflight.sh")).collect();
    assert!(plays.len() == 2 && plays.iter().all(|p| p.ends_with("-l host-a")), "{plays:?}");
    assert!(plays[0].contains("svc-deploy.yml") && plays[1].contains("release-file-deploy.yml"), "{plays:?}");
    for l in calls.lines().filter(|l| l.contains(" ansible -i")) {
        assert!(l.contains(" host-a -m shell") && !l.contains(&format!(" {GROUP} -m")), "{l}");
    }
}

#[test]
fn one_machine_of_a_group_on_both_avenues() {
    let w = World::new();
    let script = |args: &[&str]| {
        let mut c = Command::new("bash");
        c.arg(w.repo.join("ops/scripts/deploy-component.sh")).args(["svc", "--goatnet", "zagros"]).args(args);
        w.run(c)
    };
    let binary = |args: &[&str]| w.gb(&[&["deploy", "--env", "zagros", "--component", "svc"], args].concat());
    // The neighbour is down: the group does not converge, and the machine does.
    for (o, rc) in [(binary(&["--group", GROUP]), 3), (script(&["--group", GROUP]), 1)] {
        assert_eq!(o.status.code(), Some(rc), "{}", text(&o));
    }
    for o in [binary(&["--group", GROUP, "--node", "host-a"]), script(&["--group", GROUP, "--node", "host-a"])] {
        let t = text(&o);
        assert!(o.status.code() == Some(0) && t.contains("converged on sha256:3c3c"), "{t}");
        assert!(!t.contains("host-b"), "{t}");
    }
    // A machine that is not in the group, or a group's name, is refused from the
    // base inventory before any door opens or play runs; nothing wider is taken.
    let before = w.calls();
    for args in [vec!["--node", "host-z"], vec!["--node", "host-z", "--verify-only"], vec!["--node", "host-z", "--dry-run"]] {
        for o in [binary(&[&["--group", GROUP], &args[..]].concat()), script(&[&["--group", GROUP], &args[..]].concat())] {
            assert!(!o.status.success() && text(&o).contains(&format!("--node host-z is not a host of {GROUP} in ops/ansible/inventory/mgmt-hosts.ini")), "{args:?}: {}", text(&o));
        }
    }
    for group_name in [GROUP, "zagros_hosts"] {
        for o in [binary(&["--group", GROUP, "--node", group_name]), script(&["--group", GROUP, "--node", group_name])] {
            assert!(!o.status.success() && text(&o).contains(&format!("--node {group_name} is a group in ops/ansible/inventory/mgmt-hosts.ini, not one machine")), "{group_name}: {}", text(&o));
        }
    }
    assert_eq!(w.calls(), before, "a refused --node reaches no host and opens no door");
    for bad in ["all", "host-*", "host-a:host-b", "!host-a", "host-a,host-b", "@hosts"] {
        for o in [binary(&["--group", GROUP, "--node", bad]), script(&["--group", GROUP, "--node", bad])] {
            assert!(!o.status.success() && text(&o).contains("is one machine's inventory name"), "{bad}: {}", text(&o));
        }
    }
    let o = script(&["--node", "host-a"]);
    assert!(!o.status.success() && text(&o).contains("with the --group it is in"), "{}", text(&o));
    assert_eq!(w.calls(), before, "a refused name reaches no host");
}

#[test]
fn a_listed_service_with_no_component_or_pin_is_refused_before_any_phase_gb384() {
    let w = World::new();
    let refused = |why: &str| {
        let o = w.join();
        let t = text(&o);
        assert!(!o.status.success() && t.contains("GB-384") && t.contains(why) && !t.contains("=== Phase"), "{why}: {t}");
        let t = text(&w.gb(&["doctor", "--target", "example-role", "--site", "zagros"]));
        assert!(t.contains("service svc false example-role lists the service svc, and"), "{t}");
    };
    w.put(&format!("ops/ansible/group_vars/{GROUP}/pins.yml"), &format!("tool_file: \"{}\"\n", w.pin));
    refused("has no svc_image");
    w.pins("");
    std::fs::remove_file(w.repo.join("ops/ansible/svc-deploy.yml")).unwrap();
    refused("no deploy playbook for svc");
    w.put("ops/ansible/svc-deploy.yml", "- name: svc\n  hosts: \"{{ target }}\"\n  tasks: []\n");
    w.put("ops/pins/fleet-pins.yaml", &format!("goatnets:\n  zagros:\n    members: [{GROUP}]\ncomponents:\n  other:\n    var: other_image\n    fleet: \"{IMAGE}\"\n"));
    refused("declares no component of that name");
    // Absent by declaration is a skip, with the pins' own word.
    w.pins(&format!("    deviations:\n      {GROUP}:\n        absent: true\n        reason: not here\n"));
    let t = text(&w.join());
    assert!(t.contains(&format!("declares svc absent on {GROUP}")) && t.contains("--component tool") && !t.contains("--component svc"), "{t}");
}

/// standup builds a phase-14 type that lists services: after its plays, phase
/// 14 deploys each to the type's machines at the site through deploy itself,
/// and refuses one it cannot have before any phase (GB-384).
#[test]
fn standup_deploys_a_types_services_to_its_machines() {
    let w = World::new();
    w.put("ops/goatbuilder/overlay/targets/example-role.yaml", "schema: goatbuilder-target/v1\ntype: example-role\nfamilies: [host-class]\nservices: [svc, tool]\nsite: {declared_by: has_example, standup: phase-14}\nphases:\n  - label: the machine itself\n    skip: nothing beside its services\n");
    w.put("ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\nhas_example: true\n");
    let standup = || w.gb(&["standup", "zagros", "--site", "zagros", "--profile", "lan", "--dry-run"]);
    let o = standup();
    let t = text(&o);
    assert_eq!(o.status.code(), Some(0), "{t}");
    let cmds: Vec<&str> = t.lines().filter(|l| l.contains(" deploy --env zagros")).collect();
    assert_eq!(cmds.len(), 2, "{t}");
    for (cmd, c) in cmds.iter().zip(["svc", "tool"]) {
        assert!(cmd.ends_with(&format!("deploy --env zagros --component {c} --group {GROUP}")), "{cmd}");
    }
    assert!(t.contains(&format!("[example-role] service svc: {IMAGE}, deployed to {GROUP}")) && t.contains(&format!("[example-role] service tool: {}", w.pin)), "{t}");
    // The command as printed, run: the play reaches the type's group, and the
    // neighbour that is down keeps it from converging.
    let line = cmds[0].split_once(" \"$").map(|(_, rest)| format!("\"${rest}")).expect("the command");
    let mut c = Command::new("bash");
    c.args(["-euo", "pipefail", "-c", &line]);
    let o = w.run(c);
    assert!(!o.status.success(), "{line}\n{}", text(&o));
    let calls = w.calls();
    let plays: Vec<&str> = calls.lines().filter(|l| l.contains("ansible-preflight.sh")).collect();
    assert!(plays.len() == 1 && plays[0].contains(&format!("svc-deploy.yml -e target={GROUP}")) && !plays[0].contains(" -l "), "{plays:?}");
    // A service it cannot have stops the plan before any phase.
    w.put(&format!("ops/ansible/group_vars/{GROUP}/pins.yml"), &format!("tool_file: \"{}\"\n", w.pin));
    let o = standup();
    let t = text(&o);
    assert!(!o.status.success() && t.contains("GB-384") && t.contains("has no svc_image") && !t.contains("=== Phase"), "{t}");
}

/// A listed service whose rollout is not cleanly reversible (ADR 1211): a run
/// without --ack is refused before its first phase (GB-421), on join-node and
/// on standup; with one, the service's deploy gets it and runs.
#[test]
fn a_service_whose_rollout_needs_an_ack_is_refused_before_any_phase_without_one() {
    let w = World::new();
    let table = w.repo.join("ops/pins/fleet-pins.yaml");
    let text = std::fs::read_to_string(&table).unwrap();
    std::fs::write(&table, text.replacen("    rollback: pin-revert\n", "    rollback: none\n", 1)).unwrap();
    signed_intent::prepare(&w.repo, &["zagros_hosts", GROUP]);
    signed_intent::sign(&w.repo);
    let join = |extra: &[&str]| w.gb(&[&["join-node", "zagros", "--site", "zagros", "--node", "host-a", "--class", "example-role", "--address", "192.0.2.11", "--from", "13", "--to", "13"], extra].concat());
    let o = join(&[]);
    let t = text_of(&o);
    assert!(!o.status.success() && t.contains("GB-421") && t.contains("the rollout of the service svc is none") && t.contains("Pass --ack") && !t.contains("=== Phase"), "{t}");
    assert_eq!(w.calls(), "", "no phase ran");
    // A plan says so, and plans the deploy.
    let t = text_of(&w.join());
    assert!(t.contains("its rollout needs --ack: a run refuses before phase 1 without it") && t.contains("--component svc --group"), "{t}");
    let t = text_of(&w.gb(&["join-node", "zagros", "--site", "zagros", "--node", "host-a", "--class", "example-role", "--address", "192.0.2.11", "--dry-run", "--ack", "Ada Example"]));
    let cmd = t.lines().find(|l| l.contains("--component svc")).expect("the svc deploy");
    assert!(cmd.ends_with(&format!("deploy --env zagros --component svc --group {GROUP} --node host-a --ack 'Ada Example'")), "{cmd}");
    assert!(t.lines().any(|l| l.ends_with(&format!("--component tool --group {GROUP} --node host-a"))), "a clean rollout gets no --ack: {t}");
    // The command as printed runs, and the deploy takes the name.
    let line = cmd.split_once(" \"$").map(|(_, rest)| format!("\"${rest}")).unwrap();
    let mut c = Command::new("bash");
    c.args(["-euo", "pipefail", "-c", &line]);
    let o = w.run(c);
    assert!(o.status.success(), "{line}\n{}", text_of(&o));
    assert!(w.calls().contains("svc-deploy.yml"), "{}", w.calls());
    // standup, for a type it builds in phase 14.
    w.put("ops/goatbuilder/overlay/targets/example-role.yaml", "schema: goatbuilder-target/v1\ntype: example-role\nfamilies: [host-class]\nservices: [svc, tool]\nsite: {declared_by: has_example, standup: phase-14}\nphases:\n  - label: the machine itself\n    skip: nothing beside its services\n");
    w.put("ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\nhas_example: true\n");
    let before = w.calls();
    let o = w.gb(&["standup", "zagros", "--site", "zagros", "--profile", "lan", "--from", "14", "--to", "14"]);
    let t = text_of(&o);
    assert!(!o.status.success() && t.contains("GB-421") && t.contains("to standup") && !t.contains("=== Phase"), "{t}");
    assert_eq!(w.calls(), before, "no phase ran");
    let t = text_of(&w.gb(&["standup", "zagros", "--site", "zagros", "--profile", "lan", "--dry-run", "--ack", "Ada Example"]));
    assert!(t.lines().any(|l| l.ends_with(&format!("--component svc --group {GROUP} --ack 'Ada Example'"))), "{t}");
}

#[test]
fn a_type_standup_builds_as_obs_legs_lists_no_services() {
    let w = World::new();
    w.put("ops/goatbuilder/overlay/targets/example-role.yaml", "schema: goatbuilder-target/v1\ntype: example-role\nfamilies: [host-class]\nservices: [svc]\nsite: {declared_by: has_example, standup: obs-legs}\nphases:\n  - label: x\n    skip: y\n");
    let t = text(&w.join());
    assert!(t.contains("GB-374") && t.contains("deploys none"), "{t}");
}

#[test]
fn doctor_live_says_of_each_machine_what_verify_says() {
    let w = World::new();
    let t = text(&w.gb(&["doctor", "--target", "example-role", "--site", "zagros"]));
    assert!(t.contains("service svc true pinned at") && w.calls().is_empty(), "no --live, no machine: {t}");
    let t = text(&w.gb(&["doctor", "--target", "example-role", "--site", "zagros", "--node", "host-a", "--live"]));
    assert!(t.contains("service svc host-a true converged on sha256:3c3c") && t.contains("service tool host-a true converged on"), "{t}");
    let o = w.gb(&["doctor", "--target", "example-role", "--site", "zagros", "--live"]);
    let t = text(&o);
    assert!(o.status.code() == Some(1) && t.contains("service svc host-b false "), "{t}");
}
