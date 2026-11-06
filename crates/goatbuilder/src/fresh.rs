//! Contract clause C3 (fresh source) on a real run.
//!
//! The check is `ops/scripts/fresh-source.sh`, the one both avenues run before
//! the first host is touched (ADR 1201): a checkout behind the trunk in
//! `ops/ansible` or `ops/pins` is refused (GB-064), a trunk that cannot be
//! fetched is refused unless the run is `--offline` (GB-065), a checkout ahead
//! of the trunk is run with a warning naming its commits. The trunk commit the
//! check judged against is handed to every `ansible-preflight.sh` of the same
//! run (`GOAT_FRESH_SOURCE_TRUNK`), so all of its plays are judged against one
//! commit.
//!
//! An assembled tree (goatcore at the overlay's `goatcore.ref`, goatstuff at its
//! `goatstuff.ref`, the overlay's pre-disclosure patches, the overlay copied
//! over them) carries a `.goat-assembled` marker at its root. The same script
//! hands such a tree to `ops/scripts/lib/source_of_record.py`, which reads every
//! line of the marker (`goatcore_ref`, `overlay_commit`, `overlay_path`,
//! `goatstuff_ref`, `goatstuff_path`, `embargo`) and judges the tree against
//! the commits and patches it names. This module runs that one script for both
//! kinds of tree, so the binary and the scripts give the same verdict on the
//! same tree. It reads the marker itself only to refuse a tree whose goatcore
//! carries no `source_of_record.py`. The trunk handed to the plays of an
//! assembled tree is `goatcore_ref`; the rollback-ledger role records the
//! commits and the patch ids in every row a play from the tree writes.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::Check;

/// The environment variable that carries the judged trunk commit to the plays.
pub const TRUNK_ENV: &str = "GOAT_FRESH_SOURCE_TRUNK";

/// What the check said.
pub struct Fresh {
    /// The trunk commit judged against; empty on an offline run with no cached trunk.
    pub trunk: String,
    /// One row for the report: ok (current, ahead, offline) or refused.
    pub check: Check,
}

impl Fresh {
    pub fn refused(&self) -> bool {
        !self.check.ok
    }
}

/// Run the check for `repo`. `GOAT_DEPLOY_OFFLINE=1` (`--offline`) and an
/// already-pinned `GOAT_FRESH_SOURCE_TRUNK` reach the script through the
/// environment, as they do from the script avenue; so do the overlay and
/// goatstuff checkouts of an assembled tree (`GOAT_OVERLAY_DIR`,
/// `GOAT_OVERLAY_TRUNK_REMOTE`, `GOAT_GOATSTUFF_DIR`) when its marker does not
/// name them.
pub fn check(repo: &Path) -> Result<Fresh> {
    check_with(repo, &[], &[])
}

fn check_with(repo: &Path, set: &[(&str, &str)], unset: &[&str]) -> Result<Fresh> {
    if let Some(m) = marker(repo)? {
        if !repo.join(SOURCE_OF_RECORD).is_file() {
            return Ok(Fresh {
                trunk: m.goatcore_ref.clone(),
                check: Check {
                    name: "fresh-source".into(),
                    ok: false,
                    detail: format!(
                        "GB-065 REFUSE — this tree is assembled ({}) but its goatcore has no {SOURCE_OF_RECORD} to judge it with (contract C3).",
                        m.describe()
                    ),
                    fix: Some("move goatcore.ref to a goatcore commit that carries the check, re-assemble, and run from the new tree".into()),
                },
            });
        }
    }
    let mut cmd = Command::new(repo.join("ops/scripts/fresh-source.sh"));
    cmd.args(["--repo".as_ref(), repo.as_os_str()]);
    for (k, v) in set {
        cmd.env(k, v);
    }
    for k in unset {
        cmd.env_remove(k);
    }
    let out = cmd.output().with_context(|| "ops/scripts/fresh-source.sh")?;
    let said = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
    let trunk = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let check = if out.status.success() {
        let detail = if said.is_empty() {
            format!("checkout current with the trunk at {}", short(&trunk))
        } else {
            said.lines().map(|l| l.trim_start_matches("fresh-source: ")).collect::<Vec<_>>().join("\n")
        };
        Check { name: "fresh-source".into(), ok: true, detail, fix: None }
    } else {
        let fix = said
            .lines()
            .find_map(|l| l.trim_start().strip_prefix("fix: "))
            .unwrap_or("git fetch <trunk remote> main and bring this checkout onto it")
            .to_string();
        let detail = said
            .lines()
            .filter(|l| !l.trim_start().starts_with("fix: "))
            .map(|l| l.trim_start_matches("fresh-source: "))
            .collect::<Vec<_>>()
            .join("\n");
        Check { name: "fresh-source".into(), ok: false, detail, fix: Some(fix) }
    };
    Ok(Fresh { trunk, check })
}

