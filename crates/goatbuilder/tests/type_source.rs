//! A target type whose plays are in another repository at a full commit (the
//! type's own `source:`; docs/design/goatbuilder-build-targets.md section 3.10;
//! gate J26), through the built binary, with a synthetic overlay type and a
//! synthetic repository beside the checkout. `with-aws`, which starts every
//! ansible tool, is a stand-in that records its calls and the roles path it
//! was given. A refusal reaches no host: the stand-in records nothing.

#[path = "../src/test_support.rs"]
mod test_support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const GROUP: &str = "zagros_example_role_hosts";
const TYPE: &str = "ops/goatbuilder/overlay/targets/example-role.yaml";

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
        let r = root.path().to_path_buf();
        let (repo, source) = (r.join("repo"), r.join("private/example-role"));
        put(&source, "deploy/example-role.yml", "- name: example role\n  hosts: \"{{ target }}\"\n  roles: [example-thing]\n");
        put(&source, "roles/example-thing/tasks/main.yml", "- ansible.builtin.debug:\n    msg: hello\n");
        git(&source, &["init", "-q"]);
        git(&source, &["add", "-A"]);
        git(&source, &["commit", "-q", "-m", "one"]);
        let commit = git(&source, &["rev-parse", "HEAD"]);
        put(
            &repo,
            "ops/goatbuilder/goatbuilder.yaml",
            "schema: goatbuilder/v1\nrepo: {source: example-org/example-repo, image_namespace: ghcr.io/example-org}\ncomponents:\n  svc: {declared_in: ops/pins/fleet-pins.yaml}\nenvironments:\n  - name: zagros\n    converger: push-ansible\npromotion:\n  path:\n    - {name: zagros, kind: rung, environment: zagros}\ndeploy: {ssh_user: goat}\n",
        );
        put(&repo, "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\n");
        put(&repo, "ops/ansible/roles/goatcore-role/tasks/main.yml", "[]\n");
        // A role of goatcore's that reads a file beside the playbook, as many do.
        put(&repo, "ops/ansible/roles/goatcore-role/defaults/main.yml", "goatcore_role_ca: \"{{ playbook_dir }}/../enrollment/ca/root-cert.pem\"\n");
        put(&repo, "ops/ansible/inventory/mgmt-hosts.ini", &format!("[zagros_hosts]\nmgmt-zagros-01-a\n[{GROUP}]\nhost-a ansible_host=192.0.2.11\n"));
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        put(&repo, "ops/scripts/inventory-hosts.py", &std::fs::read_to_string(real.join("ops/scripts/inventory-hosts.py")).unwrap());
        test_support::executable(&repo.join("ops/scripts/with-aws"), "#!/bin/sh\nprintf '%s | roles=%s\\n' \"$*\" \"$ANSIBLE_ROLES_PATH\" >> \"$GB_STUB_DIR/calls\"\n");
        for d in ["home", "tmp", "stub"] {
            std::fs::create_dir_all(r.join(d)).unwrap();
        }
        let w = World { root, repo, source, commit };
        w.declare(&w.url(), &w.commit, "deploy/example-role.yml");
        w
    }

    fn url(&self) -> String {
        format!("file://{}", self.source.display())
    }

    fn declare(&self, url: &str, commit: &str, play: &str) {
        put(
            &self.repo,
            TYPE,
            &format!("schema: goatbuilder-target/v1\ntype: example-role\nfamilies: [host-class]\nsource:\n  repository: {url}\n  commit: \"{commit}\"\nphases:\n  - label: the example role\n    plays:\n      - {{play: {play}, args: \"-e target={{group}}\", from: source}}\n"),
        );
    }

    /// Where a run keeps the source: under the state directory.
    fn kept(&self) -> PathBuf {
        self.root.path().join("home/.goat/state/goat-type-sources/example-role").join(&self.commit)
    }

    fn join(&self, extra: &[&str]) -> Output {
        self.join_from(&self.repo, &self.repo, extra)
    }

    /// join-node with `-C <checkout>`, started in `cwd`.
    fn join_from(&self, cwd: &Path, checkout: &Path, extra: &[&str]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
        c.args(["-o", "json", "--yes", "-C"]).arg(checkout);
        c.args(["join-node", "zagros", "--site", "zagros", "--node", "host-a", "--class", "example-role", "--address", "192.0.2.11"]).args(extra);
        self.run(c, cwd)
    }

    fn run(&self, mut c: Command, cwd: &Path) -> Output {
        c.env_clear();
        self.run_keeping(c, cwd)
    }

    /// As [`World::run`], keeping what `c` already sets.
    fn run_keeping(&self, mut c: Command, cwd: &Path) -> Output {
        let r = self.root.path();
        c
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", r.join("home"))
            .env("TMPDIR", r.join("tmp"))
            .env("GB_STUB_DIR", r.join("stub"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("GOAT_MESH_PROBE", "0")
            .env("GOAT_DOOR_OPEN", "0")
            .env("GOAT_NO_FUN", "1")
            .env("GOAT_EXPLAIN", "0")
            .env("GOAT_LANG", "en")
            .env("NO_COLOR", "1")
            .env("GOATBUILDER_BIN", env!("CARGO_BIN_EXE_goatbuilder"))
            .current_dir(cwd);
        c.output().unwrap()
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.root.path().join("stub/calls")).unwrap_or_default()
    }
}

