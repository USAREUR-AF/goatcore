//! `goatbuilder federate plan <partner>`: the bounded plan of one peering,
//! written before anything changes (docs/design/federation-plan.md).
//!
//! The plan says what a run would change and what it rests on: the versions
//! of the tools and images, the agreement digest, a hash of every rendered
//! configuration file, the resources the peering owns, the restarts it needs,
//! the acceptance policy, phase 1's readiness checks and a host firewall entry
//! per host. It is written to `<work>/plan.json` with a hash over everything
//! that an approval binds to, so the same inputs always give the same hash and
//! any changed input gives a different one.
//!
//! A saved plan is not proof that the environment stayed the same.
//! [`recheck_live_prerequisites`] is called immediately before anything
//! changes: it refuses when the plan file was altered, when any planned input
//! changed since (a new plan is needed), or when a live prerequisite (ports,
//! slot list, core router, receive stores) no longer holds.
//!
//! The plan reads; it changes nothing on any host. Its only writes are under
//! `<work>`: `plan.json` and phase 1's result files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::hostalloc;
use super::hostfw;
use super::preflight;
use crate::config;
use crate::{Check, Report, EXIT_REFUSED};

/// The schema of `<work>/plan.json`.
pub const SCHEMA: &str = "goat-federation-plan/v1";

/// The phase-1 checks that read live state, rerun just before a change.
pub const LIVE_CHECKS: &[&str] = &[
    "edge-ports",
    "core-ports",
    "slot-anchors",
    "core-router",
    "ingress-stores",
    "clock",
];

pub struct PlanOptions<'a> {
    pub with: &'a str,
    pub env: Option<&'a str>,
    pub r#as: Option<&'a str>,
    pub edge_group: Option<&'a str>,
    pub work: Option<&'a str>,
    /// A fixture directory whose `goatbuilder.yaml` supplies the config. A mock
    /// runs nothing and writes nothing: the plan is printed, not saved.
    pub mock: Option<&'a Path>,
    /// Run phase 1's live reads (false sets FED_PREFLIGHT_NO_LIVE=1: each is SKIP).
    pub live: bool,
}

/// One input value the plan resolved, and where it came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resolved {
    pub name: String,
    /// `None` when nothing declares it (or it is templated): the plan names
    /// where to declare it in `source`.
    pub value: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Versions {
    pub goatbuilder: String,
    /// `git rev-parse HEAD` of the checkout, with `-dirty` when it has changes.
    pub source: String,
    /// `goat --version`, the binary phases 3 to 9 run.
    pub goat: String,
    pub images: Vec<Resolved>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flow {
    pub from: String,
    pub to: String,
    pub publish_under: String,
    pub release: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgreementFacts {
    pub path: String,
    /// sha256 over the canonical bytes, as `goat federate compose` prints it.
    pub digest: String,
    pub transport: String,
    /// `transport.link_subnet`, the link the two edges share.
    #[serde(default)]
    pub link_subnet: Option<String>,
    pub parties: Vec<String>,
    pub flows: Vec<Flow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileHash {
    /// Relative to the rendered directory.
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CertFact {
    pub file: String,
    pub subject: String,
    pub not_after: String,
}

/// Something this peering owns. A run creates, changes or removes only these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    pub kind: String,
    /// `None` when its value is not declared yet; `source` says where.
    pub name: Option<String>,
    /// The inventory group of the host it lives on, or `controller`.
    pub on: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Restart {
    pub what: String,
    pub phase: u32,
    /// `yes`, `no`, or `depends on the agreement` before one is composed.
    pub required: String,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptancePolicy {
    pub runner: String,
    /// FED_ACCEPTANCE_STRICT=1: a skipped check fails acceptance.
    pub strict: bool,
    /// What acceptance must show, one line per required check, from the
    /// agreement's flows. Empty before an agreement is composed.
    pub required: Vec<String>,
}

/// One phase-1 check's outcome. `detail` is not part of the hash: it can name
/// a process id or a time; the outcome and the code are what an approval reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Readiness {
    pub check: String,
    pub outcome: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub detail: String,
}

/// The host firewall entry for one host (federate/hostfw.rs).
pub use super::hostfw::HostFirewall;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub schema: String,
    pub peering: String,
    pub environment: String,
    pub site: String,
    /// `--as`, when given.
    #[serde(default)]
    pub us: Option<String>,
    pub edge_group: String,
    pub core_group: String,
    pub work: String,
    /// When the plan was made (RFC 3339, UTC). Outside the hash, so the same
    /// inputs give the same hash; the recheck refuses a plan older than
    /// `FED_PLAN_MAX_AGE_HOURS` (default 24).
    #[serde(default)]
    pub created_at: String,
    pub versions: Versions,
    /// Every value phase 1 and the live checks read, as the run will see it:
    /// set in the environment, else derived. Paths, URLs, groups and the core
    /// router's daemon; a moved host or another key path is a changed input.
    pub inputs: Vec<Resolved>,
    pub agreement: Option<AgreementFacts>,
    /// `<work>/applied`, file by file. Empty before phase 9 renders it.
    pub rendered: Vec<FileHash>,
    /// `<work>/pki`, file by file: a re-minted certificate is a changed input.
    pub pki: Vec<FileHash>,
    pub certificates: Vec<CertFact>,
    pub resources: Vec<Resource>,
    pub restarts: Vec<Restart>,
    pub acceptance: AcceptancePolicy,
    pub readiness: Vec<Readiness>,
    pub host_firewall: Vec<HostFirewall>,
    /// What this peering and another declared peering would share on one
    /// machine (the host-level allocation check, federate/hostalloc.rs). Only
    /// the conflicts this peering is part of.
    pub conflicts: Vec<hostalloc::Conflict>,
    /// READY, `READY, <n> check(s) not verified` (SKIP is never a pass), or BLOCKED.
    pub verdict: String,
    /// sha256 over the plan without this field and without readiness details.
    pub hash: String,
    /// The goatbuilder that made the plan (`goatbuilder <version>`), so a
    /// build that cannot read it can say which one did.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub written_by: String,
}

/// The sections an input change can move, by name, for the recheck's report.
const SECTIONS: &[&str] = &[
    "versions",
    "inputs",
    "agreement",
    "rendered",
    "pki",
    "certificates",
    "resources",
    "restarts",
    "acceptance",
    "host_firewall",
    "conflicts",
];

/// sha256 of bytes, as hex.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// The plan's hash: its JSON without `hash`, readiness details removed. The
/// field order is the struct's (serde_json keeps insertion order here), so the
/// same plan always serializes to the same bytes.
pub fn plan_hash(p: &Plan) -> String {
    hash_of_value(serde_json::to_value(p).expect("a plan serializes"))
}

/// The plan hash over a plan as a JSON value: the file as written, or a plan
/// this build serialized.
fn hash_of_value(mut v: serde_json::Value) -> String {
    if let Some(o) = v.as_object_mut() {
        o.remove("hash");
        o.remove("created_at");
        if let Some(rs) = o.get_mut("readiness").and_then(|r| r.as_array_mut()) {
            for r in rs {
                if let Some(ro) = r.as_object_mut() {
                    ro.remove("detail");
                }
            }
        }
    }
    format!(
        "sha256:{}",
        sha256_hex(&serde_json::to_vec(&v).expect("a value serializes"))
    )
}

/// The value a group's own vars give `key`, unparsed: group_vars/<group>.yml,
/// then each file in group_vars/<group>/ in name order, the last that sets it
/// winning (as Ansible merges them).
pub(super) fn group_raw(repo: &Path, group: &str, key: &str) -> Option<serde_yaml::Value> {
    let gv = repo.join("ops/ansible/group_vars");
    let mut files = vec![
        gv.join(format!("{group}.yml")),
        gv.join(format!("{group}.yaml")),
    ];
    if let Ok(rd) = std::fs::read_dir(gv.join(group)) {
        let mut more: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml"))
            .collect();
        more.sort();
        files.extend(more);
    }
    let mut found = None;
    for f in files {
        let Some(doc) = std::fs::read_to_string(&f)
            .ok()
            .and_then(|t| serde_yaml::from_str::<serde_yaml::Value>(&t).ok())
        else {
            continue;
        };
        // A null is unset, as in Python's reading: it neither declares nor overrides.
        if let Some(v) = doc.get(key).filter(|v| !v.is_null()) {
            found = Some(v.clone());
        }
    }
    found
}

fn yaml_scalar(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A value the plays will bind, resolved the way phase 1 resolves it
/// (preflight_live.py `resolve_port_inputs`): the environment variable, then
/// the group's vars, its parent group's and `all`, then the role's default.
/// A templated declaration resolves only inside a play, so it is not guessed.
pub(super) fn resolve(
    repo: &Path,
    name: &str,
    env_name: Option<&str>,
    groups: &[&str],
    role: &str,
    key: &str,
) -> Resolved {
    let r = |value: Option<String>, source: String| Resolved {
        name: name.to_string(),
        value: value.filter(|v| !v.is_empty()),
        source,
    };
    if let Some(e) = env_name {
        if let Some(v) = crate::federate::runenv::var(e)
            .ok()
            .filter(|v| !v.is_empty())
        {
            return r(Some(v), format!("${e}"));
        }
    }
    for g in groups {
        if let Some(raw) = group_raw(repo, g, key) {
            return match yaml_scalar(&raw) {
                Some(s) if s.contains("{{") => r(
                    None,
                    format!("group_vars/{g}: {key} is templated and resolves only in a play"),
                ),
                Some(s) => r(Some(s), format!("group_vars/{g}: {key}")),
                None => r(None, format!("group_vars/{g}: {key} is not a single value")),
            };
        }
    }
    let defaults = repo.join(format!("ops/ansible/roles/{role}/defaults/main.yml"));
    let default = std::fs::read_to_string(defaults)
        .ok()
        .and_then(|t| serde_yaml::from_str::<serde_yaml::Value>(&t).ok())
        .and_then(|d| d.get(key).and_then(yaml_scalar))
        .filter(|s| !s.contains("{{") && !s.is_empty());
    match default {
        Some(v) => r(Some(v), format!("roles/{role} default {key}")),
        None => r(None, format!("{key} in group_vars/{}/", groups[0])),
    }
}

/// The peering inventory as the plays read it: the hand-written (or whole
/// rendered) `ops/ansible/inventory/peering-edges.ini`, then each per-peering
/// file `goatbuilder federate allocate` renders beside it when not every
/// peering declares `hosts:` (`peering-<edge group>.ini`,
/// `peering_files::one_inventory_path`), in name order. The driver passes the
/// same files with `-i`, so a group is read from whichever holds it.
pub fn peering_inventory_text(repo: &Path) -> String {
    let dir = repo.join("ops/ansible/inventory");
    let mut text = std::fs::read_to_string(dir.join("peering-edges.ini")).unwrap_or_default();
    for p in per_peering_inventories(repo) {
        if let Ok(t) = std::fs::read_to_string(&p) {
            text.push('\n');
            text.push_str(&t);
        }
    }
    text
}

/// The `-i` arguments every play of a peering takes, relative to ops/ansible:
/// `inventory/peering-edges.ini`, then each per-peering file
/// ([`per_peering_inventories`]). The admission (admit.rs) and the back-out
/// (backout.rs) both use it, so the back-out reaches every host the forward
/// path reached.
pub fn inventory_args(repo: &Path) -> String {
    let mut a = String::from("-i inventory/peering-edges.ini");
    for p in per_peering_inventories(repo) {
        if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
            a.push_str(&format!(" -i '{}'", format!("inventory/{n}").replace('\'', "'\\''")));
        }
    }
    a
}

/// The per-peering inventory files beside peering-edges.ini, in name order.
pub fn per_peering_inventories(repo: &Path) -> Vec<PathBuf> {
    let dir = repo.join("ops/ansible/inventory");
    let mut own: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                        n.starts_with("peering-") && n.ends_with(".ini") && n != "peering-edges.ini"
                    }) && p.is_file()
                })
                .collect()
        })
        .unwrap_or_default();
    own.sort();
    own
}

