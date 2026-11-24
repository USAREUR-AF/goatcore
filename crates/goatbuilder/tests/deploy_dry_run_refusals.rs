//! `deploy --dry-run` runs every check that touches no host, on both avenues:
//! the real binary and the real ops/scripts/deploy-component.sh on one stand-in
//! checkout, whose trunk is a bare repository beside it. Each case: both exit 1
//! (refused), name the same refusals, print the whole plan, and start nothing
//! that reaches a host (the stand-in with-aws, netbird and door record calls).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const DIGEST: &str = "sha256:3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c";
const PINS: &str = "ops/ansible/group_vars/test_hosts/pins.yml";

fn exec(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let (f, t) = (e.path(), to.join(e.file_name()));
        match e.file_type().unwrap() {
            k if k.is_dir() => copy_tree(&f, &t),
            k if k.is_file() => {
                std::fs::copy(&f, &t).unwrap();
            }
            _ => {}
        }
    }
}

struct World {
    root: tempfile::TempDir,
    repo: PathBuf,
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

impl World {
    fn new() -> World {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        copy_tree(&real.join("ops/scripts"), &repo.join("ops/scripts"));
        copy_tree(&real.join("ops/secrets"), &repo.join("ops/secrets"));
        let w = |p: &str, t: &str| {
            let p = repo.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, t).unwrap();
        };
        // The release-ladder gate, with no ladder.yaml: nothing to enforce here.
        w("ops/release/ladder_gate.py", &std::fs::read_to_string(real.join("ops/release/ladder_gate.py")).unwrap());
        // Nothing that reaches a host may run: each of these records its call.
        let record = "#!/bin/sh\necho \"$(basename \"$0\") $*\" >> \"$GB_STUB_DIR/calls\"\nexit 1\n";
        for s in ["with-aws", "door-open.sh", "env-door.sh"] {
            w(&format!("ops/scripts/{s}"), record);
            exec(&repo.join("ops/scripts").join(s));
        }
        w("ops/pins/fleet-pins.yaml", &format!("goatnets:\n  test_site:\n    members: [test_hosts]\ncomponents:\n  widget:\n    var: widget_image\n    fleet: \"ghcr.io/example-org/widget:v1@{DIGEST}\"\n    deploy_playbook: widget-deploy.yml\n"));
        w(PINS, &format!("widget_image: \"ghcr.io/example-org/widget:v1@{DIGEST}\"\n"));
        w("ops/ansible/group_vars/test_hosts/vars.yml", "widget_token: \"{{ lookup('env', 'WIDGET_TOKEN') }}\"\n");
        w("ops/ansible/inventory/mgmt-hosts.ini", "[test_hosts]\nhost-a ansible_host=192.0.2.11\n");
        w("ops/ansible/widget-deploy.yml", "- name: widget\n  hosts: \"{{ target }}\"\n  tasks:\n    - ansible.builtin.debug:\n        msg: \"{{ widget_token | length }}\"\n");
        for d in ["home", "tmp", "stub", "bin"] {
            std::fs::create_dir_all(root.path().join(d)).unwrap();
        }
        let nb = root.path().join("bin/netbird");
        std::fs::write(&nb, "#!/bin/sh\nexit 0\n").unwrap();
        exec(&nb);
        let bare = root.path().join("origin.git");
        git(root.path(), &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()]);
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
        let w = World { root, repo };
        w.publish("fixture");
        w
    }

    /// Commit the checkout and make the trunk the same commit.
    fn publish(&self, why: &str) {
        git(&self.repo, &["add", "-A"]);
        git(&self.repo, &["commit", "-q", "-m", why]);
        git(&self.repo, &["push", "-q", "origin", "HEAD:main"]);
    }

    fn write(&self, p: &str, t: &str) {
        std::fs::write(self.repo.join(p), t).unwrap();
    }

    fn env(&self, c: &mut Command, token: Option<&str>) {
        let r = self.root.path();
        let mut path = std::ffi::OsString::from(r.join("bin"));
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());
        c.env_clear()
            .env("PATH", path)
            .env("HOME", r.join("home"))
            .env("TMPDIR", r.join("tmp"))
            .env("GB_STUB_DIR", r.join("stub"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GOAT_MESH_PROBE", "0")
            .env("GOAT_DOOR_OPEN", "0")
            .env("GOAT_NO_FUN", "1")
            .env("GOAT_EXPLAIN", "0")
            .env("GOAT_LANG", "en")
            .env("NO_COLOR", "1")
            .current_dir(&self.repo);
        if let Some(t) = token {
            c.env("WIDGET_TOKEN", t);
        }
    }

    fn binary(&self, token: Option<&str>, extra: &[&str]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
        c.args(["-o", "json", "deploy", "-C"]).arg(&self.repo).args(["--env", "test_site", "--component", "widget"]).args(extra);
        self.env(&mut c, token);
        c.output().unwrap()
    }

    fn script(&self, token: Option<&str>, extra: &[&str]) -> Output {
        let mut c = Command::new("bash");
        c.arg(self.repo.join("ops/scripts/deploy-component.sh")).args(["widget", "--goatnet", "test_site"]).args(extra);
        self.env(&mut c, token);
        c.output().unwrap()
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.root.path().join("stub/calls")).unwrap_or_default()
    }
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// What each avenue says a real run refuses, from its verdict line: `(a b)`.
fn verdict(t: &str) -> Option<String> {
    let at = t.find("DRY: a real run would refuse before any host (")?;
    let rest = &t[at + "DRY: a real run would refuse before any host (".len()..];
    Some(rest[..rest.find(')')?].to_string())
}

