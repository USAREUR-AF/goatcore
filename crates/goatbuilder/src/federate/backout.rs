//! `federate backout <partner>`: remove ONE peering, and leave every other
//! peering and the core as they were.
//!
//! The runbook's Step 9 ("Ending it") in order, done by the roles' own
//! `tasks/absent.yml` through `ops/ansible/peering-backout.yml`:
//!
//!   1  survey   read-only, every host of this peering: which of its resources
//!               exist, each found by the peering id and this peering's own
//!               group values; the core's candidate configuration and digest
//!   2  confirm  a person types the partner id (or --yes)
//!   3  remove   tunnel, SPIRE trust domain and forwards, edge router,
//!               republisher, forwarder mapping
//!   4  recheck  the slot list against the registry, immediately before the
//!               core router restarts (the admission path's recheck)
//!   5  core     install the reviewed candidate (the org fragment without this
//!               partner's blocks, assembled by the core's own assembler) and
//!               restart the core router; refused when the digest changed
//!   6  directory  withdraw our package for this counterparty
//!               (`/v1/parties/<us>/packages/<them>`); our single entry and every
//!               other party's entry are never touched
//!   7  record   delete the generated partner record from the checkout, for
//!               review; then survey again and write `<work>/backout/result.json`
//!
//! `--dry-run` runs the survey and prints every resource it would remove, per
//! host, and the core's candidate digest; nothing changes. `--mock` runs nothing.
//! Running it twice is fine: the second survey finds everything absent, the core
//! plan is empty, so nothing restarts.

use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use crate::config;
use crate::{Check, Report, EXIT_NOT_CONVERGED, EXIT_OK, EXIT_REFUSED};

/// The subcommand word. `goatbuilder federate backout <partner>`; a partner can
/// never be named this (config.rs RESERVED_PEERING_NAMES).
pub const VERB: &str = "backout";

/// `goatbuilder federate backout <partner>`.
#[derive(clap::Args, Debug)]
pub struct BackoutArgs {
    /// The partner whose one peering is removed (its sovereign id, as declared).
    pub partner: String,
    /// The environment the peering is declared on (checked against goatbuilder.yaml peerings:).
    #[arg(long)]
    pub env: Option<String>,
    /// Which party we are (default: read from the agreement in the working directory).
    #[arg(long = "as", value_name = "SOVEREIGN")]
    pub as_party: Option<String>,
    /// The ansible group the edge runs in (default <partner>_edge).
    #[arg(long)]
    pub edge_group: Option<String>,
    /// The working directory (default $FED_WORK, else /var/lib/goat-federation/<partner>).
    #[arg(long)]
    pub work: Option<String>,
    /// Survey the peering's hosts read-only and print every resource the back-out
    /// would remove, per host; change nothing.
    #[arg(long)]
    pub dry_run: bool,
}

impl BackoutArgs {
    pub fn options<'a>(&'a self, mock: Option<&'a Path>, yes: bool, table: bool) -> Options<'a> {
        Options {
            partner: &self.partner,
            env: self.env.as_deref(),
            r#as: self.as_party.as_deref(),
            edge_group: self.edge_group.as_deref(),
            work: self.work.as_deref(),
            mock,
            dry_run: self.dry_run,
            yes,
            table,
        }
    }
}

pub struct Options<'a> {
    pub partner: &'a str,
    pub env: Option<&'a str>,
    pub r#as: Option<&'a str>,
    pub edge_group: Option<&'a str>,
    pub work: Option<&'a str>,
    pub mock: Option<&'a Path>,
    pub dry_run: bool,
    /// --yes: no prompt. Without a terminal main.rs refuses unless --yes (C10).
    pub yes: bool,
    pub table: bool,
}

/// One resource on one host, as the play recorded it.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Row {
    pub component: String,
    pub resource: String,
    pub state: String,
    pub action: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HostRecord {
    pub host: String,
    pub peering: String,
    #[serde(default)]
    pub rows: Vec<Row>,
}

/// A sovereign id (federation-package-spec.md §3): 2-16 of a-z0-9-, and the
/// only kind of word that reaches a play, a path or a URL from here.
pub fn is_sovereign_id(s: &str) -> bool {
    (2..=16).contains(&s.len())
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && !s.starts_with('-')
}

/// `sha256:` and 64 lowercase hex digits: the only shape a core digest takes.
pub fn is_digest(s: &str) -> bool {
    s.strip_prefix("sha256:")
        .map(|h| {
            h.len() == 64
                && h.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
        .unwrap_or(false)
}

/// A working directory whose agreement names other parties is another
/// peering's: backing out from it would name the wrong partner's files.
pub fn work_belongs_to(work: &str, partner: &str) -> Result<(), String> {
    let Ok(text) = std::fs::read_to_string(format!("{work}/agreement.json")) else {
        return Ok(());
    };
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("{work}/agreement.json is not JSON: {e}"))?;
    let ids: Vec<&str> = v
        .get("parties")
        .and_then(|p| p.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|p| p.get("sovereign_id").and_then(|s| s.as_str()))
                .collect()
        })
        .unwrap_or_default();
    if ids.contains(&partner) {
        Ok(())
    } else {
        Err(format!("{work}/agreement.json is between {ids:?}, not with {partner}: --work (or FED_WORK) points at another peering"))
    }
}

/// A path or group name that may sit inside single quotes in a `bash -c` line.
fn plain(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/:@+,~=-".contains(c))
}

fn env_or(name: &str, default: String) -> String {
    crate::federate::runenv::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or(default)
}

/// Where everything of this peering is: groups, working directory.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub partner: String,
    pub edge_group: String,
    pub core_group: String,
    pub work: String,
    /// The core's seed, org fragment and live configuration, when the core is
    /// a goat-zenoh role core: the paths admission wrote (admit_role), passed
    /// to the play as peering_backout_core_{seed,org,config}.
    pub core_files: Vec<(&'static str, String)>,
}

pub fn target(o: &Options) -> Result<Target> {
    if !is_sovereign_id(o.partner) {
        return Err(goat_errors::coded("GB-220", format!("'{}' is not a sovereign id (2-16 characters of a-z0-9-): backout takes the partner whose peering it removes", o.partner)).into());
    }
    if config::RESERVED_PEERING_NAMES.contains(&o.partner) {
        return Err(goat_errors::coded(
            "GB-220",
            format!(
                "'{}' is a goatbuilder federate subcommand, never a partner",
                o.partner
            ),
        )
        .into());
    }
    let work = o.work.map(String::from).unwrap_or_else(|| {
        env_or(
            "FED_WORK",
            format!("/var/lib/goat-federation/{}", o.partner),
        )
    });
    let edge_group = o
        .edge_group
        .map(String::from)
        .unwrap_or_else(|| format!("{}_edge", o.partner));
    let core_group = format!("{}_core", o.partner);
    for (what, v) in [("--work", &work), ("--edge-group", &edge_group)] {
        if !plain(v) {
            return Err(goat_errors::coded(
                "GB-220",
                format!("{what} '{v}' holds characters a plan line cannot carry safely"),
            )
            .into());
        }
    }
    Ok(Target {
        partner: o.partner.to_string(),
        edge_group,
        core_group,
        work,
        core_files: Vec::new(),
    })
}

/// The variables that name the deployment's own route to our core. When one
/// is set, admission works on the driver (admit.rs: the org fragment at
/// FED_CORE_ACL_ORG, the seed and assembled output at FED_CORE_ACL_SEED and
/// FED_CORE_ACL_OUT, a dry run, then FED_CORE_RESTART or FED_CORE_PLAY on
/// FED_CORE_GROUP), and the back-out follows the same route there.
const CORE_ROUTE: [&str; 4] = [
    "FED_CORE_PLAY",
    "FED_CORE_RESTART",
    "FED_CORE_GROUP",
    "FED_CORE_ACL_ORG",
];

/// Our core, reached the way the deployment names: what admission placed, on
/// the driver, and how it restarted the core router.
pub struct DriverCore {
    core: super::admit::Core,
    tool: PathBuf,
    /// The peering's working folder: phase 8's leaves and admission's backups
    /// are here, and the play admission ran is passed it as fed_work.
    work: PathBuf,
    leaf: CoreLeaf,
}

/// The deployment's own route to our core, read with `var` the way admission
/// reads it (admit.rs `Core::from_vars`). `None` when none of its variables is
/// set: the core is the play's to survey (a goat-zenoh role core). A route
/// named but incomplete is refused (GB-221), naming what is missing.
pub fn driver_core(
    repo: &Path,
    partner: &str,
    env_name: &str,
    work: &Path,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Option<DriverCore>> {
    let set = |k: &str| var(k).is_some_and(|v| !v.trim().is_empty());
    if !CORE_ROUTE.iter().any(|k| set(k)) {
        return Ok(None);
    }
    // The role core exactly as admission decides it (admit.rs `role_core`).
    let role = super::admit::role_core(repo, partner, &var)
        .map_err(|e| anyhow::Error::from(goat_errors::coded("GB-221", format!("{e} (GB-221)"))))?;
    let core = super::admit::Core::from_vars(repo, partner, env_name, role, &var).map_err(|missing| {
        anyhow::Error::from(goat_errors::coded(
            "GB-221",
            format!(
                "the deployment names its own route to our core ({}), but not all of it: set {} (GB-221)",
                CORE_ROUTE.iter().filter(|k| set(k)).copied().collect::<Vec<_>>().join(", "),
                missing.join("; ")
            ),
        ))
    })?;
    // Without FED_CORE_ACL_ORG, admission placed the org fragment on the role
    // core's host: the play surveys it there, as it does with no route named.
    if core.org.is_none() {
        return Ok(None);
    }
    let leaf = CoreLeaf::new(work, core.cert.clone(), core.key.clone());
    Ok(Some(DriverCore {
        core,
        tool: repo.join("ops/federation/standup/backout_core.py"),
        work: work.to_path_buf(),
        leaf,
    }))
}

/// backout_core.py's plan of the org fragment alone, for a deployment that
/// names no seed or assembled output (the host assembles them): the same
/// `plan()`, without the proof that the output is seed plus org.
const ORG_ONLY_PLAN: &str = r#"import importlib.util, json, sys
spec = importlib.util.spec_from_file_location("backout_core", sys.argv[1])
bc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bc)
org = open(sys.argv[2], encoding="utf-8").read()
try:
    cand, rep = bc.plan(org, sys.argv[3])
    rep["refused"] = None
except ValueError as e:
    cand, rep = None, {"refused": str(e)}
if cand is not None:
    open(sys.argv[4] + "/org.candidate", "w", encoding="utf-8").write(cand)
    rep["org_candidate_sha256"] = bc.sha256(cand)
    rep["candidate_sha256"] = rep["org_candidate_sha256"]
rep["proof"] = "org fragment only: the host assembles it"
print(json.dumps(rep, sort_keys=True))
"#;