/// The hosts of one inventory group in the peering inventory
/// ([`peering_inventory_text`]: peering-edges.ini and the per-peering files),
/// as (inventory name, address), read the way Ansible reads an INI inventory
/// for what the plan needs: `[group:children]` expanded, `[group:vars]`
/// sections skipped, a header's trailing comment ignored. The address is
/// `ansible_host` from any line that names the host (quotes removed), else
/// from `host_vars/<host>.yml` or `host_vars/<host>/*.yml`, else the name. Host
/// ranges such as `edge[01:02]` are not expanded.
pub fn inventory_hosts(repo: &Path, group: &str) -> Vec<(String, String)> {
    let text = peering_inventory_text(repo);
    let unquote = |v: &str| v.trim_matches(|c| c == '"' || c == '\'').to_string();
    let mut members: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut address: BTreeMap<String, String> = BTreeMap::new();
    // (group, kind): kind is "" for hosts, "children" or "vars".
    let mut section: Option<(String, String)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest.split(']').next().unwrap_or_default().trim();
            section = Some(match name.split_once(':') {
                Some((g, k)) => (g.to_string(), k.to_string()),
                None => (name.to_string(), String::new()),
            });
            continue;
        }
        let Some((g, kind)) = &section else { continue };
        let body = line.split(" #").next().unwrap_or(line);
        let mut words = body.split_whitespace();
        let Some(name) = words.next() else { continue };
        match kind.as_str() {
            "" => {
                members.entry(g.clone()).or_default().push(name.to_string());
                for w in words {
                    if let Some(v) = w.strip_prefix("ansible_host=") {
                        address.insert(name.to_string(), unquote(v));
                    }
                }
            }
            "children" => children
                .entry(g.clone())
                .or_default()
                .push(name.to_string()),
            _ => {}
        }
    }
    fn expand(
        g: &str,
        members: &BTreeMap<String, Vec<String>>,
        children: &BTreeMap<String, Vec<String>>,
        seen: &mut Vec<String>,
        out: &mut Vec<String>,
    ) {
        if seen.iter().any(|s| s == g) {
            return;
        }
        seen.push(g.to_string());
        for h in members.get(g).into_iter().flatten() {
            if !out.contains(h) {
                out.push(h.clone());
            }
        }
        for c in children.get(g).into_iter().flatten() {
            expand(c, members, children, seen, out);
        }
    }
    let mut names = Vec::new();
    expand(group, &members, &children, &mut Vec::new(), &mut names);
    names
        .into_iter()
        .map(|n| {
            let addr = address
                .get(&n)
                .cloned()
                .or_else(|| host_var(repo, &n, "ansible_host").map(|v| unquote(&v)))
                .unwrap_or_else(|| n.clone());
            (n, addr)
        })
        .collect()
}

/// A plain value from host_vars/<host>.yml, then host_vars/<host>/*.yml in
/// name order (the last that sets it wins).
fn host_var(repo: &Path, host: &str, key: &str) -> Option<String> {
    let hv = repo.join("ops/ansible/host_vars");
    let mut files = vec![
        hv.join(format!("{host}.yml")),
        hv.join(format!("{host}.yaml")),
    ];
    if let Ok(rd) = std::fs::read_dir(hv.join(host)) {
        let mut more: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        more.sort();
        files.extend(more);
    }
    let mut found = None;
    for f in files {
        let Some(doc) = std::fs::read_to_string(&f)
            .ok()
            .and_then(|t| serde_yaml::from_str::<serde_yaml::Value>(&t).ok())
        else {
            continue;
        };
        if let Some(v) = doc.get(key).and_then(yaml_scalar) {
            found = Some(v);
        }
    }
    found
}

fn source_version(repo: &Path) -> String {
    let git = |args: &[&str]| {
        crate::federate::runenv::command("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    match git(&["rev-parse", "HEAD"]) {
        Some(head) => {
            let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|s| !s.is_empty());
            if dirty {
                format!("{head}-dirty")
            } else {
                head
            }
        }
        None => "unknown (not a git checkout)".to_string(),
    }
}

fn goat_version() -> String {
    let bin = crate::federate::runenv::var("GOAT_BIN")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "goat".to_string());
    crate::federate::runenv::command(&bin)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .map(|l| l.trim().to_string())
        })
        .unwrap_or_else(|| format!("not found ({bin} --version did not answer)"))
}

/// The agreement phase 6 wrote, when there is one. Its digest is over the
/// file's bytes, which are the canonical form.
pub fn agreement_facts(work: &str) -> Result<Option<AgreementFacts>> {
    let path = Path::new(work).join("agreement.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return Ok(None);
    };
    let v: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not JSON", path.display()))?;
    let s = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
    let flows = v["flows"]
        .as_array()
        .map(|fs| {
            fs.iter()
                .map(|f| Flow {
                    from: s(&f["from"]),
                    to: s(&f["to"]),
                    publish_under: s(&f["publish_under"]),
                    release: f["release"]
                        .as_array()
                        .map(|r| r.iter().map(s).collect())
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(AgreementFacts {
        path: path.display().to_string(),
        digest: format!("sha256:{}", sha256_hex(&bytes)),
        transport: s(&v["transport"]["kind"]),
        link_subnet: v["transport"]["link_subnet"]
            .as_str()
            .filter(|x| !x.is_empty())
            .map(String::from),
        parties: v["parties"]
            .as_array()
            .map(|ps| ps.iter().map(|p| s(&p["sovereign_id"])).collect())
            .unwrap_or_default(),
        flows,
    }))
}

/// Every file under `dir`, relative, sorted, with its sha256.
pub fn rendered_hashes(dir: &Path) -> Vec<FileHash> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<FileHash>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            // The entry's own type: a symlinked directory is not walked (no loop).
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            if is_dir {
                walk(base, &p, out);
            } else if let Ok(bytes) = std::fs::read(&p) {
                out.push(FileHash {
                    path: p.strip_prefix(base).unwrap_or(&p).display().to_string(),
                    sha256: sha256_hex(&bytes),
                });
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// The certificates phase 8 minted into `<work>/pki`: subject and expiry,
/// and a readiness row for each one expired. A key file, or anything openssl
/// does not read as a certificate, is left out; with files present and no
/// openssl to read them, expiry is SKIP, never a pass.
pub fn certificates(pki: &Path) -> (Vec<CertFact>, Vec<Readiness>) {
    let Ok(rd) = std::fs::read_dir(pki) else {
        return (Vec::new(), Vec::new());
    };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "pem" || x == "crt"))
        .collect();
    files.sort();
    let (mut facts, mut rows) = (Vec::new(), Vec::new());
    let openssl = |args: &[&str], f: &Path| {
        crate::federate::runenv::command("openssl")
            .args(args)
            .arg(f)
            .output()
    };
    for f in files {
        let name = f
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let o = match openssl(&["x509", "-noout", "-subject", "-enddate", "-in"], &f) {
            Ok(o) => o,
            Err(e) => {
                rows.push(Readiness {
                    check: "certificates".into(),
                    outcome: "SKIP".into(),
                    code: None,
                    detail: format!(
                        "{name}: openssl could not run ({e}), so its expiry is not verified"
                    ),
                });
                continue;
            }
        };
        if !o.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&o.stdout);
        let field = |k: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(k))
                .map(|v| v.trim().to_string())
                .unwrap_or_default()
        };
        let fact = CertFact {
            file: name,
            subject: field("subject="),
            not_after: field("notAfter="),
        };
        let current = openssl(&["x509", "-noout", "-checkend", "0", "-in"], &f)
            .is_ok_and(|o| o.status.success());
        if !current {
            rows.push(Readiness {
                check: "certificates".into(),
                outcome: "FAIL".into(),
                code: None,
                detail: format!(
                    "{} ({}) expired {}; run phase 8 again",
                    fact.file, fact.subject, fact.not_after
                ),
            });
        }
        facts.push(fact);
    }
    (facts, rows)
}

/// What a run of this peering restarts, and when.
fn restarts(us: Option<&str>, agreement: Option<&AgreementFacts>) -> Vec<Restart> {
    let offers = match (us, agreement) {
        (Some(us), Some(a)) => Some(a.flows.iter().any(|f| f.from == us)),
        _ => None,
    };
    let yn = |b: Option<bool>| match b {
        Some(true) => "yes".to_string(),
        Some(false) => "no".to_string(),
        None => "depends on the agreement".to_string(),
    };
    vec![
        Restart {
            what: "core router".into(),
            phase: 10,
            required: "yes".into(),
            why: "it admits the edge only after its access control and certificate change; the slot list and the live ports are rechecked immediately before".into(),
        },
        Restart {
            what: "this peering's WireGuard interface".into(),
            phase: 11,
            required: "yes".into(),
            why: "brought up, or reconfigured when the agreed link values changed; no other peering's interface is touched".into(),
        },
        Restart {
            what: "this peering's edge router container".into(),
            phase: 12,
            required: "yes".into(),
            why: "recreated when its rendered configuration or certificates changed".into(),
        },
        Restart {
            what: "this peering's outbound republisher".into(),
            phase: 12,
            required: yn(offers),
            why: "runs only when we offer a release set to the partner".into(),
        },
    ]
}

fn acceptance(agreement: Option<&AgreementFacts>) -> AcceptancePolicy {
    let mut required = Vec::new();
    for f in agreement.map(|a| a.flows.as_slice()).unwrap_or_default() {
        required.push(format!(
            "{} receives {} from {} (release sets: {})",
            f.to,
            f.publish_under,
            f.from,
            if f.release.is_empty() {
                "none".to_string()
            } else {
                f.release.join(", ")
            }
        ));
        required.push(format!(
            "{}'s edge refuses a key {} did not release, beside an allowed control",
            f.to, f.from
        ));
    }
    AcceptancePolicy {
        runner: "ops/federation/standup/acceptance.sh".into(),
        strict: crate::federate::runenv::var("FED_ACCEPTANCE_STRICT").is_ok_and(|v| v == "1"),
        required,
    }
}

