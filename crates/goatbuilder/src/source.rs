//! A component whose play lives in another repository (`source:` in
//! ops/pins/fleet-pins.yaml; docs/design/goatbuilder-build-targets.md section
//! 3.10, gate J26). The repository, the full commit and the play's path are data
//! in the pins. The commit is fetched on the operator's machine with that
//! person's own git access, into a directory of its own under the state
//! directory, and nothing is put in the checkout. goatbuilder holds no
//! credential for it and asks for none.
//!
//! The commit is the whole proof: git names content by it, and a copy that is
//! not exactly that commit's is refused. A role of goatcore's cannot be
//! replaced: a source that carries a role of the same name is refused.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};

use crate::runner::Cmd;

const CODE: &str = "GB-385";
/// The directory under the state directory that tells a source's play from one
/// of the checkout.
const DIR: &str = "goat-sources";

/// The inventory file of ops/ansible that names no host. Ansible reads
/// group_vars and host_vars beside the play or an inventory file, so a play
/// kept outside ops/ansible is run with this one as well, beside the
/// checkout's own.
pub(crate) const VARS_HERE: &str = "vars-here.ini";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Source {
    pub repository: String,
    pub commit: String,
    pub play: String,
}

/// A component's `source:`, as [`problem`] checked it.
pub(crate) fn of(spec: &Value) -> Option<Source> {
    let s = spec.get("source")?;
    let t = |k: &str| s.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    Some(Source { repository: t("repository"), commit: t("commit"), play: t("play") })
}

/// Why a `source:` mapping is not one, in the pins' words (GB-365); mirrored by
/// goat_consistency.resolve.check_source.
pub(crate) fn problem(cname: &str, spec: &Value) -> Option<String> {
    let src = spec.get("source")?;
    let Some(m) = src.as_object() else { return Some(format!("component '{cname}': `source:` is a mapping (repository, commit, play)")) };
    let mut unknown: Vec<&String> = m.keys().filter(|k| !["repository", "commit", "play"].contains(&k.as_str())).collect();
    unknown.sort();
    if let Some(k) = unknown.first() {
        return Some(format!("component '{cname}': `source:` has an unknown key '{k}' (known: repository, commit, play)"));
    }
    let s = of(spec)?;
    let re = |p: &str, t: &str| regex::Regex::new(p).unwrap().is_match(t);
    if !repository_ok(&s.repository) {
        return Some(format!("component '{cname}': `source.repository:` is a git URL (https, ssh or file) with no credential in it"));
    }
    if !re(r"\A[0-9a-f]{40}\z", &s.commit) {
        return Some(format!("component '{cname}': `source.commit:` is a full commit of 40 hexadecimal characters; a branch or a tag can move"));
    }
    if !re(r"\A[A-Za-z0-9_][A-Za-z0-9_./-]*\.ya?ml\z", &s.play) || s.play.split('/').any(|x| ["", ".", ".."].contains(&x)) {
        return Some(format!("component '{cname}': `source.play:` is a play's path inside the repository"));
    }
    if spec.get("deploy_playbook").is_some_and(|v| !v.is_null()) {
        return Some(format!("component '{cname}' declares both deploy_playbook and source; its play is in one place"));
    }
    None
}

/// A fetched source: its play, and where roles are looked for.
#[derive(Debug)]
pub(crate) struct Fetched {
    pub play: PathBuf,
    pub roles_path: String,
}

pub(crate) fn is_play(p: &Path) -> bool {
    p.components().any(|c| c.as_os_str() == DIR)
}

/// The play as `ansible-playbook` in ops/ansible takes it: by its name, or a
/// source's by its path.
pub(crate) fn play_word(p: &Path) -> String {
    if is_play(p) {
        return p.display().to_string();
    }
    p.file_name().unwrap_or_default().to_string_lossy().to_string()
}

