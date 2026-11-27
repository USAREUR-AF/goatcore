//! A component whose play is in another repository (`source:`; docs/design/
//! goatbuilder-build-targets.md section 3.10; gate J26), on both avenues: the
//! binary and the real ops/scripts/deploy-component.sh on one checkout, with a
//! synthetic repository beside it. `with-aws`, which starts every ansible tool,
//! is a stand-in that records its calls and the roles path it was given.

#[path = "../src/test_support.rs"]
mod test_support;

#[path = "common/signed_intent.rs"]
mod signed_intent;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const IMAGE: &str = "ghcr.io/example-org/svc:v1@sha256:3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c";

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

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.org", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "-c", "init.defaultBranch=main"])
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn put(root: &Path, p: &str, t: &str) {
    let p = root.join(p);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, t).unwrap();
}

struct World {
    root: tempfile::TempDir,
    repo: PathBuf,
    source: PathBuf,
    commit: String,
}

impl World {
    fn new() -> World {
        let root = tempfile::tempdir().unwrap();
        let r = &root.path().to_path_buf();
        let (repo, source) = (r.join("repo"), r.join("private/example-role"));
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        copy_tree(&real.join("ops/scripts"), &repo.join("ops/scripts"));
        put(&source, "deploy/example.yml", "- name: example\n  hosts: \"{{ target }}\"\n  roles: [example-thing]\n");
        put(&source, "roles/example-thing/tasks/main.yml", "- ansible.builtin.debug:\n    msg: hello\n");
        git(&source, &["init", "-q"]);
        git(&source, &["add", "-A"]);
        git(&source, &["commit", "-q", "-m", "one"]);
        let commit = git(&source, &["rev-parse", "HEAD"]);
        put(&repo, "ops/release/ladder_gate.py", &std::fs::read_to_string(real.join("ops/release/ladder_gate.py")).unwrap());
        // Deploy verifies the goatnet's signed intent before any host (GB-383);
        // the script avenue with the real generate-pins.py.
        copy_tree(&real.join("ops/consistency/python"), &repo.join("ops/consistency/python"));
        put(&repo, "ops/pins/generate-pins.py", &std::fs::read_to_string(real.join("ops/pins/generate-pins.py")).unwrap());
        put(&repo, "ops/secrets/posture.py", "import sys\nsys.exit(0 if sys.argv[1:2] == [\"available\"] else 2)\n");
        put(&repo, "ops/ansible/roles/goatcore-role/tasks/main.yml", "[]\n");
        put(&repo, "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nhost-a ansible_host=192.0.2.11\n");
        put(&repo, "ops/ansible/group_vars/zagros_hosts/pins.yml", &format!("svc_image: \"{IMAGE}\"\n"));
        test_support::executable(&repo.join("ops/scripts/fresh-source.sh"), "#!/bin/sh\necho 1111111111111111111111111111111111111111\n");
        test_support::executable(
            &repo.join("ops/scripts/with-aws"),
            &format!(
                r#"#!/bin/sh
printf '%s | roles=%s\n' "$*" "$ANSIBLE_ROLES_PATH" >> "$GB_STUB_DIR/calls"
shift; [ "$1" = "--" ] && shift
case "$1" in
  ansible-inventory) printf '%s\n' '{{"_meta":{{"hostvars":{{}}}},"zagros_hosts":{{"hosts":["host-a"]}}}}' ;;
  ../scripts/ansible-preflight.sh) printf 'PLAY RECAP ***\nhost-a : ok=3 changed=1 unreachable=0 failed=0\n' ;;
  ansible) printf 'host-a | CHANGED | rc=0 | (stdout) {{"ts": "t", "action": "deploy", "ref": "{IMAGE}"}}\n' ;;
esac
"#
            ),
        );
        for d in ["home", "tmp", "stub"] {
            std::fs::create_dir_all(r.join(d)).unwrap();
        }
        test_support::executable(&r.join("bin/netbird"), "#!/bin/sh\nexit 0\n");
        let w = World { repo, source, commit, root };
        w.declare(&w.commit, "deploy/example.yml");
        w
    }

    fn url(&self) -> String {
        format!("file://{}", self.source.display())
    }

    fn declare(&self, commit: &str, play: &str) {
        put(
            &self.repo,
            "ops/pins/fleet-pins.yaml",
            &format!("goatnets:\n  zagros:\n    members: [zagros_hosts]\ncomponents:\n  svc:\n    var: svc_image\n    owns_state: false\n    rollback: pin-revert\n    fleet: \"{IMAGE}\"\n    source:\n      repository: {}\n      commit: \"{commit}\"\n      play: {play}\n", self.url()),
        );
        // A declaration goatbuilder pins accepts is rendered and signed.
        if commit.len() == 40 && !play.contains("..") {
            signed_intent::prepare(&self.repo, &["zagros_hosts"]);
            signed_intent::sign(&self.repo);
        }
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

    fn both(&self, extra: &[&str]) -> [Output; 2] {
        let mut gb = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
        gb.args(["-o", "json", "--yes", "-C"]).arg(&self.repo).args(["deploy", "--env", "zagros", "--component", "svc"]).args(extra);
        let mut sh = Command::new("bash");
        sh.arg(self.repo.join("ops/scripts/deploy-component.sh")).args(["svc", "--goatnet", "zagros"]).args(extra);
        [self.run(gb), self.run(sh)]
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.root.path().join("stub/calls")).unwrap_or_default()
    }

    fn kept(&self) -> PathBuf {
        self.root.path().join("home/.goat/state/goat-sources/svc").join(&self.commit)
    }

    /// Both avenues refuse in the same words, and no host is reached.
    fn refused(&self, check: &str, says: &str) {
        let before = self.calls();
        for extra in [&[][..], &["--dry-run"]] {
            let outs = self.both(extra);
            let words: Vec<String> = outs
                .iter()
                .map(|o| {
                    let t = text(o);
                    assert_eq!(o.status.code(), Some(1), "{t}");
                    let at = t.find("refused before any host: svc's play").unwrap_or_else(|| panic!("{check}: {t}"));
                    let rest = &t[at..];
                    rest[..rest.find(&format!("(check: {check})")).unwrap_or_else(|| panic!("{check}: {t}"))].to_string()
                })
                .collect();
            assert_eq!(words[0], words[1], "{check}");
            assert!(words[0].contains(says), "{check}: {}", words[0]);
        }
        assert_eq!(self.calls(), before, "{check}: no host");
    }
}