/// The environment phase 1 runs with: this process's, plus each derived input
/// the operator has not set. Nothing is exported into this process.
fn phase1_env(
    repo: &Path,
    cfg: &config::Config,
    peering: &config::Peering,
    work: &str,
    edge_group: &str,
) -> Vec<(String, String)> {
    let inputs = preflight::derive_inputs(repo, cfg, peering, work, edge_group, None);
    let mut env: Vec<(String, String)> =
        preflight::unset_derived(&inputs, |k| crate::federate::runenv::var(k).ok())
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
    if crate::federate::runenv::var("GOAT_PROFILE_DIR")
        .map(|v| v.is_empty())
        .unwrap_or(true)
    {
        // As the driver resolves it (federate/mod.rs): a run under sudo reads
        // root's profile, not the invoking user's HOME.
        let home = crate::federate::runenv::var("HOME").unwrap_or_default();
        let dir = if is_root() {
            "/root/.goat".to_string()
        } else {
            format!("{home}/.goat")
        };
        env.push(("GOAT_PROFILE_DIR".into(), dir));
    }
    env
}

fn is_root() -> bool {
    crate::federate::runenv::command("id")
        .arg("-u")
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
}

/// The values phase 1 reads that the operator sets rather than the deployment
/// derives; each decides what a live check looks at.
const LIVE_TARGETS: &[&str] = &[
    "FED_ROUTER_CONTAINER",
    "FED_CORE_CONFIG_PATH",
    "FED_SLOT_DB",
    "FED_SLOT_REGISTRY_STORE",
    "FED_SLOT_PG_SSH",
    "DOCKER_HOST",
];

/// Every input as the run will see it: the environment's value, else the
/// derived one; then the live-check targets that only the environment sets.
fn effective_inputs(
    repo: &Path,
    cfg: &config::Config,
    peering: &config::Peering,
    work: &str,
    edge_group: &str,
) -> Vec<Resolved> {
    effective_inputs_with(repo, cfg, peering, work, edge_group, |k| {
        crate::federate::runenv::var(k).ok()
    })
}

/// [`effective_inputs`] with the environment given, so a test sets nothing in
/// the process.
fn effective_inputs_with(
    repo: &Path,
    cfg: &config::Config,
    peering: &config::Peering,
    work: &str,
    edge_group: &str,
    env: impl Fn(&str) -> Option<String>,
) -> Vec<Resolved> {
    let set = |k: &str| env(k).filter(|v| !v.is_empty());
    let mut out: Vec<Resolved> = preflight::derive_inputs(repo, cfg, peering, work, edge_group, None)
        .into_iter()
        .map(|i| match set(i.name) {
            // A value set to exactly what the deployment derives is the
            // derived value: `federate` exports derived inputs before its
            // phases, and that export must not read as a changed input.
            Some(v) if Some(&v) == i.value.as_ref() => Resolved {
                name: i.name.into(),
                value: i.value,
                source: i.source,
            },
            Some(v) => Resolved {
                name: i.name.into(),
                value: Some(v),
                source: format!("${}", i.name),
            },
            None => Resolved {
                name: i.name.into(),
                value: i.value,
                source: i.source,
            },
        })
        .collect();
    for k in LIVE_TARGETS {
        out.push(Resolved {
            name: k.to_string(),
            value: set(k),
            source: format!("${k}"),
        });
    }
    out
}