/// The marker an assembled tree carries at its root (the overlay's assemble step).
pub const MARKER: &str = ".goat-assembled";
/// The check an assembled tree is judged by, in the tree's own goatcore.
const SOURCE_OF_RECORD: &str = "ops/scripts/lib/source_of_record.py";

/// What `.goat-assembled` says.
#[derive(Debug, PartialEq, Default)]
pub struct Marker {
    /// The goatcore commit the tree was built from.
    pub goatcore_ref: String,
    /// The overlay commit copied over it; `-dirty` when the overlay had uncommitted changes.
    pub overlay_commit: String,
    /// Where the overlay checkout is, when the marker names it.
    pub overlay_path: Option<String>,
    /// The goatstuff commit layered between goatcore and the overlay, when there is one.
    pub goatstuff_ref: Option<String>,
    /// Where that goatstuff checkout is, when the marker names it.
    pub goatstuff_path: Option<String>,
    /// The ids of the pre-disclosure patches applied (`embargo=<id>/<kind>.patch:<sha256>,…`), sorted.
    pub embargo: Vec<String>,
}

impl Marker {
    /// One line naming what the tree was built from.
    pub fn describe(&self) -> String {
        let mut s = format!("goatcore {}", short(&self.goatcore_ref));
        if let Some(g) = &self.goatstuff_ref {
            s.push_str(&format!(" + goatstuff {}", short(g)));
        }
        if !self.embargo.is_empty() {
            s.push_str(&format!(" + pre-disclosure patches {}", self.embargo.join(", ")));
        }
        s.push_str(&format!(" + overlay {}", short(&self.overlay_commit)));
        s
    }
}

/// Read the marker at `repo`'s root: `None` when there is none (a plain checkout).
pub fn marker(repo: &Path) -> Result<Option<Marker>> {
    let p = repo.join(MARKER);
    if !p.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&p).with_context(|| p.display().to_string())?;
    parse_marker(&text).map(Some).with_context(|| format!("{MARKER} in {}", repo.display()))
}

