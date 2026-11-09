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

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    pub expired: bool,
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

/// The host firewall entry for one host. The baseline host firewall is not
/// built, so every entry is NOT APPLICABLE and nothing is rendered or applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostFirewall {
    pub host: String,
    pub group: String,
    pub outcome: String,
    pub detail: String,
}

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
    pub versions: Versions,
    pub agreement: Option<AgreementFacts>,
    /// `<work>/applied`, file by file. Empty before phase 9 renders it.
    pub rendered: Vec<FileHash>,
    pub certificates: Vec<CertFact>,
    pub resources: Vec<Resource>,
    pub restarts: Vec<Restart>,
    pub acceptance: AcceptancePolicy,
    pub readiness: Vec<Readiness>,
    pub host_firewall: Vec<HostFirewall>,
    /// READY, `READY, <n> check(s) not verified` (SKIP is never a pass), or BLOCKED.
    pub verdict: String,
    /// sha256 over the plan without this field and without readiness details.
    pub hash: String,
}

/// The sections an input change can move, by name, for the recheck's report.
const SECTIONS: &[&str] = &[
    "versions",
    "agreement",
    "rendered",
    "certificates",
    "resources",
    "restarts",
    "acceptance",
    "host_firewall",
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
    let mut v = serde_json::to_value(p).expect("a plan serializes");
    if let Some(o) = v.as_object_mut() {
        o.remove("hash");
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
fn group_raw(repo: &Path, group: &str, key: &str) -> Option<serde_yaml::Value> {
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
        if let Some(v) = doc.get(key) {
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
fn resolve(
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
        if let Some(v) = std::env::var(e).ok().filter(|v| !v.is_empty()) {
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

/// The hosts of one inventory group in ops/ansible/inventory/peering-edges.ini,
/// as (inventory name, address): `ansible_host` when given, else the name.
pub fn inventory_hosts(repo: &Path, group: &str) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(repo.join("ops/ansible/inventory/peering-edges.ini"))
        .unwrap_or_default();
    let mut inside = false;
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            inside = line == format!("[{group}]");
            continue;
        }
        if inside {
            let mut words = line.split_whitespace();
            let Some(name) = words.next() else { continue };
            let addr = words
                .find_map(|w| w.strip_prefix("ansible_host="))
                .unwrap_or(name);
            out.push((name.to_string(), addr.to_string()));
        }
    }
    out
}

fn source_version(repo: &Path) -> String {
    let git = |args: &[&str]| {
        Command::new("git")
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
    let bin = std::env::var("GOAT_BIN")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "goat".to_string());
    Command::new(&bin)
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
            if p.is_dir() {
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

/// The certificates phase 8 minted into `<work>/pki`: subject and expiry.
/// A key file, or anything openssl does not read as a certificate, is left out.
pub fn certificates(pki: &Path) -> Vec<CertFact> {
    let Ok(rd) = std::fs::read_dir(pki) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "pem" || x == "crt"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let Ok(o) = Command::new("openssl")
            .args(["x509", "-noout", "-subject", "-enddate", "-in"])
            .arg(&f)
            .output()
        else {
            continue;
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
        let expired = !Command::new("openssl")
            .args(["x509", "-noout", "-checkend", "0", "-in"])
            .arg(&f)
            .output()
            .is_ok_and(|o| o.status.success());
        out.push(CertFact {
            file: f
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            subject: field("subject="),
            not_after: field("notAfter="),
            expired,
        });
    }
    out
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
        strict: std::env::var("FED_ACCEPTANCE_STRICT").is_ok_and(|v| v == "1"),
        required,
    }
}

/// The environment phase 1 runs with: this process's, plus each derived input
/// the operator has not set. Nothing is exported into this process.
fn phase1_env(
    repo: &Path,
    peering: &config::Peering,
    work: &str,
    edge_group: &str,
) -> Vec<(String, String)> {
    let inputs = preflight::derive_inputs(repo, peering, work, edge_group, None);
    let mut env: Vec<(String, String)> =
        preflight::unset_derived(&inputs, |k| std::env::var(k).ok())
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
    if std::env::var("GOAT_PROFILE_DIR")
        .map(|v| v.is_empty())
        .unwrap_or(true)
    {
        let home = std::env::var("HOME").unwrap_or_default();
        env.push(("GOAT_PROFILE_DIR".into(), format!("{home}/.goat")));
    }
    env
}

/// Run phase 1 (all checks, or `only`) into its own result file, removed
/// first so a stale answer never stands in, and read the outcomes. Refuses
/// when the exit status and the result disagree.
fn run_phase1(
    repo: &Path,
    peering: &config::Peering,
    work: &str,
    edge_group: &str,
    only: Option<&[&str]>,
    result: &Path,
    live: bool,
) -> Result<Vec<Readiness>> {
    let _ = std::fs::remove_file(result);
    if let Some(dir) = result.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
    }
    let script = repo.join("ops/federation/standup/preflight.py");
    let mut cmd = Command::new("python3");
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
        .envs(phase1_env(repo, peering, work, edge_group))
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
    let (ready, rows) = read_readiness(result).with_context(|| {
        format!(
            "phase 1 exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )
    })?;
    anyhow::ensure!(
        out.status.success() == ready,
        "phase 1 exited {} but its result says ready={ready}; not trusting either",
        out.status
    );
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
/// and its outcomes.
pub fn read_readiness(path: &Path) -> Result<(bool, Vec<Readiness>)> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("no phase 1 result at {}", path.display()))?;
    let doc: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("{} is not JSON", path.display()))?;
    let s = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
    let ready = doc["ready"].as_bool() == Some(true);
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
    Ok((ready, rows))
}

/// The declared peering (C1), from the config `--mock` or the checkout names.
fn declared(repo: &Path, o: &PlanOptions) -> Result<config::Peering> {
    let cfg_path = match o.mock {
        Some(m) => m.join("goatbuilder.yaml"),
        None => repo.join("ops/goatbuilder/goatbuilder.yaml"),
    };
    let cfg = config::load(&cfg_path)?;
    cfg.peerings
        .iter()
        .find(|p| p.with == o.with && o.env.map(|e| p.environment == e).unwrap_or(true))
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "peering '{}' is not declared in {} peerings: — declare it first (C1)",
                o.with,
                cfg_path.display()
            )
        })
}