/// Run phase 1 (all checks, or `only`) into its own result file, removed
/// first so a stale answer never stands in, and read the outcomes. Refuses
/// when the exit status and the result disagree.
#[allow(clippy::too_many_arguments)]
fn run_phase1(
    repo: &Path,
    cfg: &config::Config,
    peering: &config::Peering,
    work: &str,
    edge_group: &str,
    only: Option<&[&str]>,
    result: &Path,
    live: bool,
) -> Result<Vec<Readiness>> {
    let _ = std::fs::remove_file(result);
    // The probe's answers from an earlier run are not this run's: removed, so
    // a run that does not probe (--no-live) leaves none to be read.
    if let Ok(rd) = std::fs::read_dir(Path::new(work).join("preflight/probe")) {
        for e in rd.filter_map(|e| e.ok()) {
            if e.path().extension().is_some_and(|x| x == "json") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    if let Some(dir) = result.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
    }
    let script = repo.join("ops/federation/standup/preflight.py");
    let mut cmd = crate::federate::runenv::command("python3");
    cmd.arg(&script)
        .args([
            peering.with.as_str(),
            "--work",
            work,
            "--edge-group",
            edge_group,
        ])
        .arg("--result")
        .arg(result)
        .envs(phase1_env(repo, cfg, peering, work, edge_group))
        .current_dir(repo);
    if let Some(ids) = only {
        cmd.args(["--only", &ids.join(",")]);
    }
    if !live {
        cmd.env("FED_PREFLIGHT_NO_LIVE", "1");
    }
    let out = cmd
        .output()
        .with_context(|| format!("python3 {}", script.display()))?;
    let (written, _, _) = read_result(result).with_context(|| {
        format!(
            "phase 1 exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )
    })?;
    anyhow::ensure!(
        out.status.success() == written,
        "phase 1 exited {} but its result says ready={written}; not trusting either",
        out.status
    );
    // Read again for the verdict: a one-sided result is not ready on its own.
    let (_, rows) = read_readiness(result)?;
    anyhow::ensure!(
        !rows.is_empty(),
        "phase 1 wrote no results to {}",
        result.display()
    );
    Ok(rows)
}

fn blocks(outcome: &str) -> bool {
    outcome == "FAIL" || outcome == "BLOCKED"
}

/// Whether a phase-1 result file (`goat-federation-preflight/v1`) says ready,
/// and its outcomes. Phase 1 passes only when both sides' results are ready:
/// a result that carries `side` `edge` or `core` (written by the phase-1
/// sides change, #558) is never ready on its own, and a `sides` row says why. Combining it with the other side's result
/// is the two-driver import, which is not built.
pub fn read_readiness(path: &Path) -> Result<(bool, Vec<Readiness>)> {
    let (ready, side, mut rows) = read_result(path)?;
    if let Some(side) = side.filter(|s| s != "both") {
        rows.push(Readiness {
            check: "sides".into(),
            outcome: "BLOCKED".into(),
            code: None,
            detail: format!("this phase 1 result holds only the {side} side's checks; phase 1 passes only when both sides' results are ready, and a one-sided result is never ready on its own"),
        });
        return Ok((false, rows));
    }
    Ok((ready, rows))
}

/// A phase-1 result file as written: its own ready flag, its side (absent in
/// a result written before sides), and its outcomes.
fn read_result(path: &Path) -> Result<(bool, Option<String>, Vec<Readiness>)> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("no phase 1 result at {}", path.display()))?;
    let doc: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("{} is not JSON", path.display()))?;
    let s = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
    let ready = doc["ready"].as_bool() == Some(true);
    let side = doc["side"].as_str().map(String::from);
    let rows = doc["results"]
        .as_array()
        .map(|rs| {
            rs.iter()
                .map(|r| Readiness {
                    check: s(&r["check"]),
                    outcome: s(&r["outcome"]),
                    code: r["code"]
                        .as_str()
                        .filter(|c| !c.is_empty())
                        .map(String::from),
                    detail: s(&r["detail"]),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((ready, side, rows))
}

/// The declared peering (C1), and every declared peering (for the host check),
/// from the config `--mock` or the checkout names.
fn declared(repo: &Path, o: &PlanOptions) -> Result<(config::Peering, config::Config)> {
    let cfg_path = match o.mock {
        Some(m) => m.join("goatbuilder.yaml"),
        None => repo.join("ops/goatbuilder/goatbuilder.yaml"),
    };
    let cfg = config::load(&cfg_path)?;
    let found = cfg
        .peerings
        .iter()
        .find(|p| p.with == o.with && o.env.map(|e| p.environment == e).unwrap_or(true))
        .cloned()
        .ok_or_else(|| {
            anyhow::Error::from(goat_errors::coded(
                "GB-149",
                format!(
                    "peering '{}' is not declared in {} peerings: — declare it first (C1)",
                    o.with,
                    cfg_path.display()
                ),
            ))
        })?;
    Ok((found, cfg))
}

fn work_of(o: &PlanOptions) -> String {
    o.work
        .map(String::from)
        .or_else(|| {
            crate::federate::runenv::var("FED_WORK")
                .ok()
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_else(|| format!("/var/lib/goat-federation/{}", o.with))
}

/// The values the plays bind, each one a resource of this peering: (kind, on
/// the core host rather than the edge, the variable that overrides it, the role,
/// the role variable). The same list, in the same order of sources, as
/// `PORT_INPUTS` in ops/federation/standup/preflight_live.py.
const BOUND: &[(&str, bool, &str, &str, &str)] = &[
    (
        "wireguard interface",
        false,
        "FED_WG_INTERFACE",
        "goat-peering-wg",
        "peering_wg_interface",
    ),
    (
        "udp port (tunnel)",
        false,
        "FED_WG_LISTEN_PORT",
        "goat-peering-wg",
        "peering_wg_listen_port",
    ),
    (
        "tcp port (edge router)",
        false,
        "FED_EDGE_LISTEN_PORT",
        "goat-zenoh-edge",
        "goat_zenoh_edge_listen_port",
    ),
    (
        "tcp port (republisher metrics)",
        true,
        "FED_REPUBLISHER_METRICS_PORT",
        "goat-fed-republisher",
        "goat_fed_republisher_metrics_port",
    ),
];

/// Everything in the plan but readiness, the verdict and the hash: the part
/// that depends only on inputs, which the recheck rebuilds and compares. With
/// it, the readiness rows for the certificates (expired, or not readable).
fn inputs_part(
    repo: &Path,
    o: &PlanOptions,
    peering: &config::Peering,
    cfg: &config::Config,
) -> Result<(Plan, Vec<Readiness>)> {
    let all = cfg.peerings.as_slice();
    let with = peering.with.as_str();
    let work = work_of(o);
    let edge_group = o
        .edge_group
        .map(String::from)
        .unwrap_or_else(|| format!("{with}_edge"));
    // The group the driver's phase 12 deploys the republisher to.
    let core_group = format!("{with}_core");
    let edge = [edge_group.as_str(), "peering_edges", "all"];
    let core = [core_group.as_str(), "peering_cores", "all"];
    let agreement = agreement_facts(&work)?;
    let fixed = |kind: &str, name: String, on: &str, source: &str| Resource {
        kind: kind.into(),
        name: Some(name),
        on: on.into(),
        source: source.into(),
    };
    let mut resources: Vec<Resource> = BOUND
        .iter()
        .map(|(kind, on_core, env, role, key)| {
            let (groups, on) = if *on_core {
                (&core, &core_group)
            } else {
                (&edge, &edge_group)
            };
            let v = resolve(repo, kind, Some(env), groups, role, key);
            Resource {
                kind: kind.to_string(),
                name: v.value,
                on: on.clone(),
                source: v.source,
            }
        })
        .collect();
    // The containers, named as the roles name them: the group's own
    // `*_stack`, else `<role>-<partner id>` (the roles' templated default).
    let partner_id = crate::federate::runenv::var("FED_COUNTERPARTY_ID")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| with.to_string());
    for (groups, on, role, key) in [
        (
            &edge,
            &edge_group,
            "goat-zenoh-edge",
            "goat_zenoh_edge_stack",
        ),
        (
            &core,
            &core_group,
            "goat-fed-republisher",
            "goat_fed_republisher_stack",
        ),
    ] {
        let v = resolve(repo, "container", None, groups, role, key);
        let templated = v.source.contains("templated");
        resources.push(match v.value {
            Some(name) => Resource {
                kind: "container".into(),
                name: Some(name),
                on: on.clone(),
                source: v.source,
            },
            None if templated => Resource {
                kind: "container".into(),
                name: None,
                on: on.clone(),
                source: v.source,
            },
            None => fixed(
                "container",
                format!("{role}-{partner_id}"),
                on,
                &format!("roles/{role} default {key}"),
            ),
        });
    }
    resources.extend([
        fixed(
            "core access-control fragment",
            format!("{with}-fragment.json5"),
            &core_group,
            "goat federate apply (phase 9), spliced at phase 10",
        ),
        fixed(
            "working directory",
            work.clone(),
            "controller",
            "--work, else $FED_WORK, else /var/lib/goat-federation/<peering>",
        ),
        fixed(
            "certificate directory",
            format!("{work}/pki"),
            "controller",
            "phase 8 writes it; the plays copy its files to the edge and core",
        ),
    ]);
    resources.push(Resource {
        kind: "link subnet".into(),
        name: agreement.as_ref().and_then(|a| a.link_subnet.clone()),
        on: edge_group.clone(),
        source: "the agreement's transport.link_subnet (phase 6)".into(),
    });
    if let Some(us) = o.r#as {
        resources.push(fixed(
            "republisher identity",
            format!("fed.{us}.{with}.republisher.01"),
            &core_group,
            "minted at phase 8 by our CA",
        ));
    }
    let images = vec![
        resolve(
            repo,
            "edge router image",
            None,
            &edge,
            "goat-zenoh-edge",
            "goat_zenoh_edge_image",
        ),
        resolve(
            repo,
            "republisher image",
            None,
            &core,
            "goat-fed-republisher",
            "goat_fed_republisher_image",
        ),
    ];
    // The baseline host firewall is opt-in per host (`goat_host_firewall_baseline`,
    // default false). Off: NOT APPLICABLE, nothing rendered. On: the ruleset and
    // hash from the role's own renderer, or BLOCKED naming why it cannot be shown.
    let our_package = crate::federate::runenv::var("FED_COUNTERPARTY")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(&work).join("theirs.canonical.json"));
    let fw = hostfw::HostContext {
        repo,
        our_edge_group: &edge_group,
        our_work: Path::new(&work),
        our_package,
        // A mock runs no probe, and an earlier run's answers are not this
        // plan's: under --mock there is none to read.
        probe_dir: match o.mock {
            Some(_) => Path::new(&work).join("preflight/probe/(none under --mock)"),
            None => Path::new(&work).join("preflight/probe"),
        },
    };
    let mut host_firewall = Vec::new();
    for (g, profile) in [(&edge_group, "edge"), (&core_group, "core")] {
        for (name, address) in inventory_hosts(repo, g) {
            host_firewall.push(hostfw::entry(&fw, &name, &address, g, profile));
        }
    }
    // A mock runs nothing, so it asks no binary for a version: its plan is the
    // same on every machine.
    let (source, goat) = if o.mock.is_some() {
        (
            "not read under --mock".to_string(),
            "not read under --mock".to_string(),
        )
    } else {
        (source_version(repo), goat_version())
    };
    let (certificates, mut rows) = certificates(&Path::new(&work).join("pki"));
    for h in host_firewall.iter().filter(|h| h.outcome == "BLOCKED") {
        rows.push(Readiness {
            check: "host-firewall".into(),
            outcome: "BLOCKED".into(),
            code: h.code.clone(),
            detail: format!("{} ({}): {}", h.host, h.group, h.detail),
        });
    }
    let cmp = hostalloc::compare(
        repo,
        all,
        peering,
        &edge_group,
        Path::new(&work),
        &resources,
    );
    let conflicts = hostalloc::involving(
        hostalloc::conflicts(&cmp.claims),
        &hostalloc::label(peering),
    );
    rows.push(hostalloc::readiness(&cmp, &conflicts));
    let plan = Plan {
        schema: SCHEMA.into(),
        peering: with.into(),
        environment: peering.environment.clone(),
        site: peering.site.clone(),
        us: o.r#as.map(String::from),
        versions: Versions {
            goatbuilder: env!("CARGO_PKG_VERSION").into(),
            source,
            goat,
            images,
        },
        inputs: effective_inputs(repo, cfg, peering, &work, &edge_group),
        restarts: restarts(o.r#as, agreement.as_ref()),
        acceptance: acceptance(agreement.as_ref()),
        rendered: rendered_hashes(&Path::new(&work).join("applied")),
        // Certificates and other public material only: a private key's
        // fingerprint in a reviewable file buys nothing (a re-minted key comes
        // with a new certificate, which is hashed).
        pki: rendered_hashes(&Path::new(&work).join("pki"))
            .into_iter()
            .filter(|f| !f.path.ends_with("-key.pem") && !f.path.ends_with(".key"))
            .collect(),
        certificates,
        agreement,
        resources,
        host_firewall,
        conflicts,
        edge_group,
        core_group,
        work,
        created_at: String::new(),
        readiness: Vec::new(),
        verdict: String::new(),
        hash: String::new(),
        written_by: format!("goatbuilder {}", env!("CARGO_PKG_VERSION")),
    };
    Ok((plan, rows))
}

/// Build the plan: the inputs, then phase 1 (unless a mock), the verdict and
/// the hash.
pub fn build(repo: &Path, o: &PlanOptions) -> Result<Plan> {
    let (peering, cfg) = declared(repo, o)?;
    // Phase 1 first: its live probe reads what the inputs then use (the
    // peerings the host firewall baseline stored on each edge).
    let work = work_of(o);
    let edge_group = o
        .edge_group
        .map(String::from)
        .unwrap_or_else(|| format!("{}_edge", peering.with));
    let phase1 = if o.mock.is_some() {
        vec![Readiness {
            check: "preflight".into(),
            outcome: "SKIP".into(),
            code: None,
            detail: "a mock runs nothing, so phase 1 did not run".into(),
        }]
    } else {
        let result = Path::new(&work).join("plan/preflight.json");
        run_phase1(repo, &cfg, &peering, &work, &edge_group, None, &result, o.live)?
    };
    let (mut p, input_rows) = inputs_part(repo, o, &peering, &cfg)?;
    p.readiness = phase1;
    p.readiness.extend(input_rows);
    let skipped = p.readiness.iter().filter(|r| r.outcome == "SKIP").count();
    p.verdict = if p.readiness.iter().any(|r| blocks(&r.outcome)) {
        "BLOCKED".into()
    } else if skipped > 0 {
        format!("READY, {skipped} check(s) not verified")
    } else {
        "READY".into()
    };
    p.created_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    p.hash = plan_hash(&p);
    Ok(p)
}

/// Write the plan to `<work>/plan.json`, through a temporary file renamed into
/// place, so a reader never sees half a plan.
pub fn write(p: &Plan) -> Result<PathBuf> {
    let path = Path::new(&p.work).join("plan.json");
    std::fs::create_dir_all(&p.work).with_context(|| format!("mkdir {}", p.work))?;
    let tmp = path.with_extension("json.tmp");
    let mut text = serde_json::to_string_pretty(p)?;
    text.push('\n');
    std::fs::write(&tmp, text).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("rename to {}", path.display()))?;
    Ok(path)
}

/// Read a saved plan, refusing one whose hash does not match its content.
pub fn read(path: &Path) -> Result<Plan> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        goat_errors::coded(
            "GB-201",
            format!(
                "no plan at {} ({e}) (GB-201): run `goatbuilder federate plan <partner>` first",
                path.display()
            ),
        )
    })?;
    let not_a_plan = |e: serde_json::Error| {
        goat_errors::coded(
            "GB-201",
            format!(
                "{} is not a {SCHEMA} plan ({e}) (GB-201): make a new plan",
                path.display()
            ),
        )
    };
    let v: serde_json::Value = serde_json::from_str(&text).map_err(not_a_plan)?;
    let by = v["written_by"]
        .as_str()
        .filter(|w| !w.is_empty())
        .map(|w| format!(", written by {w}"))
        .unwrap_or_default();
    let schema = v["schema"].as_str().unwrap_or_default().to_string();
    if schema != SCHEMA {
        return Err(goat_errors::coded(
            "GB-201",
            format!(
                "{} is a {schema} plan{by}, and this goatbuilder reads {SCHEMA} (GB-201): deploy it with the goatbuilder that made it, or make a new plan with this one",
                path.display()
            ),
        )
        .into());
    }
    let written_hash = v["hash"].as_str().unwrap_or_default().to_string();
    let as_written = hash_of_value(v.clone());
    let p: Plan = serde_json::from_value(v.clone()).map_err(not_a_plan)?;
    if plan_hash(&p) != p.hash && as_written == written_hash {
        // Intact, but it holds what a later goatbuilder wrote and this one
        // does not read: approving it here would approve what is not shown.
        let ours = serde_json::to_value(&p).unwrap_or_default();
        let unread: Vec<String> = v
            .as_object()
            .map(|o| o.keys().filter(|k| ours.get(k.as_str()).is_none()).cloned().collect())
            .unwrap_or_default();
        return Err(goat_errors::coded(
            "GB-201",
            format!(
                "{} was made by a later goatbuilder{by} and holds what this one does not read ({}) (GB-201): make a new plan with this goatbuilder, or deploy it with the one that made it",
                path.display(),
                if unread.is_empty() { "fields inside its sections".to_string() } else { unread.join(", ") }
            ),
        )
        .into());
    }
    if plan_hash(&p) != p.hash {
        return Err(goat_errors::coded(
            "GB-201",
            format!(
                "{} does not match its own hash (GB-201): it was changed after it was written. Make a new plan",
                path.display()
            ),
        )
        .into());
    }
    Ok(p)
}

/// The sections whose content differs between two plans.
pub fn changed_sections(a: &Plan, b: &Plan) -> Vec<String> {
    let (va, vb) = (
        serde_json::to_value(a).expect("plan"),
        serde_json::to_value(b).expect("plan"),
    );
    let mut out: Vec<String> = SECTIONS
        .iter()
        .filter(|s| va[**s] != vb[**s])
        .map(|s| s.to_string())
        .collect();
    for k in [
        "peering",
        "environment",
        "site",
        "us",
        "edge_group",
        "core_group",
        "work",
    ] {
        if va[k] != vb[k] {
            out.push(k.to_string());
        }
    }
    out
}

/// The result of rechecking a saved plan immediately before a change.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Recheck {
    /// The peering the saved plan is for, for the caller to compare.
    pub peering: String,
    pub plan_hash: String,
    /// Why the saved plan cannot be approved as it stands: it was not READY, or
    /// it skipped a check the recheck does not rerun. Empty when it can be.
    pub unready: Vec<String>,
    /// The plan sections whose inputs changed since it was written.
    pub changed: Vec<String>,
    /// The live checks, rerun now, and the rows read from the inputs now (certificates, host allocation).
    pub live: Vec<Readiness>,
    /// The checks the approved plan itself recorded as SKIP. The person
    /// approved the plan with them shown, so the same SKIP now is no change.
    pub skipped_in_plan: Vec<String>,
}

