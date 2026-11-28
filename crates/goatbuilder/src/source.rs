//! A component whose play lives in another repository (`source:` in
//! ops/pins/fleet-pins.yaml; docs/design/goatbuilder-build-targets.md section
//! 3.10, gate J26). The repository, the full commit and the play's path are data
//! in the pins. The commit is fetched on the operator's machine with that
//! person's own git access, into a directory of its own under the state
//! directory, and nothing is put in the checkout. goatbuilder holds no
//! credential for it and asks for none.
//!
//! The commit is the whole proof: git names content by it, and a copy that is
//! not exactly that commit's is refused. A signed tag stands for a commit
//! when its signature is the declared signer's SSH key: it is fetched and
//! checked on every run, so the copy kept is the one the tag names now. A role
//! of goatcore's cannot be replaced: a source that carries a role of the same
//! name is refused.

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

/// What a source is pinned by: a full commit, or a tag signed by one SSH key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ref {
    Commit(String),
    Tag { name: String, signer: String },
}

impl Ref {
    /// The `commit`, `tag` and `signer` of a `source:` mapping.
    pub(crate) fn of(m: &Value) -> Ref {
        let t = |k: &str| m.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        match m.get("tag") {
            Some(_) => Ref::Tag { name: t("tag"), signer: t("signer") },
            None => Ref::Commit(t("commit")),
        }
    }

    /// How it is shown in a message: twelve characters of the commit, or the tag.
    pub(crate) fn shown(&self) -> String {
        match self {
            Ref::Commit(c) => c.chars().take(12).collect(),
            Ref::Tag { name, .. } => format!("tag {name}"),
        }
    }

    /// What its record is named by: the commit, or the tag.
    /// A tag's is its name with the repository and the signer it is from:
    /// two deployments that pin a tag of one name from two repositories, or
    /// with two keys, keep two records.
    pub(crate) fn label(&self, repository: &str) -> String {
        match self {
            Ref::Commit(c) => c.clone(),
            Ref::Tag { name, signer } => {
                use sha2::Digest;
                let of: String = sha2::Sha256::digest(format!("{repository}\n{signer}")).iter().take(6).map(|b| format!("{b:02x}")).collect();
                format!("tag-{}-{of}", name.replace('/', "%2F"))
            }
        }
    }
}

