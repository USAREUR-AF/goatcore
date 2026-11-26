//! A private overlay adds target types and platform families, and replaces
//! none (docs/design/goatbuilder-build-targets.md section 3.10, gate J26),
//! through the built binary: a public clone loads and lists every public type;
//! an overlay's type is listed beside them; an overlay file named after a
//! public type is refused, and so is a public type's file changed in a tree
//! assembled from goatcore. Nothing here reaches a host: every run is a dry run
//! or refused before its first phase.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// The public type and family files, as a clone of goatcore has them.
fn public_files() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for dir in ["ops/goatbuilder/targets", "ops/goatbuilder/families"] {
        for e in std::fs::read_dir(repo().join(dir)).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".yaml") {
                out.push((format!("{dir}/{name}"), std::fs::read_to_string(e.path()).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// Every public type's name, from its file's.
fn public_types() -> Vec<String> {
    public_files().iter().filter_map(|(p, _)| p.strip_prefix("ops/goatbuilder/targets/")?.strip_suffix(".yaml").map(String::from)).collect()
}

/// A checkout of goatnet `zagros` with the public type and family files.
fn clone() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    for (rel, text) in public_files() {
        put(t.path(), &rel, &text);
    }
    put(
        t.path(),
        "ops/goatbuilder/goatbuilder.yaml",
        "schema: goatbuilder/v1\nrepo: {source: example-org/example-repo, image_namespace: ghcr.io/example-org}\ncomponents:\n  netbird-mgmt: {declared_in: ops/pins/fleet-pins.yaml}\nenvironments:\n  - name: zagros\n    converger: push-ansible\npromotion:\n  path:\n    - {name: zagros, kind: rung, environment: zagros}\ndeploy: {ssh_user: operator}\n",
    );
    put(t.path(), "ops/ansible/group_vars/zagros_hosts/vars.yml", "goatnet_founder: true\n");
    put(t.path(), "ops/ansible/inventory/mgmt-hosts.ini", "[zagros_hosts]\nmgmt-zagros-01-a\n[zagros_example_role_hosts]\nex-1\n");
    put(t.path(), "ops/scripts/inventory-hosts.py", &std::fs::read_to_string(repo().join("ops/scripts/inventory-hosts.py")).unwrap());
    t
}

/// A private overlay's type and family.
fn with_overlay(root: &Path) {
    put(root, "ops/goatbuilder/overlay/families/example-board.yaml", "schema: goatbuilder-family/v1\nfamily: example-board\ntitle: A synthetic board\nmodels: [example-board-1]\n");
    put(
        root,
        "ops/goatbuilder/overlay/targets/example-role.yaml",
        "schema: goatbuilder-target/v1\ntype: example-role\nfamilies: [example-board]\nphases:\n  - label: the example role at {site}\n    plays:\n      - {play: example-role.yml, args: \"-e target={group}\", from: overlay}\n",
    );
}

fn run(root: &Path, args: &[&str]) -> (i32, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["-o", "json", "--yes", args[0], "-C"])
        .arg(root)
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

/// The types a run says it knows, from the GB-247 refusal of a type that is none.
fn listed(root: &Path) -> Vec<String> {
    let (rc, out) = run(root, &["join-node", "zagros", "--site", "zagros", "--node", "ex-1", "--class", "no-such-type", "--dry-run"]);
    assert!(rc != 0 && out.contains("GB-247"), "{out}");
    let known = out.split("known: ").nth(1).unwrap_or_else(|| panic!("{out}"));
    known.split(['"', '\n']).next().unwrap().split(", ").map(String::from).collect()
}

#[test]
fn a_public_clone_loads_and_lists_every_public_type() {
    let c = clone();
    let public = public_types();
    assert!(public.len() >= 7, "{public:?}");
    assert_eq!(listed(c.path()), public);
    // Each loads, and doctor --target reads its file: no type or family
    // file is refused.
    for t in &public {
        let (_, out) = run(c.path(), &["doctor", "--target", t, "--site", "zagros"]);
        for code in ["GB-374", "GB-375", "GB-378", "GB-247"] {
            assert!(!out.contains(code), "{t}: {out}");
        }
    }
    // An overlay's type is listed beside them, and the public ones are as they were.
    with_overlay(c.path());
    let mut both = public.clone();
    both.push("example-role".into());
    both.sort();
    assert_eq!(listed(c.path()), both);
}

#[test]
fn an_overlay_file_that_replaces_a_public_type_is_refused() {
    for t in public_types() {
        let c = clone();
        let text = std::fs::read_to_string(repo().join(format!("ops/goatbuilder/targets/{t}.yaml"))).unwrap();
        put(c.path(), &format!("ops/goatbuilder/overlay/targets/{t}.yaml"), &text);
        let (rc, out) = run(c.path(), &["join-node", "zagros", "--site", "zagros", "--node", "ex-1", "--class", "core", "--dry-run"]);
        assert!(rc != 0 && out.contains("GB-374") && out.contains(&format!("ops/goatbuilder/targets/{t}.yaml already has that name; an overlay adds names and replaces none")), "{t}: {out}");
    }
    let c = clone();
    put(c.path(), "ops/goatbuilder/overlay/families/host-class.yaml", "schema: goatbuilder-family/v1\nfamily: host-class\ntitle: Another\nmodels: [x]\n");
    let (rc, out) = run(c.path(), &["join-node", "zagros", "--site", "zagros", "--node", "ex-1", "--class", "core", "--dry-run"]);
    assert!(rc != 0 && out.contains("GB-374") && out.contains("ops/goatbuilder/families/host-class.yaml already has that name"), "{out}");
}

/// In a tree assembled from goatcore (its marker names the commit), a public
/// type's file is held to that commit's bytes: an overlay copied over it is
/// refused, and the same file in a plain checkout is a person at work.
#[test]
fn an_assembled_tree_holds_the_public_types_to_goatcores_bytes() {
    let c = clone();
    let git = |args: &[&str]| {
        let o = Command::new("git").arg("-C").arg(c.path()).args(["-c", "user.name=t", "-c", "user.email=t@example.org", "-c", "commit.gpgsign=false"]).args(args).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["commit", "-qm", "goatcore"]);
    let head = git(&["rev-parse", "HEAD"]);
    put(c.path(), ".goat-assembled", &format!("goatcore_ref={head}\noverlay_commit={}\n", "b".repeat(40)));
    with_overlay(c.path());
    assert!(listed(c.path()).contains(&"example-role".to_string()), "an overlay's type in an assembled tree");
    let bg = "ops/goatbuilder/targets/breakglass.yaml";
    let text = std::fs::read_to_string(c.path().join(bg)).unwrap();
    put(c.path(), bg, &text.replace("families:", "# changed by an overlay\nfamilies:"));
    let (rc, out) = run(c.path(), &["join-node", "zagros", "--site", "zagros", "--node", "ex-1", "--class", "core", "--dry-run"]);
    assert!(rc != 0 && out.contains("GB-378") && out.contains(&format!("{bg} differs from goatcore {head}")), "{out}");
    std::fs::remove_file(c.path().join(".goat-assembled")).unwrap();
    let (rc, out) = run(c.path(), &["join-node", "zagros", "--site", "zagros", "--node", "ex-1", "--class", "example-role", "--dry-run"]);
    assert_eq!(rc, 0, "{out}");
}