impl Recheck {
    /// A change may go ahead only when the saved plan was ready, no input
    /// changed, every live check answered, and each answer and input row
    /// is PASS or NOT APPLICABLE, or a SKIP the approved plan already
    /// recorded for that check. Any other SKIP is not a pass.
    pub fn allows_change(&self) -> bool {
        self.unready.is_empty()
            && self.changed.is_empty()
            && LIVE_CHECKS
                .iter()
                .all(|c| self.live.iter().any(|r| r.check == *c))
            && self.live.iter().all(|r| self.holds(r))
    }

    /// Whether one rechecked row lets the change go ahead.
    pub fn holds(&self, r: &Readiness) -> bool {
        r.outcome == "PASS"
            || r.outcome == "NOT APPLICABLE"
            || (r.outcome == "SKIP" && self.skipped_in_plan.contains(&r.check))
    }
}

/// The oldest a plan may be at the recheck, in hours: `FED_PLAN_MAX_AGE_HOURS`,
/// else 24. A saved plan is not proof the environment stayed the same, and an
/// old one with unchanged inputs is aged out by this.
fn plan_max_age_hours() -> i64 {
    crate::federate::runenv::var("FED_PLAN_MAX_AGE_HOURS")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|h| *h > 0)
        .unwrap_or(24)
}

/// Why a saved plan cannot be approved as it stands: a FAIL or BLOCKED in its
/// readiness, a SKIP of a check the recheck does not rerun, or its age.
fn unready(saved: &Plan) -> Vec<String> {
    unready_at(saved, chrono::Utc::now(), plan_max_age_hours())
}

fn unready_at(saved: &Plan, now: chrono::DateTime<chrono::Utc>, max_hours: i64) -> Vec<String> {
    let mut out: Vec<String> = saved
        .readiness
        .iter()
        .filter(|r| {
            blocks(&r.outcome) || (r.outcome == "SKIP" && !LIVE_CHECKS.contains(&r.check.as_str()))
        })
        .map(|r| format!("{} was {} when the plan was made", r.check, r.outcome))
        .collect();
    match chrono::DateTime::parse_from_rfc3339(&saved.created_at) {
        Ok(t) => {
            let age = now.signed_duration_since(t.with_timezone(&chrono::Utc));
            if age > chrono::Duration::hours(max_hours) {
                out.push(format!(
                    "the plan was made {} hour(s) ago, more than the {max_hours} allowed (FED_PLAN_MAX_AGE_HOURS); make a new plan",
                    age.num_hours()
                ));
            }
        }
        Err(_) => out.push("the plan has no creation time; make a new plan".into()),
    }
    out
}

/// The saved plan must be the one for this peering, in this working
/// directory: a plan copied from another peering's directory is refused.
fn check_binding(saved: &Plan, plan_path: &Path, with: &str) -> Result<()> {
    let dir = plan_path
        .parent()
        .map(|d| std::fs::canonicalize(d).unwrap_or_else(|_| d.to_path_buf()));
    let work = std::fs::canonicalize(&saved.work).unwrap_or_else(|_| PathBuf::from(&saved.work));
    if saved.peering != with || dir.as_deref() != Some(work.as_path()) {
        return Err(goat_errors::coded(
            "GB-201",
            format!(
                "{} is the plan of peering '{}' in {}, not of '{with}' in {} (GB-201): make a plan for this peering here",
                plan_path.display(),
                saved.peering,
                saved.work,
                dir.map(|d| d.display().to_string()).unwrap_or_default()
            ),
        )
        .into());
    }
    Ok(())
}

fn saved_options<'a>(saved: &'a Plan, mock: Option<&'a Path>) -> PlanOptions<'a> {
    PlanOptions {
        with: &saved.peering,
        env: Some(&saved.environment),
        r#as: saved.us.as_deref(),
        edge_group: Some(&saved.edge_group),
        work: Some(&saved.work),
        mock,
        live: false,
    }
}

/// The inputs half of the recheck: the saved plan (its hash checked, and
/// bound to `with` and the directory it is in), the sections a plan built now
/// would change, and the rows read from the inputs now (certificates, host allocation).
pub fn recheck_inputs(
    repo: &Path,
    plan_path: &Path,
    with: &str,
    mock: Option<&Path>,
) -> Result<(Plan, Vec<String>, Vec<Readiness>)> {
    let (saved, now, input_rows) = inputs_now(repo, plan_path, with, mock)?;
    let changed = changed_sections(&saved, &now);
    Ok((saved, changed, input_rows))
}

/// The saved plan (its hash checked, bound to `with` and its directory), the
/// plan's sections as they read now, and the rows read from the inputs now.
/// A deploy keeps the sections it read after each of its own phases, so the
/// next phase is compared with what deploy itself left, not with the plan
/// written before deploy changed anything.
pub fn inputs_now(
    repo: &Path,
    plan_path: &Path,
    with: &str,
    mock: Option<&Path>,
) -> Result<(Plan, Plan, Vec<Readiness>)> {
    let saved = read(plan_path)?;
    check_binding(&saved, plan_path, with)?;
    let o = saved_options(&saved, mock);
    let (peering, cfg) = declared(repo, &o)?;
    let (now, input_rows) = inputs_part(repo, &o, &peering, &cfg)?;
    Ok((saved, now, input_rows))
}

/// The live prerequisite recheck, for the deploy step to call immediately
/// before it changes anything, with the peering it is about to change:
/// refuses a plan file that was altered or belongs to another peering or
/// directory (GB-201), reports a plan that was not ready, the sections whose
/// inputs changed (a new plan is needed), and reruns the live phase-1 checks
/// into `<work>/plan/recheck.json`. Go ahead only when
/// [`Recheck::allows_change`] is true.
pub fn recheck_live_prerequisites(repo: &Path, plan_path: &Path, with: &str) -> Result<Recheck> {
    // The plan is checked first, so no host is probed for a plan that is not
    // this peering's; the live checks run before the inputs are rebuilt,
    // because the inputs read what the probe answers.
    let saved = read(plan_path)?;
    check_binding(&saved, plan_path, with)?;
    let (peering, cfg) = declared(repo, &saved_options(&saved, None))?;
    let result = Path::new(&saved.work).join("plan/recheck.json");
    let mut live = run_phase1(
        repo,
        &cfg,
        &peering,
        &saved.work,
        &saved.edge_group,
        Some(LIVE_CHECKS),
        &result,
        true,
    )?;
    let (_, changed, input_rows) = recheck_inputs(repo, plan_path, with, None)?;
    live.extend(input_rows);
    let skipped_in_plan = saved
        .readiness
        .iter()
        .filter(|r| r.outcome == "SKIP")
        .map(|r| r.check.clone())
        .collect();
    Ok(Recheck {
        peering: saved.peering.clone(),
        unready: unready(&saved),
        plan_hash: saved.hash,
        changed,
        live,
        skipped_in_plan,
    })
}

fn row(name: &str, ok: bool, detail: String, fix: Option<String>) -> Check {
    Check {
        name: name.into(),
        ok,
        detail,
        fix,
    }
}

fn rows_of(p: &Plan) -> Vec<Check> {
    let mut c = vec![row(
        "plan",
        true,
        format!(
            "peering {} on {} (site {}), work {}",
            p.peering, p.environment, p.site, p.work
        ),
        None,
    )];
    c.push(row(
        "version",
        true,
        format!("goatbuilder {}", p.versions.goatbuilder),
        None,
    ));
    c.push(row(
        "version",
        true,
        format!("source {}", p.versions.source),
        None,
    ));
    c.push(row(
        "version",
        true,
        format!("goat {}", p.versions.goat),
        None,
    ));
    for i in &p.versions.images {
        let v = i.value.clone().unwrap_or_else(|| "not declared".into());
        c.push(row(
            "version",
            true,
            format!("{} {v} ({})", i.name, i.source),
            None,
        ));
    }
    match &p.agreement {
        Some(a) => c.push(row(
            "agreement",
            true,
            format!(
                "{} digest {}, transport {}, {} flow(s)",
                a.path,
                a.digest,
                a.transport,
                a.flows.len()
            ),
            None,
        )),
        None => c.push(row(
            "agreement",
            true,
            "not composed yet (phase 6)".into(),
            None,
        )),
    }
    c.push(row(
        "rendered",
        true,
        if p.rendered.is_empty() {
            "nothing rendered yet (phase 9)".into()
        } else {
            format!(
                "{} file(s) under {}/applied, each hashed",
                p.rendered.len(),
                p.work
            )
        },
        None,
    ));
    for x in &p.certificates {
        c.push(row(
            "certificate",
            true,
            format!("{} {} until {}", x.file, x.subject, x.not_after),
            None,
        ));
    }
    for r in &p.resources {
        let name = r.name.clone().unwrap_or_else(|| "not declared".into());
        c.push(row(
            "resource",
            true,
            format!("{} {name} on {} ({})", r.kind, r.on, r.source),
            None,
        ));
    }
    for r in &p.restarts {
        c.push(row(
            "restart",
            true,
            format!("phase {}: {}: {}; {}", r.phase, r.what, r.required, r.why),
            None,
        ));
    }
    c.push(row(
        "acceptance",
        true,
        format!(
            "{}{}; a skipped check is never a pass",
            p.acceptance.runner,
            if p.acceptance.strict {
                " with FED_ACCEPTANCE_STRICT=1"
            } else {
                ""
            }
        ),
        None,
    ));
    for r in &p.acceptance.required {
        c.push(row("acceptance", true, r.clone(), None));
    }
    for r in &p.readiness {
        c.push(row(
            "readiness",
            !blocks(&r.outcome),
            format!("{} {}: {}", r.check, r.outcome, r.detail),
            r.code.clone().filter(|_| blocks(&r.outcome)),
        ));
    }
    for h in &p.host_firewall {
        c.push(row(
            "host-firewall",
            true,
            format!("{} ({}) {}: {}", h.host, h.group, h.outcome, h.detail),
            None,
        ));
    }
    c.push(row(
        "verdict",
        p.verdict != "BLOCKED",
        p.verdict.clone(),
        None,
    ));
    c.push(row("hash", true, p.hash.clone(), None));
    c
}

/// `goatbuilder federate plan <partner>`: build the plan, write it (not under
/// `--mock`), and report it. Exit 0 when READY, refused when BLOCKED.
pub fn plan(repo: &Path, o: &PlanOptions) -> Result<Report> {
    let p = build(repo, o)?;
    let mut checks = rows_of(&p);
    if o.mock.is_none() {
        let path = write(&p)?;
        checks.push(row("wrote", true, path.display().to_string(), None));
    }
    Ok(Report::new("federate plan", checks, EXIT_REFUSED))
}