/// The roles path a source's play runs with, read back from where it is.
pub(crate) fn roles_path(repo: &Path, play: &Path) -> Option<String> {
    let mut root = PathBuf::new();
    let mut parts = play.components();
    loop {
        let c = parts.next()?;
        root.push(c);
        if c.as_os_str() == DIR {
            break;
        }
    }
    let (component, commit) = (parts.next()?, parts.next()?);
    Some(roles(repo, &root.join(component).join(commit), play))
}

/// A path as an absolute one, against the working directory: a play run from
/// ops/ansible reads every path it is given from there.
pub(crate) fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

fn roles(repo: &Path, at: &Path, play: &Path) -> String {
    let mut v = vec![absolute(repo).join("ops/ansible/roles"), play.parent().unwrap_or(at).join("roles"), at.join("roles")];
    v.dedup();
    v.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(":")
}

fn git(at: &Path, args: &[&str]) -> Result<String, String> {
    git_in(at, args, true)
}

/// git on a kept copy alone, with none of the person's or the system's
/// configuration: no filter, hook or helper it names runs on the copy's files.
fn local_git(at: &Path, args: &[&str]) -> Result<String, String> {
    git_in(at, args, false)
}

fn git_in(at: &Path, args: &[&str], fetching: bool) -> Result<String, String> {
    let mut c = Cmd::new("git");
    // The person's own access, and no prompt: a run that would ask is refused.
    c.arg("-C").arg(at).args(["-c", "core.hooksPath=/dev/null", "-c", "protocol.ext.allow=never", "-c", "advice.detachedHead=false"]).args(args);
    c.env("GIT_TERMINAL_PROMPT", "0");
    c.env("GIT_LFS_SKIP_SMUDGE", "1");
    // Never a repository above the copy.
    if let Some(up) = at.parent() {
        c.env("GIT_CEILING_DIRECTORIES", up);
    }
    if !fetching {
        c.env("GIT_CONFIG_GLOBAL", "/dev/null");
        c.env("GIT_CONFIG_NOSYSTEM", "1");
    }
    let out = c.output().map_err(|e| format!("could not start git: {e}"))?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        return Err(said.lines().find(|l| !l.trim().is_empty()).unwrap_or("git said nothing").trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The URL shape a `source.repository:` has: a git URL (https, ssh or file)
/// with no credential in it. A type's source is held to the same.
pub(crate) fn repository_ok(r: &str) -> bool {
    let re = |p: &str| regex::Regex::new(p).unwrap().is_match(r);
    re(r"\A(https://|ssh://|file:///|[A-Za-z0-9._-]+@[A-Za-z0-9.-]+:)[A-Za-z0-9._~/:@-]+\z") && !re(r"\Ahttps://[^/]*@")
}

/// Where `repository` at `commit` is kept for `key` under `dir` of the state
/// directory: fetched there when it is not, then shown to be that commit
/// exactly. `bad(check, why)` makes the refusal.
fn kept(repo: &Path, dir: &str, key: &str, repository: &str, commit: &str, bad: &dyn Fn(&str, String) -> anyhow::Error) -> Result<PathBuf> {
    let base = absolute(Path::new(&crate::settings::state_dir_abs(repo))).join(dir).join(key);
    let at = base.join(commit);
    if !at.is_dir() {
        let tmp = base.join(format!(".{commit}.{}", std::process::id()));
        std::fs::create_dir_all(&tmp).map_err(|e| bad("fetch", format!("{} cannot be made: {e}", tmp.display())))?;
        let done = (|| {
            local_git(&tmp, &["init", "-q"])?;
            // A server that gives no commit by its name gives its branches.
            if git(&tmp, &["fetch", "-q", "--depth", "1", "--", repository, commit]).is_err() {
                git(&tmp, &["fetch", "-q", "--tags", "--", repository, "+refs/heads/*:refs/remotes/o/*"])?;
            }
            local_git(&tmp, &["checkout", "-q", "--detach", commit])
        })();
        if let Err(e) = done {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(bad("fetch", format!("not fetched with this person's access: {e}")));
        }
        std::fs::rename(&tmp, &at).map_err(|e| bad("fetch", format!("{} cannot be made: {e}", at.display())))?;
    }
    // Content, not names: HEAD is the commit, and every file is as it has it.
    let head = local_git(&at, &["rev-parse", "HEAD"]).map_err(|e| bad("commit", e))?;
    if head != commit {
        return Err(bad("commit", format!("{} is at {head}", at.display())));
    }
    let dirty = local_git(&at, &["status", "--porcelain", "--ignored", "--untracked-files=all"]).map_err(|e| bad("commit", e))?;
    if let Some(l) = dirty.lines().next() {
        return Err(bad("commit", format!("{} is not the commit's content ({}); remove it and run again", at.display(), l.trim())));
    }
    let files = local_git(&at, &["ls-files", "-s", "-z"]).map_err(|e| bad("commit", e))?;
    if let Some((check, why)) = tree_problem(repo, &files) {
        return Err(bad(check, why));
    }
    Ok(at)
}

/// Why a source's files, as `git ls-files -s -z` lists them, could change what
/// a play of goatcore's means, if they could. Its plays run with the
/// checkout's variables, roles, modules and plugins: a copy that carries any of
/// its own of a kind Ansible reads beside a play, wherever it is, is refused, and
/// so is a link or a submodule, whose content the commit does not name.
fn tree_problem(repo: &Path, listing: &str) -> Option<(&'static str, String)> {
    let goatcore_roles: std::collections::BTreeSet<String> = std::fs::read_dir(repo.join("ops/ansible/roles")).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    for entry in listing.split('\0').filter(|e| !e.is_empty()) {
        let (meta, path) = entry.split_once('\t').unwrap_or((entry, ""));
        let mode = meta.split(' ').next().unwrap_or_default();
        match mode {
            "120000" => return Some(("tree", format!("{path} is a link: a link's target is not the commit's content"))),
            "160000" => return Some(("tree", format!("{path} is a submodule, whose content the commit does not hold"))),
            _ => {}
        }
        let dirs: Vec<&str> = path.split('/').collect();
        let dirs = &dirs[..dirs.len().saturating_sub(1)];
        for d in dirs {
            let kind = match *d {
                "group_vars" | "host_vars" => Some("variables, which would win over the checkout's"),
                "library" | "module_utils" | "collections" => Some("modules or collections, which would replace those goatcore's roles call"),
                x if x.ends_with("_plugins") => Some("plugins, which would replace those goatcore's roles use"),
                _ => None,
            };
            if let Some(kind) = kind {
                return Some(("tree", format!("{path} is in a directory {d}/ of its own: {kind}")));
            }
            if goatcore_roles.contains(*d) {
                return Some(("role", format!("it carries a role {d}, which goatcore has: a source adds roles and replaces none ({path})")));
            }
        }
    }
    None
}

/// The play at `rel` inside a kept copy that [`kept`] checked: a file, as no
/// link on its way can be.
fn checked_play(at: &Path, rel: &str, bad: &dyn Fn(&str, String) -> anyhow::Error) -> Result<PathBuf> {
    let play = at.join(rel);
    if !std::fs::symlink_metadata(&play).is_ok_and(|m| m.is_file()) {
        return Err(bad("play", format!("it has no file {rel}")));
    }
    Ok(play)
}

/// Fetch `component`'s source at its commit, or take the copy an earlier run
/// fetched once it is shown to be that commit exactly. GB-385 otherwise.
pub(crate) fn fetch(repo: &Path, component: &str, s: &Source) -> Result<Fetched> {
    let short: String = s.commit.chars().take(12).collect();
    let bad = |check: &str, why: String| -> anyhow::Error { goat_errors::coded(CODE, format!("refused before any host: {component}'s play at {short} in its source repository: {why} (check: {check})")).into() };
    if let Some(p) = problem(component, &json!({"source": {"repository": s.repository, "commit": s.commit, "play": s.play}})) {
        return Err(bad("declaration", p));
    }
    let at = kept(repo, DIR, component, &s.repository, &s.commit, &bad)?;
    let play = checked_play(&at, &s.play, &bad)?;
    let roles_path = roles(repo, &at, &play);
    Ok(Fetched { play, roles_path })
}

/// The code a type's source is refused with.
pub(crate) const TYPE_CODE: &str = "GB-420";
/// Where a type's source is kept, apart from the components'.
const TYPE_DIR: &str = "goat-type-sources";

/// Where the type's source at `commit` is kept, fetched or not.
pub(crate) fn type_kept(repo: &Path, type_name: &str, commit: &str) -> PathBuf {
    absolute(Path::new(&crate::settings::state_dir_abs(repo))).join(TYPE_DIR).join(type_name).join(commit)
}

/// The roles path a play of a kept copy runs with: goatcore's roles first,
/// then those beside the play, then the copy's own.
pub(crate) fn type_roles(repo: &Path, at: &Path, play: &Path) -> String {
    roles(repo, at, play)
}

/// Fetch a type's source at its commit, or take the kept copy once it is
/// shown to be that commit exactly, and find each of `plays` in it. GB-420
/// otherwise, refused `before` what would run them.
pub(crate) fn fetch_type(repo: &Path, type_name: &str, repository: &str, commit: &str, plays: &[&str], before: &str) -> Result<PathBuf> {
    let short: String = commit.chars().take(12).collect();
    let bad = |check: &str, why: String| -> anyhow::Error { goat_errors::coded(TYPE_CODE, format!("refused before {before}: the type {type_name}'s plays at {short} in its source repository: {why} (check: {check})")).into() };
    if !repository_ok(repository) {
        return Err(bad("declaration", "`source.repository:` is a git URL (https, ssh or file) with no credential in it".into()));
    }
    if !regex::Regex::new(r"\A[0-9a-f]{40}\z").unwrap().is_match(commit) {
        return Err(bad("declaration", "`source.commit:` is a full commit of 40 hexadecimal characters; a branch or a tag can move".into()));
    }
    let at = kept(repo, TYPE_DIR, type_name, repository, commit, &bad)?;
    for p in plays {
        checked_play(&at, p, &bad)?;
    }
    Ok(at)
}

/// The hidden `source-play` verb, for ops/scripts/deploy-component.sh.
pub fn run(repo: &Path, component: &str) -> i32 {
    let spec = crate::pins::load_table(&repo.join("ops/pins/fleet-pins.yaml")).ok().and_then(|t| t.components.get(component).cloned());
    let got = match spec.as_ref().and_then(of) {
        Some(s) => fetch(repo, component, &s),
        None => Err(goat_errors::coded(CODE, format!("refused before any host: {component} declares no source in ops/pins/fleet-pins.yaml (check: declaration)")).into()),
    };
    match got {
        Ok(f) => {
            println!("{}", json!({"play": f.play, "roles_path": f.roles_path}));
            0
        }
        Err(e) => {
            println!("{}", json!({"refused": format!("{CODE} {e:#}")}));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    /// Every `git fetch` here ends its options with `--` before the repository,
    /// so no repository value is ever read as an option, whatever the URL
    /// pattern lets through.
    #[test]
    fn every_fetch_ends_its_options_before_the_repository() {
        let src = include_str!("source.rs");
        let fetches: Vec<&str> = src.lines().filter(|l| l.contains("&[\"fetch\"")).collect();
        assert!(fetches.len() >= 2, "{fetches:?}");
        for l in fetches {
            let (opts, rest) = l.split_once("\"--\", ").unwrap_or_else(|| panic!("no -- in: {l}"));
            assert!(rest.trim_start().starts_with("repository,"), "{l}");
            assert!(!opts.contains("repository"), "{l}");
        }
    }
}