impl World {
    /// The component pinned by a tag and the key that signs it.
    fn declare_tag(&self, tag: &str, signer: &str) {
        put(
            &self.repo,
            "ops/pins/fleet-pins.yaml",
            &format!("goatnets:\n  zagros:\n    members: [zagros_hosts]\ncomponents:\n  svc:\n    var: svc_image\n    owns_state: false\n    rollback: pin-revert\n    fleet: \"{IMAGE}\"\n    source:\n      repository: {}\n      tag: {tag}\n      signer: {signer}\n      play: deploy/example.yml\n", self.url()),
        );
        signed_intent::prepare(&self.repo, &["zagros_hosts"]);
        signed_intent::sign(&self.repo);
    }
}

/// An SSH key pair: its private key file, and its public key as `signer:`.
fn key(dir: &Path, name: &str) -> (PathBuf, String) {
    std::fs::create_dir_all(dir).unwrap();
    let at = dir.join(name);
    let o = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-C", "", "-f"]).arg(&at).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let public = std::fs::read_to_string(at.with_extension("pub")).unwrap();
    (at, public.split_whitespace().take(2).collect::<Vec<_>>().join(" "))
}

/// A component's play pinned by a signed tag, on both avenues: its signer's
/// tag deploys, and another key's is refused before any host.
#[test]
fn a_signed_tag_pins_a_components_play_by_its_signer() {
    let w = World::new();
    let ((k, signer), (other, _)) = (key(&w.root.path().join("keys"), "release"), key(&w.root.path().join("keys"), "other"));
    let signed = |key: &Path, name: &str| git(&w.source, &["-c", "gpg.format=ssh", "-c", &format!("user.signingkey={}", key.display()), "tag", "-s", name, "-m", name]);
    signed(&k, "v1");
    w.declare_tag("v1", &signer);
    // A dry run judges the tag and records nothing.
    let records = w.root.path().join("home/.goat/state/goat-source-tags");
    for o in w.both(&["--dry-run"]) {
        assert!(o.status.code() == Some(0), "{}", text(&o));
    }
    assert!(std::fs::read_dir(&records).map(|d| d.flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".accepted"))).unwrap_or(true), "a dry run wrote a record");
    let play = w.root.path().join(format!("home/.goat/state/goat-sources/svc/{}/deploy/example.yml", w.commit));
    for o in w.both(&[]) {
        assert!(o.status.code() == Some(0) && text(&o).contains("converged on sha256:3c3c"), "{}", text(&o));
    }
    assert!(w.calls().contains(&format!("-i vars-here.ini {} -e target=zagros_hosts", play.display())), "{}", w.calls());
    signed(&other, "v2");
    w.declare_tag("v2", &signer);
    w.refused("signer", "v2 is not signed by the ssh-ed25519 key");
}