/// `goatbuilder federate plan <partner> --recheck`: the recheck the deploy step
/// runs, on demand. Exit 0 only when a change may go ahead.
pub fn recheck(repo: &Path, o: &PlanOptions) -> Result<Report> {
    if o.mock.is_some() {
        return Err(anyhow!(
            "--recheck reads the hosts, and --mock reads none: recheck without --mock"
        ));
    }
    if !o.live {
        return Err(anyhow!(
            "--recheck reruns the live checks; --no-live would skip exactly those"
        ));
    }
    let path = Path::new(&work_of(o)).join("plan.json");
    let saved = read(&path)?;
    let differs = |given: Option<&str>, planned: Option<&str>| given.is_some() && given != planned;
    if differs(o.edge_group, Some(&saved.edge_group)) || differs(o.r#as, saved.us.as_deref()) {
        return Err(anyhow!(
            "--edge-group or --as differs from the saved plan's ({}, {}): make a new plan with them",
            saved.edge_group,
            saved.us.as_deref().unwrap_or("not given")
        ));
    }
    let r = recheck_live_prerequisites(repo, &path, o.with)?;
    let mut checks = vec![row(
        "plan",
        true,
        format!("{} {}", path.display(), r.plan_hash),
        None,
    )];
    for u in &r.unready {
        checks.push(row(
            "unready",
            false,
            u.clone(),
            Some("fix it and make a new plan: a plan that was not ready cannot be approved".into()),
        ));
    }
    for s in &r.changed {
        checks.push(row(
            "changed",
            false,
            format!("{s} changed since the plan was written"),
            Some(
                "GB-200: make a new plan (goatbuilder federate plan <partner>) and approve it"
                    .into(),
            ),
        ));
    }
    for l in &r.live {
        let ok = l.outcome == "PASS" || l.outcome == "NOT APPLICABLE";
        checks.push(row(
            "live",
            ok,
            format!("{} {}: {}", l.check, l.outcome, l.detail),
            (!ok).then(|| {
                format!(
                    "GB-202{}",
                    l.code
                        .as_ref()
                        .map(|c| format!(" ({c})"))
                        .unwrap_or_default()
                )
            }),
        ));
    }
    let go = r.allows_change();
    checks.push(row(
        "verdict",
        go,
        if go {
            "the plan still holds; a change may go ahead".into()
        } else {
            "the plan no longer holds; nothing may change".into()
        },
        None,
    ));
    Ok(Report::new("federate plan", checks, EXIT_REFUSED))
}

/// What `goatbuilder federate <word>` runs instead of the whole peering: one
/// of the six federation verbs (federate/verbs.rs), a stage of the one
/// command, or the back-out of one peering (federate/backout.rs).
#[derive(clap::Subcommand, Debug)]
pub enum Stage {
    #[command(flatten)]
    Verb(goat_federation::verbs::FederateCmd),
    /// The bounded plan of one peering before anything changes: versions,
    /// agreement digest, rendered-configuration hashes, owned resources,
    /// restarts, acceptance policy and phase 1's readiness, written to
    /// <work>/plan.json with a stable hash. Reads only; changes no host.
    Plan {
        /// The peering name (the counterparty, as declared).
        with: String,
        /// The environment the peering is declared on.
        #[arg(long)]
        env: Option<String>,
        /// Which party we are.
        #[arg(long = "as", value_name = "SOVEREIGN")]
        as_party: Option<String>,
        /// The ansible group the edge runs in (default <peering>_edge).
        #[arg(long)]
        edge_group: Option<String>,
        /// The working directory (default $FED_WORK, else /var/lib/goat-federation/<peering>).
        #[arg(long)]
        work: Option<String>,
        /// Skip phase 1's live reads of the hosts; each is reported SKIP.
        #[arg(long)]
        no_live: bool,
        /// Recheck the saved plan instead of writing one: refused when the
        /// file was altered, an input changed, or a live prerequisite fails.
        #[arg(long)]
        recheck: bool,
    },
    /// Back out ONE peering: survey its hosts, confirm, remove its tunnel,
    /// SPIRE federation, edge router, republisher, forwarder mapping and core
    /// admission, and leave every other peering and the core as they were.
    #[command(name = super::backout::VERB)]
    Backout(super::backout::BackoutArgs),
    /// A two-driver peering's import of an edge result, as a hand-off object
    /// names it: checked against the edge driver's confirm code, placed under
    /// the working directory and journaled. Built for phase 1; a later phase's
    /// import refuses with GB-267 (docs/design/federation-two-drivers.md).
    Import(super::handoff::ImportArgs),
}

