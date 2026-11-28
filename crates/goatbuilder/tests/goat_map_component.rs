//! Gate row J25: goat-map is a component of the public pins, and `goatbuilder
//! deploy goat-map` is the only verb it needs. The row is the public file's own,
//! read from this checkout: opt-in, on the one host `goat_map_host` names, its
//! play found in a tree that layers goatstuff (here a stand-in of that play).
//! Both avenues, the binary and the real ops/scripts/deploy-component.sh, run on
//! one stand-in checkout; `with-aws`, which starts every ansible tool, records
//! its calls and answers for the hosts.

#[path = "../src/test_support.rs"]
mod test_support;

#[path = "common/signed_intent.rs"]
mod signed_intent;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const DIGEST: &str = "sha256:808ecef0399eeedb0c0f6783ff4cff428ead5823b66090763b2f379f4af92a2d";

fn real() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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

fn put(root: &Path, p: &str, t: &str) {
    let p = root.join(p);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, t).unwrap();
}

/// The public pins file's goat-map row, as text, exactly as committed: from its
/// name to the next line that is not indented under it.
fn public_row() -> String {
    let t = std::fs::read_to_string(real().join("ops/pins/fleet-pins.yaml")).unwrap();
    let mut lines = t.lines().skip_while(|l| *l != "  goat-map:");
    let head = lines.next().expect("the public pins declare goat-map (gate J25)");
    let body: Vec<&str> = lines.take_while(|l| l.starts_with("    ")).collect();
    format!("{head}\n{}\n", body.join("\n"))
}

struct World {
    root: tempfile::TempDir,
    repo: PathBuf,
}

impl World {
    fn new() -> World {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let repo = r.join("repo");
        copy_tree(&real().join("ops/scripts"), &repo.join("ops/scripts"));
        put(&repo, "ops/release/ladder_gate.py", &std::fs::read_to_string(real().join("ops/release/ladder_gate.py")).unwrap());
        copy_tree(&real().join("ops/consistency/python"), &repo.join("ops/consistency/python"));
        put(&repo, "ops/pins/generate-pins.py", &std::fs::read_to_string(real().join("ops/pins/generate-pins.py")).unwrap());
        put(&repo, "ops/secrets/posture.py", "import sys\nsys.exit(0 if sys.argv[1:2] == [\"available\"] else 2)\n");
        // A tree that layers goatstuff carries goatstuff's play under ops/ansible.
        put(&repo, "ops/ansible/goat-map-deploy.yml", "- name: Deploy goat-map (stand-in of goatstuff's play)\n  hosts: \"{{ target | mandatory }}\"\n  tasks: []\n");
        put(&repo, "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nhost-a ansible_host=192.0.2.11\nhost-b ansible_host=192.0.2.12\n[other_hosts]\nhost-c ansible_host=192.0.2.13\n");
        put(&repo, "ops/ansible/group_vars/zagros_hosts/goat-map.yml", "goat_map_host: host-b\n");
        std::fs::create_dir_all(repo.join("ops/ansible/group_vars/other_hosts")).unwrap();
        put(
            &repo,
            "ops/pins/fleet-pins.yaml",
            &format!("goatnets:\n  zagros:\n    members: [zagros_hosts]\n    opt_in: [goat-map]\n  other:\n    members: [other_hosts]\ncomponents:\n{}", public_row()),
        );
        test_support::executable(&repo.join("ops/scripts/fresh-source.sh"), "#!/bin/sh\necho 1111111111111111111111111111111111111111\n");
        test_support::executable(
            &repo.join("ops/scripts/with-aws"),
            &format!(
                r#"#!/bin/sh
printf '%s\n' "$*" >> "$GB_STUB_DIR/calls"
shift; [ "$1" = "--" ] && shift
case "$1" in
  ansible-inventory) printf '%s\n' '{{"_meta":{{"hostvars":{{}}}},"zagros_hosts":{{"hosts":["host-a","host-b"]}},"other_hosts":{{"hosts":["host-c"]}}}}' ;;
  ../scripts/ansible-preflight.sh) printf 'PLAY RECAP ***\nhost-a : ok=1 changed=0 unreachable=0 failed=0\nhost-b : ok=3 changed=1 unreachable=0 failed=0\n' ;;
  ansible) printf 'host-a | CHANGED | rc=0 | (stdout) NO_LEDGER\nhost-b | CHANGED | rc=0 | (stdout) {{"ts": "t", "action": "deploy", "ref": "ghcr.io/example-org/goat-map@{DIGEST}"}}\n' ;;
esac
"#
            ),
        );
        for d in ["home", "tmp", "stub"] {
            std::fs::create_dir_all(r.join(d)).unwrap();
        }
        test_support::executable(&r.join("bin/netbird"), "#!/bin/sh\nexit 0\n");
        signed_intent::prepare(&repo, &["zagros_hosts", "other_hosts"]);
        signed_intent::sign(&repo);
        World { root, repo }
    }

    fn run(&self, mut c: Command) -> Output {
        let r = self.root.path();
        c.env_clear()
            .env("PATH", format!("{}:{}", r.join("bin").display(), std::env::var("PATH").unwrap_or_default()))
            .env("HOME", r.join("home"))
            .env("TMPDIR", r.join("tmp"))
            .env("GB_STUB_DIR", r.join("stub"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GOATBUILDER_BIN", env!("CARGO_BIN_EXE_goatbuilder"))
            .env("GOAT_MESH_PROBE", "0")
            .env("GOAT_DOOR_OPEN", "0")
            .env("GOAT_NO_FUN", "1")
            .env("GOAT_EXPLAIN", "0")
            .env("GOAT_LANG", "en")
            .env("NO_COLOR", "1")
            .current_dir(&self.repo);
        c.output().unwrap()
    }

    fn both(&self, goatnet: &str, extra: &[&str]) -> [Output; 2] {
        let mut gb = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
        gb.args(["-o", "json", "--yes", "-C"]).arg(&self.repo).args(["deploy", "--env", goatnet, "--component", "goat-map"]).args(extra);
        let mut sh = Command::new("bash");
        sh.arg(self.repo.join("ops/scripts/deploy-component.sh")).args(["goat-map", "--goatnet", goatnet]).args(extra);
        [self.run(gb), self.run(sh)]
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.root.path().join("stub/calls")).unwrap_or_default()
    }
}