/// What a run said: each row's name and detail as plain text when it printed
/// its report, else its output as it is.
fn text(o: &Output) -> String {
    let out = String::from_utf8_lossy(&o.stdout);
    let rows = serde_json::from_str::<serde_json::Value>(&out).ok().and_then(|v| v["checks"].as_array().cloned());
    let shown = match rows {
        Some(rows) => rows.iter().map(|c| format!("{} {}\n", c["name"].as_str().unwrap_or(""), c["detail"].as_str().unwrap_or(""))).collect(),
        None => out.to_string(),
    };
    format!("{shown}{}", String::from_utf8_lossy(&o.stderr))
}

#[test]
fn the_types_play_is_fetched_at_its_commit_and_run_with_goatcores_roles_first() {
    let w = World::new();
    let play = w.kept().join("deploy/example-role.yml");
    let roles = format!("{}:{}:{}", w.repo.join("ops/ansible/roles").display(), w.kept().join("deploy/roles").display(), w.kept().join("roles").display());
    // A plan names where a run keeps it, and fetches nothing.
    let o = w.join(&["--dry-run"]);
    let t = text(&o);
    assert_eq!(o.status.code(), Some(0), "{t}");
    let checked = format!("\"${{GOATBUILDER_BIN:-goatbuilder}}\" -C '{}' type-source example-role deploy/example-role.yml --commit {} && ANSIBLE_ROLES_PATH='{roles}' ", w.repo.display(), w.commit);
    assert!(t.contains(&checked) && t.contains(&format!("-i vars-here.ini '{}' -e target={GROUP} -l host-a", play.display())), "{t}");
    assert!(!w.kept().exists() && w.calls().is_empty(), "a plan fetches nothing and reaches no host");
    // A checkout named relative to where goatbuilder starts: every path the
    // play is given is absolute, for it runs from ops/ansible.
    // (The working directory is read as the system names it: on macOS the
    // temporary directory is behind a link.)
    let o = w.join_from(w.root.path(), Path::new("repo"), &["--dry-run"]);
    let t = text(&o);
    let real = w.repo.canonicalize().unwrap();
    let roles_here = roles.replacen(&w.repo.display().to_string(), &real.display().to_string(), 1);
    assert!(o.status.success() && t.contains(&format!("ANSIBLE_ROLES_PATH='{roles_here}' ")), "{t}");
    // The repository moves on; the pinned commit is still what runs.
    put(&w.source, "deploy/example-role.yml", "- name: moved on\n  hosts: all\n");
    git(&w.source, &["commit", "-qam", "two"]);
    let o = w.join(&["--from", "13", "--to", "13"]);
    let t = text(&o);
    assert_eq!(o.status.code(), Some(0), "{t}");
    let calls = w.calls();
    let plays: Vec<&str> = calls.lines().filter(|l| l.contains("example-role.yml")).collect();
    assert!(plays.len() == 1 && plays[0].contains(&format!("-i vars-here.ini {} -e target={GROUP} -l host-a", play.display())) && plays[0].ends_with(&format!("roles={roles}")), "{calls}");
    assert!(std::fs::read_to_string(&play).unwrap().contains("example role"), "the commit's play, not the branch's");
    // Nothing of it is in the checkout.
    assert!(!w.repo.join("deploy").exists() && !w.repo.join("ops/ansible/example-role.yml").exists());
}