/// The candidate the driver's files give without `partner`: backout_core.py
/// plans it, from the seed, org fragment and assembled output when the
/// deployment names all three (proving the output is seed plus org), else
/// from the org fragment alone (the host assembles it). Writes into `dir`;
/// returns the plan.
fn driver_plan(dc: &DriverCore, partner: &str, dir: &Path) -> Result<serde_json::Value> {
    let _ = std::fs::create_dir_all(dir);
    let org = dc.core.org.as_ref().expect("driver_core requires it");
    let out = match (&dc.core.seed, &dc.core.out) {
        (Some(seed), Some(live)) => Command::new("python3")
            .arg(&dc.tool)
            .args(["plan", "--partner", partner, "--seed"])
            .arg(seed)
            .arg("--org")
            .arg(org)
            .arg("--live")
            .arg(live)
            .arg("--out-dir")
            .arg(dir)
            .output(),
        // The org fragment alone: the same plan, without the assembled proof.
        _ => Command::new("python3")
            .args(["-c", ORG_ONLY_PLAN])
            .arg(&dc.tool)
            .arg(org)
            .arg(partner)
            .arg(dir)
            .output(),
    }
    .with_context(|| format!("run {}", dc.tool.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(text.trim()).with_context(|| {
        format!(
            "{} gave no plan: {}",
            dc.tool.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )
    })
}

/// Our core router's leaf as admission placed it (FED_CORE_CERT and
/// FED_CORE_KEY, or where a goat-zenoh role core reads it on this machine:
/// admit_leaf.rs `router_leaf`), each path with phase 8's file it was placed
/// from, in the peering's working folder.
pub struct CoreLeaf {
    work: PathBuf,
    targets: Vec<(PathBuf, PathBuf)>,
    /// The goat-zenoh role core that delivers the leaf to its host, when the
    /// leaf is the one the role reads (`role_leaf`).
    role: Option<super::admit_role::RoleCore>,
}

/// What a leaf path holds, and what the back-out does with it.
enum Leaf {
    /// Absent, or another's bytes: left alone.
    NotOurs,
    /// This peering's, and another peering's working folder holds the same
    /// leaf: kept, because that peering uses it too.
    Shared(String),
    /// This peering's: put back to what was there before its first admission.
    Restore(Vec<u8>),
    /// This peering's, and nothing was there before: removed.
    Remove,
}

impl CoreLeaf {
    fn new(work: &Path, cert: Option<PathBuf>, key: Option<PathBuf>) -> CoreLeaf {
        let pki = work.join("pki");
        CoreLeaf {
            work: work.to_path_buf(),
            targets: [(cert, "core.pem"), (key, "core-key.pem")]
                .into_iter()
                .filter_map(|(t, ours)| t.map(|t| (t, pki.join(ours))))
                .collect(),
            role: None,
        }
    }

    /// How many leaf paths go back to what was there, and how many are
    /// removed.
    fn counts(&self) -> (usize, usize) {
        let plans: Vec<Leaf> = self.targets.iter().map(|(t, o)| self.plan(t, o)).collect();
        (
            plans
                .iter()
                .filter(|l| matches!(l, Leaf::Restore(_)))
                .count(),
            plans.iter().filter(|l| matches!(l, Leaf::Remove)).count(),
        )
    }

    /// The earliest record of `target` in this peering's admission runs, in
    /// order (admit.rs `publish`): `Some(Some(backup))` when something was
    /// there, `Some(None)` when nothing was, `None` with no record. A backup
    /// with no record beside it (an older admission) is taken as of a file of
    /// its name.
    fn first_record(&self, target: &Path) -> Option<Option<PathBuf>> {
        let base = target.file_name()?.to_string_lossy().to_string();
        let side = format!("{base}.target");
        let first_line = |p: &Path| -> Option<(PathBuf, bool)> {
            let t = std::fs::read_to_string(p).ok()?;
            let mut lines = t.lines().map(str::trim);
            let path = PathBuf::from(lines.next()?);
            Some((path, lines.next() == Some(super::admit::ABSENT)))
        };
        let mut runs: Vec<PathBuf> = std::fs::read_dir(self.work.join("admission"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path().join("backup"))
            .filter(|p| p.is_dir())
            .collect();
        runs.sort();
        for backup in runs {
            let mut found: Vec<(u64, Option<PathBuf>)> = Vec::new();
            for p in std::fs::read_dir(&backup).into_iter().flatten().flatten() {
                let p = p.path();
                let Some((n, rest)) = p
                    .file_name()
                    .and_then(|f| f.to_str())
                    .and_then(|f| f.split_once('-'))
                else {
                    continue;
                };
                if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                let n: u64 = n.parse().unwrap_or(u64::MAX);
                if rest == base {
                    let rec = p.with_file_name(format!("{n}-{side}"));
                    match first_line(&rec) {
                        Some((of, _)) if of != target => continue,
                        _ => found.push((n, Some(p))),
                    }
                } else if rest == side {
                    if let Some((of, true)) = first_line(&p) {
                        if of == target {
                            found.push((n, None));
                        }
                    }
                }
            }
            found.sort();
            if let Some((_, b)) = found.into_iter().next() {
                return Some(b);
            }
        }
        None
    }

    fn plan(&self, target: &Path, ours: &Path) -> Leaf {
        let (Ok(now), Ok(mine)) = (std::fs::read(target), std::fs::read(ours)) else {
            return Leaf::NotOurs;
        };
        if now != mine {
            return Leaf::NotOurs;
        }
        let name = ours.file_name().unwrap_or_default();
        if let Some(parent) = self.work.parent() {
            let shared = std::fs::read_dir(parent)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| *p != self.work)
                .find(|p| std::fs::read(p.join("pki").join(name)).is_ok_and(|b| b == mine));
            if let Some(p) = shared {
                return Leaf::Shared(p.display().to_string());
            }
        }
        // What was there before the peering: what the earliest admission that
        // touched this path found there. A later run's backup is this
        // peering's own earlier leaf (phase 8 renews it), never what was there.
        match self.first_record(target) {
            Some(Some(b)) => match std::fs::read(&b) {
                Ok(bytes) => Leaf::Restore(bytes),
                Err(_) => Leaf::NotOurs,
            },
            Some(None) | None => Leaf::Remove,
        }
    }

    /// One survey row per leaf path.
    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (target, ours) in &self.targets {
            let t = target.display().to_string();
            let row = |state: &str, action: &str, note: String| Row {
                component: "core leaf".into(),
                resource: t.clone(),
                state: state.into(),
                action: action.into(),
                note,
            };
            rows.push(match self.plan(target, ours) {
                Leaf::NotOurs => row(
                    "absent",
                    "none",
                    format!("does not hold {}", ours.display()),
                ),
                Leaf::Shared(p) => row(
                    "present",
                    "kept",
                    format!("the same leaf is in {p}, another peering's"),
                ),
                Leaf::Restore(_) => row(
                    "present",
                    "remove",
                    "put back to what was there before the peering".into(),
                ),
                Leaf::Remove => row(
                    "present",
                    "remove",
                    "nothing was there before the peering".into(),
                ),
            });
        }
        rows
    }

    /// Put each leaf path back, or remove it, keeping what was there in
    /// `backups` (in `bdir`). Run again, it changes nothing: a path put back
    /// no longer holds this peering's leaf.
    fn take_back(
        &self,
        backups: &mut Vec<(PathBuf, Option<PathBuf>)>,
        bdir: &Path,
    ) -> Result<(), String> {
        for (target, ours) in &self.targets {
            let r = match self.plan(target, ours) {
                Leaf::Restore(bytes) => super::admit::publish(&bytes, target, backups, bdir),
                Leaf::Remove => super::admit::publish(b"", target, backups, bdir)
                    .and_then(|_| std::fs::remove_file(target)),
                Leaf::NotOurs | Leaf::Shared(_) => Ok(()),
            };
            r.map_err(|e| format!("cannot take back {} ({e})", target.display()))?;
        }
        Ok(())
    }
}

/// Our core router's leaf on a goat-zenoh role core admitted with no route of
/// its own (no FED_CORE_ACL_ORG): admission placed it on this machine where
/// the role reads it (admit_leaf.rs `router_leaf`, FED_CORE_CERT and
/// FED_CORE_KEY overriding), so the back-out takes it back there. `None`
/// when no leaf path resolves.
pub fn role_leaf(
    repo: &Path,
    partner: &str,
    work: &Path,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Option<CoreLeaf>> {
    let env = |k: &str| var(k).filter(|v| !v.trim().is_empty());
    let role = super::admit::role_core(repo, partner, &var)
        .map_err(|e| anyhow::Error::from(goat_errors::coded("GB-221", format!("{e} (GB-221)"))))?;
    let (cert, key) = match &role {
        Some(r) => super::admit_leaf::router_leaf(repo, r),
        None => (None, None),
    };
    let mut leaf = CoreLeaf::new(
        work,
        env("FED_CORE_CERT")
            .map(PathBuf::from)
            .or(cert.map(|c| c.0)),
        env("FED_CORE_KEY").map(PathBuf::from).or(key.map(|k| k.0)),
    );
    leaf.role = role;
    Ok((!leaf.targets.is_empty()).then_some(leaf))
}

/// The driver's part of the survey, as one more host record (`driver`): the
/// partner's subjects, rules and policies in the org fragment, and the restart
/// it needs, with the candidate's digest; or why the fragment cannot be shown
/// to change by this peering alone (refused).
pub fn driver_survey(dc: &DriverCore, partner: &str, dir: &Path) -> Result<HostRecord> {
    let plan = driver_plan(dc, partner, dir)?;
    let leaf_rows = dc.leaf.rows();
    let org = dc.core.org.as_ref().expect("checked").display().to_string();
    let r = |component: &str, resource: String, state: &str, action: &str, note: String| Row {
        component: component.into(),
        resource,
        state: state.into(),
        action: action.into(),
        note,
    };
    let mut rows = Vec::new();
    if let Some(why) = plan["refused"].as_str() {
        rows.push(r(
            "core admission",
            format!("fragment {org}"),
            "refused",
            "kept",
            why.to_string(),
        ));
    } else if plan["absent"].as_bool().unwrap_or(true) {
        rows.push(r(
            "core admission",
            format!("fragment {org}"),
            "absent",
            "none",
            format!("no block of {partner} is spliced; no restart"),
        ));
    } else {
        let list = |k: &str| -> Vec<String> {
            plan["removed"][k]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        for (k, what) in [
            ("subjects", "subject"),
            ("rules", "rule"),
            ("policies", "policy for"),
        ] {
            for id in list(k) {
                rows.push(r(
                    "core admission",
                    format!("{what} {id}"),
                    "present",
                    "remove",
                    String::new(),
                ));
            }
        }
        let digest = plan["candidate_sha256"].as_str().unwrap_or_default();
        rows.push(r(
            "core admission",
            format!("router restart through {}", dc.core.route()),
            "present",
            "restart",
            format!(
                "{} lines leave; candidate {digest}; stays admitted: {}",
                plan["removed_lines"].as_u64().unwrap_or(0),
                plan["others"]
                    .as_array()
                    .map(|a| a
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", "))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "no other peering".into())
            ),
        ));
    }
    rows.extend(leaf_rows);
    Ok(HostRecord {
        host: "driver".into(),
        peering: partner.into(),
        rows,
    })
}

/// Take `partner` out of the driver's files and restart our core the way
/// admission did: plan again and refuse unless the candidate is the reviewed
/// one (`expect`), place the org fragment (and the assembled output) with
/// backups, dry-run, then restart; a failed dry run puts every file back.
/// Commands run through `exec`, as the plays do.
pub fn driver_apply(
    repo: &Path,
    dc: &DriverCore,
    partner: &str,
    dir: &Path,
    expect: &str,
    exec: &mut dyn FnMut(&Path, &str) -> Result<bool>,
) -> Result<()> {
    let plan = driver_plan(dc, partner, &dir.join("plan"))?;
    let got = plan["candidate_sha256"].as_str().unwrap_or_default();
    if got != expect {
        return Err(goat_errors::coded(
            "GB-224",
            format!("our core's files changed since the survey: the candidate is now {got}, and {expect} was reviewed; nothing was placed (GB-224)"),
        )
        .into());
    }
    let mut placed = vec![(
        std::fs::read(dir.join("plan").join("org.candidate"))?,
        dc.core.org.clone().expect("checked"),
    )];
    if let Some(out) = &dc.core.out {
        placed.push((
            std::fs::read(dir.join("plan").join("live.candidate"))?,
            out.clone(),
        ));
    }
    let mut backups = Vec::new();
    let bdir = dir.join("backup");
    for (bytes, to) in &placed {
        if let Err(e) = super::admit::publish(bytes, to, &mut backups, &bdir) {
            let back = super::admit::put_back(&backups);
            return Err(goat_errors::coded(
                "GB-224",
                format!("cannot place {} ({e}); {back} (GB-224)", to.display()),
            )
            .into());
        }
    }
    // The router leaf, under the same backups: put back, or removed.
    if let Err(e) = dc.leaf.take_back(&mut backups, &bdir) {
        let back = super::admit::put_back(&backups);
        return Err(goat_errors::coded("GB-224", format!("{e}; {back} (GB-224)")).into());
    }
    // The play admission ran is passed the peering's working folder, as
    // admission passes it; it converges the core to what the files now hold.
    let work = dc.work.as_path();
    let dry = match (
        dc.core.play_cmd(repo, work, true),
        dc.core.dry_run.as_deref(),
    ) {
        (Some(cmd), _) => Some(cmd),
        (None, Some("skip")) | (None, None) => None,
        (None, Some(cmd)) => Some(cmd.to_string()),
    };
    if let Some(cmd) = dry {
        if !exec(repo, &cmd)? {
            let back = super::admit::put_back(&backups);
            return Err(goat_errors::coded(
                "GB-224",
                format!("the core router's dry run failed; {back} (GB-224)"),
            )
            .into());
        }
    }
    let restart = dc
        .core
        .play_cmd(repo, work, false)
        .or_else(|| dc.core.restart.clone())
        .unwrap_or_default();
    if !exec(repo, &restart)? {
        return Err(goat_errors::coded(
            "GB-224",
            format!("the core restart through {} failed; the candidate is in place, and what was there before is in {} (GB-224)", dc.core.route(), bdir.display()),
        )
        .into());
    }
    Ok(())
}

/// The files of a goat-zenoh role core, resolved by the code admission uses
/// (`admit_role::find` and `role_value`), so the back-out reads and replaces
/// exactly what admission wrote. Nothing is set without a role core: the
/// core's own group vars decide, as before. A role core whose paths do not
/// resolve is refused (GB-221), never surveyed at a guessed path.
pub fn core_files(repo: &Path, t: &mut Target) -> Result<()> {
    let refuse = |e: String| -> anyhow::Error {
        goat_errors::coded(
            "GB-221",
            format!(
                "the back-out cannot find our core's files the way admission did: {e} (GB-221)"
            ),
        )
        .into()
    };
    // The role core exactly as admission decides it (admit.rs `role_core`):
    // FED_CORE_GROUP's group when it is set, else <partner>_core's.
    let Some(core) =
        super::admit::role_core(repo, &t.partner, |k| std::env::var(k).ok()).map_err(refuse)?
    else {
        return Ok(());
    };
    for (var, key) in [
        ("peering_backout_core_seed", "goat_zenoh_acl_seed_path"),
        ("peering_backout_core_org", "goat_zenoh_acl_org_path"),
        ("peering_backout_core_config", "goat_zenoh_config_path"),
    ] {
        let v = core.role_value(repo, key).map_err(refuse)?;
        if !plain(&v) {
            return Err(refuse(format!(
                "{key} '{v}' holds characters a plan line cannot carry safely"
            )));
        }
        t.core_files.push((var, v));
    }
    Ok(())
}

/// The play line, the way the peering plays run (federate/mod.rs `play_on`):
/// this peering's two groups and the forwarders, and nothing of another peering.
pub fn play_cmd(repo: &Path, t: &Target, extra: &[(&str, String)]) -> String {
    let r = repo.display();
    let aws = crate::settings::aws_profile(repo);
    // A value that could not sit inside single quotes falls back to the default.
    let counterparty = Some(env_or("FED_COUNTERPARTY", String::new()))
        .filter(|v| plain(v))
        .unwrap_or_else(|| format!("{}/theirs.canonical.json", t.work));
    let mut e = format!(
        "-e peering_backout_id='{}' -e fed_edge_group='{}' -e fed_work='{}' -e fed_counterparty='{}'",
        t.partner, t.edge_group, t.work, counterparty
    );
    for (k, v) in t
        .core_files
        .iter()
        .map(|(k, v)| (*k, v))
        .chain(extra.iter().map(|(k, v)| (*k, v)))
    {
        e.push_str(&format!(" -e {k}='{v}'"));
    }
    // The inventories the forward path reads (plan::inventory_args): on a site
    // whose inventory allocate renders, the hosts are in peering-<group>.ini.
    let inventories = super::plan::inventory_args(repo);
    format!(
        "cd '{r}/ops/ansible' && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws {aws} -- ../scripts/ansible-preflight.sh {inventories} --limit '{}:{}:peering_forwarders' {e} peering-backout.yml",
        t.edge_group, t.core_group
    )
}

/// Read every host record a stage wrote.
pub fn read_records(dir: &Path) -> Result<Vec<HostRecord>> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Ok(out);
    };
    let mut paths: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    paths.sort();
    for p in paths {
        let text = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
        let rec: HostRecord = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a back-out host record", p.display()))?;
        out.push(rec);
    }
    Ok(out)
}