/// Run the dry run on both avenues and hold them to the same refusals.
fn both(w: &World, token: Option<&str>, refusals: &[&str], words: &[&str]) {
    let (b, s) = (w.binary(token, &["--dry-run"]), w.script(token, &["--dry-run"]));
    let (bt, st) = (text(&b), text(&s));
    let want = if refusals.is_empty() { None } else { Some(refusals.join(" ")) };
    let code = if refusals.is_empty() { 0 } else { 1 };
    for (side, o, t) in [("binary", &b, &bt), ("script", &s, &st)] {
        assert_eq!(o.status.code(), Some(code), "{side}:\n{t}");
        assert_eq!(verdict(t), want, "{side}:\n{t}");
        for x in words {
            assert!(t.contains(x), "{side} does not say {x:?}:\n{t}");
        }
        // A dry run is a plan too: the play and the ledger read are printed
        // whatever was refused.
        for x in ["widget-deploy.yml -e target=test_hosts", "DRY: would read /var/lib/goat/widget/deploy-history.jsonl on test_hosts"] {
            assert!(t.contains(x), "{side} does not print the plan ({x:?}):\n{t}");
        }
    }
    let hosts: Vec<String> = w.calls().lines().filter(|l| l.starts_with("with-aws") || l.starts_with("door-open")).map(String::from).collect();
    assert!(hosts.is_empty(), "a dry run started something that reaches a host: {hosts:?}");
}

/// A real run on both avenues refuses with exit 1 and starts nothing.
fn both_refuse_for_real(w: &World, token: Option<&str>, words: &[&str]) {
    let (b, s) = (w.binary(token, &["--ack", "A Person", "--yes"]), w.script(token, &[]));
    for (side, o) in [("binary", &b), ("script", &s)] {
        let t = text(o);
        assert_eq!(o.status.code(), Some(1), "{side}:\n{t}");
        for x in words {
            assert!(t.contains(x), "{side} does not say {x:?}:\n{t}");
        }
    }
    let hosts = w.calls();
    assert!(!hosts.contains("with-aws") && !hosts.contains("door-open"), "a refused run reached for a host: {hosts}");
}

#[test]
fn a_clean_dry_run_passes_on_both_avenues() {
    let w = World::new();
    both(&w, Some("x"), &[], &["DRY: nothing was read"]);
}

#[test]
fn a_checkout_behind_the_trunk_fails_the_dry_run_and_the_plan_is_still_printed() {
    let w = World::new();
    git(&w.repo, &["checkout", "-q", "-b", "stale"]);
    git(&w.repo, &["checkout", "-q", "main"]);
    w.write("ops/ansible/widget-extra.yml", "- name: newer\n  hosts: all\n");
    w.publish("a role change this checkout lacks");
    git(&w.repo, &["checkout", "-q", "stale"]);
    both(&w, Some("x"), &["fresh-source"], &["GB-064"]);
    both_refuse_for_real(&w, Some("x"), &["GB-064"]);
}

#[test]
fn a_pin_with_no_digest_fails_deploy_itself_whatever_fresh_source_says() {
    let w = World::new();
    w.write(PINS, "widget_image: \"ghcr.io/example-org/widget:v1\"\n");
    // Published, so the checkout is current: fresh-source passes.
    w.publish("a pin with no digest");
    let words = ["GB-382 REFUSE — widget_image in group_vars/test_hosts/pins.yml is ghcr.io/example-org/widget:v1, with no @sha256: digest"];
    both(&w, Some("x"), &["digest"], &words);
    both_refuse_for_real(&w, Some("x"), &["GB-382"]);
}