fn text(o: &Output) -> String {
    let out = String::from_utf8_lossy(&o.stdout);
    match serde_json::from_str::<serde_json::Value>(&out) {
        Ok(v) if v["checks"].is_array() => v["checks"].as_array().unwrap().iter().map(|c| format!("{} {}\n", c["name"].as_str().unwrap_or(""), c["detail"].as_str().unwrap_or(""))).collect(),
        _ => format!("{out}{}", String::from_utf8_lossy(&o.stderr)),
    }
}

#[test]
fn the_play_is_fetched_at_its_commit_and_run_with_goatcores_roles_first() {
    let w = World::new();
    let play = w.kept().join("deploy/example.yml");
    for o in w.both(&["--dry-run"]) {
        let t = text(&o);
        assert!(o.status.code() == Some(0) && t.contains(&format!("-i vars-here.ini {} -e target=zagros_hosts", play.display())), "{t}");
    }
    assert_eq!(w.calls(), "", "a dry run reaches no host");
    // The repository moves on; the pinned commit is still what runs.
    put(&w.source, "deploy/example.yml", "- name: moved on\n  hosts: all\n  tasks: []\n");
    git(&w.source, &["commit", "-qam", "two"]);
    for o in w.both(&[]) {
        assert!(o.status.code() == Some(0) && text(&o).contains("converged on sha256:3c3c"), "{}", text(&o));
    }
    assert!(std::fs::read_to_string(&play).unwrap().contains("name: example"));
    let roles = format!("roles={}:{}:{}", w.repo.join("ops/ansible/roles").display(), w.kept().join("deploy/roles").display(), w.kept().join("roles").display());
    let calls = w.calls();
    let plays: Vec<&str> = calls.lines().filter(|l| l.contains("ansible-preflight.sh")).collect();
    assert!(plays.len() == 2 && plays.iter().all(|p| p.contains(&format!("-i vars-here.ini {} -e target=zagros_hosts", play.display())) && p.ends_with(&roles)), "{plays:?}\n{roles}");
    // Nothing of it is in the checkout.
    assert!(!w.repo.join("deploy").exists() && !w.repo.join("ops/ansible/example.yml").exists());
}

#[test]
fn a_source_that_is_not_exactly_the_commit_is_refused_before_any_host() {
    let w = World::new();
    w.both(&["--dry-run"]);
    std::fs::write(w.kept().join("deploy/example.yml"), "- name: changed here\n  hosts: all\n  tasks: []\n").unwrap();
    w.refused("commit", "is not the commit's content");
    git(&w.kept(), &["checkout", "-q", "--", "."]);
    put(&w.kept(), "roles/extra/tasks/main.yml", "[]\n");
    w.refused("commit", "is not the commit's content");
    std::fs::remove_dir_all(w.kept().join("roles/extra")).unwrap();
    git(&w.kept(), &["commit", "-q", "--allow-empty", "-m", "moved"]);
    w.refused("commit", " is at ");
}

#[test]
fn an_unreachable_repository_a_missing_play_and_a_replaced_role_are_each_refused() {
    let w = World::new();
    w.declare(&"e".repeat(40), "deploy/example.yml");
    w.refused("fetch", "not fetched with this person's access");
    assert!(std::fs::read_dir(w.kept().parent().unwrap()).unwrap().flatten().next().is_none(), "nothing is left behind");
    w.declare(&w.commit, "deploy/other.yml");
    w.refused("play", "it has no file deploy/other.yml");
    put(&w.source, "roles/goatcore-role/tasks/main.yml", "[]\n");
    git(&w.source, &["add", "-A"]);
    git(&w.source, &["commit", "-q", "-m", "a role of goatcore's"]);
    let w2 = World { commit: git(&w.source, &["rev-parse", "HEAD"]), ..w };
    w2.declare(&w2.commit, "deploy/example.yml");
    w2.refused("role", "it carries a role goatcore-role, which goatcore has");
    // Variables beside the play would win over the checkout's, and a link's
    // target is not the commit's: both are refused on both avenues.
    git(&w2.source, &["rm", "-rq", "roles/goatcore-role"]);
    put(&w2.source, "deploy/group_vars/all.yml", "ca_id: another\n");
    git(&w2.source, &["add", "-A"]);
    git(&w2.source, &["commit", "-q", "-m", "variables of its own"]);
    let w3 = World { commit: git(&w2.source, &["rev-parse", "HEAD"]), ..w2 };
    w3.declare(&w3.commit, "deploy/example.yml");
    w3.refused("tree", "deploy/group_vars/all.yml is in a directory group_vars/ of its own");
    git(&w3.source, &["rm", "-rq", "deploy/group_vars"]);
    std::os::unix::fs::symlink("/etc", w3.source.join("deploy/outside")).unwrap();
    git(&w3.source, &["add", "-A"]);
    git(&w3.source, &["commit", "-q", "-m", "a link"]);
    let w2 = World { commit: git(&w3.source, &["rev-parse", "HEAD"]), ..w3 };
    w2.declare(&w2.commit, "deploy/example.yml");
    w2.refused("tree", "deploy/outside is a link");
    std::fs::rename(&w2.source, w2.root.path().join("private/gone")).unwrap();
    std::fs::remove_dir_all(w2.kept()).unwrap();
    w2.refused("fetch", "not fetched with this person's access");
}