#[test]
fn a_source_that_cannot_be_taken_is_refused_before_any_phase_gb420() {
    let w = World::new();
    let refused = |why: &str, check: &str| {
        let o = w.join(&["--from", "13", "--to", "13"]);
        let t = text(&o);
        assert!(!o.status.success() && t.contains("GB-420") && t.contains(why) && t.contains(&format!("(check: {check})")) && !t.contains("=== Phase"), "{why}: {t}");
        assert_eq!(w.calls(), "", "{why}: no host is reached");
    };
    w.declare("file:///nonexistent/example-role", &w.commit, "deploy/example-role.yml");
    refused("not fetched with this person's access", "fetch");
    w.declare(&w.url(), &"a".repeat(40), "deploy/example-role.yml");
    refused("not fetched with this person's access", "fetch");
    w.declare(&w.url(), &w.commit, "deploy/other.yml");
    refused("it has no file deploy/other.yml", "play");
    // A role of goatcore's is not replaced.
    put(&w.source, "roles/goatcore-role/tasks/main.yml", "[]\n");
    git(&w.source, &["add", "-A"]);
    git(&w.source, &["commit", "-qm", "a role goatcore has"]);
    let with_role = git(&w.source, &["rev-parse", "HEAD"]);
    w.declare(&w.url(), &with_role, "deploy/example-role.yml");
    refused("carries a role goatcore-role, which goatcore has", "role");
    // The copy a run kept is held to its commit on every run.
    w.declare(&w.url(), &w.commit, "deploy/example-role.yml");
    assert!(w.join(&["--from", "13", "--to", "13"]).status.success());
    std::fs::remove_file(w.root.path().join("stub/calls")).unwrap();
    put(&w.kept(), "deploy/example-role.yml", "- name: changed here\n  hosts: all\n");
    refused("is not the commit's content", "commit");
    std::fs::remove_dir_all(w.kept()).unwrap();
    put(&w.kept(), "x", "not a clone\n");
    refused(".git is not a directory of its own", "commit");
}

#[test]
fn a_type_file_that_misdeclares_its_source_is_refused_gb374() {
    let w = World::new();
    let refused = |body: &str, says: &str| {
        put(&w.repo, TYPE, body);
        let o = w.join(&["--dry-run"]);
        let t = text(&o);
        assert!(!o.status.success() && t.contains("GB-374") && t.contains(says), "{says}: {t}");
    };
    let good = std::fs::read_to_string(w.repo.join(TYPE)).unwrap();
    refused(&good.replace("file://", "https://user:secret@"), "no credential in it");
    refused(&good.replace(&w.commit, "main"), "commit");
    refused(&good.replace("from: source", "from: overlay"), "only a play from: source has a path");
    refused(&good.replace("deploy/example-role.yml", "example-role.yml").replace("from: source", "from: overlay"), "no play is from: source");
    let no_source: String = good.lines().filter(|l| !l.starts_with("source:") && !l.starts_with("  repository:") && !l.starts_with("  commit:")).map(|l| format!("{l}\n")).collect();
    refused(&no_source, "the type declares no source:");
    refused(&good.replace("deploy/example-role.yml", "../example-role.yml"), "does not match");
}