#[test]
fn an_unset_secret_fails_the_dry_run_on_both_avenues() {
    let w = World::new();
    both(&w, None, &["secrets"], &["NOT SET: WIDGET_TOKEN"]);
    both_refuse_for_real(&w, None, &["NOT SET: WIDGET_TOKEN"]);
}

#[test]
fn a_ref_the_rung_below_does_not_pin_fails_the_dry_run_and_the_plan_is_still_printed() {
    let w = World::new();
    w.write("ops/release/ladder.yaml", "rungs:\n  - {name: dev, goatnet: dev_site, status: live}\n  - {name: prod, goatnet: test_site, status: live}\nhops: []\n");
    let pins = std::fs::read_to_string(w.repo.join("ops/pins/fleet-pins.yaml")).unwrap();
    w.write("ops/pins/fleet-pins.yaml", &pins.replace("goatnets:\n", "goatnets:\n  dev_site:\n    members: [dev_hosts]\n"));
    std::fs::create_dir_all(w.repo.join("ops/ansible/group_vars/dev_hosts")).unwrap();
    w.write("ops/ansible/group_vars/dev_hosts/pins.yml", "widget_image: \"ghcr.io/example-org/widget:v0@sha256:aaaa\"\n");
    w.publish("a ladder");
    both(&w, Some("x"), &["ladder"], &["GB-373"]);
    both_refuse_for_real(&w, Some("x"), &["GB-373"]);
}

#[test]
fn every_refusal_is_reported_in_one_dry_run() {
    let w = World::new();
    git(&w.repo, &["checkout", "-q", "-b", "stale"]);
    w.write(PINS, "widget_image: \"ghcr.io/example-org/widget:v1\"\n");
    git(&w.repo, &["commit", "-q", "-am", "stale and unpinned"]);
    git(&w.repo, &["checkout", "-q", "main"]);
    w.write("ops/ansible/widget-extra.yml", "- name: newer\n  hosts: all\n");
    w.publish("a role change this checkout lacks");
    git(&w.repo, &["checkout", "-q", "stale"]);
    both(&w, None, &["fresh-source", "digest", "secrets"], &["GB-064", "GB-382", "NOT SET: WIDGET_TOKEN"]);
}

/// An assembled tree whose pin digest was edited by hand: fresh-source judges
/// it against the overlay commit and refuses, in a dry run as in a real run.
#[test]
fn an_edited_digest_in_an_assembled_tree_fails_the_dry_run() {
    let w = World::new();
    let overlay = w.root.path().join("overlay");
    std::fs::create_dir_all(overlay.join(Path::new(PINS).parent().unwrap())).unwrap();
    let pin = format!("widget_image: \"ghcr.io/example-org/widget:v1@{DIGEST}\"\n");
    std::fs::write(overlay.join(PINS), &pin).unwrap();
    let rev = |d: &Path| {
        let o = Command::new("git").arg("-C").arg(d).args(["rev-parse", "HEAD"]).output().unwrap();
        String::from_utf8(o.stdout).unwrap().trim().to_string()
    };
    std::fs::write(overlay.join("goatcore.ref"), format!("{}\n", rev(&w.repo))).unwrap();
    let bare = w.root.path().join("overlay.git");
    git(w.root.path(), &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()]);
    git(&overlay, &["init", "-q", "-b", "main"]);
    git(&overlay, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&overlay, &["add", "-A"]);
    git(&overlay, &["commit", "-q", "-m", "overlay"]);
    git(&overlay, &["push", "-q", "origin", "HEAD:main"]);
    w.write(
        ".goat-assembled",
        &format!("goatcore_ref={}\noverlay_commit={}\noverlay_path={}\n", rev(&w.repo), rev(&overlay), overlay.display()),
    );
    // The assemble step copies every overlay file over goatcore's.
    std::fs::copy(overlay.join("goatcore.ref"), w.repo.join("goatcore.ref")).unwrap();
    both(&w, Some("x"), &[], &["DRY: nothing was read"]);
    w.write(PINS, &pin.replace("3c3c3c", "4d4d4d"));
    both(&w, Some("x"), &["fresh-source"], &["GB-064"]);
    both_refuse_for_real(&w, Some("x"), &["GB-064"]);
}