/// Why a `source:` mapping's `commit`, `tag` and `signer` pin nothing, if they
/// do not: the words both resolvers, and the type loader, give.
pub(crate) fn ref_problem(m: &Value) -> Option<String> {
    let re = |p: &str, t: &str| regex::Regex::new(p).unwrap().is_match(t);
    // A key with no value is no key, as a type file's loader reads it.
    let has = |k: &str| m.get(k).is_some_and(|v| !v.is_null());
    let text = |k: &str| m.get(k).and_then(|v| v.as_str()).unwrap_or("");
    if has("commit") && has("tag") {
        return Some("`source:` is pinned by a full commit or by a signed tag, not both".into());
    }
    if has("signer") && !has("tag") {
        return Some("`source.signer:` is the key that signs `source.tag:`, and there is no tag".into());
    }
    if !has("tag") {
        if !commit_ok(text("commit")) {
            return Some("`source.commit:` is a full commit of 40 hexadecimal characters; a branch or a tag can move".into());
        }
        return None;
    }
    let tag = text("tag");
    // It starts with a letter, and is no word a YAML reader of either version
    // takes for true or false, so that both resolvers read the same pins alike.
    let yaml_word = ["y", "n", "yes", "no", "true", "false", "on", "off"].contains(&tag.to_lowercase().as_str());
    if !re(r"\A[A-Za-z][A-Za-z0-9._/-]*\z", tag) || yaml_word || tag.contains("..") || tag.contains("//") || tag.ends_with('/') || tag.ends_with(".lock") {
        return Some("`source.tag:` is the name of a tag".into());
    }
    if !has("signer") {
        return Some("`source.tag:` is pinned only by its signer: name `source.signer:`, the SSH public key that signs it, or pin a full commit; a tag no one's key is held to can move".into());
    }
    if !re(r"\A(ssh-ed25519|ecdsa-sha2-nistp256|ecdsa-sha2-nistp384|ecdsa-sha2-nistp521|sk-ssh-ed25519@openssh\.com|sk-ecdsa-sha2-nistp256@openssh\.com) [A-Za-z0-9+/]+={0,3}\z", text("signer")) {
        return Some("`source.signer:` is an SSH public key, its type and its base64 text, with no comment".into());
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Source {
    pub repository: String,
    pub reference: Ref,
    pub play: String,
}

impl Source {
    /// The mapping it was read from.
    fn as_json(&self) -> Value {
        let mut m = json!({"repository": self.repository, "play": self.play});
        match &self.reference {
            Ref::Commit(c) => m["commit"] = json!(c),
            Ref::Tag { name, signer } => {
                m["tag"] = json!(name);
                m["signer"] = json!(signer);
            }
        }
        m
    }
}

/// A component's `source:`, as [`problem`] checked it.
pub(crate) fn of(spec: &Value) -> Option<Source> {
    let s = spec.get("source")?;
    let t = |k: &str| s.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    Some(Source { repository: t("repository"), reference: Ref::of(s), play: t("play") })
}

/// The keys a component's `source:` may have.
const KEYS: [&str; 5] = ["repository", "commit", "tag", "signer", "play"];

/// Why a `source:` mapping is not one, in the pins' words (GB-365); mirrored by
/// goat_consistency.resolve.check_source.
pub(crate) fn problem(cname: &str, spec: &Value) -> Option<String> {
    let src = spec.get("source")?;
    let Some(m) = src.as_object() else { return Some(format!("component '{cname}': `source:` is a mapping (repository, commit or tag and signer, play)")) };
    let mut unknown: Vec<&String> = m.keys().filter(|k| !KEYS.contains(&k.as_str())).collect();
    unknown.sort();
    if let Some(k) = unknown.first() {
        return Some(format!("component '{cname}': `source:` has an unknown key '{k}' (known: {})", KEYS.join(", ")));
    }
    let s = of(spec)?;
    let re = |p: &str, t: &str| regex::Regex::new(p).unwrap().is_match(t);
    if !repository_ok(&s.repository) {
        return Some(format!("component '{cname}': `source.repository:` is a git URL (https, ssh or file) with no credential in it"));
    }
    if let Some(why) = ref_problem(src) {
        return Some(format!("component '{cname}': {why}"));
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

pub(crate) fn is_play(repo: &Path, p: &Path) -> bool {
    kept_root(repo, p).is_some()
}

/// The kept copy a play is in: `<state_dir>/goat-sources/<key>/<label>`, a git
/// checkout. A path anywhere else, a checkout under a directory that happens
/// to have that name among them, is in none.
fn kept_root(repo: &Path, p: &Path) -> Option<PathBuf> {
    let rest = p.strip_prefix(state(repo).join(DIR)).ok()?;
    let mut parts = rest.components();
    let (key, label) = (parts.next()?, parts.next()?);
    parts.next()?;
    let root = state(repo).join(DIR).join(key).join(label);
    root.join(".git").is_dir().then_some(root)
}

/// The state directory, as an absolute path: a relative `deploy.state_dir` is
/// the checkout's, whatever directory goatbuilder or a phase runs in.
pub(crate) fn state(repo: &Path) -> PathBuf {
    let s = PathBuf::from(crate::settings::state_dir_abs(repo));
    if s.is_absolute() {
        s
    } else {
        absolute(repo).join(s)
    }
}

/// The play as `ansible-playbook` in ops/ansible takes it: by its name, or a
/// source's by its path.
pub(crate) fn play_word(repo: &Path, p: &Path) -> String {
    if is_play(repo, p) {
        return p.display().to_string();
    }
    p.file_name().unwrap_or_default().to_string_lossy().to_string()
}

/// The roles path a source's play runs with, read back from where it is.
pub(crate) fn roles_path(repo: &Path, play: &Path) -> Option<String> {
    Some(roles(repo, &kept_root(repo, play)?, play))
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

/// What `git rev-parse --local-env-vars` lists, the configuration a parent git
/// passes down, and what else would make git read files the copy's commit
/// does not name: a template for `init`, attributes from elsewhere, other git
/// programs.
const LOCAL_GIT_ENV: [&str; 21] = [
    "GIT_TEMPLATE_DIR",
    "GIT_DEFAULT_REF_FORMAT",
    "GIT_DEFAULT_HASH",
    "GIT_ATTR_SOURCE",
    "GIT_EXEC_PATH",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_INTERNAL_SUPER_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

fn git(at: &Path, args: &[&str]) -> Result<String, String> {
    git_in(at, args, true)
}

/// git on a kept copy alone, with none of the person's or the system's
/// configuration: no filter, hook or helper it names runs on the copy's files.
fn local_git(at: &Path, args: &[&str]) -> Result<String, String> {
    git_in(at, args, false)
}

fn git_in(at: &Path, args: &[&str], fetching: bool) -> Result<String, String> {
    git_bytes(at, args, fetching).map(|b| String::from_utf8_lossy(&b).trim().to_string())
}

fn git_bytes(at: &Path, args: &[&str], fetching: bool) -> Result<Vec<u8>, String> {
    let mut c = Cmd::new("git");
    // The person's own access, and no prompt: a run that would ask is refused.
    // No hook, and no file-system monitor: a monitor can tell status that a
    // changed file is unchanged.
    c.arg("-C").arg(at).args(["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", "-c", "protocol.ext.allow=never", "-c", "advice.detachedHead=false"]).args(args);
    c.env("GIT_TERMINAL_PROMPT", "0");
    c.env("GIT_LFS_SKIP_SMUDGE", "1");
    // A git that runs goatbuilder (a hook, rebase --exec, bisect run) exports
    // where its own repository is: none of it may point at another.
    for v in LOCAL_GIT_ENV {
        c.env_remove(v);
    }
    // Objects as they are, never a replacement ref's.
    c.env("GIT_NO_REPLACE_OBJECTS", "1");
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
    Ok(out.stdout)
}

/// A full commit: 40 lowercase hexadecimal characters, and none that a YAML
/// reader of either version takes for a number (all digits with one `e`, or
/// `0b` and binary digits), so that both resolvers read the same pins alike.
pub(crate) fn commit_ok(c: &str) -> bool {
    let re = |p: &str| regex::Regex::new(p).unwrap().is_match(c);
    re(r"\A[0-9a-f]{40}\z") && !re(r"\A[0-9]+([eE][0-9]+)?\z") && !re(r"\A0b[01]+\z")
}

/// The URL shape a `source.repository:` has: a git URL (https, ssh or file)
/// with no credential in it. A type's source is held to the same.
pub(crate) fn repository_ok(r: &str) -> bool {
    let re = |p: &str| regex::Regex::new(p).unwrap().is_match(r);
    re(r"\A(https://|ssh://|file:///|[A-Za-z0-9._-]+@[A-Za-z0-9.-]+:)[A-Za-z0-9._~/:@-]+\z") && !re(r"\Ahttps://[^/]*@") && !re(r"\Assh://[^/@]*:[^/@]*@")
}

/// Where the copy at `commit` is kept for `key` under `dir`. A copy is named
/// by its commit alone, so a run never replaces one another run is using: a
/// tag that names another commit now gets another copy.
fn kept_at(repo: &Path, dir: &str, key: &str, commit: &str) -> PathBuf {
    state(repo).join(dir).join(key).join(commit)
}

/// Set by a run that goes on to run the play: only then is a tag taken
/// recorded. Every other path (a dry run, verify, doctor, history, a plan)
/// judges a tag as a run does and records nothing.
static RECORDS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// This process runs a source's play: a tag it takes is recorded.
pub(crate) fn record_tags() {
    RECORDS.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// How far after this machine's clock a tag may say it was made: a clock a
/// little behind the signer's, and no more.
const TAG_CLOCK_SKEW_SECONDS: i64 = 24 * 3600;

/// Where the record of the last tag taken for a tag pin is, beside its lock:
/// one for each tag, repository and signer, whatever component or type pins it.
const TAG_RECORDS: &str = "goat-source-tags";

/// The kept copy of `repository` pinned by `r`, for `key` under `dir`:
/// a tag's signature checked first, then the commit fetched when it is not
/// kept, then the copy shown to be that commit exactly. `bad(check, why)`
/// makes the refusal.
fn resolve(repo: &Path, dir: &str, key: &str, repository: &str, r: &Ref, bad: &dyn Fn(&str, String) -> anyhow::Error) -> Result<PathBuf> {
    let base = state(repo).join(dir).join(key);
    match r {
        Ref::Commit(c) => {
            let at = kept_at(repo, dir, key, c);
            kept(repo, &at, repository, c, bad)?;
            Ok(at)
        }
        Ref::Tag { name, signer } => {
            let (c, made) = signed_commit(&base, repository, name, signer, bad)?;
            let records = state(repo).join(TAG_RECORDS);
            std::fs::create_dir_all(&records).map_err(|e| bad("tag", format!("{} cannot be made: {e}", records.display())))?;
            let record = records.join(format!("{}.accepted", r.label(repository)));
            let newer = |held: Option<(i64, String)>| -> Result<()> {
                match held {
                    Some((was, was_commit)) if made < was || (made == was && c != was_commit) => Err(bad("tag", format!("the tag {name} given now names {c}, made at {made}, and an earlier run here took one made at {was} naming {was_commit}: an older signed tag served again, or the tag moved back. If the signer moved it back on purpose, remove {} and run again", record.display()))),
                    _ => Ok(()),
                }
            };
            // Never older than the tag an earlier run took: a signed tag served
            // again after the signer moved it is a tag the signer replaced.
            newer(accepted_tag(&record, bad)?)?;
            let at = kept_at(repo, dir, key, &c);
            kept(repo, &at, repository, &c, bad)?;
            // A plan judges the tag and records nothing.
            if !RECORDS.load(std::sync::atomic::Ordering::Relaxed) {
                return Ok(at);
            }
            // Judged again and written under the record's lock, whole or not at
            // all: two runs at once never lower it.
            let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(records.join(format!("{}.lock", r.label(repository)))).map_err(|e| bad("tag", format!("the lock beside {} cannot be opened: {e}", record.display())))?;
            lock.lock().map_err(|e| bad("tag", format!("the lock beside {} cannot be taken: {e}", record.display())))?;
            newer(accepted_tag(&record, bad)?)?;
            let tmp = records.join(format!(".{}.accepted.{}", r.label(repository), std::process::id()));
            // On the disk before it takes the record's name: a crash leaves the
            // old record or the new one, never an empty one.
            let written = (|| {
                use std::io::Write;
                let mut f = std::fs::File::create(&tmp)?;
                f.write_all(format!("{made} {c}\n").as_bytes())?;
                f.sync_all()?;
                std::fs::rename(&tmp, &record)
            })();
            written.map_err(|e| bad("tag", format!("{} cannot be written: {e}", record.display())))?;
            Ok(at)
        }
    }
}

/// The tag an earlier run took for a source: when it was made, and its
/// commit. A record that cannot be read is refused, never read as none.
fn accepted_tag(record: &Path, bad: &dyn Fn(&str, String) -> anyhow::Error) -> Result<Option<(i64, String)>> {
    let text = match std::fs::read_to_string(record) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(bad("tag", format!("{}, the record of the tag an earlier run took, cannot be read ({e}); remove it only once the tag the repository gives is known to be the signer's latest", record.display()))),
    };
    let read = text.trim().split_once(' ').and_then(|(a, b)| Some((a.parse::<i64>().ok()?, b.to_string()))).filter(|(_, c)| regex::Regex::new(r"\A[0-9a-f]{40}\z").unwrap().is_match(c));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or_default();
    match read {
        Some((made, _)) if made > now + TAG_CLOCK_SKEW_SECONDS => Err(bad("tag", format!("{}, the record of the tag an earlier run took, says it was made at {made}, more than a day after this machine's clock: remove it only once the tag the repository gives is known to be the signer's latest", record.display()))),
        Some(r) => Ok(Some(r)),
        None => Err(bad("tag", format!("{}, the record of the tag an earlier run took, cannot be read; remove it only once the tag the repository gives is known to be the signer's latest", record.display()))),
    }
}

/// The commit `tag` of `repository` names, once its signature is shown to be
/// `signer`'s: an annotated tag of that name, with one SSH signature, which
/// ssh-keygen verifies for that key alone in git's namespace. Fetched with
/// the person's access into a scratch directory under `base`, removed after.
fn signed_commit(base: &Path, repository: &str, tag: &str, signer: &str, bad: &dyn Fn(&str, String) -> anyhow::Error) -> Result<(String, i64)> {
    let scratch = base.join(format!(".tag.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| bad("fetch", format!("{} cannot be made: {e}", scratch.display())))?;
    let out = signed_commit_in(&scratch, repository, tag, signer, bad);
    let _ = std::fs::remove_dir_all(&scratch);
    out
}

fn signed_commit_in(scratch: &Path, repository: &str, tag: &str, signer: &str, bad: &dyn Fn(&str, String) -> anyhow::Error) -> Result<(String, i64)> {
    let git_dir = scratch.join("git");
    std::fs::create_dir_all(&git_dir).map_err(|e| bad("fetch", format!("{} cannot be made: {e}", git_dir.display())))?;
    local_git(&git_dir, &["init", "-q", "--bare"]).map_err(|e| bad("fetch", e))?;
    let refname = format!("refs/tags/{tag}");
    git(&git_dir, &["fetch", "-q", "--no-tags", "--depth", "1", "--", repository, &format!("+{refname}:{refname}")]).map_err(|e| bad("fetch", format!("no tag {tag} fetched with this person's access: {e}")))?;
    let kind = local_git(&git_dir, &["cat-file", "-t", &refname]).map_err(|e| bad("tag", e))?;
    if kind != "tag" {
        return Err(bad("tag", format!("{tag} is a lightweight tag, which carries no signature: it can be moved by anyone who can push")));
    }
    let object = git_bytes(&git_dir, &["cat-file", "tag", &refname], false).map_err(|e| bad("tag", e))?;
    // One signature, an SSH one, at the end: what git would check, and nothing
    // a person's gpg keyring could vouch for. Found in the bytes as they are.
    let find = |needle: &[u8]| object.windows(needle.len()).position(|w| w == needle);
    let begins = object.windows(11).filter(|w| *w == b"-----BEGIN ").count();
    let Some(at) = find(b"-----BEGIN SSH SIGNATURE-----").filter(|_| begins == 1 && object.ends_with(b"-----END SSH SIGNATURE-----\n")) else {
        return Err(bad("signer", format!("{tag} carries no SSH signature of its own, so its signer cannot be {}", short_key(signer))));
    };
    let (payload, signature) = object.split_at(at);
    // What it says, read from the signed bytes alone, in git's own order.
    let head = payload.split(|b| *b == b'\n').take(4).map(|l| std::str::from_utf8(l).unwrap_or_default()).collect::<Vec<_>>();
    let line = |i: usize, k: &str| head.get(i).and_then(|l| l.strip_prefix(&format!("{k} "))).unwrap_or_default();
    if line(2, "tag") != tag {
        return Err(bad("tag", format!("the tag fetched as {tag} names itself '{}': it is another tag", line(2, "tag"))));
    }
    let commit = line(0, "object");
    if line(1, "type") != "commit" || !regex::Regex::new(r"\A[0-9a-f]{40}\z").unwrap().is_match(commit) {
        return Err(bad("tag", format!("{tag} does not name one commit")));
    }
    // The time is the first word after the tagger's address, as git reads it.
    let made: Option<i64> = line(3, "tagger").rsplit_once('>').and_then(|(_, t)| t.split_whitespace().next()).and_then(|t| t.parse().ok()).filter(|t| *t >= 0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or_default();
    if let Some(m) = made.filter(|m| *m > now + TAG_CLOCK_SKEW_SECONDS) {
        return Err(bad("tag", format!("{tag} says it was made at {m}, more than a day after this machine's clock ({now}): a tag from the future would refuse every later one")));
    }
    let Some(made) = made else {
        return Err(bad("tag", format!("{tag} says neither who made it nor when, so a later run cannot tell it from an older one")));
    };
    let allowed = scratch.join("allowed_signers");
    let sig = scratch.join("tag.sig");
    let write = |p: &Path, b: &[u8]| std::fs::write(p, b).map_err(|e| bad("signer", format!("{} cannot be written: {e}", p.display())));
    write(&allowed, format!("goat-source namespaces=\"git\" {signer}\n").as_bytes())?;
    write(&sig, signature)?;
    let signed = scratch.join("tag.payload");
    write(&signed, payload)?;
    let stdin = std::fs::File::open(&signed).map_err(|e| bad("signer", format!("{} cannot be read: {e}", signed.display())))?;
    let mut c = Cmd::new("ssh-keygen");
    c.args(["-Y", "verify", "-n", "git", "-I", "goat-source", "-f"]).arg(&allowed).arg("-s").arg(&sig);
    c.stdin(stdin);
    let verified = c.output();
    match verified {
        Ok(o) if o.status.success() => Ok((commit.to_string(), made)),
        Ok(_) => Err(bad("signer", format!("{tag} is not signed by {}", short_key(signer)))),
        Err(e) => Err(bad("signer", format!("ssh-keygen, which checks a tag's signature, could not be run: {e}"))),
    }
}

/// A key as a message shows it: its type and the start of its text.
fn short_key(k: &str) -> String {
    let (kind, body) = k.split_once(' ').unwrap_or((k, ""));
    format!("the {kind} key {}...", body.chars().take(16).collect::<String>())
}

/// `repository` at `commit`, kept at `at`: fetched there when it is not, then
/// shown to be that commit exactly.
fn kept(repo: &Path, at: &Path, repository: &str, commit: &str, bad: &dyn Fn(&str, String) -> anyhow::Error) -> Result<()> {
    let base = at.parent().unwrap_or(at);
    if !at.is_dir() {
        let tmp = base.join(format!(".{commit}.{}", std::process::id()));
        std::fs::create_dir_all(&tmp).map_err(|e| bad("fetch", format!("{} cannot be made: {e}", tmp.display())))?;
        let done = (|| {
            // No template: nothing a template directory holds comes with it.
            local_git(&tmp, &["init", "-q", "--template="])?;
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
        if let Err(e) = std::fs::rename(&tmp, at) {
            let _ = std::fs::remove_dir_all(&tmp);
            // Another run kept the same commit first: it is checked below as
            // any kept copy is.
            if !at.is_dir() {
                return Err(bad("fetch", format!("{} cannot be made: {e}", at.display())));
            }
        }
    }
    // Its git is only what goatbuilder made: read before any git runs on it.
    if let Some(why) = foreign_git(at) {
        return Err(bad("commit", format!("{}: {why}; remove it and run again", at.display())));
    }
    // Content, not names: HEAD is the commit, and every file is as it has it.
    let head = local_git(at, &["rev-parse", "HEAD"]).map_err(|e| bad("commit", e))?;
    if head != commit {
        return Err(bad("commit", format!("{} is at {head}", at.display())));
    }
    let dirty = local_git(at, &["status", "--porcelain", "--ignored", "--untracked-files=all"]).map_err(|e| bad("commit", e))?;
    if let Some(l) = dirty.lines().next() {
        return Err(bad("commit", format!("{} is not the commit's content ({}); remove it and run again", at.display(), l.trim())));
    }
    // The index's own word is not trusted: an entry marked assume-unchanged
    // or skip-worktree is one status does not look at.
    let flags = local_git(at, &["ls-files", "-v", "-z"]).map_err(|e| bad("commit", e))?;
    if let Some(e) = flags.split('\0').find(|e| !e.is_empty() && !e.starts_with("H ")) {
        return Err(bad("commit", format!("{} marks {} so that git does not compare it with the commit; remove it and run again", at.display(), e.get(2..).unwrap_or(e))));
    }
    let files = local_git(at, &["ls-tree", "-r", "-z", "HEAD"]).map_err(|e| bad("commit", e))?;
    if let Some((check, why)) = tree_problem(repo, &files) {
        return Err(bad(check, why));
    }
    Ok(())
}

/// What in a kept copy's `.git` goatbuilder never writes and git would act
/// on: a configuration key beyond what `git init` writes (a work tree
/// elsewhere, a file-system monitor, a filter), attributes, grafts, other
/// object stores.
fn foreign_git(at: &Path) -> Option<String> {
    let g = at.join(".git");
    if !std::fs::symlink_metadata(&g).is_ok_and(|m| m.is_dir()) {
        return Some(".git is not a directory of its own".into());
    }
    for f in ["info/attributes", "info/grafts", "objects/info/alternates", "objects/info/http-alternates", "commondir"] {
        if std::fs::symlink_metadata(g.join(f)).is_ok() {
            return Some(format!(".git/{f} is there, which goatbuilder never writes"));
        }
    }
    // Read as git reads it, and nothing it includes.
    let listed = match git_in(at, &["config", "--file", ".git/config", "--no-includes", "--list", "-z"], false) {
        Ok(l) => l,
        Err(e) => return Some(format!(".git/config cannot be read ({e})")),
    };
    for entry in listed.split('\0').filter(|e| !e.is_empty()) {
        let key = entry.split('\n').next().unwrap_or_default().to_lowercase();
        let ok = ["core.repositoryformatversion", "core.filemode", "core.bare", "core.logallrefupdates", "core.ignorecase", "core.precomposeunicode", "core.symlinks", "extensions.objectformat", "extensions.refstorage"].contains(&key.as_str());
        if !ok {
            return Some(format!(".git/config sets {key}, which goatbuilder never writes"));
        }
    }
    None
}

/// Why a source's files, as `git ls-tree -r -z HEAD` lists them, could change what
/// a play of goatcore's means, if they could. Its plays run with the
/// checkout's variables, roles, modules and plugins: a copy that carries any of
/// its own of a kind Ansible reads beside a play, wherever it is, is refused, and
/// so is a link or a submodule, whose content the commit does not name.
fn tree_problem(repo: &Path, listing: &str) -> Option<(&'static str, String)> {
    // Names are compared without regard to case: a file system that ignores
    // case (macOS's, by default) finds Zenoh_Stack/ when a play asks for zenoh_stack.
    let goatcore_roles: std::collections::BTreeSet<String> = std::fs::read_dir(repo.join("ops/ansible/roles")).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect();
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
        // Letters a file system may fold or normalise into another name.
        if let Some(d) = dirs.iter().find(|d| !d.is_ascii()) {
            return Some(("tree", format!("{path} is in a directory {d}/, whose name is not plain ASCII: a file system may take it for another name")));
        }
        for d in dirs {
            let lower = d.to_lowercase();
            let kind = match lower.as_str() {
                "group_vars" | "host_vars" => Some("variables, which would win over the checkout's"),
                "library" | "module_utils" | "collections" => Some("modules or collections, which would replace those goatcore's roles call"),
                x if x.ends_with("_plugins") => Some("plugins, which would replace those goatcore's roles use"),
                _ => None,
            };
            if let Some(kind) = kind {
                return Some(("tree", format!("{path} is in a directory {d}/ of its own: {kind}")));
            }
            if goatcore_roles.contains(&lower) {
                return Some(("role", format!("it carries a role {d}, which goatcore has: a source adds roles and replaces none ({path})")));
            }
        }
    }
    None
}

/// The play at `rel` inside a kept copy that [`kept`] checked: a file, as no
/// link on its way can be, and none of whose neighbours a role of goatcore's
/// would read in place of the checkout's own.
fn checked_play(repo: &Path, at: &Path, rel: &str, bad: &dyn Fn(&str, String) -> anyhow::Error) -> Result<PathBuf> {
    let play = at.join(rel);
    if !std::fs::symlink_metadata(&play).is_ok_and(|m| m.is_file()) {
        return Err(bad("play", format!("it has no file {rel}")));
    }
    let files = local_git(at, &["ls-tree", "-r", "-z", "--name-only", "HEAD"]).map_err(|e| bad("commit", e))?;
    let files: Vec<&str> = files.split('\0').filter(|f| !f.is_empty()).collect();
    if let Some((reads, here)) = beside_the_play(repo, rel, &files) {
        return Err(bad("tree", format!("{here} is where goatcore's roles read {reads} from, beside the playbook: run from {rel}, they would read the source's in place of the checkout's")));
    }
    Ok(play)
}

/// What a role of goatcore's reads beside the playbook (`{{ playbook_dir }}/...`,
/// which for a source's play is the play's own directory in the copy), as
/// paths from it: `../enrollment/ca/`, `netbird_acl_taxonomy.yml`.
fn read_beside_the_playbook(repo: &Path) -> std::collections::BTreeSet<String> {
    let re = regex::Regex::new(r#"playbook_dir(?:\s*\}\}|\s*~\s*'|\s*\|\s*dirname\s*\}\}(/\.\.)?)(/[A-Za-z0-9_./-]+)"#).unwrap();
    let mut out = std::collections::BTreeSet::new();
    let mut stack = vec![repo.join("ops/ansible/roles")];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(text) = std::fs::read_to_string(&p) {
                for c in re.captures_iter(&text) {
                    // `playbook_dir | dirname` is the directory above it.
                    let up = if c.get(0).is_some_and(|m| m.as_str().contains("dirname")) { "/.." } else { "" };
                    out.insert(format!("{up}{}{}", c.get(1).map_or("", |m| m.as_str()), &c[2]).trim_start_matches('/').to_string());
                }
            }
        }
    }
    out
}

/// The first of goatcore's reads beside the playbook that the copy answers,
/// for a play at `rel`: the read, and the copy's path it would take.
fn beside_the_play(repo: &Path, rel: &str, files: &[&str]) -> Option<(String, String)> {
    let dir: Vec<&str> = rel.split('/').collect();
    let dir = &dir[..dir.len() - 1];
    for read in read_beside_the_playbook(repo) {
        let mut at: Vec<&str> = dir.to_vec();
        let mut outside = false;
        for part in read.split('/').filter(|p| !p.is_empty() && *p != ".") {
            if part == ".." {
                outside |= at.pop().is_none();
            } else {
                at.push(part);
            }
        }
        if outside || at.is_empty() {
            continue;
        }
        let target = at.join("/");
        let lower = target.to_lowercase();
        if let Some(hit) = files.iter().find(|f| {
            let f = f.to_lowercase();
            f == lower || f.starts_with(&format!("{lower}/"))
        }) {
            return Some((format!("{{{{ playbook_dir }}}}/{read}"), hit.to_string()));
        }
    }
    None
}

/// Fetch `component`'s source at its commit, or take the copy an earlier run
/// fetched once it is shown to be that commit exactly. GB-385 otherwise.
pub(crate) fn fetch(repo: &Path, component: &str, s: &Source) -> Result<Fetched> {
    let short = s.reference.shown();
    let bad = |check: &str, why: String| -> anyhow::Error { goat_errors::coded(CODE, format!("refused before any host: {component}'s play at {short} in its source repository: {why} (check: {check})")).into() };
    if let Some(p) = problem(component, &json!({"source": s.as_json()})) {
        return Err(bad("declaration", p));
    }
    let at = resolve(repo, DIR, component, &s.repository, &s.reference, &bad)?;
    let play = checked_play(repo, &at, &s.play, &bad)?;
    let roles_path = roles(repo, &at, &play);
    Ok(Fetched { play, roles_path })
}

/// The code a type's source is refused with.
pub(crate) const TYPE_CODE: &str = "GB-420";
/// Where a type's source is kept, apart from the components'.
const TYPE_DIR: &str = "goat-type-sources";

/// Where the type's source at `commit` is kept, fetched or not.
/// Where a type's copy is kept, when its pin says: a commit's is known before
/// any run, a tag's only once a run has checked the tag.
pub(crate) fn type_kept(repo: &Path, type_name: &str, r: &Ref) -> Option<PathBuf> {
    match r {
        Ref::Commit(c) => Some(kept_at(repo, TYPE_DIR, type_name, c)),
        Ref::Tag { .. } => None,
    }
}

/// The roles path a play of a kept copy runs with: goatcore's roles first,
/// then those beside the play, then the copy's own.
pub(crate) fn type_roles(repo: &Path, at: &Path, play: &Path) -> String {
    roles(repo, at, play)
}

/// Fetch a type's source at its commit, or take the kept copy once it is
/// shown to be that commit exactly, and find each of `plays` in it. GB-420
/// otherwise, refused `before` what would run them.
pub(crate) fn fetch_type(repo: &Path, type_name: &str, src: &Value, plays: &[&str], before: &str) -> Result<PathBuf> {
    let r = Ref::of(src);
    let short = r.shown();
    let bad = |check: &str, why: String| -> anyhow::Error { goat_errors::coded(TYPE_CODE, format!("refused before {before}: the type {type_name}'s plays at {short} in its source repository: {why} (check: {check})")).into() };
    let repository = src.get("repository").and_then(|v| v.as_str()).unwrap_or_default();
    if !repository_ok(repository) {
        return Err(bad("declaration", "`source.repository:` is a git URL (https, ssh or file) with no credential in it".into()));
    }
    if let Some(why) = ref_problem(src) {
        return Err(bad("declaration", why));
    }
    let at = resolve(repo, TYPE_DIR, type_name, repository, &r, &bad)?;
    for p in plays {
        checked_play(repo, &at, p, &bad)?;
    }
    Ok(at)
}

/// The hidden `source-play` verb, for ops/scripts/deploy-component.sh; a tag
/// taken is recorded only with `--record`, which the script passes when it
/// goes on to run the play.
pub fn run(repo: &Path, component: &str, record: bool) -> i32 {
    if record {
        record_tags();
    }
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