/// standup builds a phase-14 type the same way: its plan names the kept copy,
/// and a run that cannot take the source is refused before its first phase.
#[test]
fn standup_takes_a_phase_14_types_plays_from_its_source() {
    let w = World::new();
    let body = std::fs::read_to_string(w.repo.join(TYPE)).unwrap();
    put(&w.repo, TYPE, &body.replace("phases:", "site: {declared_by: has_example_role, standup: phase-14}\nphases:"));
    put(&w.repo, "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\nhas_example_role: true\n");
    let standup = |extra: &[&str]| {
        let r = w.root.path();
        let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
        c.args(["-o", "json", "--yes", "standup", "-C"]).arg(&w.repo).args(["zagros", "--site", "zagros", "--profile", "lan"]).args(extra);
        c.env_clear().env("PATH", std::env::var("PATH").unwrap_or_default()).env("HOME", r.join("home")).env("TMPDIR", r.join("tmp")).env("GB_STUB_DIR", r.join("stub")).env("PYTHONDONTWRITEBYTECODE", "1").env("GOAT_MESH_PROBE", "0").env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1");
        c.output().unwrap()
    };
    let o = standup(&["--dry-run"]);
    let t = text(&o);
    assert!(o.status.success() && t.contains("=== Phase 14 [A] the example role ===") && t.contains(&format!("-i vars-here.ini '{}' -e target={GROUP}", w.kept().join("deploy/example-role.yml").display())), "{t}");
    w.declare("file:///nonexistent/example-role", &w.commit, "deploy/example-role.yml");
    let body = std::fs::read_to_string(w.repo.join(TYPE)).unwrap();
    put(&w.repo, TYPE, &body.replace("phases:", "site: {declared_by: has_example_role, standup: phase-14}\nphases:"));
    let o = standup(&[]);
    let t = text(&o);
    assert!(!o.status.success() && t.contains("GB-420") && t.contains("(check: fetch)") && !t.contains("=== Phase"), "{t}");
    assert_eq!(w.calls(), "", "no host is reached");
}