/// Run one stage.
pub fn run(repo: &Path, stage: &Stage, mock: Option<&Path>) -> Result<Report> {
    match stage {
        // main.rs runs the verbs before it reads a repository.
        Stage::Verb(_) | Stage::Backout(_) => {
            Err(anyhow!("the federation verbs and backout are run before this point"))
        }
        Stage::Import(a) => {
            let work = a.work.clone().unwrap_or_else(|| super::env_or("FED_WORK", format!("/var/lib/goat-federation/{}", a.with)));
            super::handoff::import(a, Path::new(&work))
        }
        Stage::Plan {
            with,
            env,
            as_party,
            edge_group,
            work,
            no_live,
            recheck: again,
        } => {
            let o = PlanOptions {
                with,
                env: env.as_deref(),
                r#as: as_party.as_deref(),
                edge_group: edge_group.as_deref(),
                work: work.as_deref(),
                mock,
                live: !no_live,
            };
            if *again {
                recheck(repo, &o)
            } else {
                plan(repo, &o)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn fixture() -> PathBuf {
        repo().join("ops/goatbuilder/fixtures/goatnet-from-scratch/03-boundary-node")
    }

    fn vector(name: &str) -> PathBuf {
        repo()
            .join("ops/federation/vectors/compose-crossed-extremes")
            .join(name)
    }

    /// From its declared fields, as the config loader reads it.
    /// A config declaring only `p` (and its environment), for the paths that
    /// take the config.
    fn cfg_of(p: config::Peering) -> config::Config {
        let mut c: config::Config = serde_json::from_value(serde_json::json!({
            "schema": "goatbuilder/v1",
            "environments": [{"name": p.environment.clone(), "converger": "push-ansible"}],
            "promotion": {"path": [{"name": p.environment.clone(), "kind": "rung", "environment": p.environment.clone()}]},
        }))
        .expect("a config");
        c.peerings = vec![p];
        c
    }

    fn peering() -> config::Peering {
        serde_json::from_value(serde_json::json!({
            "with": "velmara",
            "environment": "zagros",
            "site": "alpha",
            "edge_class": "edge",
        }))
        .expect("a peering declaration")
    }

    /// Built as a mock: it asks no binary for its version, so a test here
    /// never runs another test's stub `goat` (they share the process
    /// environment), and the versions are the same on every machine.
    fn opts(work: &str) -> PlanOptions<'_> {
        static FIXTURE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        PlanOptions {
            with: "velmara",
            env: None,
            r#as: Some("xaa"),
            edge_group: None,
            work: Some(work),
            mock: Some(FIXTURE.get_or_init(fixture).as_path()),
            live: false,
        }
    }

    /// A work directory with an agreement and two rendered files.
    fn work_with_agreement() -> tempfile::TempDir {
        let w = tempfile::tempdir().unwrap();
        std::fs::copy(
            vector("agreement.canonical.json"),
            w.path().join("agreement.json"),
        )
        .unwrap();
        std::fs::create_dir_all(w.path().join("applied/acl")).unwrap();
        std::fs::write(w.path().join("applied/acl/velmara-fragment.json5"), "{}").unwrap();
        std::fs::write(w.path().join("applied/edge.json5"), "{a: 1}").unwrap();
        w
    }

    fn planned(repo: &Path, work: &str) -> Plan {
        let (mut p, _) = inputs_part(repo, &opts(work), &peering(), &cfg_of(peering())).unwrap();
        p.readiness = vec![Readiness {
            check: "edge-ports".into(),
            outcome: "PASS".into(),
            code: None,
            detail: "pid 41".into(),
        }];
        p.verdict = "READY".into();
        p.hash = plan_hash(&p);
        p
    }

    /// The same inputs give the same hash; a changed rendered file moves the
    /// hash and is named as the changed section; a readiness detail does not
    /// move it, a readiness outcome does.
    #[test]
    fn the_hash_is_stable_and_moves_with_every_input() {
        let r = tempfile::tempdir().unwrap();
        let w = work_with_agreement();
        let ws = w.path().display().to_string();
        let a = planned(r.path(), &ws);
        let b = planned(r.path(), &ws);
        assert_eq!(a.hash, b.hash);
        assert!(
            a.hash.starts_with("sha256:") && a.hash.len() == 71,
            "{}",
            a.hash
        );
        assert_eq!(a.rendered.len(), 2);

        let mut d = a.clone();
        d.readiness[0].detail = "pid 97".into();
        assert_eq!(
            plan_hash(&d),
            a.hash,
            "a detail is not what an approval reads"
        );
        d.readiness[0].outcome = "FAIL".into();
        assert_ne!(plan_hash(&d), a.hash);

        std::fs::write(w.path().join("applied/edge.json5"), "{a: 2}").unwrap();
        let c = planned(r.path(), &ws);
        assert_ne!(c.hash, a.hash);
        assert_eq!(changed_sections(&a, &c), ["rendered"]);
    }

    /// The digest is the one compose prints: sha256 over the canonical bytes.
    #[test]
    fn the_agreement_digest_is_the_composed_digest() {
        let w = work_with_agreement();
        let a = agreement_facts(&w.path().display().to_string())
            .unwrap()
            .unwrap();
        let want = std::fs::read_to_string(vector("agreement.digest.txt")).unwrap();
        assert_eq!(a.digest, want.trim());
        assert_eq!(a.parties, ["xaa", "xab"]);
        assert_eq!(a.flows.len(), 2);
        assert!(
            agreement_facts(&tempfile::tempdir().unwrap().path().display().to_string())
                .unwrap()
                .is_none()
        );
    }

    /// Restarts and acceptance follow the agreement's flows: the republisher
    /// runs only when we offer, and each flow requires a reception and a refusal.
    #[test]
    fn restarts_and_acceptance_follow_the_flows() {
        let r = tempfile::tempdir().unwrap();
        let w = work_with_agreement();
        let p = planned(r.path(), &w.path().display().to_string());
        let republisher = p
            .restarts
            .iter()
            .find(|x| x.what.contains("republisher"))
            .unwrap();
        assert_eq!(
            republisher.required, "yes",
            "xaa offers a release set in this agreement"
        );
        assert_eq!(
            p.acceptance.required.len(),
            4,
            "{:?}",
            p.acceptance.required
        );
        assert!(p
            .acceptance
            .required
            .iter()
            .any(|l| l.starts_with("xab receives from/xaa/**")));
        let none = restarts(Some("xaa"), None);
        assert_eq!(
            none.iter()
                .find(|x| x.what.contains("republisher"))
                .unwrap()
                .required,
            "depends on the agreement"
        );
    }

    /// A saved plan reads back; a plan edited after it was written is refused.
    #[test]
    fn a_saved_plan_reads_back_and_an_edited_one_is_refused() {
        let r = tempfile::tempdir().unwrap();
        let w = work_with_agreement();
        let p = planned(r.path(), &w.path().display().to_string());
        let path = write(&p).unwrap();
        assert_eq!(read(&path).unwrap(), p);
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\"site\": \"alpha\"", "\"site\": \"beta\"");
        std::fs::write(&path, text).unwrap();
        let e = read(&path).unwrap_err().to_string();
        assert!(e.contains("GB-201"), "{e}");
        let e = read(&w.path().join("nothing.json"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("GB-201"), "{e}");
    }

    /// A plan a later goatbuilder made, intact but holding a field this one
    /// does not read, is refused by name, not as an edited plan; a later
    /// schema names the writer.
    #[test]
    fn a_plan_from_a_later_goatbuilder_is_refused_by_name() {
        let r = tempfile::tempdir().unwrap();
        let w = work_with_agreement();
        let p = planned(r.path(), &w.path().display().to_string());
        let path = write(&p).unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["written_by"] = "goatbuilder 0.9.0".into();
        v["rotation"] = serde_json::json!({"key": "transport"});
        let h = hash_of_value(v.clone());
        v["hash"] = h.into();
        std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        let e = read(&path).unwrap_err().to_string();
        assert!(
            e.contains("later goatbuilder, written by goatbuilder 0.9.0") && e.contains("rotation") && e.contains("GB-201"),
            "{e}"
        );
        assert!(!e.contains("changed after it was written"), "{e}");
        v["schema"] = "goat-federation-plan/v2".into();
        std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        let e = read(&path).unwrap_err().to_string();
        assert!(e.contains("goat-federation-plan/v2") && e.contains("goatbuilder 0.9.0"), "{e}");
    }

    /// The recheck rebuilds the inputs and names what moved since the plan.
    /// A mock plan writes nothing, so the plan here is written by hand.
    #[test]
    fn the_recheck_names_what_changed_since_the_plan() {
        let w = work_with_agreement();
        let ws = w.path().display().to_string();
        let fx = fixture();
        let o = PlanOptions {
            with: "hello-ally",
            env: None,
            r#as: Some("xaa"),
            edge_group: None,
            work: Some(&ws),
            mock: Some(&fx),
            live: false,
        };
        let p = build(&repo(), &o).unwrap();
        assert!(
            p.verdict.starts_with("READY, 2 check(s) not verified"),
            "a mock's skipped phase 1, and no inventory host to compare, are not passes: {}",
            p.verdict
        );
        let path = write(&p).unwrap();
        let (_, changed, _) = recheck_inputs(&repo(), &path, "hello-ally", Some(&fx)).unwrap();
        assert!(changed.is_empty(), "{changed:?}");
        std::fs::remove_file(w.path().join("agreement.json")).unwrap();
        let (_, changed, _) = recheck_inputs(&repo(), &path, "hello-ally", Some(&fx)).unwrap();
        assert!(
            changed.contains(&"agreement".to_string())
                && changed.contains(&"acceptance".to_string()),
            "{changed:?}"
        );
    }

    /// A change may go ahead only with nothing changed and every live check
    /// PASS or NOT APPLICABLE; a SKIP, a FAIL or no answer stops it.
    #[test]
    fn only_a_held_plan_allows_a_change() {
        let l = |o: &str| Readiness {
            check: "edge-ports".into(),
            outcome: o.into(),
            code: None,
            detail: String::new(),
        };
        let all = |o: &str| {
            LIVE_CHECKS
                .iter()
                .map(|c| Readiness {
                    check: c.to_string(),
                    ..l(o)
                })
                .collect::<Vec<_>>()
        };
        let r = |unready: Vec<String>, changed: Vec<String>, live: Vec<Readiness>| Recheck {
            peering: "velmara".into(),
            plan_hash: "h".into(),
            unready,
            changed,
            live,
            skipped_in_plan: Vec::new(),
        };
        assert!(r(vec![], vec![], all("PASS")).allows_change());
        assert!(r(vec![], vec![], all("NOT APPLICABLE")).allows_change());
        let mut skip = all("PASS");
        skip[2].outcome = "SKIP".into();
        assert!(!r(vec![], vec![], skip.clone()).allows_change());
        // A SKIP the approved plan itself recorded for that check is no
        // change; a SKIP of another check still refuses.
        let recorded = |checks: &[&str], live: Vec<Readiness>| Recheck {
            skipped_in_plan: checks.iter().map(|c| c.to_string()).collect(),
            ..r(vec![], vec![], live)
        };
        assert!(recorded(&[LIVE_CHECKS[2]], skip.clone()).allows_change());
        assert!(!recorded(&[LIVE_CHECKS[0]], skip).allows_change());
        assert!(!r(vec![], vec![], all("BLOCKED")).allows_change());
        assert!(!r(vec![], vec![], vec![]).allows_change());
        assert!(
            !r(vec![], vec![], vec![l("PASS")]).allows_change(),
            "every live check must answer"
        );
        assert!(!r(vec![], vec!["rendered".into()], all("PASS")).allows_change());
        assert!(!r(
            vec!["inputs was FAIL when the plan was made".into()],
            vec![],
            all("PASS")
        )
        .allows_change());
        let mut certs = all("PASS");
        certs.push(Readiness {
            check: "certificates".into(),
            ..l("FAIL")
        });
        assert!(
            !r(vec![], vec![], certs).allows_change(),
            "a certificate that expired since blocks"
        );
    }

    /// The values the plays bind resolve as phase 1 resolves them: the group's
    /// own vars over the role's default; a templated one is not guessed.
    #[test]
    fn bound_values_resolve_like_phase_1() {
        let r = tempfile::tempdir().unwrap();
        let gv = r.path().join("ops/ansible/group_vars/velmara_edge");
        std::fs::create_dir_all(&gv).unwrap();
        std::fs::create_dir_all(r.path().join("ops/ansible/roles/goat-peering-wg/defaults"))
            .unwrap();
        std::fs::write(
            r.path()
                .join("ops/ansible/roles/goat-peering-wg/defaults/main.yml"),
            "peering_wg_interface: wg-fed0\npeering_wg_listen_port: 51820\n",
        )
        .unwrap();
        std::fs::write(
            gv.join("peering.yml"),
            "peering_wg_listen_port: 51825\npeering_wg_interface: \"{{ x }}\"\n",
        )
        .unwrap();
        let groups = ["velmara_edge", "peering_edges", "all"];
        let port = resolve(
            r.path(),
            "p",
            None,
            &groups,
            "goat-peering-wg",
            "peering_wg_listen_port",
        );
        assert_eq!(port.value.as_deref(), Some("51825"));
        assert_eq!(
            port.source,
            "group_vars/velmara_edge: peering_wg_listen_port"
        );
        let iface = resolve(
            r.path(),
            "i",
            None,
            &groups,
            "goat-peering-wg",
            "peering_wg_interface",
        );
        assert_eq!(iface.value, None);
        assert!(iface.source.contains("templated"), "{}", iface.source);
        std::fs::write(gv.join("peering.yml"), "{}\n").unwrap();
        let iface = resolve(
            r.path(),
            "i",
            None,
            &groups,
            "goat-peering-wg",
            "peering_wg_interface",
        );
        assert_eq!(iface.value.as_deref(), Some("wg-fed0"));
    }

    /// Each host of the edge and core groups has a host firewall entry, NOT
    /// APPLICABLE while no baseline is declared.
    #[test]
    fn every_host_has_a_host_firewall_entry() {
        let r = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(r.path().join("ops/ansible/inventory")).unwrap();
        std::fs::write(
            r.path().join("ops/ansible/inventory/peering-edges.ini"),
            "[velmara_edge]\nedge-a ansible_host=192.0.2.10\n\n[velmara_core]\ncore-a ansible_host=192.0.2.20 ansible_user=ops\n",
        )
        .unwrap();
        assert_eq!(
            inventory_hosts(r.path(), "velmara_core"),
            [("core-a".to_string(), "192.0.2.20".to_string())]
        );
        let w = tempfile::tempdir().unwrap();
        let p = planned(r.path(), &w.path().display().to_string());
        let hosts: Vec<(&str, &str, &str)> = p
            .host_firewall
            .iter()
            .map(|h| (h.host.as_str(), h.address.as_str(), h.outcome.as_str()))
            .collect();
        assert_eq!(
            hosts,
            [
                ("edge-a", "192.0.2.10", "NOT APPLICABLE"),
                ("core-a", "192.0.2.20", "NOT APPLICABLE")
            ]
        );
        // A host that moves to another address is a changed input.
        std::fs::write(
            r.path().join("ops/ansible/inventory/peering-edges.ini"),
            "[velmara_edge]\nedge-a ansible_host=192.0.2.99\n\n[velmara_core]\ncore-a ansible_host=192.0.2.20 ansible_user=ops\n",
        )
        .unwrap();
        let moved = planned(r.path(), &w.path().display().to_string());
        let changed = changed_sections(&p, &moved);
        assert!(
            changed.contains(&"host_firewall".to_string()),
            "{changed:?}"
        );
    }

    /// Phase 1's result file: its ready flag and its outcomes.
    #[test]
    fn a_phase_1_result_reads_with_its_ready_flag() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("r.json");
        std::fs::write(&f, r#"{"schema":"goat-federation-preflight/v1","ready":false,"results":[{"check":"edge-ports","outcome":"FAIL","code":"GB-131","detail":"held"},{"check":"inputs","outcome":"PASS","code":"","detail":""}]}"#).unwrap();
        let (ready, rows) = read_readiness(&f).unwrap();
        assert!(!ready);
        assert_eq!(rows[0].code.as_deref(), Some("GB-131"));
        assert_eq!(rows[1].code, None);
        assert!(read_readiness(&d.path().join("none.json")).is_err());
    }

    /// Phase 1 passes only when both sides' results are ready: a result that
    /// carries `side` `edge` or `core` (written by the phase-1 sides change,
    /// #558) is never ready on its own, however its own checks went. A result for both, or with no side, reads as before.
    #[test]
    fn a_one_sided_phase_1_result_is_never_ready_on_its_own() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("r.json");
        let doc = |side: &str| format!(r#"{{"schema":"goat-federation-preflight/v1","ready":true,{side}"results":[{{"check":"inputs","outcome":"PASS","code":"","detail":""}}]}}"#);
        for side in ["edge", "core"] {
            std::fs::write(&f, doc(&format!(r#""side":"{side}","#))).unwrap();
            let (ready, rows) = read_readiness(&f).unwrap();
            assert!(!ready, "a {side}-only result read as ready");
            let row = rows.iter().find(|r| r.check == "sides").expect("a row saying why");
            assert_eq!(row.outcome, "BLOCKED");
            assert!(row.detail.contains(side) && row.detail.contains("both sides"), "{}", row.detail);
        }
        for side in [r#""side":"both","#, ""] {
            std::fs::write(&f, doc(side)).unwrap();
            let (ready, rows) = read_readiness(&f).unwrap();
            assert!(ready, "{side}");
            assert!(rows.iter().all(|r| r.check != "sides"));
        }
    }

    /// A plan that was not ready cannot be approved later: a FAIL, a BLOCKED,
    /// or a SKIP of a check the recheck does not rerun. A SKIP of a live check
    /// is fine here, because the recheck reruns it.
    #[test]
    fn a_plan_that_was_not_ready_is_unready() {
        let r = tempfile::tempdir().unwrap();
        let w = tempfile::tempdir().unwrap();
        let mut p = planned(r.path(), &w.path().display().to_string());
        let row = |c: &str, o: &str| Readiness {
            check: c.into(),
            outcome: o.into(),
            code: None,
            detail: String::new(),
        };
        p.readiness = vec![row("edge-ports", "SKIP"), row("inputs", "PASS")];
        p.created_at = chrono::Utc::now().to_rfc3339();
        assert!(unready(&p).is_empty());
        p.readiness.push(row("signing-ca", "BLOCKED"));
        p.readiness.push(row("preflight", "SKIP"));
        assert_eq!(
            unready(&p),
            [
                "signing-ca was BLOCKED when the plan was made",
                "preflight was SKIP when the plan was made"
            ]
        );
    }

    /// The saved plan is bound to its peering and to the directory it is in:
    /// a plan copied into another peering's directory is refused.
    #[test]
    fn a_plan_for_another_peering_or_directory_is_refused() {
        let w = work_with_agreement();
        let ws = w.path().display().to_string();
        let fx = fixture();
        let o = PlanOptions {
            with: "hello-ally",
            env: None,
            r#as: None,
            edge_group: None,
            work: Some(&ws),
            mock: Some(&fx),
            live: false,
        };
        let path = write(&build(&repo(), &o).unwrap()).unwrap();
        let e = recheck_inputs(&repo(), &path, "velmara", Some(&fx))
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("GB-201") && e.contains("not of 'velmara'"),
            "{e}"
        );
        let other = tempfile::tempdir().unwrap();
        let copied = other.path().join("plan.json");
        std::fs::copy(&path, &copied).unwrap();
        let e = recheck_inputs(&repo(), &copied, "hello-ally", Some(&fx))
            .unwrap_err()
            .to_string();
        assert!(e.contains("GB-201"), "{e}");
    }

    /// What the live checks look at is an input: another core router daemon
    /// or container is a changed input, not the same plan.
    #[test]
    fn the_live_check_targets_are_inputs() {
        let r = tempfile::tempdir().unwrap();
        let w = tempfile::tempdir().unwrap();
        let ws = w.path().display().to_string();
        let inputs = effective_inputs(r.path(), &cfg_of(peering()), &peering(), &ws, "velmara_edge");
        for k in [
            "FED_ROUTER_DOCKER_HOST",
            "FED_CA_CERT",
            "FED_SIGNING_KEY",
            "FED_ROUTER_CONTAINER",
            "FED_CORE_CONFIG_PATH",
            "FED_SLOT_DB",
        ] {
            assert!(
                inputs.iter().any(|i| i.name == k),
                "{k} is not a planned input"
            );
        }
    }

    /// A symlinked directory under the rendered tree is not walked, so a loop
    /// cannot hang the plan.
    #[cfg(unix)]
    #[test]
    fn a_symlink_loop_under_rendered_files_is_not_followed() {
        let w = tempfile::tempdir().unwrap();
        let applied = w.path().join("applied");
        std::fs::create_dir_all(&applied).unwrap();
        std::fs::write(applied.join("a"), "x").unwrap();
        std::os::unix::fs::symlink(&applied, applied.join("loop")).unwrap();
        let h = rendered_hashes(&applied);
        assert!(h.iter().any(|f| f.path == "a"));
        assert!(h.len() <= 2, "{h:?}");
    }

    /// The deploy step copies owned resources from the plan: the link subnet
    /// (from the agreement) and the certificate directory are among them.
    #[test]
    fn the_link_subnet_and_certificate_directory_are_resources() {
        let r = tempfile::tempdir().unwrap();
        let w = work_with_agreement();
        let ws = w.path().display().to_string();
        let p = planned(r.path(), &ws);
        let get = |k: &str| {
            p.resources
                .iter()
                .find(|x| x.kind == k)
                .and_then(|x| x.name.clone())
        };
        assert_eq!(get("link subnet").as_deref(), Some("169.254.10.0/31"));
        assert_eq!(get("certificate directory"), Some(format!("{ws}/pki")));
    }

    /// With the baseline host firewall off, each host is NOT APPLICABLE; with
    /// it on and no ruleset to show (here the checkout has no renderer and no
    /// management source), the host and the plan are BLOCKED rather than
    /// silently passing.
    #[test]
    fn a_host_firewall_baseline_that_cannot_render_blocks_the_plan() {
        let r = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(r.path().join("ops/ansible/inventory")).unwrap();
        std::fs::write(
            r.path().join("ops/ansible/inventory/peering-edges.ini"),
            "[peering_edges:children]\nvelmara_edge\n[velmara_edge]\nedge-a ansible_host=192.0.2.10\n",
        )
        .unwrap();
        let w = tempfile::tempdir().unwrap();
        let ws = w.path().display().to_string();
        let (p, rows) = inputs_part(r.path(), &opts(&ws), &peering(), &cfg_of(peering())).unwrap();
        assert_eq!(p.host_firewall[0].outcome, "NOT APPLICABLE");
        assert!(p.host_firewall[0]
            .detail
            .starts_with("host firewall baseline: off"));
        assert!(rows.iter().all(|x| x.check != "host-firewall"));
        let gv = r.path().join("ops/ansible/group_vars/peering_edges");
        std::fs::create_dir_all(&gv).unwrap();
        std::fs::write(gv.join("fw.yml"), "goat_host_firewall_baseline: true\n").unwrap();
        let (p, rows) = inputs_part(r.path(), &opts(&ws), &peering(), &cfg_of(peering())).unwrap();
        assert_eq!(p.host_firewall[0].outcome, "BLOCKED");
        assert!(
            rows.iter()
                .any(|x| x.check == "host-firewall" && x.outcome == "BLOCKED"),
            "{rows:?}"
        );
    }

    /// `federate plan` (and `deploy`'s recheck, through the same function)
    /// sees what goatbuilder.yaml declares for the run exactly as the plain
    /// run exports it: the signing key, the CA and the directory, with
    /// nothing in the environment and nothing in group_vars.
    #[test]
    fn the_plan_sees_the_declared_key_ca_and_directory_as_the_plain_run_does() {
        let r = tempfile::tempdir().unwrap();
        let w = tempfile::tempdir().unwrap();
        let ws = w.path().display().to_string();
        let mut c = cfg_of(peering());
        c.secrets = Some(serde_json::from_value(serde_json::json!({"store": "env", "federation": {
            "signing_key": "/srv/fed/signing.key", "ca_cert": "/srv/fed/ca.pem", "ca_key": "/srv/fed/ca-key.pem"}})).unwrap());
        c.environments[0].peering_directory = Some(serde_json::from_value(serde_json::json!({
            "url": "https://directory.example.org:8443", "ca_cert": "/srv/fed/directory-ca.pem"})).unwrap());
        let plan = effective_inputs_with(r.path(), &c, &peering(), &ws, "velmara_edge", |_: &str| None);
        let plain = preflight::derive_inputs(r.path(), &c, &peering(), &ws, "velmara_edge", None);
        let want = [
            ("FED_SIGNING_KEY", "/srv/fed/signing.key"),
            ("FED_CA_CERT", "/srv/fed/ca.pem"),
            ("FED_CA_KEY", "/srv/fed/ca-key.pem"),
            ("FED_DIRECTORY_URL", "https://directory.example.org:8443"),
            ("SSL_CERT_FILE", "/srv/fed/directory-ca.pem"),
        ];
        for (k, v) in want {
            let p = plan.iter().find(|i| i.name == k).unwrap_or_else(|| panic!("{k} in the plan"));
            let q = plain.iter().find(|i| i.name == k).unwrap_or_else(|| panic!("{k} in the plain run"));
            assert_eq!(p.value.as_deref(), Some(v), "{k}");
            assert_eq!((p.value.as_ref(), &p.source), (q.value.as_ref(), &q.source), "{k}");
        }
        // Without the declarations the plan derives none of them, as before.
        let bare = effective_inputs_with(r.path(), &cfg_of(peering()), &peering(), &ws, "velmara_edge", |_: &str| None);
        for (k, _) in want {
            assert!(bare.iter().find(|i| i.name == k).and_then(|i| i.value.as_ref()).is_none(), "{k}");
        }
    }

    /// `federate` exports each derived input before its phases. A recheck in
    /// the same process must see the same inputs, not a change of source.
    #[test]
    fn an_exported_derived_input_is_not_a_changed_input() {
        let r = tempfile::tempdir().unwrap();
        let w = tempfile::tempdir().unwrap();
        let ws = w.path().display().to_string();
        let none = |_: &str| None;
        let before = effective_inputs_with(r.path(), &cfg_of(peering()), &peering(), &ws, "velmara_edge", none);
        let exported: Vec<(String, String)> = before
            .iter()
            .filter_map(|i| i.value.clone().map(|v| (i.name.clone(), v)))
            .collect();
        let env = |k: &str| {
            exported
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
        };
        let after = effective_inputs_with(r.path(), &cfg_of(peering()), &peering(), &ws, "velmara_edge", env);
        assert_eq!(before, after);
        let other = |k: &str| (k == "FED_WORK").then(|| "/elsewhere".to_string());
        let changed = effective_inputs_with(r.path(), &cfg_of(peering()), &peering(), &ws, "velmara_edge", other);
        let wk = changed.iter().find(|i| i.name == "FED_WORK").unwrap();
        assert_eq!(
            (wk.value.as_deref(), wk.source.as_str()),
            (Some("/elsewhere"), "$FED_WORK")
        );
    }

    /// A plan carries when it was made, outside the hash; the recheck refuses
    /// one older than the limit, or one with no creation time.
    #[test]
    fn an_old_plan_is_aged_out_and_the_time_is_not_hashed() {
        let r = tempfile::tempdir().unwrap();
        let w = tempfile::tempdir().unwrap();
        let mut p = planned(r.path(), &w.path().display().to_string());
        p.readiness.clear();
        let made = chrono::DateTime::parse_from_rfc3339("2026-11-09T08:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        p.created_at = made.to_rfc3339();
        let h = plan_hash(&p);
        p.created_at = (made + chrono::Duration::hours(1)).to_rfc3339();
        assert_eq!(plan_hash(&p), h, "the creation time is not hashed");
        p.created_at = made.to_rfc3339();
        assert!(unready_at(&p, made + chrono::Duration::hours(23), 24).is_empty());
        let old = unready_at(&p, made + chrono::Duration::hours(25), 24);
        assert!(
            old.len() == 1 && old[0].contains("25 hour(s) ago"),
            "{old:?}"
        );
        p.created_at = String::new();
        assert_eq!(
            unready_at(&p, made, 24),
            ["the plan has no creation time; make a new plan"]
        );
    }

    /// Private keys under <work>/pki are not hashed into the plan; the
    /// certificates are.
    #[test]
    fn private_keys_are_not_in_the_plan() {
        let r = tempfile::tempdir().unwrap();
        let w = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(w.path().join("pki")).unwrap();
        std::fs::write(w.path().join("pki/core.pem"), "cert").unwrap();
        std::fs::write(w.path().join("pki/core-key.pem"), "key").unwrap();
        let p = planned(r.path(), &w.path().display().to_string());
        let names: Vec<&str> = p.pki.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, ["core.pem"]);
    }
}