/// A `source:` key with an empty value is a source on both avenues: refused
/// (GB-385), never read as no source and deployed from goatcore's own play.
#[test]
fn an_empty_source_is_refused_on_both_avenues_and_not_taken_for_none() {
    let w = World::new();
    put(&w.repo, "ops/ansible/svc-deploy.yml", "- name: goatcore's own play\n  hosts: \"{{ target }}\"\n  tasks: []\n");
    let table = w.repo.join("ops/pins/fleet-pins.yaml");
    let good = std::fs::read_to_string(&table).unwrap();
    let at = good.find("    source:\n").unwrap();
    let before = w.calls();
    for empty in ["{}", "null", "\"\""] {
        std::fs::write(&table, format!("{}    source: {empty}\n", &good[..at])).unwrap();
        for extra in [&[][..], &["--dry-run"]] {
            for o in w.both(extra) {
                let t = text(&o);
                assert!(o.status.code() == Some(1) && t.contains("GB-385"), "source: {empty}: {t}");
                assert!(!t.contains("svc-deploy.yml"), "source: {empty} took goatcore's play: {t}");
            }
        }
    }
    assert_eq!(w.calls(), before, "no host");
}

#[test]
fn both_resolvers_refuse_a_malformed_source_in_the_same_words() {
    let w = World::new();
    let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let table = w.repo.join("ops/pins/fleet-pins.yaml");
    let good = std::fs::read_to_string(&table).unwrap();
    for (from, to, says) in [
        (w.commit.as_str(), "main", "a branch or a tag can move"),
        ("play: deploy/example.yml", "play: ../x.yml", "a play's path inside the repository"),
        ("play: deploy/example.yml", "play: deploy/example.sh", "a play's path inside the repository"),
        (w.url().as_str(), "https://user:secret@git.example.invalid/x", "with no credential in it"),
        (w.url().as_str(), "ext::sh -c id", "is a git URL"),
        (w.url().as_str(), "ssh://user:secret@git.example.invalid/x", "with no credential in it"),
        ("      play:", "      branch: main\n      play:", "unknown key 'branch'"),
        (&format!("      commit: \"{}\"\n", w.commit), "      tag: v1\n", "is pinned only by its signer"),
        (&format!("      commit: \"{}\"\n", w.commit), "      tag: v1\n      signer: ssh-rsa x\n", "is an SSH public key"),
        ("      play:", "      tag: v1\n      play:", "not both"),
        ("      play:", "      signer: ssh-ed25519 AAAA\n      play:", "and there is no tag"),
        (&format!("      commit: \"{}\"\n", w.commit), "      tag: \"on\"\n      signer: ssh-ed25519 AAAA\n", "is the name of a tag"),
        (&format!("      commit: \"{}\"\n", w.commit), "      tag: 1e5\n      signer: ssh-ed25519 AAAA\n", "is the name of a tag"),
        (&format!("      commit: \"{}\"\n", w.commit), "      commit: \"0000000000000000000000000000000000000e12\"\n", "a full commit of 40 hexadecimal characters"),
        ("    source:\n", "    deploy_playbook: x.yml\n    source:\n", "declares both deploy_playbook and source"),
    ] {
        std::fs::write(&table, good.replacen(from, to, 1)).unwrap();
        let mut gb = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
        gb.arg("-C").arg(&w.repo).args(["pins", "--check"]);
        let rust = text(&w.run(gb));
        let mut py = Command::new("python3");
        py.args(["-c", "import sys, yaml; sys.path.insert(0, sys.argv[1]); from goat_consistency import resolve\ntry: resolve.check_source('svc', yaml.safe_load(open(sys.argv[2]))['components']['svc'])\nexcept (SystemExit, Exception) as e: print(e)"]).arg(real.join("ops/consistency/python")).arg(&table);
        let python = text(&w.run(py));
        for (t, who) in [(&rust, "goatbuilder"), (&python, "resolve.py")] {
            assert!(t.contains(says), "{who}, {to}: {t}");
        }
        assert!(!rust.contains("secret@") && !python.contains("secret@"), "no credential is printed");
    }
}