/// A relative deploy.state_dir is the checkout's: the plan, the check before
/// phase 1 and the phase's own check all name the one copy, wherever each runs.
#[test]
fn a_relative_state_dir_is_the_checkouts() {
    let w = World::new();
    let cfg = w.repo.join("ops/goatbuilder/goatbuilder.yaml");
    let text_cfg = std::fs::read_to_string(&cfg).unwrap();
    std::fs::write(&cfg, text_cfg.replace("deploy: {ssh_user: goat}", "deploy: {ssh_user: goat, state_dir: local-state}")).unwrap();
    let kept = w.repo.join("local-state/goat-type-sources/example-role").join(&w.commit);
    let o = w.join(&["--from", "13", "--to", "13"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(kept.join(".git").is_dir() && w.calls().contains(&format!("{} -e target={GROUP}", kept.join("deploy/example-role.yml").display())), "{}", w.calls());
    assert!(!w.repo.join("ops/ansible/local-state").exists(), "no copy is fetched inside ops/ansible");
}

/// The phase's check takes the commit its plan named: a type file that pins
/// another commit by then is refused, and the play does not run.
#[test]
fn a_type_file_changed_during_a_run_is_refused_before_the_play() {
    let w = World::new();
    let t = text(&w.join(&["--dry-run"]));
    let cmd = t.lines().filter_map(|l| l.strip_prefix("cmd ")).find(|l| l.contains("type-source")).expect("the phase's command").to_string();
    assert!(w.join(&["--from", "13", "--to", "13"]).status.success());
    let ran = w.calls().lines().count();
    put(&w.source, "deploy/example-role.yml", "- name: two\n  hosts: all\n");
    git(&w.source, &["commit", "-qam", "two"]);
    let two = git(&w.source, &["rev-parse", "HEAD"]);
    w.declare(&w.url(), &two, "deploy/example-role.yml");
    let mut c = Command::new("bash");
    c.args(["-euo", "pipefail", "-c", &cmd]);
    let o = w.run(c, &w.repo);
    assert!(!o.status.success() && text(&o).contains("GB-420") && text(&o).contains(&format!("this run was planned with commit {}", w.commit)), "{}", text(&o));
    assert_eq!(w.calls().lines().count(), ran, "the play did not run");
}

/// A source carries nothing Ansible would read beside its play in place of
/// the checkout's: variables, modules, plugins, collections, a role of
/// goatcore's name anywhere, a link or a submodule. Each is refused before any
/// phase, and the kept copy is held to its commit again just before its play.
#[test]
fn a_source_that_would_change_goatcores_meaning_is_refused_gb420() {
    let w = World::new();
    for file in [
        ("deploy/group_vars/all.yml", "tree", "deploy/group_vars/all.yml is in a directory group_vars/ of its own"),
        ("host_vars/host-a.yml", "tree", "directory host_vars/ of its own"),
        ("deploy/library/ping.py", "tree", "directory library/ of its own"),
        ("deploy/filter_plugins/f.py", "tree", "directory filter_plugins/ of its own"),
        ("deploy/collections/ansible_collections/x/y/plugins/modules/m.py", "tree", "directory collections/ of its own"),
        ("other/roles/goatcore-role/tasks/main.yml", "role", "it carries a role goatcore-role, which goatcore has"),
        // A file system that ignores case finds these by goatcore's names.
        ("roles/GoatCore-Role/tasks/main.yml", "role", "it carries a role GoatCore-Role, which goatcore has"),
        ("deploy/Group_Vars/all.yml", "tree", "directory Group_Vars/ of its own"),
        ("ex\u{0101}mple/notes.md", "tree", "whose name is not plain ASCII"),
        // Beside the play, where goatcore-role reads {{ playbook_dir }}/../enrollment/ca/.
        ("enrollment/ca/root-cert.pem", "tree", "enrollment/ca/root-cert.pem is where goatcore's roles read {{ playbook_dir }}/../enrollment/ca/root-cert.pem from"),
    ]
    .into_iter()
    .enumerate()
    {
        // One directory for each, by number: a file system that ignores case
        // would take two names that differ in case for one.
        let (i, (file, check, says)) = (file.0, file.1);
        let at = w.root.path().join(format!("src-{i}"));
        copy(&w.source, &at);
        put(&at, file, "x: 1\n");
        git(&at, &["add", "-A"]);
        git(&at, &["commit", "-qm", "one more"]);
        let commit = git(&at, &["rev-parse", "HEAD"]);
        w.declare(&format!("file://{}", at.display()), &commit, "deploy/example-role.yml");
        let o = w.join(&["--from", "13", "--to", "13"]);
        let t = text(&o);
        assert!(!o.status.success() && t.contains("GB-420") && t.contains(says) && t.contains(&format!("(check: {check})")), "{file}: {t}");
    }
    let at = w.root.path().join("src-link");
    copy(&w.source, &at);
    std::os::unix::fs::symlink("/etc", at.join("outside")).unwrap();
    git(&at, &["add", "-A"]);
    git(&at, &["commit", "-qm", "a link"]);
    let commit = git(&at, &["rev-parse", "HEAD"]);
    w.declare(&format!("file://{}", at.display()), &commit, "deploy/example-role.yml");
    let t = text(&w.join(&["--from", "13", "--to", "13"]));
    assert!(t.contains("GB-420") && t.contains("outside is a link") && t.contains("(check: tree)"), "{t}");
    assert_eq!(w.calls(), "", "no host is reached");
    // Between the check before phase 1 and the play, the copy changes: the
    // phase's own check refuses it, and the play does not run.
    w.declare(&w.url(), &w.commit, "deploy/example-role.yml");
    let t = text(&w.join(&["--dry-run"]));
    let cmd = t.lines().filter_map(|l| l.strip_prefix("cmd ")).find(|l| l.contains("type-source")).expect("the phase's command").to_string();
    assert!(w.join(&["--from", "13", "--to", "13"]).status.success());
    let ran = w.calls().lines().count();
    put(&w.kept(), "deploy/example-role.yml", "- name: changed here\n  hosts: all\n");
    let mut c = Command::new("bash");
    c.args(["-euo", "pipefail", "-c", &cmd]);
    let o = w.run(c, &w.repo);
    assert!(!o.status.success() && text(&o).contains("GB-420") && text(&o).contains("is not the commit's content"), "{cmd}\n{}", text(&o));
    assert_eq!(w.calls().lines().count(), ran, "the changed play did not run");
    // A kept copy's git holds only what goatbuilder made.
    for (file, text, says) in [("config", "[core]\n\tworktree = /\n", "sets core.worktree"), ("config", "[core] worktree = /\n", "sets core.worktree"), ("info/attributes", "* filter=x\n", ".git/info/attributes is there")] {
        std::fs::remove_dir_all(w.kept()).unwrap();
        assert!(w.join(&["--from", "13", "--to", "13"]).status.success());
        let p = w.kept().join(".git").join(file);
        let mut was = std::fs::read_to_string(&p).unwrap_or_default();
        was.push_str(text);
        put(&w.kept().join(".git"), file, &was);
        let t = text_of_run(&w);
        assert!(t.contains("GB-420") && t.contains(says) && t.contains("(check: commit)"), "{file}: {t}");
    }
    // A file the index is told not to compare is not taken on its word.
    std::fs::remove_dir_all(w.kept()).unwrap();
    assert!(w.join(&["--from", "13", "--to", "13"]).status.success());
    git(&w.kept(), &["update-index", "--assume-unchanged", "deploy/example-role.yml"]);
    put(&w.kept(), "deploy/example-role.yml", "- name: changed, the index told not to look\n  hosts: all\n");
    let t = text(&w.join(&["--from", "13", "--to", "13"]));
    assert!(t.contains("GB-420") && t.contains("so that git does not compare it with the commit") && t.contains("(check: commit)"), "{t}");
    std::fs::remove_dir_all(w.kept()).unwrap();
    // A git that runs goatbuilder exports where its own repository is: the
    // source is still fetched into its own copy, and that repository is left as it was.
    let decoy = w.root.path().join("decoy");
    std::fs::create_dir_all(&decoy).unwrap();
    git(&decoy, &["init", "-q"]);
    git(&decoy, &["commit", "-q", "--allow-empty", "-m", "the operator's own"]);
    let head = git(&decoy, &["rev-parse", "HEAD"]);
    let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
    c.args(["-o", "json", "--yes", "-C"]).arg(&w.repo).args(["join-node", "zagros", "--site", "zagros", "--node", "host-a", "--class", "example-role", "--address", "192.0.2.11", "--from", "13", "--to", "13"]);
    // And a template for new repositories whose configuration runs a program
    // on every status: nothing of it is taken.
    let template = w.root.path().join("template");
    let marker = w.root.path().join("monitor-ran");
    put(&template, "config", &format!("[core]\n\tfsmonitor = touch {}\n", marker.display()));
    c.env_clear().env("GIT_DIR", decoy.join(".git")).env("GIT_WORK_TREE", &decoy).env("GIT_TEMPLATE_DIR", &template);
    let o = w.run_keeping(c, &w.repo);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(git(&decoy, &["rev-parse", "HEAD"]), head, "the exported repository is left as it was");
    assert!(!marker.exists(), "no program a template names ran");
    assert!(w.kept().join(".git").is_dir() && std::fs::read_to_string(w.kept().join("deploy/example-role.yml")).unwrap().contains("example role"));
}

fn copy(from: &Path, to: &Path) {
    let o = Command::new("cp").arg("-a").arg(from).arg(to).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// An SSH key pair in `dir`: its private key file, and its public key as a
/// source's `signer:` names it (type and text, no comment).
fn key(dir: &Path, name: &str) -> (PathBuf, String) {
    let at = dir.join(name);
    let o = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-C", "", "-f"]).arg(&at).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let public = std::fs::read_to_string(at.with_extension("pub")).unwrap();
    (at, public.split_whitespace().take(2).collect::<Vec<_>>().join(" "))
}

/// `git tag` in `dir`, signing with `key` when there is one.
fn tag(dir: &Path, key: Option<&Path>, args: &[&str]) {
    tag_at(dir, key, args, "2026-11-01T00:00:00Z");
}

/// `git tag`, made at `when`.
fn tag_at(dir: &Path, key: Option<&Path>, args: &[&str], when: &str) {
    let mut c = Command::new("git");
    c.args(["-c", "user.name=t", "-c", "user.email=t@example.org", "-c", "gpg.format=ssh"]).arg("-C").arg(dir);
    if let Some(k) = key {
        c.arg("-c").arg(format!("user.signingkey={}", k.display()));
    }
    let o = c.arg("tag").args(args).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_COMMITTER_DATE", when).output().unwrap();
    assert!(o.status.success(), "git tag {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

impl World {
    fn declare_tag(&self, tag: &str, signer: Option<&str>) {
        let signer = signer.map(|s| format!("  signer: {s}\n")).unwrap_or_default();
        put(
            &self.repo,
            TYPE,
            &format!("schema: goatbuilder-target/v1\ntype: example-role\nfamilies: [host-class]\nsource:\n  repository: {}\n  tag: {tag}\n{signer}phases:\n  - label: the example role\n    plays:\n      - {{play: deploy/example-role.yml, args: \"-e target={{group}}\", from: source}}\n", self.url()),
        );
    }

    /// Where a run keeps the copy at `commit`.
    fn kept_at(&self, commit: &str) -> PathBuf {
        self.root.path().join("home/.goat/state/goat-type-sources/example-role").join(commit)
    }

    /// The record of the last `tag` signed by `signer` a run took: one for the
    /// tag's name with the repository and the signer it is from.
    fn record(&self, tag: &str, signer: &str) -> PathBuf {
        use sha2::Digest;
        let of: String = sha2::Sha256::digest(format!("{}\n{signer}", self.url())).iter().take(6).map(|b| format!("{b:02x}")).collect();
        self.root.path().join("home/.goat/state/goat-source-tags").join(format!("tag-{}-{of}.accepted", tag.replace('/', "%2F")))
    }
}

/// A signed tag pins a source by its signer: the commit it names now runs,
/// once its SSH signature is the declared key's. A tag that is missing,
/// unsigned or lightweight, signed by another key, or another tag under its
/// name, is refused before any phase; a tag with no signer is refused by the
/// loader as a ref that can move.
#[test]
fn a_signed_tag_pins_a_types_source_by_its_signer() {
    let w = World::new();
    let keys = w.root.path().join("keys");
    std::fs::create_dir_all(&keys).unwrap();
    let ((k, signer), (other, _)) = (key(&keys, "release"), key(&keys, "other"));
    tag(&w.source, Some(&k), &["-s", "rel/v1", "-m", "release one"]);
    let first = git(&w.source, &["rev-parse", "refs/tags/rel/v1"]);
    w.declare_tag("rel/v1", Some(&signer));
    // A plan cannot know the commit a tag will name: the play is run from
    // the copy its check names when it runs.
    let o = w.join(&["--dry-run"]);
    let t = text(&o);
    let roles = format!("ANSIBLE_ROLES_PATH='{}'\":$root/deploy/roles:$root/roles\"", w.repo.join("ops/ansible/roles").display());
    assert!(o.status.success() && t.contains("root=$(\"${GOATBUILDER_BIN:-goatbuilder}\" -C ") && t.contains(&roles) && t.contains(&format!("-i vars-here.ini \"$root/deploy/example-role.yml\" -e target={GROUP} -l host-a")), "{t}");
    let cmd = t.lines().filter_map(|l| l.strip_prefix("cmd ")).find(|l| l.contains("type-source")).expect("the phase's command").to_string();
    let run = || w.join(&["--from", "13", "--to", "13"]);
    let o = run();
    assert!(o.status.success(), "{}", text(&o));
    // The phase's check holds the type file to the pin its plan was made with:
    // the type file pinned another way by then is refused, and nothing runs.
    let ran = w.calls().lines().count();
    let pinned = std::fs::read_to_string(w.repo.join(TYPE)).unwrap();
    w.declare(&w.url(), &w.commit, "deploy/example-role.yml");
    let mut c = Command::new("bash");
    c.args(["-euo", "pipefail", "-c", &cmd]);
    let o2 = w.run(c, &w.repo);
    assert!(!o2.status.success() && text(&o2).contains("is not the one this run was planned with"), "{}", text(&o2));
    assert_eq!(w.calls().lines().count(), ran, "the play did not run");
    put(&w.repo, TYPE, &pinned);
    let play = w.kept_at(&w.commit).join("deploy/example-role.yml");
    assert!(w.calls().contains(&format!("{} -e target={GROUP}", play.display())) && std::fs::read_to_string(&play).unwrap().contains("example role"), "{}", w.calls());
    // The signer tags another commit: that commit's copy is the one run.
    put(&w.source, "deploy/example-role.yml", "- name: release two\n  hosts: \"{{ target }}\"\n  roles: [example-thing]\n");
    git(&w.source, &["commit", "-qam", "two"]);
    let two = git(&w.source, &["rev-parse", "HEAD"]);
    tag_at(&w.source, Some(&k), &["-f", "-s", "rel/v1", "-m", "release one, again"], "2026-11-02T00:00:00Z");
    assert!(run().status.success());
    let play = w.kept_at(&two).join("deploy/example-role.yml");
    assert!(w.calls().contains(&format!("{} -e target={GROUP}", play.display())) && std::fs::read_to_string(&play).unwrap().contains("release two"), "the commit the tag names now");
    // The signer's older tag, served again under the name, is not taken.
    let second = git(&w.source, &["rev-parse", "refs/tags/rel/v1"]);
    git(&w.source, &["update-ref", "refs/tags/rel/v1", &first]);
    let t = text(&run());
    assert!(t.contains("GB-420") && t.contains("an older signed tag served again") && t.contains("(check: tag)"), "{t}");
    assert!(w.kept_at(&w.commit).is_dir() && w.kept_at(&two).is_dir(), "no run removes a copy another may use");
    git(&w.source, &["update-ref", "refs/tags/rel/v1", &second]);
    // A record of the tag taken that cannot be read is refused, never read as none.
    let record = w.record("rel/v1", &signer);
    let kept = std::fs::read_to_string(&record).unwrap();
    std::fs::write(&record, "").unwrap();
    let t = text(&run());
    assert!(t.contains("GB-420") && t.contains("the record of the tag an earlier run took, cannot be read"), "{t}");
    std::fs::write(&record, kept).unwrap();
    assert!(run().status.success());
    let before = w.calls();
    let refused = |why: &str, check: &str| {
        let o = run();
        let t = text(&o);
        assert!(!o.status.success() && t.contains("GB-420") && t.contains(why) && t.contains(&format!("(check: {check})")) && !t.contains("=== Phase"), "{why}: {t}");
        assert_eq!(w.calls(), before, "{why}: no host is reached");
    };
    w.declare_tag("rel/v9", Some(&signer));
    refused("no tag rel/v9 fetched with this person's access", "fetch");
    tag(&w.source, Some(&other), &["-s", "by-other", "-m", "another key"]);
    w.declare_tag("by-other", Some(&signer));
    refused("by-other is not signed by the ssh-ed25519 key", "signer");
    tag(&w.source, None, &["-a", "unsigned", "-m", "no signature"]);
    w.declare_tag("unsigned", Some(&signer));
    refused("unsigned carries no SSH signature of its own", "signer");
    tag_at(&w.source, Some(&k), &["-s", "future", "-m", "from the future"], "2099-01-01T00:00:00Z");
    w.declare_tag("future", Some(&signer));
    refused("future says it was made at", "tag");
    tag(&w.source, None, &["light"]);
    w.declare_tag("light", Some(&signer));
    refused("light is a lightweight tag", "tag");
    // A tag of the signer's, fetched under another tag's name.
    tag(&w.source, Some(&k), &["-s", "rel/v0", "-m", "an older release"]);
    let v0 = git(&w.source, &["rev-parse", "refs/tags/rel/v0"]);
    git(&w.source, &["update-ref", "refs/tags/rel/v2", &v0]);
    w.declare_tag("rel/v2", Some(&signer));
    refused("the tag fetched as rel/v2 names itself 'rel/v0'", "tag");
    // The loader: a tag with no signer can move; and the rest of the shape.
    for (body, says) in [
        (None, "is pinned only by its signer"),
        (Some("ssh-ed25519 AAAA something"), "is an SSH public key, its type and its base64 text, with no comment"),
    ] {
        w.declare_tag("rel/v1", body);
        let t = text(&w.join(&["--dry-run"]));
        assert!(t.contains("GB-374") && t.contains(says), "{says}: {t}");
    }
    w.declare_tag("rel/v1", Some(&signer));
    let good = std::fs::read_to_string(w.repo.join(TYPE)).unwrap();
    for (body, says) in [
        (good.replace("  tag: rel/v1\n", &format!("  tag: rel/v1\n  commit: \"{}\"\n", w.commit)), "not both"),
        (good.replace("  tag: rel/v1\n", &format!("  commit: \"{}\"\n", w.commit)), "and there is no tag"),
        (good.replace("tag: rel/v1", "tag: ../v1"), "is the name of a tag"),
    ] {
        put(&w.repo, TYPE, &body);
        let t = text(&w.join(&["--dry-run"]));
        assert!(t.contains("GB-374") && t.contains(says), "{says}: {t}");
    }
}

fn text_of_run(w: &World) -> String {
    text(&w.join(&["--from", "13", "--to", "13"]))
}