fn parse_marker(text: &str) -> Result<Marker> {
    let get = |k: &str| {
        text.lines()
            .filter_map(|l| l.trim().split_once('='))
            .find(|(n, _)| n.trim() == k)
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let goatcore_ref = get("goatcore_ref").ok_or_else(|| anyhow::anyhow!("no goatcore_ref= line"))?;
    let overlay_commit = get("overlay_commit").ok_or_else(|| anyhow::anyhow!("no overlay_commit= line"))?;
    let mut embargo: Vec<String> = get("embargo")
        .unwrap_or_default()
        .split(',')
        .filter_map(|e| e.trim().split_once('/').map(|(id, _)| id.to_string()))
        .filter(|id| !id.is_empty())
        .collect();
    embargo.sort();
    embargo.dedup();
    Ok(Marker {
        goatcore_ref,
        overlay_commit,
        overlay_path: get("overlay_path").or_else(|| get("overlay_dir")),
        goatstuff_ref: get("goatstuff_ref"),
        goatstuff_path: get("goatstuff_path"),
        embargo,
    })
}

fn short(sha: &str) -> &str {
    if sha.is_empty() {
        "(none cached)"
    } else {
        &sha[..sha.len().min(9)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?}");
    }

    /// A bare trunk `origin` with ops/ansible and the real script, and a clone of it.
    fn pair(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("gb-fresh-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let (bare, a, b) = (root.join("o.git"), root.join("a"), root.join("b"));
        git(&root, &["init", "-q", "--bare", "o.git"]);
        git(&bare, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        git(&root, &["init", "-q", "-b", "main", "a"]);
        fs::create_dir_all(a.join("ops/ansible")).unwrap();
        fs::create_dir_all(a.join("ops/scripts")).unwrap();
        fs::create_dir_all(a.join("docs")).unwrap();
        fs::write(a.join("ops/ansible/x"), "1").unwrap();
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts/fresh-source.sh");
        fs::copy(&script, a.join("ops/scripts/fresh-source.sh")).unwrap();
        git(&a, &["add", "-A"]);
        git(&a, &["commit", "-qm", "base"]);
        git(&a, &["remote", "add", "origin", bare.to_str().unwrap()]);
        git(&a, &["push", "-q", "origin", "main"]);
        git(&root, &["clone", "-q", bare.to_str().unwrap(), "b"]);
        (root, a, b)
    }

    // The trunk remote is named on the child, as the hermetic harness names it.
    fn run(b: &Path) -> Fresh {
        check_with(b, &[("GOAT_ANSIBLE_TRUNK_REMOTE", "origin")], &[TRUNK_ENV, "GOAT_DEPLOY_OFFLINE"]).unwrap()
    }

    #[test]
    fn current_runs_behind_refuses_ahead_warns() {
        let (root, a, b) = pair("c3");
        let f = run(&b);
        assert!(!f.refused(), "{}", f.check.detail);
        assert_eq!(f.trunk.len(), 40);

        // The trunk gains a role change this checkout lacks: refused, GB-064, with the command.
        fs::write(a.join("ops/ansible/x"), "2").unwrap();
        git(&a, &["commit", "-qam", "role change"]);
        git(&a, &["push", "-q", "origin", "main"]);
        let f = run(&b);
        assert!(f.refused());
        assert!(f.check.detail.contains("GB-064") && f.check.detail.contains("role change"), "{}", f.check.detail);
        assert!(f.check.fix.as_deref().unwrap().contains("pull --ff-only origin main"));

        // Brought current, then a branch with its own commit: runs, and names it.
        git(&b, &["pull", "-q", "--ff-only", "origin", "main"]);
        git(&b, &["checkout", "-qb", "feat"]);
        fs::create_dir_all(b.join("docs")).unwrap();
        fs::write(b.join("docs/f"), "f").unwrap();
        git(&b, &["add", "-A"]);
        git(&b, &["commit", "-qm", "feature commit"]);
        let f = run(&b);
        assert!(!f.refused());
        assert!(f.check.detail.contains("WARN") && f.check.detail.contains("feature commit"), "{}", f.check.detail);

        // A trunk change outside what a deploy reads does not block a branch.
        fs::create_dir_all(a.join("docs")).unwrap();
        fs::write(a.join("docs/d"), "d").unwrap();
        git(&a, &["add", "-A"]);
        git(&a, &["commit", "-qm", "docs only"]);
        git(&a, &["push", "-q", "origin", "main"]);
        assert!(!run(&b).refused());

        // The trunk cannot be fetched: refused, GB-065, naming --offline.
        git(&b, &["remote", "set-url", "origin", "/nonexistent/o.git"]);
        let f = run(&b);
        assert!(f.refused());
        assert!(f.check.detail.contains("GB-065") && f.check.detail.contains("--offline"), "{}", f.check.detail);
        let _ = fs::remove_dir_all(root);
    }

    fn head(dir: &Path) -> String {
        let out = Command::new("git").arg("-C").arg(dir).args(["rev-parse", "HEAD"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn commit_file(dir: &Path, rel: &str, body: &str, msg: &str) {
        let f = dir.join(rel);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, body).unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-qm", msg]);
    }

    const OVERLAY_DIR_ENV: &str = "GOAT_OVERLAY_DIR";
    const OVERLAY_REMOTE_ENV: &str = "GOAT_OVERLAY_TRUNK_REMOTE";

    /// An overlay trunk (bare), two overlay checkouts of it, and an assembled
    /// tree: a goatcore commit carrying the real check scripts, with the
    /// overlay's goatcore.ref and the marker beside it.
    fn assembled_fixture(tag: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("gb-assembled-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let (bare, ov, other, tree) = (root.join("ov.git"), root.join("ov"), root.join("other"), root.join("tree"));
        git(&root, &["init", "-q", "--bare", "ov.git"]);
        git(&bare, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        git(&root, &["init", "-q", "-b", "main", "tree"]);
        let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts");
        fs::create_dir_all(tree.join("ops/scripts/lib")).unwrap();
        fs::copy(scripts.join("fresh-source.sh"), tree.join("ops/scripts/fresh-source.sh")).unwrap();
        fs::copy(scripts.join("lib/source_of_record.py"), tree.join(SOURCE_OF_RECORD)).unwrap();
        commit_file(&tree, "ops/ansible/x", "1", "goatcore base");
        let gref = head(&tree);
        git(&root, &["init", "-q", "-b", "main", "ov"]);
        commit_file(&ov, "goatcore.ref", &format!("{gref}\n"), "overlay base");
        git(&ov, &["remote", "add", "origin", bare.to_str().unwrap()]);
        git(&ov, &["push", "-q", "origin", "main"]);
        git(&root, &["clone", "-q", bare.to_str().unwrap(), "other"]);
        fs::write(tree.join("goatcore.ref"), format!("{gref}\n")).unwrap();
        fs::write(tree.join(MARKER), format!("goatcore_ref={gref}\noverlay_commit={}\n", head(&ov))).unwrap();
        (root, ov, other, tree)
    }

    fn run_assembled(tree: &Path, overlay: Option<&Path>, offline: bool) -> Fresh {
        let mut set: Vec<(&str, &str)> = vec![(OVERLAY_REMOTE_ENV, "origin")];
        let mut unset = vec![TRUNK_ENV, "GOAT_OVERLAY_REMOTE", "GOAT_GOATSTUFF_DIR"];
        let o = overlay.map(|p| p.to_str().unwrap().to_string()).unwrap_or_default();
        if overlay.is_some() {
            set.push((OVERLAY_DIR_ENV, o.as_str()));
        } else {
            unset.push(OVERLAY_DIR_ENV);
        }
        if offline {
            set.push(("GOAT_DEPLOY_OFFLINE", "1"));
        } else {
            unset.push("GOAT_DEPLOY_OFFLINE");
        }
        check_with(tree, &set, &unset).unwrap()
    }

    #[test]
    fn marker_parses_every_line_the_check_reads() {
        let m = parse_marker("goatcore_ref=abc\noverlay_commit=def-dirty\n").unwrap();
        assert_eq!(m, Marker { goatcore_ref: "abc".into(), overlay_commit: "def-dirty".into(), ..Default::default() });
        let m = parse_marker("goatcore_ref=abc\noverlay_commit=def\noverlay_path=/o\n").unwrap();
        assert_eq!(m.overlay_path.as_deref(), Some("/o"));
        assert_eq!(parse_marker("goatcore_ref=abc\noverlay_commit=def\noverlay_dir=/d\n").unwrap().overlay_path.as_deref(), Some("/d"));
        let m = parse_marker(
            "goatcore_ref=abc\ngoatstuff_ref=fed\noverlay_commit=def\ngoatstuff_path=/s\nembargo=B/core.patch:11,A/core.patch:22,A/stuff.patch:33\n",
        )
        .unwrap();
        assert_eq!(m.goatstuff_ref.as_deref(), Some("fed"));
        assert_eq!(m.goatstuff_path.as_deref(), Some("/s"));
        assert_eq!(m.embargo, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(m.describe(), "goatcore abc + goatstuff fed + pre-disclosure patches A, B + overlay def");
        assert!(parse_marker("goatcore_ref=abc\n").is_err());
        assert!(parse_marker("overlay_commit=def\n").is_err());
        assert!(parse_marker("goatcore_ref=\noverlay_commit=def\n").is_err());
        // A plain checkout carries no marker: the script judges it against its trunk.
        let d = std::env::temp_dir().join(format!("gb-nomarker-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        assert!(marker(&d).unwrap().is_none());
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn assembled_current_runs_and_hands_the_goatcore_ref() {
        let (root, ov, _, tree) = assembled_fixture("cur");
        let f = run_assembled(&tree, Some(&ov), false);
        assert!(!f.refused(), "{}", f.check.detail);
        assert_eq!(f.trunk, head(&tree));
        assert!(f.check.detail.contains("assembled tree: goatcore"), "{}", f.check.detail);
        // The overlay checkout named in the marker instead of the environment.
        let m = fs::read_to_string(tree.join(MARKER)).unwrap();
        fs::write(tree.join(MARKER), format!("{m}overlay_path={}\n", ov.display())).unwrap();
        assert!(!run_assembled(&tree, None, false).refused());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn assembled_goatcore_not_as_declared_refuses() {
        let (root, ov, _, tree) = assembled_fixture("gref");
        // goatcore.ref declares another commit than the tree was built from.
        fs::write(tree.join("goatcore.ref"), "0123456789012345678901234567890123456789\n").unwrap();
        let f = run_assembled(&tree, Some(&ov), false);
        assert!(f.refused());
        assert!(f.check.detail.contains("GB-064") && f.check.detail.contains("goatcore.ref in this tree names 012345678"), "{}", f.check.detail);
        assert!(f.check.fix.as_deref().unwrap().contains("assemble.sh"));
        // The declaration matches, but the goatcore checkout moved off it.
        let gref = head(&tree);
        fs::write(tree.join("goatcore.ref"), format!("{gref}\n")).unwrap();
        commit_file(&tree, "ops/ansible/y", "1", "moved");
        let f = run_assembled(&tree, Some(&ov), false);
        assert!(f.refused());
        assert!(f.check.detail.contains("GB-064") && f.check.detail.contains("checkout is at"), "{}", f.check.detail);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn assembled_overlay_behind_refuses_ahead_warns_dirty_refuses() {
        let (root, ov, other, tree) = assembled_fixture("ov");
        // The overlay trunk gains a docs-only change: not what a deploy reads, runs.
        commit_file(&other, "docs/findings/f.md", "f", "docs only");
        git(&other, &["push", "-q", "origin", "main"]);
        assert!(!run_assembled(&tree, Some(&ov), false).refused());

        // The overlay trunk gains a pins change the tree lacks: refused, GB-064, named.
        commit_file(&other, "ops/pins/fleet-pins.yaml", "p", "pin bump");
        git(&other, &["push", "-q", "origin", "main"]);
        let f = run_assembled(&tree, Some(&ov), false);
        assert!(f.refused());
        assert!(f.check.detail.contains("GB-064") && f.check.detail.contains("pin bump"), "{}", f.check.detail);
        assert!(!f.check.detail.contains("docs only"), "{}", f.check.detail);

        // Assembled from an overlay commit on top of the trunk: runs, names it.
        git(&ov, &["pull", "-q", "--ff-only", "origin", "main"]);
        commit_file(&ov, "ops/ansible/group_vars/g/vars.yml", "v", "overlay change");
        for rel in ["ops/pins/fleet-pins.yaml", "ops/ansible/group_vars/g/vars.yml", "docs/findings/f.md"] {
            fs::create_dir_all(tree.join(rel).parent().unwrap()).unwrap();
            fs::copy(ov.join(rel), tree.join(rel)).unwrap();
        }
        let gref = head(&tree);
        fs::write(tree.join(MARKER), format!("goatcore_ref={gref}\noverlay_commit={}\n", head(&ov))).unwrap();
        let f = run_assembled(&tree, Some(&ov), false);
        assert!(!f.refused(), "{}", f.check.detail);
        assert!(f.check.detail.contains("WARN") && f.check.detail.contains("overlay change"), "{}", f.check.detail);

        // Assembled from uncommitted overlay changes: refused, as the script avenue refuses it.
        fs::write(tree.join(MARKER), format!("goatcore_ref={gref}\noverlay_commit={}-dirty\n", head(&ov))).unwrap();
        let f = run_assembled(&tree, Some(&ov), false);
        assert!(f.refused());
        assert!(f.check.detail.contains("GB-064") && f.check.detail.contains("uncommitted"), "{}", f.check.detail);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn assembled_overlay_unknown_unreachable_or_unnamed_refuses() {
        let (root, ov, _, tree) = assembled_fixture("un");
        // No overlay checkout named: refused, naming the variable; offline runs.
        let f = run_assembled(&tree, None, false);
        assert!(f.refused());
        assert!(f.check.detail.contains("GB-065") && f.check.fix.as_deref().unwrap().contains(OVERLAY_DIR_ENV), "{}", f.check.detail);
        let f = run_assembled(&tree, None, true);
        assert!(!f.refused() && f.check.detail.contains("OFFLINE"), "{}", f.check.detail);

        // The overlay trunk cannot be fetched: GB-065; offline judges the cached trunk.
        git(&ov, &["remote", "set-url", "origin", "/nonexistent/ov.git"]);
        let f = run_assembled(&tree, Some(&ov), false);
        assert!(f.refused());
        assert!(f.check.detail.contains("GB-065") && f.check.detail.contains("--offline"), "{}", f.check.detail);
        let f = run_assembled(&tree, Some(&ov), true);
        assert!(!f.refused() && f.check.detail.contains("OFFLINE"), "{}", f.check.detail);

        // An overlay commit the checkout does not have: refused.
        let gref = head(&tree);
        fs::write(tree.join(MARKER), format!("goatcore_ref={gref}\noverlay_commit=unknown\n")).unwrap();
        let f = run_assembled(&tree, Some(&ov), true);
        assert!(f.refused());
        assert!(f.check.detail.contains("not in the overlay checkout"), "{}", f.check.detail);
        let _ = fs::remove_dir_all(root);
    }

    /// A goatstuff checkout layered into the tree: the binary judges it as the scripts do.
    #[test]
    fn assembled_goatstuff_is_judged() {
        let (root, ov, _, tree) = assembled_fixture("gs");
        git(&root, &["init", "-q", "-b", "main", "stuff"]);
        let stuff = root.join("stuff");
        commit_file(&stuff, "ops/ansible/roles/y/tasks/main.yml", "- debug: msg=stuff\n", "goatstuff");
        let sref = head(&stuff);
        fs::create_dir_all(tree.join("ops/ansible/roles/y/tasks")).unwrap();
        fs::copy(stuff.join("ops/ansible/roles/y/tasks/main.yml"), tree.join("ops/ansible/roles/y/tasks/main.yml")).unwrap();
        let gref = head(&tree);
        fs::write(
            tree.join(MARKER),
            format!("goatcore_ref={gref}\noverlay_commit={}\ngoatstuff_ref={sref}\ngoatstuff_path={}\n", head(&ov), stuff.display()),
        )
        .unwrap();
        let f = run_assembled(&tree, Some(&ov), false);
        assert!(!f.refused(), "{}", f.check.detail);
        assert!(f.check.detail.contains(&format!("goatstuff {}", &sref[..12])), "{}", f.check.detail);
        // A goatstuff file edited after assembly: refused.
        fs::write(tree.join("ops/ansible/roles/y/tasks/main.yml"), "- debug: msg=hand\n").unwrap();
        let f = run_assembled(&tree, Some(&ov), false);
        assert!(f.refused());
        assert!(f.check.detail.contains("differ from goatstuff"), "{}", f.check.detail);
        let _ = fs::remove_dir_all(root);
    }

    /// A pre-disclosure patch recorded in the marker: the patched file runs, any other change is refused.
    #[test]
    fn assembled_pre_disclosure_patch_is_accepted_only_as_recorded() {
        let (root, ov, _, tree) = assembled_fixture("emb");
        let patch = "diff --git a/ops/ansible/x b/ops/ansible/x\nindex d00491f..0cfbf08 100644\n--- a/ops/ansible/x\n+++ b/ops/ansible/x\n@@ -1 +1 @@\n-1\n\\ No newline at end of file\n+2\n\\ No newline at end of file\n";
        commit_file(&ov, "embargo/A/core.patch", patch, "embargo A");
        commit_file(&ov, "embargo/A/README.md", "# A\n", "embargo A readme");
        git(&ov, &["push", "-q", "origin", "main"]);
        for rel in ["embargo/A/core.patch", "embargo/A/README.md"] {
            fs::create_dir_all(tree.join(rel).parent().unwrap()).unwrap();
            fs::copy(ov.join(rel), tree.join(rel)).unwrap();
        }
        fs::write(tree.join("ops/ansible/x"), "2").unwrap();
        let digest = {
            let out = Command::new("python3")
                .args(["-c", "import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest())"])
                .arg(tree.join("embargo/A/core.patch"))
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let gref = head(&tree);
        let body = format!("goatcore_ref={gref}\noverlay_commit={}\n", head(&ov));
        fs::write(tree.join(MARKER), format!("{body}embargo=A/core.patch:{digest}\n")).unwrap();
        let f = run_assembled(&tree, Some(&ov), false);
        assert!(!f.refused(), "{}", f.check.detail);
        assert!(f.check.detail.contains("pre-disclosure patches A"), "{}", f.check.detail);
        // The marker does not record the patch the overlay carries: refused.
        fs::write(tree.join(MARKER), &body).unwrap();
        let f = run_assembled(&tree, Some(&ov), false);
        assert!(f.refused() && f.check.detail.contains("not the ones the overlay commit carries"), "{}", f.check.detail);
        // Recorded, but the file is not as the patch makes it: refused.
        fs::write(tree.join(MARKER), format!("{body}embargo=A/core.patch:{digest}\n")).unwrap();
        fs::write(tree.join("ops/ansible/x"), "3").unwrap();
        let f = run_assembled(&tree, Some(&ov), false);
        assert!(f.refused() && f.check.detail.contains("pre-disclosure patches change are not"), "{}", f.check.detail);
        let _ = fs::remove_dir_all(root);
    }

    /// A goatcore without the assembled-tree check cannot judge the tree: refused before the script runs.
    #[test]
    fn assembled_without_the_check_refuses() {
        let (root, _, _, tree) = assembled_fixture("nocheck");
        fs::remove_file(tree.join(SOURCE_OF_RECORD)).unwrap();
        let f = check_with(&tree, &[], &[TRUNK_ENV]).unwrap();
        assert!(f.refused());
        assert!(f.check.detail.contains("GB-065") && f.check.detail.contains(SOURCE_OF_RECORD), "{}", f.check.detail);
        let _ = fs::remove_dir_all(root);
    }
}