/// What a survey says, summed up.
#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub to_remove: usize,
    pub absent: usize,
    pub kept: Vec<String>,
    pub refused: Vec<String>,
    /// The core candidate digest when the core will restart.
    pub core_candidate: Option<String>,
    /// The partner is still admitted by the deployment's configuration.
    pub role_declared: bool,
}

pub fn summarize(recs: &[HostRecord], partner: &str) -> Summary {
    let mut s = Summary::default();
    for rec in recs {
        for row in &rec.rows {
            match (row.state.as_str(), row.action.as_str()) {
                ("not-ours" | "refused", _) => s
                    .refused
                    .push(format!("{}: {} ({})", rec.host, row.resource, row.note)),
                // A row a host could not read is not one that is absent: the
                // resource may still be there (the core may still admit the
                // partner), so the back-out stops.
                ("unknown", _) => s
                    .refused
                    .push(format!("{}: {} ({})", rec.host, row.resource, row.note)),
                (_, "remove" | "restart") => s.to_remove += 1,
                ("present", "kept") => s
                    .kept
                    .push(format!("{}: {} ({})", rec.host, row.resource, row.note)),
                _ => s.absent += 1,
            }
            if row.resource.starts_with("router restart") && row.action == "restart" {
                s.core_candidate = row
                    .note
                    .split_whitespace()
                    .find(|w| w.starts_with("sha256:"))
                    .map(|w| w.trim_end_matches(';').to_string());
            }
            if row
                .resource
                .starts_with(&format!("zenoh_sovereign_peerings[sovereign={partner}]"))
            {
                s.role_declared = true;
            }
        }
    }
    s
}

/// The plan as a person reads it: per host, what goes and what stays.
pub fn plan_view(t: &Target, recs: &[HostRecord], extra: &[(String, String)]) -> String {
    let mut s = format!("Back-out plan: peering {}\nNothing has changed. Every resource below was found by this peering's id and its own group values.\n\n", t.partner);
    s.push_str(&format!(
        "  Working folder: {}\n  Groups:         {}, {}, peering_forwarders\n",
        t.work, t.edge_group, t.core_group
    ));
    if recs.is_empty() {
        s.push_str("\nNo host answered for these groups: nothing of this peering is deployed where the inventory points.\n");
    }
    for rec in recs {
        s.push_str(&format!("\n{}\n", rec.host));
        let mut by: BTreeMap<&str, Vec<&Row>> = BTreeMap::new();
        for r in &rec.rows {
            by.entry(r.component.as_str()).or_default().push(r);
        }
        for (comp, rows) in by {
            for r in rows {
                let verb = match (r.state.as_str(), r.action.as_str()) {
                    ("not-ours" | "refused", _) => "REFUSED",
                    (_, "remove") => "remove",
                    (_, "restart") => "restart",
                    (_, "removed") => "removed",
                    (_, "restarted") => "restarted",
                    ("present", "kept") => "keep",
                    _ => "absent",
                };
                let note = if r.note.is_empty() || r.note == "ours" {
                    String::new()
                } else {
                    format!("  ({})", r.note)
                };
                s.push_str(&format!("  {verb:<9} {comp:<15} {}{note}\n", r.resource));
            }
        }
    }
    for (k, v) in extra {
        s.push_str(&format!("\n{k}\n  {v}\n"));
    }
    s
}

/// The generated partner record in the checkout. Deleted only when it is the
/// file `goat federate apply` writes for exactly this partner; a hand-written
/// file or another partner's is never touched.
pub fn partner_record(repo: &Path, partner: &str) -> (PathBuf, Option<bool>) {
    let p = repo.join(format!("ops/federation/partners/{partner}.yaml"));
    match std::fs::read_to_string(&p) {
        Err(_) => (p, None),
        Ok(text) => {
            // The header render writes, in either wording (goat-federation render::GENERATED_MARKERS).
            let first = text.lines().next().unwrap_or_default();
            let generated = goat_federation::render::GENERATED_MARKERS
                .iter()
                .any(|m| first.starts_with(&format!("# {m}")));
            let named = text
                .lines()
                .any(|l| l.trim() == format!("sovereign_id: {partner}"));
            (p, Some(generated && named))
        }
    }
}

/// The slot-list recheck before the core router restarts. The same check the
/// admission path runs (federation-preflight.md "The restart recheck"):
/// `ops/scripts/sync-slot-anchors.py --env <env> --check`. Returns the outcome
/// word and whether the restart may go ahead.
pub fn slot_recheck(repo: &Path, env: Option<&str>, run: bool) -> (String, bool, String) {
    let Some(env) = env.filter(|e| !e.is_empty()) else {
        return ("BLOCKED".into(), false, "no environment to read the slot list of: declare the peering in goatbuilder.yaml peerings:, or set FED_SLOT_ENV".into());
    };
    let file = repo.join(format!(
        "ops/ansible/group_vars/{}_hosts/zenoh-lazy-storage.yml",
        env.replace('-', "_")
    ));
    if !file.is_file() {
        return (
            "NOT APPLICABLE".into(),
            true,
            format!(
                "no {}: this deployment declares no lazy slot storage",
                file.strip_prefix(repo).unwrap_or(&file).display()
            ),
        );
    }
    let mut argv = vec![
        repo.join("ops/scripts/sync-slot-anchors.py")
            .display()
            .to_string(),
        "--env".into(),
        env.to_string(),
        "--check".into(),
    ];
    let store = env_or("FED_SLOT_REGISTRY_STORE", "sqlite".into());
    if store == "postgres" {
        let ssh = env_or("FED_SLOT_PG_SSH", String::new());
        if ssh.is_empty() {
            return (
                "BLOCKED".into(),
                false,
                "FED_SLOT_REGISTRY_STORE=postgres needs FED_SLOT_PG_SSH".into(),
            );
        }
        argv.extend([
            "--registry-store".into(),
            "postgres".into(),
            "--pg-ssh".into(),
            ssh,
        ]);
    } else {
        let db = env_or("FED_SLOT_DB", String::new());
        if db.is_empty() {
            return ("BLOCKED".into(), false, "the slot registry is not named: FED_SLOT_DB, or FED_SLOT_REGISTRY_STORE=postgres with FED_SLOT_PG_SSH".into());
        }
        argv.extend(["--db".into(), db]);
    }
    if !run {
        return (
            "NOT RUN".into(),
            false,
            format!("python3 {}", argv.join(" ")),
        );
    }
    match crate::federate::runenv::command("python3")
        .args(&argv)
        .current_dir(repo)
        .output()
    {
        Ok(o) => {
            let said = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
            .trim()
            .lines()
            .take(4)
            .collect::<Vec<_>>()
            .join(" / ");
            match o.status.code() {
                Some(0) => ("PASS".into(), true, said),
                Some(1) => (
                    "FAIL".into(),
                    false,
                    format!("the slot list differs from the registry (GB-134): {said}"),
                ),
                _ => (
                    "FAIL".into(),
                    false,
                    format!("the registry could not be read (GB-135): {said}"),
                ),
            }
        }
        Err(e) => ("FAIL".into(), false, format!("python3: {e}")),
    }
}

fn run_bash(repo: &Path, cmd: &str) -> Result<bool> {
    Ok(crate::federate::runenv::command("bash")
        .args(["-euo", "pipefail", "-c", cmd])
        .current_dir(repo)
        .status()
        .context("bash")?
        .success())
}

fn clear(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.path().extension().map(|x| x == "json").unwrap_or(false) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn row(name: &str, ok: bool, detail: String, fix: Option<String>) -> Check {
    Check {
        name: name.into(),
        ok,
        detail,
        fix,
    }
}

/// Our sovereign id: --as, else the agreement's other party, else our package.
fn our_id(o: &Options, work: &str) -> Option<String> {
    if let Some(a) = o.r#as {
        return Some(a.to_string());
    }
    let read = |p: String| {
        std::fs::read_to_string(p)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
    };
    if let Some(ag) = read(format!("{work}/agreement.json")) {
        if let Some(ps) = ag.get("parties").and_then(|p| p.as_array()) {
            if let Some(id) = ps
                .iter()
                .filter_map(|p| p.get("sovereign_id").and_then(|s| s.as_str()))
                .find(|s| *s != o.partner)
            {
                return Some(id.to_string());
            }
        }
    }
    read(format!("{work}/ours.canonical.json")).and_then(|v| {
        v.get("sovereign_id")
            .and_then(|s| s.as_str())
            .map(String::from)
    })
}

/// The directory step: `ops/federation/standup/directory_withdraw.py`, or why not.
fn directory(repo: &Path, o: &Options, t: &Target, dry: bool) -> Option<(String, Vec<String>)> {
    let url = env_or("FED_DIRECTORY_URL", String::new());
    if url.is_empty() || env_or("FED_NO_DIRECTORY", String::new()) == "1" {
        return None;
    }
    let us = our_id(o, &t.work).unwrap_or_default();
    let mut a = vec![
        repo.join("ops/federation/standup/directory_withdraw.py")
            .display()
            .to_string(),
        "--url".into(),
        url.clone(),
        "--us".into(),
        us,
        "--them".into(),
        t.partner.clone(),
    ];
    let key = env_or("FED_SIGNING_KEY", String::new());
    if !key.is_empty() {
        a.extend(["--key".into(), key]);
    }
    if dry {
        a.push("--dry-run".into());
    }
    Some((url, a))
}

pub fn backout(repo: &Path, o: &Options) -> Result<Report> {
    backout_with(repo, o, &mut |r: &Path, c: &str| run_bash(r, c))
}