fn work_of(o: &PlanOptions) -> String {
    o.work
        .map(String::from)
        .or_else(|| std::env::var("FED_WORK").ok().filter(|v| !v.is_empty()))
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
/// that depends only on inputs, which the recheck rebuilds and compares.
fn inputs_part(repo: &Path, o: &PlanOptions, peering: &config::Peering) -> Result<Plan> {
    let with = peering.with.as_str();
    let work = work_of(o);
    let edge_group = o
        .edge_group
        .map(String::from)
        .unwrap_or_else(|| format!("{with}_edge"));
    let core_group = std::env::var("FED_CORE_GROUP")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{with}_core"));
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
    resources.extend([
        fixed(
            "container",
            format!("goat-zenoh-edge-{with}"),
            &edge_group,
            "roles/goat-zenoh-edge goat_zenoh_edge_stack",
        ),
        fixed(
            "container",
            format!("goat-fed-republisher-{with}"),
            &core_group,
            "roles/goat-fed-republisher goat_fed_republisher_stack",
        ),
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
    ]);
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
    let mut host_firewall = Vec::new();
    for g in [&edge_group, &core_group] {
        for (name, _) in inventory_hosts(repo, g) {
            host_firewall.push(HostFirewall {
                host: name,
                group: g.clone(),
                outcome: "NOT APPLICABLE".into(),
                detail: "no baseline host firewall is declared for this host; the plan renders and applies no rule".into(),
            });
        }
    }
    Ok(Plan {
        schema: SCHEMA.into(),
        peering: with.into(),
        environment: peering.environment.clone(),
        site: peering.site.clone(),
        us: o.r#as.map(String::from),
        versions: Versions {
            goatbuilder: env!("CARGO_PKG_VERSION").into(),
            source: source_version(repo),
            goat: goat_version(),
            images,
        },
        restarts: restarts(o.r#as, agreement.as_ref()),
        acceptance: acceptance(agreement.as_ref()),
        rendered: rendered_hashes(&Path::new(&work).join("applied")),
        certificates: certificates(&Path::new(&work).join("pki")),
        agreement,
        resources,
        host_firewall,
        edge_group,
        core_group,
        work,
        readiness: Vec::new(),
        verdict: String::new(),
        hash: String::new(),
    })
}

/// Build the plan: the inputs, then phase 1 (unless a mock), the verdict and
/// the hash.
pub fn build(repo: &Path, o: &PlanOptions) -> Result<Plan> {
    let peering = declared(repo, o)?;
    let mut p = inputs_part(repo, o, &peering)?;
    p.readiness = if o.mock.is_some() {
        vec![Readiness {
            check: "preflight".into(),
            outcome: "SKIP".into(),
            code: None,
            detail: "a mock runs nothing, so phase 1 did not run".into(),
        }]
    } else {
        let result = Path::new(&p.work).join("plan/preflight.json");
        run_phase1(
            repo,
            &peering,
            &p.work,
            &p.edge_group,
            None,
            &result,
            o.live,
        )?
    };
    for c in p.certificates.iter().filter(|c| c.expired) {
        p.readiness.push(Readiness {
            check: "certificates".into(),
            outcome: "FAIL".into(),
            code: None,
            detail: format!(
                "{} ({}) expired {}; run phase 8 again",
                c.file, c.subject, c.not_after
            ),
        });
    }
    let skipped = p.readiness.iter().filter(|r| r.outcome == "SKIP").count();
    p.verdict = if p.readiness.iter().any(|r| blocks(&r.outcome)) {
        "BLOCKED".into()
    } else if skipped > 0 {
        format!("READY, {skipped} check(s) not verified")
    } else {
        "READY".into()
    };
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
    let p: Plan = serde_json::from_str(&text).map_err(|e| {
        goat_errors::coded(
            "GB-201",
            format!(
                "{} is not a {SCHEMA} plan ({e}) (GB-201): make a new plan",
                path.display()
            ),
        )
    })?;
    if p.schema != SCHEMA || plan_hash(&p) != p.hash {
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
    pub plan_hash: String,
    /// The plan sections whose inputs changed since it was written.
    pub changed: Vec<String>,
    /// The live checks, rerun now.
    pub live: Vec<Readiness>,
}

impl Recheck {
    /// A change may go ahead only when no input changed and every live check
    /// passes or does not apply. SKIP is not a pass.
    pub fn allows_change(&self) -> bool {
        self.changed.is_empty()
            && !self.live.is_empty()
            && self
                .live
                .iter()
                .all(|r| r.outcome == "PASS" || r.outcome == "NOT APPLICABLE")
    }
}

/// The inputs half of the recheck: the saved plan, its hash checked, and the
/// sections a plan built now would change.
pub fn recheck_inputs(
    repo: &Path,
    plan_path: &Path,
    mock: Option<&Path>,
) -> Result<(Plan, Vec<String>)> {
    let saved = read(plan_path)?;
    let o = PlanOptions {
        with: &saved.peering,
        env: Some(&saved.environment),
        r#as: saved.us.as_deref(),
        edge_group: Some(&saved.edge_group),
        work: Some(&saved.work),
        mock,
        live: false,
    };
    let peering = declared(repo, &o)?;
    let now = inputs_part(repo, &o, &peering)?;
    let changed = changed_sections(&saved, &now);
    Ok((saved, changed))
}

/// The live prerequisite recheck, for the deploy step to call immediately
/// before it changes anything: refuses a plan file that was altered (GB-201),
/// reports the sections whose inputs changed (a new plan is needed), and reruns
/// the live phase-1 checks into `<work>/plan/recheck.json`. Call
/// [`Recheck::allows_change`] on the result.
pub fn recheck_live_prerequisites(repo: &Path, plan_path: &Path) -> Result<Recheck> {
    let (saved, changed) = recheck_inputs(repo, plan_path, None)?;
    let o = PlanOptions {
        with: &saved.peering,
        env: Some(&saved.environment),
        r#as: saved.us.as_deref(),
        edge_group: Some(&saved.edge_group),
        work: Some(&saved.work),
        mock: None,
        live: true,
    };
    let peering = declared(repo, &o)?;
    let result = Path::new(&saved.work).join("plan/recheck.json");
    let live = run_phase1(
        repo,
        &peering,
        &saved.work,
        &saved.edge_group,
        Some(LIVE_CHECKS),
        &result,
        true,
    )?;
    Ok(Recheck {
        plan_hash: saved.hash,
        changed,
        live,
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
            !x.expired,
            format!("{} {} until {}", x.file, x.subject, x.not_after),
            x.expired.then(|| "run phase 8 again".to_string()),
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
    let path = Path::new(&work_of(o)).join("plan.json");
    let r = recheck_live_prerequisites(repo, &path)?;
    let mut checks = vec![row(
        "plan",
        true,
        format!("{} {}", path.display(), r.plan_hash),
        None,
    )];
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
/// of the six federation verbs (federate/verbs.rs), or a stage of the one
/// command.
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
}

/// Run one stage.
pub fn run(repo: &Path, stage: &Stage, mock: Option<&Path>) -> Result<Report> {
    match stage {
        // main.rs runs the verbs before it reads a repository.
        Stage::Verb(_) => Err(anyhow!("the federation verbs are run before this point")),
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

    fn peering() -> config::Peering {
        config::Peering {
            with: "velmara".into(),
            environment: "zagros".into(),
            site: "alpha".into(),
            edge_class: "edge".into(),
        }
    }

    fn opts(work: &str) -> PlanOptions<'_> {
        PlanOptions {
            with: "velmara",
            env: None,
            r#as: Some("xaa"),
            edge_group: None,
            work: Some(work),
            mock: None,
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
        let mut p = inputs_part(repo, &opts(work), &peering()).unwrap();
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
            p.verdict.starts_with("READY, 1 check(s) not verified"),
            "a mock's skipped phase 1 is not a pass: {}",
            p.verdict
        );
        let path = write(&p).unwrap();
        let (_, changed) = recheck_inputs(&repo(), &path, Some(&fx)).unwrap();
        assert!(changed.is_empty(), "{changed:?}");
        std::fs::remove_file(w.path().join("agreement.json")).unwrap();
        let (_, changed) = recheck_inputs(&repo(), &path, Some(&fx)).unwrap();
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
        let r = |changed: Vec<String>, live: Vec<Readiness>| Recheck {
            plan_hash: "h".into(),
            changed,
            live,
        };
        assert!(r(vec![], vec![l("PASS"), l("NOT APPLICABLE")]).allows_change());
        assert!(!r(vec![], vec![l("PASS"), l("SKIP")]).allows_change());
        assert!(!r(vec![], vec![l("BLOCKED")]).allows_change());
        assert!(!r(vec![], vec![]).allows_change());
        assert!(!r(vec!["rendered".into()], vec![l("PASS")]).allows_change());
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
        let hosts: Vec<(&str, &str)> = p
            .host_firewall
            .iter()
            .map(|h| (h.host.as_str(), h.outcome.as_str()))
            .collect();
        assert_eq!(
            hosts,
            [("edge-a", "NOT APPLICABLE"), ("core-a", "NOT APPLICABLE")]
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
}