fn text(o: &Output) -> String {
    let out = String::from_utf8_lossy(&o.stdout);
    match serde_json::from_str::<serde_json::Value>(&out) {
        Ok(v) if v["checks"].is_array() => v["checks"].as_array().unwrap().iter().map(|c| format!("{} {}\n", c["name"].as_str().unwrap_or(""), c["detail"].as_str().unwrap_or(""))).collect(),
        _ => format!("{out}{}", String::from_utf8_lossy(&o.stderr)),
    }
}

#[test]
fn the_public_row_is_opt_in_on_one_host_at_the_published_index() {
    let row = public_row();
    for want in ["    opt_in: true\n", "    one_host_var: goat_map_host\n", "    var: goat_map_image\n", "    ledger_component: goat-map\n", &format!("    fleet: ghcr.io/{{{{ ghcr_org }}}}/goat-map@{DIGEST}\n")] {
        assert!(row.contains(want), "{want:?} in\n{row}");
    }
    assert!(!row.contains("source:"), "deployed from a tree that layers goatstuff, not by source:\n{row}");
}

#[test]
fn goatbuilder_deploy_goat_map_runs_its_play_and_verifies_the_one_map_host() {
    let w = World::new();
    for o in w.both("zagros", &["--dry-run"]) {
        let t = text(&o);
        assert!(o.status.code() == Some(0) && t.contains("goat-map-deploy.yml -e target=zagros_hosts"), "{t}");
    }
    assert_eq!(w.calls().lines().filter(|l| l.contains("ansible-preflight.sh")).count(), 0, "a dry run runs no play");
    for o in w.both("zagros", &[]) {
        let t = text(&o);
        assert_eq!(o.status.code(), Some(0), "{t}");
        assert!(t.contains("on one host, goat_map_host=host-b") && t.contains("not expected on: host-a"), "{t}");
        assert!(t.contains("converged on sha256:808ecef0"), "{t}");
    }
}

#[test]
fn a_goatnet_that_did_not_opt_in_deploys_no_map() {
    let w = World::new();
    let before = w.calls();
    for extra in [&[][..], &["--dry-run"]] {
        for o in w.both("other", extra) {
            let t = text(&o);
            assert_ne!(o.status.code(), Some(0), "{t}");
            assert!(t.contains("does not run component") && t.contains("it is opt-in, and the goatnet does not list it under opt_in:"), "{t}");
        }
    }
    assert_eq!(w.calls(), before, "no host is reached");
}