/// [`backout`], with the play runner passed in, so a test can stand in for the
/// hosts and see which commands would have run.
pub fn backout_with(
    repo: &Path,
    o: &Options,
    exec: &mut dyn FnMut(&Path, &str) -> Result<bool>,
) -> Result<Report> {
    let mut t = target(o)?;
    if let Err(e) = work_belongs_to(&t.work, &t.partner) {
        return Err(goat_errors::coded("GB-220", e).into());
    }
    let cfg_path = match o.mock {
        Some(m) => m.join("goatbuilder.yaml"),
        None => repo.join("ops/goatbuilder/goatbuilder.yaml"),
    };
    let cfg = config::load(&cfg_path)?;
    if let Some(env) = o.env {
        if !cfg.environments.iter().any(|e| e.name == env) {
            return Err(anyhow!(
                "unknown environment \"{env}\"; declared: {:?}",
                cfg.environments
                    .iter()
                    .map(|e| e.name.as_str())
                    .collect::<Vec<_>>()
            ));
        }
    }
    let declared = cfg
        .peerings
        .iter()
        .find(|p| p.with == t.partner && o.env.map(|e| p.environment == e).unwrap_or(true));
    let env = declared
        .map(|p| p.environment.clone())
        .or_else(|| Some(env_or("FED_SLOT_ENV", String::new())).filter(|v| !v.is_empty()));
    // Our core: the deployment's own route, on the driver, when it names one
    // (the way admission went); else a goat-zenoh role core, surveyed by the
    // play at the paths admission resolved.
    let driver = driver_core(
        repo,
        &t.partner,
        env.as_deref().unwrap_or_default(),
        Path::new(&t.work),
        |k| std::env::var(k).ok(),
    )?;
    if driver.is_some() {
        t.core_files
            .push(("peering_backout_core_by_driver", "true".into()));
    } else {
        core_files(repo, &mut t)?;
    }
    // A role core admitted with no route of its own: the router leaf
    // admission placed on this machine, where the role reads it.
    let role_leaf = match &driver {
        Some(_) => None,
        None => role_leaf(repo, &t.partner, Path::new(&t.work), |k| {
            std::env::var(k).ok()
        })?,
    };
    let mut checks = vec![row(
        "banner",
        true,
        format!(
            "federate backout: peering={}  work={}  groups={},{},peering_forwarders  {}",
            t.partner,
            t.work,
            t.edge_group,
            t.core_group,
            if o.dry_run {
                "(DRY RUN — nothing changes)"
            } else if o.mock.is_some() {
                "(MOCK — nothing executes)"
            } else {
                ""
            }
        ),
        None,
    )];
    checks.push(row(
        "declaration",
        true,
        match declared {
            Some(p) => format!("{} is declared in peerings: on {}; remove that entry by review once the back-out is done", t.partner, p.environment),
            None => format!("{} is not declared in {} peerings:; backing out whatever of it is deployed", t.partner, cfg_path.display()),
        },
        None,
    ));

    let survey_cmd = play_cmd(repo, &t, &[]);
    // A leaf removed on this machine is deleted on the role core's host by
    // the play, which compares it with the peering's own first.
    let mut apply_extra = vec![("peering_backout_apply", "true".to_string())];
    if role_leaf
        .as_ref()
        .is_some_and(|l| l.role.is_some() && l.counts().1 > 0)
    {
        apply_extra.push(("peering_backout_core_leaf_remove", "true".into()));
    }
    let apply_cmd = play_cmd(repo, &t, &apply_extra);
    if o.mock.is_some() {
        for (n, (label, cmd)) in [
            (
                "survey every host of this peering (read-only)",
                survey_cmd.clone(),
            ),
            (
                "remove tunnel, SPIRE, edge router, republisher, forwarder mapping",
                apply_cmd.clone(),
            ),
            (
                "slot-list recheck before the core restart",
                slot_recheck(repo, env.as_deref(), false).2,
            ),
            (
                "install the reviewed core candidate and restart the core router",
                play_cmd(
                    repo,
                    &t,
                    &[
                        ("peering_backout_core_apply", "true".into()),
                        (
                            "peering_backout_core_expect",
                            "<candidate digest from the survey>".into(),
                        ),
                    ],
                ),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            checks.push(row("step", true, format!("{} {label}", n + 1), None));
            checks.push(row("cmd", true, cmd, None));
        }
        checks.push(row(
            "mock",
            true,
            "recorded against the fixture; nothing executed".into(),
            None,
        ));
        return Ok(Report::new("federate-backout", checks, EXIT_REFUSED));
    }

    // 1 survey
    let base = Path::new(&t.work).join("backout");
    let survey_dir = base.join("survey");
    let _ = std::fs::create_dir_all(&survey_dir);
    clear(&survey_dir);
    checks.push(row("cmd", true, survey_cmd.clone(), None));
    if !exec(repo, &survey_cmd)? {
        let recs = read_records(&survey_dir).unwrap_or_default();
        let s = summarize(&recs, &t.partner);
        let detail = if s.refused.is_empty() {
            "the survey play failed before every host answered".to_string()
        } else {
            format!("a resource under this peering's names is another peering's, or not provably this one's: {}", s.refused.join("; "))
        };
        let code = if s.refused.is_empty() {
            "GB-222"
        } else {
            "GB-221"
        };
        checks.push(row("survey", false, format!("{detail} ({code})"), Some("nothing was changed; read the play output above, fix the group values it names, then run the back-out again".into())));
        let mut r = Report::new("federate-backout", checks, EXIT_REFUSED);
        r.exit = EXIT_REFUSED;
        return Ok(r);
    }
    let mut recs = read_records(&survey_dir)?;
    if let Some(dc) = &driver {
        recs.push(driver_survey(
            dc,
            &t.partner,
            &base.join("core/driver/survey"),
        )?);
    }
    if let Some(leaf) = &role_leaf {
        recs.push(HostRecord {
            host: "driver".into(),
            peering: t.partner.clone(),
            rows: leaf.rows(),
        });
    }
    // No host answered, yet this working folder holds the peering's agreement:
    // the play matched no host (an inventory that does not name them), which
    // is not the same as nothing being deployed.
    if recs.iter().all(|r| r.host == "driver")
        && Path::new(&t.work).join("agreement.json").is_file()
    {
        checks.push(row(
            "survey",
            false,
            format!(
                "no host of {}, {} or peering_forwarders answered the survey, but {}/agreement.json shows this peering was set up from here: the inventory names none of its hosts (GB-222)",
                t.edge_group, t.core_group, t.work
            ),
            Some(format!(
                "nothing was changed; check that ops/ansible/inventory/peering-edges.ini or peering-{}.ini names the hosts of [{}] and [{}], then goatbuilder federate backout {} --dry-run",
                t.edge_group, t.edge_group, t.core_group, t.partner
            )),
        ));
        return Ok(Report::new("federate-backout", checks, EXIT_REFUSED));
    }
    let s = summarize(&recs, &t.partner);
    // The plan as surveyed, kept: the survey after the back-out reuses survey/.
    let plan_dir = base.join("plan");
    let _ = std::fs::create_dir_all(&plan_dir);
    clear(&plan_dir);
    for rec in &recs {
        let _ = std::fs::copy(
            survey_dir.join(format!("{}.json", rec.host)),
            plan_dir.join(format!("{}.json", rec.host)),
        );
    }
    for rec in &recs {
        if rec.peering != t.partner {
            return Err(anyhow!(
                "{} answered for peering '{}', not '{}'",
                rec.host,
                rec.peering,
                t.partner
            ));
        }
        for r in &rec.rows {
            checks.push(row(
                "resource",
                true,
                format!(
                    "{}  {}  {}  {}  {}",
                    rec.host, r.component, r.resource, r.state, r.action
                ),
                None,
            ));
        }
    }
    // The slot check runs here, before anything is removed: when the core will
    // restart and the check is BLOCKED or FAIL, the run stops now, so the edge is
    // never taken down while the core still admits the partner. It runs again
    // immediately before the restart (step 4).
    let (slot_word, slot_ok_now, slot_said) =
        slot_recheck(repo, env.as_deref(), s.core_candidate.is_some());
    let slot_blocks = s.core_candidate.is_some() && !slot_ok_now;
    let (rec_path, rec_ours) = partner_record(repo, &t.partner);
    let dir_plan = directory(repo, o, &t, true);
    let mut extra: Vec<(String, String)> = Vec::new();
    extra.push((
        "Core router".into(),
        match &s.core_candidate {
            Some(d) => format!(
                "restarts on candidate {d} after the slot recheck: {slot_word} — {slot_said}"
            ),
            None => "no restart: no block of this peering is spliced into the core".into(),
        },
    ));
    extra.push((
        "Directory".into(),
        match &dir_plan {
            Some((url, _)) => format!("withdraw our package for {} at {url}/v1/parties/<us>/packages/{}; our single entry and every other party's are left", t.partner, t.partner),
            None => "not applicable: FED_DIRECTORY_URL is unset or FED_NO_DIRECTORY=1".into(),
        },
    ));
    extra.push((
        "Partner record".into(),
        match rec_ours {
            None => format!("absent: {}", rec_path.display()),
            Some(true) => format!("delete {} from the checkout (generated for {}); commit the deletion through review", rec_path.display(), t.partner),
            Some(false) => format!("keep {}: it is not the file apply generates for {}", rec_path.display(), t.partner),
        },
    ));
    if !s.kept.is_empty() {
        extra.push((
            "Kept: another peering's, or the deployment's configuration to remove by review".into(),
            s.kept.join("\n  "),
        ));
    }
    let view = plan_view(&t, &recs, &extra);

    if slot_blocks {
        if o.table {
            print!("{view}");
        }
        checks.push(row(
            "slot-recheck",
            false,
            format!("{slot_word}: {slot_said} (GB-223)"),
            Some("nothing was changed: the core restart this back-out needs could not be cleared, so no resource is removed. Name the slot registry (FED_SLOT_DB, or FED_SLOT_REGISTRY_STORE=postgres with FED_SLOT_PG_SSH) or regenerate and commit the slot list, then run it again".into()),
        ));
        let mut r = Report::new("federate-backout", checks, EXIT_REFUSED);
        r.printed = o.table;
        return Ok(r);
    }

    if !s.refused.is_empty() {
        if o.table {
            print!("{view}");
        }
        checks.push(row(
            "survey",
            false,
            format!("refused (GB-221): {}", s.refused.join("; ")),
            Some(
                "nothing was changed; fix what each line names, then run the back-out again".into(),
            ),
        ));
        let mut r = Report::new("federate-backout", checks, EXIT_REFUSED);
        r.printed = o.table;
        return Ok(r);
    }

    if o.dry_run {
        if o.table {
            print!(
                "{view}\nTo run it for real: goatbuilder federate backout {}\n",
                t.partner
            );
        }
        checks.push(row(
            "plan",
            true,
            format!(
                "{} to remove, {} absent, core restart: {}",
                s.to_remove,
                s.absent,
                s.core_candidate.as_deref().unwrap_or("none")
            ),
            None,
        ));
        checks.push(row(
            "footer",
            true,
            "PLAN ONLY — nothing above changed.".into(),
            None,
        ));
        let mut r = Report::new("federate-backout", checks, EXIT_REFUSED);
        r.printed = o.table;
        return Ok(r);
    }

    // 2 confirm (the plan on stderr when stdout carries -o json or yaml)
    if o.table {
        print!("{view}");
    } else {
        eprint!("{view}");
    }
    if !o.yes {
        if !std::io::stdin().is_terminal() {
            return Err(goat_errors::coded(
                "GB-008",
                "federate backout is a mutation: pass --yes without a terminal (C10)",
            )
            .into());
        }
        eprint!(
            "\nType the partner id ({}) to back this peering out, anything else to stop: ",
            t.partner
        );
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .context("read the confirmation")?;
        if line.trim() != t.partner {
            checks.push(row(
                "confirm",
                false,
                "not confirmed; nothing was changed".into(),
                None,
            ));
            let mut r = Report::new("federate-backout", checks, EXIT_REFUSED);
            r.printed = true;
            return Ok(r);
        }
    }

    // 3 remove
    let mut exit = EXIT_OK;
    checks.push(row("cmd", true, apply_cmd.clone(), None));
    let apply_dir = base.join("apply");
    let _ = std::fs::create_dir_all(&apply_dir);
    clear(&apply_dir);
    if !exec(repo, &apply_cmd)? {
        checks.push(row("remove", false, "the removal play failed partway (GB-225); run the back-out again: every step is idempotent".into(), Some(format!("goatbuilder federate backout {}", t.partner))));
        let mut r = Report::new("federate-backout", checks, EXIT_REFUSED);
        r.exit = EXIT_NOT_CONVERGED;
        r.printed = true;
        return Ok(r);
    }

    // 4 + 5 the core. The digest is the survey's own, shaped as one before it
    // reaches a command line.
    let mut core_done = true;
    if let Some(digest) = s.core_candidate.as_ref().filter(|d| is_digest(d)) {
        let (slot_word, slot_ok, slot_said) = slot_recheck(repo, env.as_deref(), true);
        if !slot_ok {
            core_done = false;
            checks.push(row("slot-recheck", false, format!("{slot_word}: {slot_said} (GB-223)"), Some("regenerate and commit the slot list, then run the back-out again; the edge side is already removed".into())));
            exit = EXIT_NOT_CONVERGED;
        } else {
            checks.push(row(
                "slot-recheck",
                true,
                format!("{slot_word}: {slot_said}"),
                None,
            ));
            if let Some(dc) = &driver {
                // The way admission went: the driver's files, then its restart.
                match driver_apply(
                    repo,
                    dc,
                    &t.partner,
                    &base.join("core/driver/apply"),
                    digest,
                    exec,
                ) {
                    Ok(()) => checks.push(row(
                        "core",
                        true,
                        format!(
                            "core router restarted through {} on {digest}",
                            dc.core.route()
                        ),
                        None,
                    )),
                    Err(e) => {
                        core_done = false;
                        checks.push(row("core", false, format!("{e:#}"), Some(format!("goatbuilder federate backout {} --dry-run, review the new candidate, then run it again", t.partner))));
                        exit = EXIT_NOT_CONVERGED;
                    }
                }
            } else {
                let core_cmd = play_cmd(
                    repo,
                    &t,
                    &[
                        ("peering_backout_core_apply", "true".into()),
                        ("peering_backout_core_expect", digest.clone()),
                    ],
                );
                checks.push(row("cmd", true, core_cmd.clone(), None));
                let core_dir = base.join("core-apply");
                let _ = std::fs::create_dir_all(&core_dir);
                clear(&core_dir);
                if !exec(repo, &core_cmd)? {
                    core_done = false;
                    checks.push(row("core", false, "the core router was not restarted on the reviewed candidate (GB-224): its configuration changed since the survey, or the restart failed".into(), Some(format!("goatbuilder federate backout {} --dry-run, review the new candidate, then run it again", t.partner))));
                    exit = EXIT_NOT_CONVERGED;
                } else {
                    // Say what the core step did, from its own record, not from the plan.
                    let restarted = read_records(&core_dir)
                        .unwrap_or_default()
                        .iter()
                        .flat_map(|r| r.rows.iter())
                        .any(|r| {
                            r.resource.starts_with("router restart") && r.action == "restarted"
                        });
                    checks.push(row(
                    "core",
                    true,
                    if restarted { format!("core router restarted on {digest}") } else { "the core held no block of this peering any more at the restart step; nothing was restarted".to_string() },
                    None,
                ));
                }
            }
        }
    } else if s.core_candidate.is_some() {
        core_done = false;
        checks.push(row(
            "core",
            false,
            "the survey's core digest is not a sha256 digest; nothing was installed (GB-224)"
                .into(),
            None,
        ));
        exit = EXIT_NOT_CONVERGED;
    }
    // The router leaf admission placed on this machine: put back, or removed,
    // whether or not the core still held a block (a second run changes
    // nothing). On the deployment's own route driver_apply did it already.
    if core_done {
        if let Some(leaf) = driver.as_ref().map(|d| &d.leaf).or(role_leaf.as_ref()) {
            let (restored, _) = leaf.counts();
            let todo: Vec<String> = leaf
                .rows()
                .into_iter()
                .filter(|r| r.action == "remove")
                .map(|r| r.resource)
                .collect();
            let bdir = base.join("core/driver/leaf/backup");
            let mut backups = Vec::new();
            let taken = leaf.take_back(&mut backups, &bdir);
            // The host's copy: the role's play delivers what was put back,
            // the route admission took (dry run, then deploy).
            let taken = match (&taken, &leaf.role) {
                (Ok(()), Some(role)) if restored > 0 => {
                    let mut ok = true;
                    for check in [true, false] {
                        let cmd = role.play_cmd(repo, check);
                        checks.push(row("cmd", true, cmd.clone(), None));
                        if !exec(repo, &cmd)? {
                            ok = false;
                            break;
                        }
                    }
                    if ok {
                        taken
                    } else {
                        Err(format!(
                            "the role's play on [{}] did not deliver the leaf put back",
                            role.group
                        ))
                    }
                }
                _ => taken,
            };
            match taken {
                Ok(()) if todo.is_empty() => {}
                Ok(()) => checks.push(row(
                    "core leaf",
                    true,
                    format!(
                        "taken back: {}; what each held before this back-out is in {}",
                        todo.join(", "),
                        bdir.display()
                    ),
                    None,
                )),
                Err(e) => {
                    let back = super::admit::put_back(&backups);
                    core_done = false;
                    checks.push(row(
                        "core leaf",
                        false,
                        format!("{e}; {back} (GB-224)"),
                        Some(format!(
                            "goatbuilder federate backout {} --dry-run, then run it again",
                            t.partner
                        )),
                    ));
                    exit = EXIT_NOT_CONVERGED;
                }
            }
        }
    }
    if !core_done {
        // The partner is still admitted on the core: keep its directory slot and
        // record until the core step has run, so the record matches the core.
        checks.push(row(
            "directory",
            true,
            "not done: the core step did not complete".into(),
            None,
        ));
        checks.push(row(
            "partner-record",
            true,
            "not done: the core step did not complete".into(),
            None,
        ));
    }

    // 6 directory
    if let Some((_, args)) = directory(repo, o, &t, false).filter(|_| core_done) {
        let out = crate::federate::runenv::command("python3")
            .args(&args)
            .current_dir(repo)
            .output();
        let said = out
            .as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|e| e.to_string());
        let failed = out.map(|o| !o.status.success()).unwrap_or(true);
        // Advisory, as the directory is: listed grants nothing, so a failure is a warning.
        checks.push(row(
            "directory",
            true,
            if failed {
                format!("WARN (GB-226) {said}")
            } else {
                said
            },
            None,
        ));
    }

    // 7 the partner record, then the survey again
    if core_done && rec_ours == Some(true) {
        std::fs::remove_file(&rec_path)
            .with_context(|| format!("delete {}", rec_path.display()))?;
        checks.push(row(
            "partner-record",
            true,
            format!(
                "deleted {}; commit the deletion through review",
                rec_path.display()
            ),
            None,
        ));
    }
    clear(&survey_dir);
    let after_ok = exec(repo, &survey_cmd)?;
    let mut after = read_records(&survey_dir).unwrap_or_default();
    if let Some(dc) = &driver {
        after.push(driver_survey(
            dc,
            &t.partner,
            &base.join("core/driver/after"),
        )?);
    }
    if let Some(leaf) = &role_leaf {
        after.push(HostRecord {
            host: "driver".into(),
            peering: t.partner.clone(),
            rows: leaf.rows(),
        });
    }
    let left = summarize(&after, &t.partner);
    if !after_ok || left.to_remove > 0 {
        checks.push(row(
            "converged",
            false,
            format!(
                "{} resources of {} are still present after the back-out (GB-225)",
                left.to_remove, t.partner
            ),
            Some(format!("goatbuilder federate backout {}", t.partner)),
        ));
        exit = EXIT_NOT_CONVERGED;
    } else if left.role_declared {
        checks.push(row(
            "converged",
            false,
            format!(
                "the core still admits {} through zenoh_sovereign_peerings (GB-227)",
                t.partner
            ),
            Some(
                "remove the entry from the core's configuration by review and run the core play"
                    .into(),
            ),
        ));
        exit = EXIT_NOT_CONVERGED;
    } else {
        checks.push(row(
            "converged",
            true,
            format!(
                "nothing of {} is left on its hosts; left to configuration: {}",
                t.partner,
                if left.kept.is_empty() {
                    "nothing".to_string()
                } else {
                    left.kept.join("; ")
                }
            ),
            None,
        ));
    }
    let result = serde_json::json!({
        "schema": "goat-federation-backout/v1",
        "peering": t.partner,
        "converged": exit == EXIT_OK,
        "removed_planned": s.to_remove,
        "core_candidate": s.core_candidate,
        "left_to_configuration": left.kept,
        "rows": checks.iter().map(|c| serde_json::json!({"name": c.name, "ok": c.ok, "detail": c.detail})).collect::<Vec<_>>(),
    });
    let _ = std::fs::write(
        base.join("result.json"),
        serde_json::to_string_pretty(&result)? + "\n",
    );
    let mut r = Report::new("federate-backout", checks, EXIT_NOT_CONVERGED);
    r.exit = exit;
    r.ok = exit == EXIT_OK;
    r.printed = true;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(partner: &str) -> Options<'_> {
        Options {
            partner,
            env: None,
            r#as: None,
            edge_group: None,
            work: Some("/var/lib/goat-federation/xac"),
            mock: None,
            dry_run: true,
            yes: false,
            table: false,
        }
    }

    fn rec(host: &str, peering: &str, rows: &[(&str, &str, &str, &str, &str)]) -> HostRecord {
        HostRecord {
            host: host.into(),
            peering: peering.into(),
            rows: rows
                .iter()
                .map(|(c, r, s, a, n)| Row {
                    component: c.to_string(),
                    resource: r.to_string(),
                    state: s.to_string(),
                    action: a.to_string(),
                    note: n.to_string(),
                })
                .collect(),
        }
    }

    /// One of two peerings on the same edge and core: xab and xac share
    /// pasture's edge and core. A survey of xac, as the play records it.
    fn xac_survey() -> Vec<HostRecord> {
        vec![
            rec(
                "core-xac",
                "xac",
                &[
                    (
                        "republisher",
                        "container goat-fed-republisher-xac",
                        "present",
                        "remove",
                        "Up",
                    ),
                    ("core admission", "subject fed-xac", "present", "remove", ""),
                    (
                        "core admission",
                        "subject fed-rp-xac",
                        "present",
                        "remove",
                        "",
                    ),
                    ("core admission", "rule fed-w-xac", "present", "remove", ""),
                    (
                        "core admission",
                        "router restart on /etc/xaa-zenoh/config.json5",
                        "present",
                        "restart",
                        "89 lines leave; candidate sha256:abc123; stays admitted: xab",
                    ),
                ],
            ),
            rec(
                "edge-xac",
                "xac",
                &[
                    ("tunnel", "interface wg-fed1", "present", "remove", "ours"),
                    (
                        "tunnel",
                        "nft table inet goat_peering_wg_fed1",
                        "present",
                        "remove",
                        "ours",
                    ),
                    (
                        "spire",
                        "unit goat-peering-bundle-in-xac.socket",
                        "present",
                        "remove",
                        "",
                    ),
                    (
                        "spire",
                        "unit goat-peering-bundle-in.socket",
                        "present",
                        "kept",
                        "fixed name, another peering's addresses or unproven",
                    ),
                    (
                        "edge router",
                        "container goat-zenoh-edge-xac",
                        "present",
                        "remove",
                        "",
                    ),
                ],
            ),
            rec(
                "fwd",
                "xac",
                &[
                    (
                        "forwarder",
                        "carrier wg-carry1",
                        "present",
                        "remove",
                        "public 51826, carrier 51827",
                    ),
                    (
                        "forwarder",
                        "declaration peering_forwarder_peerings[name=xac]",
                        "present",
                        "kept",
                        "configuration",
                    ),
                ],
            ),
        ]
    }

    #[test]
    fn a_partner_is_a_sovereign_id_and_never_a_subcommand() {
        for bad in [
            "", "x", "XAB", "x/../y", "-xab", "a b", "backout", "apply", "status",
        ] {
            assert!(target(&opts(bad)).is_err(), "{bad} must be refused");
        }
        assert!(target(&opts("xac")).is_ok());
        assert!(config::RESERVED_PEERING_NAMES.contains(&VERB));
    }

    #[test]
    fn the_play_names_only_this_peerings_groups() {
        let t = target(&opts("xac")).unwrap();
        let cmd = play_cmd(
            Path::new("/r"),
            &t,
            &[("peering_backout_apply", "true".into())],
        );
        assert!(
            cmd.contains("--limit 'xac_edge:xac_core:peering_forwarders'"),
            "{cmd}"
        );
        assert!(cmd.contains("-e peering_backout_id='xac'"));
        assert!(cmd.contains("-e peering_backout_apply='true'"));
        assert!(cmd.ends_with("peering-backout.yml"));
        assert!(!cmd.contains("xab"));
    }

    #[test]
    fn a_work_path_a_plan_line_cannot_carry_is_refused() {
        let mut o = opts("xac");
        o.work = Some("/tmp/x'; rm -rf /");
        assert!(target(&o).is_err());
        let mut o = opts("xac");
        o.edge_group = Some("xac_edge $(id)");
        assert!(target(&o).is_err());
    }

    #[test]
    fn the_dry_run_view_shows_this_peering_per_host_and_nothing_of_the_other() {
        let t = target(&opts("xac")).unwrap();
        let recs = xac_survey();
        let v = plan_view(
            &t,
            &recs,
            &[(
                "Core router".into(),
                "restarts on candidate sha256:abc123".into(),
            )],
        );
        assert!(v.contains("Nothing has changed"));
        for host in ["core-xac", "edge-xac", "fwd"] {
            assert!(v.contains(&format!("\n{host}\n")), "{v}");
        }
        assert!(
            v.contains("remove    tunnel          interface wg-fed1"),
            "{v}"
        );
        assert!(
            v.contains("restart   core admission  router restart"),
            "{v}"
        );
        assert!(
            v.contains("keep      spire           unit goat-peering-bundle-in.socket"),
            "{v}"
        );
        // Every line that removes something names xac's resources; xab is named only
        // where the plan says it stays admitted.
        for line in v.lines().filter(|l| {
            l.trim_start().starts_with("remove") || l.trim_start().starts_with("restart ")
        }) {
            let resource = line.split("  (").next().unwrap();
            assert!(
                !resource.contains("xab")
                    && !resource.contains("wg-fed0")
                    && !resource.contains("wg-carry0"),
                "{line}"
            );
        }
        let s = summarize(&recs, "xac");
        assert_eq!(s.to_remove, 10);
        assert_eq!(s.core_candidate.as_deref(), Some("sha256:abc123"));
        assert_eq!(s.kept.len(), 2);
        assert!(s.refused.is_empty());
        assert!(!s.role_declared);
    }

    #[test]
    fn a_resource_another_peering_holds_refuses_the_back_out() {
        let recs = vec![rec(
            "edge-xac",
            "xac",
            &[(
                "tunnel",
                "interface wg-fed0",
                "not-ours",
                "kept",
                "other:xab",
            )],
        )];
        let s = summarize(&recs, "xac");
        assert_eq!(s.refused.len(), 1);
        assert!(s.refused[0].contains("other:xab"));
        assert_eq!(s.to_remove, 0);
    }

    #[test]
    fn a_second_run_finds_nothing_and_restarts_nothing() {
        let recs = vec![
            rec(
                "core-xac",
                "xac",
                &[(
                    "core admission",
                    "fragment /etc/xaa-zenoh/acl-org-fragment.json5",
                    "absent",
                    "none",
                    "no block of xac is spliced; no restart",
                )],
            ),
            rec(
                "edge-xac",
                "xac",
                &[("tunnel", "interface wg-fed1", "absent", "none", "ours")],
            ),
        ];
        let s = summarize(&recs, "xac");
        assert_eq!(s.to_remove, 0);
        assert_eq!(s.core_candidate, None);
    }

    #[test]
    fn a_role_declared_core_is_seen() {
        let recs = vec![rec(
            "core-xac",
            "xac",
            &[(
                "core admission",
                "zenoh_sovereign_peerings[sovereign=xac]",
                "present",
                "kept",
                "configuration",
            )],
        )];
        assert!(summarize(&recs, "xac").role_declared);
        assert!(!summarize(&recs, "xa").role_declared);
    }

    #[test]
    fn only_the_generated_record_of_exactly_this_partner_is_deleted() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("ops/federation/partners");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("xac.yaml"), "# GENERATED by `goat federate apply` from the composed agreement\n# sha256:00\nsovereign_id: xac\nlevel: 1\n").unwrap();
        std::fs::write(
            dir.join("xab.yaml"),
            "# GENERATED by `goat federate apply` from the composed agreement\nsovereign_id: xab\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("xad.yaml"),
            "# written by hand\nsovereign_id: xad\n",
        )
        .unwrap();
        assert_eq!(partner_record(d.path(), "xac").1, Some(true));
        assert_eq!(partner_record(d.path(), "xad").1, Some(false));
        assert_eq!(partner_record(d.path(), "xa").1, None);
        std::fs::write(dir.join("xaf.yaml"), "# GENERATED by `goatbuilder federate apply` from the composed agreement\nsovereign_id: xaf\n").unwrap();
        assert_eq!(
            partner_record(d.path(), "xaf").1,
            Some(true),
            "the goatbuilder wording is recognized too"
        );
        std::fs::write(
            dir.join("xae.yaml"),
            "# GENERATED by `goat federate apply` from the composed agreement\nsovereign_id: xab\n",
        )
        .unwrap();
        assert_eq!(
            partner_record(d.path(), "xae").1,
            Some(false),
            "a generated file naming another partner is not this one's"
        );
    }

    #[test]
    fn the_slot_recheck_is_not_applicable_without_lazy_storage_and_blocked_without_a_registry() {
        let d = tempfile::tempdir().unwrap();
        let (w, ok, _) = slot_recheck(d.path(), Some("dev"), true);
        assert_eq!((w.as_str(), ok), ("NOT APPLICABLE", true));
        let (w, ok, _) = slot_recheck(d.path(), None, true);
        assert_eq!((w.as_str(), ok), ("BLOCKED", false));
        let f = d.path().join("ops/ansible/group_vars/dev_hosts");
        std::fs::create_dir_all(&f).unwrap();
        std::fs::write(
            f.join("zenoh-lazy-storage.yml"),
            "zenoh_active_slot_anchors: []\n",
        )
        .unwrap();
        if crate::federate::runenv::var("FED_SLOT_DB").is_err()
            && crate::federate::runenv::var("FED_SLOT_REGISTRY_STORE").is_err()
        {
            let (w, ok, said) = slot_recheck(d.path(), Some("dev"), true);
            assert_eq!((w.as_str(), ok), ("BLOCKED", false), "{said}");
        }
    }

    #[test]
    fn a_digest_is_sha256_hex_and_nothing_else() {
        assert!(is_digest(&format!("sha256:{}", "a".repeat(64))));
        for bad in [
            "sha256:abc",
            "",
            "sha256:'; id #",
            &format!("sha256:{}", "A".repeat(64)),
        ] {
            assert!(!is_digest(bad), "{bad}");
        }
    }

    #[test]
    fn a_working_directory_of_another_peering_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        assert!(
            work_belongs_to(&w, "xac").is_ok(),
            "no agreement yet: nothing to contradict"
        );
        std::fs::write(
            d.path().join("agreement.json"),
            r#"{"parties":[{"sovereign_id":"xaa"},{"sovereign_id":"xab"}]}"#,
        )
        .unwrap();
        assert!(work_belongs_to(&w, "xac").is_err());
        assert!(work_belongs_to(&w, "xab").is_ok());
    }

    /// A repository whose deployment declares lazy slot storage, a work
    /// directory, and a stand-in for the hosts: the survey finds the partner's
    /// edge router and a core admission that needs a restart.
    fn stand_in(dir: &Path) -> (PathBuf, String) {
        let repo = dir.join("repo");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../ops/goatbuilder/fixtures/goatnet-from-scratch/03-boundary-node/goatbuilder.yaml",
        );
        std::fs::create_dir_all(repo.join("ops/goatbuilder")).unwrap();
        std::fs::copy(fixture, repo.join("ops/goatbuilder/goatbuilder.yaml")).unwrap();
        let gv = repo.join("ops/ansible/group_vars/zagros_hosts");
        std::fs::create_dir_all(&gv).unwrap();
        std::fs::write(
            gv.join("zenoh-lazy-storage.yml"),
            "zenoh_active_slot_anchors: []\n",
        )
        .unwrap();
        (repo, dir.join("work").display().to_string())
    }

    fn fake_hosts(
        work: String,
        ran: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    ) -> impl FnMut(&Path, &str) -> Result<bool> {
        move |_: &Path, cmd: &str| {
            ran.borrow_mut().push(cmd.to_string());
            let survey = Path::new(&work).join("backout/survey");
            std::fs::create_dir_all(&survey).unwrap();
            std::fs::write(
                survey.join("core-x.json"),
                r#"{"host":"core-x","peering":"hello-ally","rows":[
                  {"component":"edge router","resource":"container goat-zenoh-edge-hello-ally","state":"present","action":"remove","note":""},
                  {"component":"core admission","resource":"router restart on /etc/x/config.json5","state":"present","action":"restart","note":"8 lines leave; candidate sha256:0000000000000000000000000000000000000000000000000000000000000000; stays admitted: no other peering"}]}"#,
            )
            .unwrap();
            Ok(true)
        }
    }

    #[test]
    fn a_blocked_slot_check_stops_the_run_before_anything_is_removed() {
        if crate::federate::runenv::var("FED_SLOT_DB").is_ok()
            || crate::federate::runenv::var("FED_SLOT_REGISTRY_STORE").is_ok()
        {
            return; // the check would not be BLOCKED in this environment
        }
        let d = tempfile::tempdir().unwrap();
        let (repo, work) = stand_in(d.path());
        let ran = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut exec = fake_hosts(work.clone(), ran.clone());
        let o = Options {
            partner: "hello-ally",
            env: None,
            r#as: None,
            edge_group: None,
            work: Some(&work),
            mock: None,
            dry_run: false,
            yes: true,
            table: false,
        };
        let r = backout_with(&repo, &o, &mut exec).unwrap();
        assert!(!r.ok);
        assert_eq!(r.exit, EXIT_REFUSED);
        let gate = r
            .checks
            .iter()
            .find(|c| c.name == "slot-recheck")
            .expect("a slot-recheck row");
        assert!(
            gate.detail.starts_with("BLOCKED") && gate.detail.contains("GB-223"),
            "{}",
            gate.detail
        );
        let ran = ran.borrow();
        assert_eq!(ran.len(), 1, "only the survey ran: {ran:?}");
        assert!(
            !ran[0].contains("peering_backout_apply")
                && !ran[0].contains("peering_backout_core_apply")
        );
        // The dry run says the same, and also removes nothing.
        let ran2 = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut exec2 = fake_hosts(work.clone(), ran2.clone());
        let o2 = Options { dry_run: true, ..o };
        let r2 = backout_with(&repo, &o2, &mut exec2).unwrap();
        assert_eq!(r2.exit, EXIT_REFUSED);
        assert_eq!(ran2.borrow().len(), 1);
    }

    #[test]
    fn the_back_out_reads_the_same_inventories_as_the_forward_path() {
        // A nothing-hand-written site: allocate rendered peering-<edge group>.ini
        // and there is no peering-edges.ini. The forward path (admit.rs) passes
        // every per-peering file; the back-out must reach the same hosts.
        let d = tempfile::tempdir().unwrap();
        let inv = d.path().join("ops/ansible/inventory");
        std::fs::create_dir_all(&inv).unwrap();
        std::fs::write(
            inv.join("peering-xac_edge.ini"),
            "[xac_edge]\nedge-1\n[xac_core]\ncore-1\n",
        )
        .unwrap();
        std::fs::write(inv.join("peering-xab_edge.ini"), "[xab_edge]\nedge-2\n").unwrap();
        let t = target(&opts("xac")).unwrap();
        let cmd = play_cmd(d.path(), &t, &[]);
        assert!(
            cmd.contains(&format!(
                " {} ",
                super::super::plan::inventory_args(d.path())
            )),
            "{cmd}"
        );
        assert!(
            cmd.contains("-i 'inventory/peering-xab_edge.ini' -i 'inventory/peering-xac_edge.ini'"),
            "{cmd}"
        );
    }

    #[test]
    fn a_survey_no_host_answered_is_refused_while_the_peering_has_an_agreement() {
        let d = tempfile::tempdir().unwrap();
        let (repo, work) = stand_in(d.path());
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(
            Path::new(&work).join("agreement.json"),
            r#"{"parties":[{"sovereign_id":"xaa"},{"sovereign_id":"hello-ally"}]}"#,
        )
        .unwrap();
        let ran = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let r2 = ran.clone();
        // Ansible matched no host: the play "passes" and writes no record.
        let mut exec = move |_: &Path, cmd: &str| -> Result<bool> {
            r2.borrow_mut().push(cmd.to_string());
            Ok(true)
        };
        for dry_run in [true, false] {
            ran.borrow_mut().clear();
            let o = Options {
                partner: "hello-ally",
                env: None,
                r#as: None,
                edge_group: None,
                work: Some(&work),
                mock: None,
                dry_run,
                yes: true,
                table: false,
            };
            let r = backout_with(&repo, &o, &mut exec).unwrap();
            assert!(!r.ok, "dry_run={dry_run}");
            assert_eq!(r.exit, EXIT_REFUSED);
            let row = r
                .checks
                .iter()
                .find(|c| c.name == "survey")
                .expect("a survey row");
            assert!(
                row.detail.contains("GB-222") && row.detail.contains("no host"),
                "{}",
                row.detail
            );
            assert_eq!(
                ran.borrow().len(),
                1,
                "only the survey ran: {:?}",
                ran.borrow()
            );
        }
        // Without an agreement in the working folder, nothing of the peering
        // was ever deployed from here, and an empty survey is not a refusal.
        std::fs::remove_file(Path::new(&work).join("agreement.json")).unwrap();
        let o = Options {
            partner: "hello-ally",
            env: None,
            r#as: None,
            edge_group: None,
            work: Some(&work),
            mock: None,
            dry_run: true,
            yes: true,
            table: false,
        };
        let r = backout_with(&repo, &o, &mut exec).unwrap();
        assert!(r.checks.iter().all(|c| !c.detail.contains("GB-222")));
    }

    /// A core admitted through the goat-zenoh role keeps its files where the
    /// role's group resolves them; the back-out must read those, the files
    /// admission wrote, not paths from the peering core group's own vars.
    fn role_core_checkout() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (rel, text) in [
            ("ops/ansible/roles/goat-zenoh/defaults/main.yml", "zenoh_router_image: \"x\"\nzenoh_stack: \"{{ site_id }}-zenoh\"\ngoat_zenoh_config_path: \"/etc/{{ zenoh_stack }}/config.json5\"\ngoat_zenoh_acl_seed_path: \"/etc/{{ zenoh_stack }}/config.seed.json5\"\ngoat_zenoh_acl_org_path: \"/etc/{{ zenoh_stack }}/acl-org-fragment.json5\"\nzenoh_sovereign_peerings: []\n"),
            ("ops/ansible/zenoh-deploy.yml", "- hosts: \"{{ target | mandatory }}\"\n"),
            ("ops/pins/fleet-pins.yaml", "components:\n  zenoh-router:\n    var: zenoh_router_image\n    ledger_component: xaa-zenoh\n    deploy_playbook: zenoh-deploy.yml\n"),
            ("ops/ansible/inventory/core-router.ini", "[core_router]\nxaa-core ansible_host=192.0.2.10\n"),
            ("ops/ansible/group_vars/core_router/zenoh.yml", "zenoh_router_image: \"ghcr.io/example-org/zenoh@sha256:00\"\nsite_id: xaa\n"),
        ] {
            let f = d.path().join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        d
    }

    #[test]
    fn the_router_leaf_on_a_role_core_without_fed_core_acl_org_is_taken_back() {
        // A goat-zenoh role core admitted with no route of its own: admission
        // placed our core's router leaf on this machine where the role reads
        // it. The back-out takes it back there, by the same earliest-record
        // rule: the certificate goes back to what was there, the key, placed
        // where nothing was, is removed.
        let d = tempfile::tempdir().unwrap();
        let (repo, work) = stand_in(d.path());
        let role = role_core_checkout();
        for rel in [
            "ops/ansible/roles/goat-zenoh/defaults/main.yml",
            "ops/ansible/zenoh-deploy.yml",
            "ops/pins/fleet-pins.yaml",
            "ops/ansible/inventory/core-router.ini",
            "ops/ansible/group_vars/core_router/zenoh.yml",
        ] {
            let to = repo.join(rel);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(role.path().join(rel), &to).unwrap();
        }
        let leaves = d.path().join("xaa-leaves");
        std::fs::create_dir_all(&leaves).unwrap();
        let (cert, key) = (
            leaves.join("router-cert.pem"),
            leaves.join("router-key.pem"),
        );
        std::fs::write(
            repo.join("ops/ansible/group_vars/core_router/leaf.yml"),
            format!(
                "zenoh_router_leaf_cert_path: \"{}\"\nzenoh_router_leaf_key_path: \"{}\"\n",
                cert.display(),
                key.display()
            ),
        )
        .unwrap();
        let pki = Path::new(&work).join("pki");
        std::fs::create_dir_all(&pki).unwrap();
        std::fs::write(pki.join("core.pem"), b"CERT-OF-HELLO-ALLY").unwrap();
        std::fs::write(pki.join("core-key.pem"), b"KEY-OF-HELLO-ALLY").unwrap();
        std::fs::write(&cert, b"THE-CERT-BEFORE").unwrap();
        let mut backups = Vec::new();
        let bdir = Path::new(&work).join("admission/run-20261112T000000Z-1/backup");
        super::super::admit::publish(b"CERT-OF-HELLO-ALLY", &cert, &mut backups, &bdir).unwrap();
        super::super::admit::publish(b"KEY-OF-HELLO-ALLY", &key, &mut backups, &bdir).unwrap();
        // The hosts: every play answers, with nothing of the peering left.
        let w = work.clone();
        let ran = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let r2 = ran.clone();
        let mut exec = move |_: &Path, cmd: &str| -> Result<bool> {
            r2.borrow_mut().push(cmd.to_string());
            let survey = Path::new(&w).join("backout/survey");
            std::fs::create_dir_all(&survey).unwrap();
            std::fs::write(
                survey.join("core-x.json"),
                r#"{"host":"core-x","peering":"hello-ally","rows":[]}"#,
            )
            .unwrap();
            Ok(true)
        };
        let o = Options {
            partner: "hello-ally",
            env: None,
            r#as: None,
            edge_group: None,
            work: Some(&work),
            mock: None,
            dry_run: false,
            yes: true,
            table: false,
        };
        let r = backout_with(&repo, &o, &mut exec).unwrap();
        assert_eq!(
            std::fs::read(&cert).unwrap(),
            b"THE-CERT-BEFORE",
            "{:?}",
            r.checks
        );
        assert!(
            !key.exists(),
            "no private key is left behind: {:?}",
            r.checks
        );
        let converged = r.checks.iter().find(|c| c.name == "converged").unwrap();
        assert!(converged.ok, "{:?}", r.checks);
        // The core host's copy follows, the way admission delivered it: the
        // role's play, dry run then deploy, delivers the certificate put back;
        // the key placed where nothing was is deleted on the host by the
        // back-out play, told so.
        let ran = ran.borrow();
        let plays: Vec<&String> = ran
            .iter()
            .filter(|c| c.contains("zenoh-deploy.yml"))
            .collect();
        assert_eq!(plays.len(), 2, "the role's dry run and deploy: {ran:?}");
        assert!(
            plays[0].contains("--check") && !plays[1].contains("--check"),
            "{plays:?}"
        );
        assert!(
            ran.iter()
                .any(|c| c.contains("peering_backout_apply='true'")
                    && c.contains("-e peering_backout_core_leaf_remove='true'")),
            "{ran:?}"
        );
    }

    #[test]
    fn a_role_cores_files_are_the_ones_admission_wrote() {
        let r = role_core_checkout();
        let mut t = target(&opts("xab")).unwrap();
        core_files(r.path(), &mut t).unwrap();
        let cmd = play_cmd(r.path(), &t, &[]);
        for (k, v) in [
            (
                "peering_backout_core_seed",
                "/etc/xaa-zenoh/config.seed.json5",
            ),
            (
                "peering_backout_core_org",
                "/etc/xaa-zenoh/acl-org-fragment.json5",
            ),
            ("peering_backout_core_config", "/etc/xaa-zenoh/config.json5"),
        ] {
            assert!(cmd.contains(&format!("-e {k}='{v}'")), "{cmd}");
        }
        // Without a role core, nothing is added: the core's own vars decide, as before.
        let plain_repo = tempfile::tempdir().unwrap();
        let mut t = target(&opts("xab")).unwrap();
        core_files(plain_repo.path(), &mut t).unwrap();
        assert!(!play_cmd(plain_repo.path(), &t, &[]).contains("peering_backout_core_org"));
    }

    #[test]
    fn a_core_admission_that_could_not_be_read_stops_the_back_out() {
        let recs = vec![rec(
            "core-x",
            "xab",
            &[(
                "core admission",
                "/etc/-zenoh/config.json5",
                "unknown",
                "kept",
                "not readable: /etc/-zenoh/config.seed.json5",
            )],
        )];
        let s = summarize(&recs, "xab");
        assert!(
            s.refused.iter().any(|r| r.contains("not readable")),
            "an unreadable core admission is not 'absent': {s:?}"
        );
    }

    /// Any row a host could not read is a refusal, whatever its component:
    /// the SPIRE row that could not read the server included.
    #[test]
    fn every_row_a_host_could_not_read_stops_the_back_out() {
        let recs = vec![rec(
            "edge-x",
            "xab",
            &[
                ("spire", "federation (trust domain unknown)", "unknown", "none", "no agreement or rendered trust domain, or no peering_spire_server_host: the server was not read"),
                ("tunnel", "interface wg-fed1", "absent", "none", "ours"),
            ],
        )];
        let s = summarize(&recs, "xab");
        assert_eq!(s.refused.len(), 1, "{s:?}");
        assert!(s.refused[0].contains("trust domain unknown"), "{s:?}");
        assert_eq!(s.absent, 1);
    }

    /// The probe contract the roles follow (ops/ansible/filter_plugins/
    /// backout_probe.py): a probe that could not read gives `unknown`, and
    /// every such row, for each of the five probes, stops the back-out.
    #[test]
    fn every_probe_that_could_not_read_stops_the_back_out() {
        let unknown = [
            ("edge router", "container goat-zenoh-edge-xab"),
            ("republisher", "container goat-fed-republisher-xab"),
            ("forwarder", "carrier wg-carry0"),
            ("spire", "federation xab.fed.test"),
            ("spire", "bundle xab.fed.test"),
        ];
        for (component, resource) in unknown {
            let recs = vec![rec(
                "h",
                "xab",
                &[(
                    component,
                    resource,
                    "unknown",
                    "none",
                    "rc 1: permission denied",
                )],
            )];
            let s = summarize(&recs, "xab");
            assert_eq!(s.refused.len(), 1, "{component} {resource}: {s:?}");
            assert_eq!((s.to_remove, s.absent), (0, 0), "{component} {resource}");
        }
    }

    /// Admission through the deployment's own route (FED_CORE_ACL_ORG, with
    /// FED_CORE_ACL_SEED/OUT, restarted by FED_CORE_RESTART or FED_CORE_PLAY on
    /// FED_CORE_GROUP) keeps our core's files on the driver. The back-out
    /// follows the same route: it surveys those files, places the candidate,
    /// dry-runs and restarts the way admission did, and the files are what they
    /// were before the peering, byte for byte.
    struct RouteCase {
        dir: tempfile::TempDir,
        vars: BTreeMap<String, String>,
        before_org: String,
        before_live: Option<String>,
        repo: PathBuf,
    }

    impl RouteCase {
        fn work(&self) -> PathBuf {
            self.dir.path().join("peerings/xab")
        }
    }

    /// Admission placing our core router's leaf (FED_CORE_CERT/KEY) from
    /// phase 8's pki/core.pem and core-key.pem, with a backup of what was at
    /// each path before, as admit.rs `publish` keeps it.
    fn place_leaf(c: &mut RouteCase, prior: Option<&[u8]>) {
        let pki = c.work().join("pki");
        std::fs::create_dir_all(&pki).unwrap();
        std::fs::write(pki.join("core.pem"), b"CERT-OF-XAB").unwrap();
        std::fs::write(pki.join("core-key.pem"), b"KEY-OF-XAB").unwrap();
        let leaves = c.dir.path().join("leaves");
        std::fs::create_dir_all(&leaves).unwrap();
        let (cert, key) = (
            leaves.join("router-cert.pem"),
            leaves.join("router-key.pem"),
        );
        if let Some(p) = prior {
            std::fs::write(&cert, p).unwrap();
            std::fs::write(&key, p).unwrap();
        }
        let mut backups = Vec::new();
        let bdir = c.work().join("admission/run-20261112T000000Z-1/backup");
        super::super::admit::publish(b"CERT-OF-XAB", &cert, &mut backups, &bdir).unwrap();
        super::super::admit::publish(b"KEY-OF-XAB", &key, &mut backups, &bdir).unwrap();
        c.vars
            .insert("FED_CORE_CERT".into(), cert.display().to_string());
        c.vars
            .insert("FED_CORE_KEY".into(), key.display().to_string());
    }

    const SEED: &str = "{\n  // BEGIN: per-vendor storages\n  // END: per-vendor storages\n  // BEGIN: per-vendor write rules\n  // END: per-vendor write rules\n  // BEGIN: per-vendor subjects\n  // END: per-vendor subjects\n  // BEGIN: per-vendor policies\n  // END: per-vendor policies\n}\n";
    const BEFORE_ORG: &str = "##ACL-ORG-REGION storages\n##ACL-ORG-REGION write-rules\n      { id: \"vendor-a\", messages: [\"put\"], flows: [\"ingress\"], permission: \"allow\", key_exprs: [\"vendor/a/**\"] },\n##ACL-ORG-REGION subjects\n##ACL-ORG-REGION policies\n";
    const FRAGMENT: &str = "##ACL-ORG-REGION storages\n##ACL-ORG-REGION write-rules\n      {\n        id: \"fed-w-xab\",\n        messages: [\"put\", \"delete\"],\n        flows: [\"ingress\"],\n        permission: \"allow\",\n        key_exprs: [\n          \"from/xab/**\",\n        ],\n      },\n##ACL-ORG-REGION subjects\n      {\n        id: \"fed-xab\",\n        cert_common_names: [\"fed.xab.xaa.01\"],\n      },\n##ACL-ORG-REGION policies\n      { rules: [\"fed-w-xab\"], subjects: [\"fed-xab\"] },\n";

    fn real() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn assemble(seed: &Path, org: &Path, out: &Path) {
        let st = std::process::Command::new("python3")
            .arg(real().join("ops/ansible/roles/goat-zenoh/files/acl-assemble.py"))
            .args(["assemble", "--seed"])
            .arg(seed)
            .arg("--org")
            .arg(org)
            .arg("--out")
            .arg(out)
            .status()
            .unwrap();
        assert!(st.success());
    }

    /// A core admitted xab on the driver's files, as admission writes them.
    fn admitted(with_seed: bool, route: &[(&str, &str)]) -> RouteCase {
        let dir = tempfile::tempdir().unwrap();
        let org = dir.path().join("org.json5");
        std::fs::write(&org, BEFORE_ORG).unwrap();
        let mut vars: BTreeMap<String, String> = BTreeMap::new();
        vars.insert("FED_CORE_ACL_ORG".into(), org.display().to_string());
        let mut before_live = None;
        if with_seed {
            let seed = dir.path().join("seed.json5");
            let out = dir.path().join("config.json5");
            std::fs::write(&seed, SEED).unwrap();
            assemble(&seed, &org, &out);
            before_live = Some(std::fs::read_to_string(&out).unwrap());
            vars.insert("FED_CORE_ACL_SEED".into(), seed.display().to_string());
            vars.insert("FED_CORE_ACL_OUT".into(), out.display().to_string());
        }
        // Admission: admit.rs merge_org, then the assembler.
        let merged = super::super::admit::merge_org(BEFORE_ORG, FRAGMENT, "xab").unwrap();
        std::fs::write(&org, &merged).unwrap();
        if with_seed {
            assemble(
                Path::new(&vars["FED_CORE_ACL_SEED"]),
                &org,
                Path::new(&vars["FED_CORE_ACL_OUT"]),
            );
        }
        for (k, v) in route {
            vars.insert(
                k.to_string(),
                v.replace("{log}", &dir.path().join("log").display().to_string()),
            );
        }
        vars.insert("FED_SLOT_CHECK".into(), "skip".into());
        RouteCase {
            dir,
            vars,
            before_org: BEFORE_ORG.into(),
            before_live,
            repo: real(),
        }
    }

    fn back_out(c: &RouteCase) -> Vec<String> {
        let var = |k: &str| c.vars.get(k).cloned();
        let core = driver_core(&c.repo, "xab", "zagros", &c.work(), var)
            .unwrap()
            .expect("the deployment's own route");
        let work = c.dir.path().join("backout");
        let rec = driver_survey(&core, "xab", &work.join("survey")).unwrap();
        let s = summarize(&[rec], "xab");
        assert!(s.refused.is_empty(), "{s:?}");
        let digest = s.core_candidate.clone().expect("a candidate to restart on");
        let ran = std::cell::RefCell::new(Vec::new());
        let mut exec = |_: &Path, cmd: &str| -> Result<bool> {
            ran.borrow_mut().push(cmd.to_string());
            // A play is recorded, not run: this checkout's plays reach no host here.
            if cmd.contains("ansible-preflight.sh") {
                return Ok(true);
            }
            Ok(std::process::Command::new("bash")
                .args(["-c", cmd])
                .status()?
                .success())
        };
        driver_apply(
            &c.repo,
            &core,
            "xab",
            &work.join("core-apply"),
            &digest,
            &mut exec,
        )
        .unwrap();
        // Run again: nothing of xab is left, nothing restarts.
        let again = driver_survey(&core, "xab", &work.join("survey-after")).unwrap();
        let left = summarize(&[again], "xab");
        assert!(
            left.core_candidate.is_none() && left.to_remove == 0,
            "{left:?}"
        );
        ran.into_inner()
    }

    #[test]
    fn the_back_out_follows_fed_core_acl_org_with_its_seed_and_restart() {
        let c = admitted(
            true,
            &[
                ("FED_CORE_RESTART", "echo restart >> {log}"),
                ("FED_CORE_DRY_RUN", "echo dry >> {log}"),
            ],
        );
        back_out(&c);
        assert_eq!(
            std::fs::read_to_string(&c.vars["FED_CORE_ACL_ORG"]).unwrap(),
            c.before_org
        );
        assert_eq!(
            std::fs::read_to_string(&c.vars["FED_CORE_ACL_OUT"]).unwrap(),
            c.before_live.clone().unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(c.dir.path().join("log")).unwrap(),
            "dry\nrestart\n"
        );
    }

    #[test]
    fn the_back_out_follows_fed_core_acl_org_alone_when_the_host_assembles() {
        // The role-core shape: a driver copy of the org fragment, carried to the
        // core and assembled there by FED_CORE_RESTART.
        let c = admitted(
            false,
            &[
                ("FED_CORE_RESTART", "echo restart >> {log}"),
                ("FED_CORE_DRY_RUN", "skip"),
            ],
        );
        back_out(&c);
        assert_eq!(
            std::fs::read_to_string(&c.vars["FED_CORE_ACL_ORG"]).unwrap(),
            c.before_org
        );
        assert_eq!(
            std::fs::read_to_string(c.dir.path().join("log")).unwrap(),
            "restart\n"
        );
    }

    #[test]
    fn the_back_out_follows_fed_core_play_on_fed_core_group() {
        let c = admitted(
            true,
            &[
                ("FED_CORE_PLAY", "core-deploy.yml"),
                ("FED_CORE_GROUP", "core_x"),
            ],
        );
        let ran = back_out(&c);
        assert_eq!(
            std::fs::read_to_string(&c.vars["FED_CORE_ACL_ORG"]).unwrap(),
            c.before_org
        );
        let plays: Vec<&String> = ran
            .iter()
            .filter(|r| r.contains("core-deploy.yml"))
            .collect();
        assert_eq!(plays.len(), 2, "a dry run and the deploy: {ran:?}");
        assert!(
            plays[0].contains("--check") && plays.iter().all(|p| p.contains("--limit 'core_x'")),
            "{plays:?}"
        );
    }

    #[test]
    fn the_router_leaf_admission_placed_goes_back_to_what_was_there() {
        // Nothing there before: the leaf and its key are removed.
        let mut c = admitted(
            true,
            &[("FED_CORE_RESTART", "true"), ("FED_CORE_DRY_RUN", "skip")],
        );
        place_leaf(&mut c, None);
        back_out(&c);
        assert!(!Path::new(&c.vars["FED_CORE_CERT"]).exists());
        assert!(
            !Path::new(&c.vars["FED_CORE_KEY"]).exists(),
            "no private key is left behind"
        );
        // Another leaf there before: its bytes come back.
        let mut c = admitted(
            true,
            &[("FED_CORE_RESTART", "true"), ("FED_CORE_DRY_RUN", "skip")],
        );
        place_leaf(&mut c, Some(b"THE-LEAF-BEFORE"));
        back_out(&c);
        assert_eq!(
            std::fs::read(&c.vars["FED_CORE_CERT"]).unwrap(),
            b"THE-LEAF-BEFORE"
        );
        assert_eq!(
            std::fs::read(&c.vars["FED_CORE_KEY"]).unwrap(),
            b"THE-LEAF-BEFORE"
        );
    }

    #[test]
    fn the_leafs_backup_is_found_in_whichever_admission_run_kept_it() {
        // An earlier admission run stopped after the org fragment, before the
        // leaf: its backup folder holds no leaf. The leaf's own backup, the
        // core's previous leaf, is in the later run, and is what comes back.
        let mut c = admitted(
            true,
            &[("FED_CORE_RESTART", "true"), ("FED_CORE_DRY_RUN", "skip")],
        );
        let early = c.work().join("admission/run-20261111T000000Z-1/backup");
        std::fs::create_dir_all(&early).unwrap();
        std::fs::write(early.join("0-org.json5"), BEFORE_ORG).unwrap();
        place_leaf(&mut c, Some(b"THE-LEAF-BEFORE"));
        back_out(&c);
        assert_eq!(
            std::fs::read(&c.vars["FED_CORE_CERT"]).unwrap(),
            b"THE-LEAF-BEFORE"
        );
        assert_eq!(
            std::fs::read(&c.vars["FED_CORE_KEY"]).unwrap(),
            b"THE-LEAF-BEFORE"
        );
        // A later admission's backup of this peering's own leaf is not what was
        // there before: the first run placed it where nothing was, so it goes.
        let mut c = admitted(
            true,
            &[("FED_CORE_RESTART", "true"), ("FED_CORE_DRY_RUN", "skip")],
        );
        place_leaf(&mut c, None);
        let mut backups = Vec::new();
        let again = c.work().join("admission/run-20261113T000000Z-1/backup");
        super::super::admit::publish(
            b"CERT-OF-XAB",
            Path::new(&c.vars["FED_CORE_CERT"]),
            &mut backups,
            &again,
        )
        .unwrap();
        back_out(&c);
        assert!(!Path::new(&c.vars["FED_CORE_CERT"]).exists());
    }

    #[test]
    fn a_leaf_renewed_after_it_was_placed_where_nothing_was_is_removed() {
        // The first admission placed the leaf where nothing was. Phase 8 then
        // renewed it, and a later admission placed the new leaf, keeping the
        // first one as its backup. That backup is this peering's own old leaf,
        // not what was there before: the earliest admission that touched the
        // path decides, and it found nothing, so the leaf and its key go.
        let mut c = admitted(
            true,
            &[("FED_CORE_RESTART", "true"), ("FED_CORE_DRY_RUN", "skip")],
        );
        place_leaf(&mut c, None);
        let pki = c.work().join("pki");
        std::fs::write(pki.join("core.pem"), b"CERT-OF-XAB-2").unwrap();
        std::fs::write(pki.join("core-key.pem"), b"KEY-OF-XAB-2").unwrap();
        let mut backups = Vec::new();
        let again = c.work().join("admission/run-20261113T000000Z-1/backup");
        for (bytes, var) in [
            (&b"CERT-OF-XAB-2"[..], "FED_CORE_CERT"),
            (&b"KEY-OF-XAB-2"[..], "FED_CORE_KEY"),
        ] {
            super::super::admit::publish(bytes, Path::new(&c.vars[var]), &mut backups, &again)
                .unwrap();
        }
        back_out(&c);
        assert!(!Path::new(&c.vars["FED_CORE_CERT"]).exists());
        assert!(
            !Path::new(&c.vars["FED_CORE_KEY"]).exists(),
            "no private key is left behind"
        );
    }

    #[test]
    fn a_leaf_another_peering_uses_is_left_in_place() {
        let mut c = admitted(
            true,
            &[("FED_CORE_RESTART", "true"), ("FED_CORE_DRY_RUN", "skip")],
        );
        place_leaf(&mut c, None);
        // Peering xac minted the same leaf: it is theirs too.
        let other = c.dir.path().join("peerings/xac/pki");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("core.pem"), b"CERT-OF-XAB").unwrap();
        std::fs::write(other.join("core-key.pem"), b"KEY-OF-XAB").unwrap();
        back_out(&c);
        assert_eq!(
            std::fs::read(&c.vars["FED_CORE_CERT"]).unwrap(),
            b"CERT-OF-XAB"
        );
        // And a leaf that is no longer ours (another admission since) is left.
        let mut c = admitted(
            true,
            &[("FED_CORE_RESTART", "true"), ("FED_CORE_DRY_RUN", "skip")],
        );
        place_leaf(&mut c, None);
        std::fs::write(&c.vars["FED_CORE_CERT"], b"A-LATER-LEAF").unwrap();
        back_out(&c);
        assert_eq!(
            std::fs::read(&c.vars["FED_CORE_CERT"]).unwrap(),
            b"A-LATER-LEAF"
        );
    }

    #[test]
    fn fed_core_acl_org_alone_on_a_role_core_is_restarted_by_the_roles_play() {
        // Admission takes the role's own play whenever FED_CORE_PLAY and
        // FED_CORE_RESTART are unset; the back-out decides the same way.
        let mut c = admitted(false, &[]);
        let role = role_core_checkout();
        let tool = role.path().join("ops/federation/standup/backout_core.py");
        std::fs::create_dir_all(tool.parent().unwrap()).unwrap();
        std::fs::copy(real().join("ops/federation/standup/backout_core.py"), &tool).unwrap();
        c.repo = role.path().to_path_buf();
        let ran = back_out(&c);
        assert_eq!(
            std::fs::read_to_string(&c.vars["FED_CORE_ACL_ORG"]).unwrap(),
            c.before_org
        );
        let plays: Vec<&String> = ran
            .iter()
            .filter(|r| r.contains("zenoh-deploy.yml"))
            .collect();
        assert_eq!(plays.len(), 2, "the role's dry run and deploy: {ran:?}");
        assert!(plays[0].contains("--check"), "{plays:?}");
    }

    #[test]
    fn fed_core_play_is_run_with_the_peerings_working_folder() {
        let c = admitted(
            true,
            &[
                ("FED_CORE_PLAY", "core-deploy.yml"),
                ("FED_CORE_GROUP", "core_x"),
            ],
        );
        let ran = back_out(&c);
        let want = format!("-e fed_work='{}'", c.work().display());
        assert!(
            ran.iter()
                .filter(|r| r.contains("core-deploy.yml"))
                .all(|r| r.contains(&want)),
            "{ran:?}"
        );
    }

    #[test]
    fn without_a_route_of_its_own_the_role_core_is_the_plays_to_survey() {
        assert!(driver_core(
            &real(),
            "xab",
            "zagros",
            Path::new("/nonexistent/w"),
            |_| None
        )
        .unwrap()
        .is_none());
        // A route named but incomplete is refused, naming what is missing.
        let e = driver_core(&real(), "xab", "zagros", Path::new("/nonexistent/w"), |k| {
            (k == "FED_CORE_RESTART").then(|| "x".into())
        })
        .err()
        .unwrap();
        assert!(e.to_string().contains("FED_CORE_ACL_ORG"), "{e}");
    }

    /// The back-out reads the core route the run holds (runenv), not only the
    /// process environment: a route recorded for this run on this thread, with
    /// none in the process environment, is refused (GB-221) by the entry itself,
    /// before any host is read.
    #[test]
    fn a_core_route_held_for_this_run_is_refused_by_the_back_out() {
        if std::env::var_os("FED_CORE_PLAY").is_some() {
            return; // the process environment already names one
        }
        let d = tempfile::tempdir().unwrap();
        let (repo, work) = stand_in(d.path());
        let ran = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut exec = fake_hosts(work.clone(), ran.clone());
        let o = Options {
            partner: "hello-ally",
            env: None,
            r#as: None,
            edge_group: None,
            work: Some(&work),
            mock: None,
            dry_run: true,
            yes: true,
            table: false,
        };
        crate::federate::runenv::set("FED_CORE_PLAY", "goat-zenoh-core-by-hand.yml");
        let got = backout_with(&repo, &o, &mut exec);
        crate::federate::runenv::unset("FED_CORE_PLAY");
        let e = got.expect_err("a back-out with a hand-set core route is refused");
        assert_eq!(
            e.downcast_ref::<goat_errors::Coded>()
                .map(|c| c.code.as_str()),
            Some("GB-221"),
            "{e}"
        );
        assert!(e.to_string().contains("FED_CORE_PLAY"), "{e}");
        assert!(
            ran.borrow().is_empty(),
            "no host was read: {:?}",
            ran.borrow()
        );
    }

    #[test]
    fn records_are_read_from_the_stage_directory() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("edge-xac.json"), r#"{"schema":"goat-federation-backout-host/v1","host":"edge-xac","peering":"xac","stage":"survey","rows":[{"component":"tunnel","resource":"interface wg-fed1","state":"absent","action":"none","note":"ours"}]}"#).unwrap();
        std::fs::write(d.path().join("ignore.txt"), "x").unwrap();
        let r = read_records(d.path()).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].rows[0].resource, "interface wg-fed1");
        assert!(read_records(&d.path().join("missing")).unwrap().is_empty());
    }
}
