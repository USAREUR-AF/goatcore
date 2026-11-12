//! `goatbuilder federate status [<partner>]`: every declared peering, read
//! from its run journal and its evidence. Read-only: it writes nothing,
//! creates nothing and takes no lock.
//!
//! For each peering: the last run and how it ended, the furthest phase
//! recorded and each phase's last outcome, the confirmations given, the
//! resources it holds on shared hosts, the digest of the agreement in its
//! working folder, and the last acceptance evidence
//! (`<work>/evidence/acceptance/result.json`, schema
//! `goat-federation-acceptance/v1`) with its verdict, age, scope (counts by
//! outcome word) and whether it was produced against the agreement that is
//! there now. A table for a person; `-o json` or `-o yaml` is
//! `goatbuilder.federate.status/v1`.
//!
//! A run with no end in the journal is `in progress` when the process id in
//! its `.lock` file is alive, and `killed` when it is not. (A process id can
//! be reused, so `in progress` can be wrong after a reboot; a lock is never
//! taken to find out, because a status read must never make a starting run
//! refuse.)
//!
//! Live health of the deployed edge is not measured here; the acceptance
//! evidence and its age are what this reports.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::lock::PEERING_LOCK;
use super::state::{Outcome, State};
use crate::config;
use crate::{Report, EXIT_OK};

pub const SCHEMA: &str = "goatbuilder.federate.status/v1";
const DEFAULT_ROOT: &str = "/var/lib/goat-federation";

#[derive(Serialize, Debug)]
pub struct LastRun {
    pub id: String,
    pub stage: String,
    pub started: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ended: String,
    /// `passed`, `failed`, `stopped`, `in progress`, `killed`, or
    /// `unknown: <why>` when its lock file cannot be read.
    pub outcome: String,
    pub from: u32,
    pub to: u32,
    pub resume: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub plan: String,
}

/// What a phase's outcome word this goatbuilder does not know is shown with.
pub const LATER_NOTE: &str = "written by a later goatbuilder";

#[derive(Serialize, Debug)]
pub struct PhaseLine {
    pub phase: u32,
    /// The word as the journal holds it.
    pub outcome: String,
    /// Set when this goatbuilder does not know the word: a later goatbuilder
    /// wrote it, and it is shown as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<&'static str>,
    pub at: String,
    pub run: String,
    pub attempts: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

#[derive(Serialize, Debug)]
pub struct Held {
    pub kind: String,
    pub scope: String,
    pub value: String,
    pub status: String,
}

#[derive(Serialize, Debug)]
pub struct Acceptance {
    /// PASS, FAIL or INCOMPLETE as acceptance decided it; `unreadable: <why>`
    /// when the file cannot be read or parsed; `unknown` when it names no
    /// verdict.
    pub verdict: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ended: String,
    /// Seconds between `ended` and now; absent when `ended` is unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_seconds: Option<i64>,
    /// The scope: how many checks ended in each outcome word.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub counts: BTreeMap<String, u64>,
    /// `current` (the agreement it names is the one in the working folder),
    /// `stale` (it was produced against another agreement), or `unknown`.
    pub agreement: String,
    pub path: String,
}

#[derive(Serialize, Debug)]
pub struct PeeringStatus {
    pub peering: String,
    pub environment: String,
    pub site: String,
    pub work: String,
    /// `none` (no journal), `ok`, or `unreadable: <reason>`.
    pub journal: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<LastRun>,
    pub phases: Vec<PhaseLine>,
    /// The kinds of confirmation recorded, with how each was given.
    pub confirmations: Vec<String>,
    pub owned: Vec<Held>,
    /// `sha256:` over `<work>/agreement.json`; empty when there is none;
    /// `unreadable: <why>` when it cannot be read.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub agreement_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<Acceptance>,
    /// The command that carries this peering on.
    pub next: String,
}

#[derive(Serialize, Debug)]
pub struct StatusDoc {
    pub schema: &'static str,
    pub peerings: Vec<PeeringStatus>,
}

/// `sha256:` over a file, the way acceptance names the agreement; empty when
/// it is absent.
fn agreement_digest(p: &Path) -> String {
    match std::fs::read(p) {
        Ok(b) => format!("sha256:{:x}", Sha256::digest(&b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => format!("unreadable: {e}"),
    }
}

fn str_at(v: &serde_json::Value, path: &[&str]) -> String {
    let mut cur = v;
    for k in path {
        match cur.get(*k) {
            Some(x) => cur = x,
            None => return String::new(),
        }
    }
    cur.as_str().map(String::from).unwrap_or_default()
}

/// The last acceptance evidence in `dir`, read leniently so a newer schema
/// still shows its verdict. Absent only when the file is absent.
fn acceptance(
    dir: &Path,
    agreement: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<Acceptance> {
    let p = dir.join("result.json");
    let path = p.display().to_string();
    let unreadable = |why: String| Acceptance {
        verdict: format!("unreadable: {why}"),
        ended: String::new(),
        age_seconds: None,
        counts: BTreeMap::new(),
        agreement: "unknown".into(),
        path: path.clone(),
    };
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => return Some(unreadable(e.to_string())),
    };
    let v: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return Some(unreadable(e.to_string())),
    };
    let mut verdict = str_at(&v, &["verdict"]);
    if verdict.is_empty() {
        verdict = "unknown".into();
    }
    let ended = [
        str_at(&v, &["run", "ended_at"]),
        str_at(&v, &["run", "started_at"]),
    ]
    .into_iter()
    .find(|s| !s.is_empty())
    .unwrap_or_default();
    let age_seconds = chrono::DateTime::parse_from_rfc3339(&ended)
        .ok()
        .map(|t| (now - t.with_timezone(&chrono::Utc)).num_seconds());
    let counts = v
        .get("counts")
        .and_then(|c| c.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, n)| n.as_u64().map(|n| (k.clone(), n)))
                .collect()
        })
        .unwrap_or_default();
    let named = str_at(&v, &["agreement", "sha256"]);
    let agreement = if named.is_empty() || !agreement.starts_with("sha256:") {
        "unknown"
    } else if named == agreement {
        "current"
    } else {
        "stale"
    };
    Some(Acceptance {
        verdict,
        ended,
        age_seconds,
        counts,
        agreement: agreement.into(),
        path,
    })
}

/// How a run with no end reads: by the process id in its `.lock` file.
fn unended(work: &Path) -> String {
    let p = work.join(PEERING_LOCK);
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return "killed".into(),
        Err(e) => return format!("unknown: cannot read {} ({e})", p.display()),
    };
    let Some(pid) = text
        .trim()
        .strip_prefix("pid ")
        .and_then(|n| n.trim().parse::<u32>().ok())
    else {
        return format!("unknown: {} names no process", p.display());
    };
    if pid_alive(pid) {
        "in progress".into()
    } else {
        "killed".into()
    }
}

fn pid_alive(pid: u32) -> bool {
    if Path::new("/proc/self").exists() {
        return Path::new(&format!("/proc/{pid}")).exists();
    }
    crate::federate::runenv::command("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Where a peering's working folder and evidence are, as status reads them.
pub struct Folders {
    pub work: PathBuf,
    pub evidence: PathBuf,
}

/// One peering's status.
pub fn peering(with: &str, environment: &str, site: &str, f: &Folders) -> PeeringStatus {
    peering_at(with, environment, site, f, chrono::Utc::now())
}

fn peering_at(
    with: &str,
    environment: &str,
    site: &str,
    f: &Folders,
    now: chrono::DateTime<chrono::Utc>,
) -> PeeringStatus {
    let work = &f.work;
    let (state, journal) = match State::read(work) {
        Ok(Some(st)) if st.peering == with => (Some(st), "ok".to_string()),
        Ok(Some(st)) => (
            None,
            format!(
                "unreadable: the journal is for peering '{}' (GB-182)",
                st.peering
            ),
        ),
        Ok(None) => (None, "none".to_string()),
        Err(e) => (None, format!("unreadable: {e} (GB-182)")),
    };
    let agreement = agreement_digest(&work.join("agreement.json"));
    let last_run = state
        .as_ref()
        .and_then(|st| st.runs.last())
        .map(|r| LastRun {
            id: r.id.clone(),
            stage: r.stage.clone(),
            started: r.started.clone(),
            ended: r.ended.clone(),
            outcome: if r.ended.is_empty() {
                unended(work)
            } else {
                r.outcome.clone()
            },
            from: r.from,
            to: r.to,
            resume: r.resume,
            plan: r.plan.clone(),
        });
    let phases: Vec<PhaseLine> = state
        .as_ref()
        .map(|st| {
            st.phases
                .iter()
                .map(|(n, r)| PhaseLine {
                    phase: *n,
                    outcome: r.outcome.word().into(),
                    note: (!r.outcome.known()).then_some(LATER_NOTE),
                    at: r.at.clone(),
                    run: r.run.clone(),
                    attempts: r.attempts,
                    detail: r.detail.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let confirmations = state
        .as_ref()
        .map(|st| {
            st.confirmations
                .iter()
                .map(|c| format!("{} at phase {} ({}, {})", c.kind, c.phase, c.how, c.at))
                .collect()
        })
        .unwrap_or_default();
    let owned = state
        .as_ref()
        .map(|st| {
            st.owned
                .iter()
                .filter(|o| o.held())
                .map(|o| Held {
                    kind: o.kind.clone(),
                    scope: o.scope.clone(),
                    value: o.value.clone(),
                    status: o.status.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let acceptance = acceptance(&f.evidence, &agreement, now);
    let again = again(with, work);
    let next = if journal.starts_with("unreadable") {
        format!("move {} aside (or use the goatbuilder that wrote it), or give each peering its own --work; then: {again}", State::path(work).display())
    } else {
        next_step(
            &again,
            state.as_ref(),
            last_run.as_ref(),
            acceptance.as_ref(),
            &f.evidence,
        )
    };
    PeeringStatus {
        peering: with.into(),
        environment: environment.into(),
        site: site.into(),
        work: work.display().to_string(),
        journal,
        last_run,
        phases,
        confirmations,
        owned,
        agreement_digest: agreement,
        acceptance,
        next,
    }
}

/// The command that runs this peering again, aimed at the same folder.
fn again(with: &str, work: &Path) -> String {
    if work == Path::new(DEFAULT_ROOT).join(with) {
        format!("goatbuilder federate {with}")
    } else {
        format!("goatbuilder federate {with} --work {}", work.display())
    }
}

/// The phase the last run ended on, with its record: the highest phase that
/// run recorded an outcome for, else the phase it last started.
fn ended_on(st: &State, run: &str) -> Option<(u32, Option<Outcome>)> {
    if let Some((n, r)) = st.phases.iter().rfind(|(_, r)| r.run == run) {
        return Some((*n, Some(r.outcome.clone())));
    }
    st.attempts
        .iter()
        .rev()
        .find(|a| a.run == run && a.event == "start" && a.phase > 0)
        .map(|a| (a.phase, None))
}

/// What carries the peering on, from the last run and the evidence.
fn next_step(
    again: &str,
    state: Option<&State>,
    last: Option<&LastRun>,
    acc: Option<&Acceptance>,
    evidence: &Path,
) -> String {
    let (Some(st), Some(last)) = (state, last) else {
        return again.to_string();
    };
    if last.outcome == "in progress" {
        return "wait: a run of this peering is in progress".into();
    }
    let on = ended_on(st, &last.id);
    match (last.outcome.as_str(), on.clone()) {
        ("stopped", Some((n, _))) => {
            return format!(
                "do the operator step of phase {n}, then: {again} --from {}",
                n + 1
            )
        }
        ("failed", Some((n, Some(Outcome::Failed)))) => {
            return format!("fix the cause, then: {again} --from {n}   (or: {again} --resume)")
        }
        ("killed", Some((n, _))) => {
            return format!("run it again: {again} --resume   (the run was killed in phase {n})")
        }
        ("failed" | "killed", _) => return again.to_string(),
        (o, _) if o.starts_with("unknown") => {
            return format!("check whether a run is still going ({o}); if not: {again} --resume")
        }
        _ => {}
    }
    let passed_14 = st
        .phases
        .get(&14)
        .is_some_and(|r| r.outcome == Outcome::Passed);
    if !passed_14 {
        let next = on.map(|(n, _)| n + 1).unwrap_or(1);
        return format!("{again} --from {next}");
    }
    match acc {
        Some(a) if a.verdict == "PASS" && a.agreement == "current" => "nothing: acceptance passed against the agreement in place".into(),
        Some(a) if a.verdict == "PASS" => format!("run acceptance again (the evidence is not tied to the agreement in place): {again} --from 14"),
        Some(_) => format!("fix what acceptance found (read {}), then: {again} --from 14", evidence.join("summary.md").display()),
        None => format!("{again} --from 14"),
    }
}

/// Where status reads a peering. With a partner named: the run's own rule,
/// `--work`, else `$FED_WORK`, else /var/lib/goat-federation/<partner>, and
/// evidence in `$FED_EVIDENCE_DIR` when set. Without one: each peering at
/// `$FED_WORK_ROOT/<partner>`, else /var/lib/goat-federation/<partner>.
pub fn folders_for(with: &str, named: bool, work: Option<&str>) -> Folders {
    let env = |k: &str| {
        crate::federate::runenv::var(k)
            .ok()
            .filter(|v| !v.is_empty())
    };
    let work = if named {
        work.map(PathBuf::from)
            .or_else(|| env("FED_WORK").map(PathBuf::from))
            .unwrap_or_else(|| Path::new(DEFAULT_ROOT).join(with))
    } else {
        env("FED_WORK_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_ROOT))
            .join(with)
    };
    let evidence = match env("FED_EVIDENCE_DIR").filter(|_| named) {
        Some(d) => PathBuf::from(d),
        None => work.join("evidence/acceptance"),
    };
    Folders { work, evidence }
}

fn short(d: &str) -> String {
    match d.strip_prefix("sha256:") {
        Some(h) if h.len() > 12 => format!("sha256:{}", &h[..12]),
        _ if d.starts_with("unreadable") => "unreadable".into(),
        _ => d.to_string(),
    }
}

fn age(secs: i64) -> String {
    match secs {
        s if s < 0 => "in the future".into(),
        s if s < 120 => format!("{s}s ago"),
        s if s < 7200 => format!("{}m ago", s / 60),
        s if s < 172_800 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

/// Phases as ranges by outcome: `1-9 passed, 10 stopped`.
fn phase_summary(phases: &[PhaseLine]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < phases.len() {
        let mut j = i;
        while j + 1 < phases.len()
            && phases[j + 1].outcome == phases[i].outcome
            && phases[j + 1].phase == phases[j].phase + 1
        {
            j += 1;
        }
        let range = if i == j {
            phases[i].phase.to_string()
        } else {
            format!("{}-{}", phases[i].phase, phases[j].phase)
        };
        out.push(format!(
            "{range} {}{}",
            phases[i].outcome,
            phases[i].note.map(|n| format!(" ({n})")).unwrap_or_default()
        ));
        i = j + 1;
    }
    out.join(", ")
}

fn line(label: &str, text: &str) -> String {
    format!("  {:<11}{text}\n", format!("{label}:"))
}

/// The page a person reads.
pub fn table(doc: &StatusDoc) -> String {
    if doc.peerings.is_empty() {
        return "No peering is declared. Declare one under peerings: in goatbuilder.yaml,\nthen run: goatbuilder federate <partner>\n".into();
    }
    let w = doc
        .peerings
        .iter()
        .map(|p| p.peering.chars().count())
        .max()
        .unwrap_or(7)
        .max(7);
    let e = doc
        .peerings
        .iter()
        .map(|p| p.environment.chars().count())
        .max()
        .unwrap_or(3)
        .max(3);
    let mut out = format!(
        "{:<w$}  {:<e$}  {:<12}  {:<20}  {}\n",
        "PEERING", "ENV", "LAST RUN", "AGREEMENT", "ACCEPTANCE"
    );
    for p in &doc.peerings {
        let run = match (&p.last_run, p.journal.as_str()) {
            (Some(r), _) if r.outcome.starts_with("unknown") => "unknown".into(),
            (Some(r), _) => r.outcome.clone(),
            (None, "none") => "not started".into(),
            (None, _) => "unreadable".into(),
        };
        let ag = if p.agreement_digest.is_empty() {
            "none".to_string()
        } else {
            short(&p.agreement_digest)
        };
        let acc = match &p.acceptance {
            None => "not run".to_string(),
            Some(a) if a.verdict.starts_with("unreadable") => "unreadable".to_string(),
            Some(a) => {
                let mut s = a.verdict.clone();
                if let Some(secs) = a.age_seconds {
                    s.push_str(&format!(" {}", age(secs)));
                }
                if a.agreement == "stale" {
                    s.push_str(" (stale)");
                }
                s
            }
        };
        out.push_str(&format!(
            "{:<w$}  {:<e$}  {:<12}  {:<20}  {}\n",
            p.peering, p.environment, run, ag, acc
        ));
    }
    for p in &doc.peerings {
        out.push_str(&format!("\n{}:\n", p.peering));
        out.push_str(&line("Work", &p.work));
        if p.journal.starts_with("unreadable") {
            out.push_str(&line("Journal", &p.journal));
        }
        if let Some(r) = &p.last_run {
            let resumed = if r.resume { ", resumed" } else { "" };
            out.push_str(&line(
                "Last run",
                &format!(
                    "{} phases {}-{}, started {}{resumed}: {}",
                    r.stage,
                    r.from,
                    r.to.min(14),
                    r.started,
                    r.outcome
                ),
            ));
        }
        if !p.phases.is_empty() {
            out.push_str(&line("Phases", &phase_summary(&p.phases)));
            let last_id = p.last_run.as_ref().map(|r| r.id.as_str()).unwrap_or("");
            if let Some(l) = p
                .phases
                .iter()
                .rev()
                .find(|l| l.run == last_id && l.outcome != "passed" && !l.detail.is_empty())
            {
                out.push_str(&line(&format!("Phase {}", l.phase), &l.detail));
            }
        }
        if p.agreement_digest.starts_with("unreadable") {
            out.push_str(&line("Agreement", &p.agreement_digest));
        }
        if !p.owned.is_empty() {
            let parts: Vec<String> = p
                .owned
                .iter()
                .map(|o| format!("{} {} on {} ({})", o.kind, o.value, o.scope, o.status))
                .collect();
            out.push_str(&line("Holds", &parts.join("; ")));
        }
        if let Some(a) = &p.acceptance {
            if a.verdict.starts_with("unreadable") {
                out.push_str(&line("Evidence", &format!("{}: {}", a.path, a.verdict)));
            }
            if !a.counts.is_empty() {
                let parts: Vec<String> = a
                    .counts
                    .iter()
                    .filter(|(_, n)| **n > 0)
                    .map(|(k, n)| format!("{k} {n}"))
                    .collect();
                out.push_str(&line("Checks", &parts.join(", ")));
            }
            if a.agreement == "stale" {
                out.push_str(&line(
                    "Evidence",
                    "produced against another agreement; run acceptance again",
                ));
            }
        }
        out.push_str(&line("Next", &p.next));
    }
    out
}

/// How status prints: a table, or the versioned document.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Table,
    Json,
    Yaml,
}

/// The status of every declared peering, or of one; `env` narrows to the
/// peerings declared on that environment.
pub fn status(
    cfg_path: &Path,
    partner: Option<&str>,
    env: Option<&str>,
    work: Option<&str>,
    format: Format,
) -> Result<Report> {
    let cfg = config::load(cfg_path)?;
    if partner.is_none() && work.is_some() {
        return Err(anyhow::anyhow!(
            "--work names one peering's folder: give the partner (goatbuilder federate status <partner> --work <dir>), or set FED_WORK_ROOT for every peering"
        ));
    }
    let on_env = |e: &str| env.map(|x| x == e).unwrap_or(true);
    if let Some(p) = partner {
        if !cfg
            .peerings
            .iter()
            .any(|x| x.with == p && on_env(&x.environment))
        {
            let declared: Vec<String> = cfg
                .peerings
                .iter()
                .map(|x| format!("{} on {}", x.with, x.environment))
                .collect();
            return Err(goat_errors::coded(
                "GB-185",
                format!(
                    "status: peering '{p}' is not declared in {} peerings: (declared: {})",
                    cfg_path.display(),
                    if declared.is_empty() {
                        "none".to_string()
                    } else {
                        declared.join(", ")
                    }
                ),
            )
            .into());
        }
    }
    let peerings = cfg
        .peerings
        .iter()
        .filter(|p| partner.map(|n| n == p.with).unwrap_or(true) && on_env(&p.environment))
        .map(|p| {
            peering(
                &p.with,
                &p.environment,
                &p.site,
                &folders_for(&p.with, partner.is_some(), work),
            )
        })
        .collect();
    let doc = StatusDoc {
        schema: SCHEMA,
        peerings,
    };
    match format {
        Format::Json => println!("{}", serde_json::to_string_pretty(&doc)?),
        Format::Yaml => print!("{}", serde_yaml::to_string(&doc)?),
        Format::Table => print!("{}", table(&doc)),
    }
    let mut r = Report::new("federate", Vec::new(), EXIT_OK);
    r.printed = true;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap as Map;

    fn at(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn folders(d: &Path) -> Folders {
        Folders {
            work: d.to_path_buf(),
            evidence: d.join("evidence/acceptance"),
        }
    }

    fn journal(d: &Path, phases: &[(u32, Outcome)], run_outcome: &str) {
        let mut st = State::load(d, "xab").unwrap();
        let run = format!("run-{}", st.runs.len());
        st.runs.push(super::super::state::RunEntry {
            id: run.clone(),
            stage: "federate".into(),
            started: super::super::state::now(),
            ended: String::new(),
            from: 1,
            to: 99,
            resume: false,
            plan: String::new(),
            outcome: String::new(),
            extra: Default::default(),
        });
        for (n, o) in phases {
            st.record(
                &run,
                *n,
                o.clone(),
                "i".into(),
                "o".into(),
                if *o == Outcome::Passed { "" } else { "why" },
            );
        }
        if !run_outcome.is_empty() {
            st.close_run(&run, run_outcome);
        }
        st.save(d).unwrap();
    }

    fn evidence(d: &Path, agreement_sha: &str, verdict: &str) {
        std::fs::create_dir_all(d.join("evidence/acceptance")).unwrap();
        std::fs::write(
            d.join("evidence/acceptance/result.json"),
            format!(
                r#"{{"schema":"goat-federation-acceptance/v1","run":{{"id":"r","started_at":"2026-11-09T10:00:00Z","ended_at":"2026-11-09T11:00:00Z"}},"agreement":{{"sha256":"{agreement_sha}"}},"counts":{{"PASS":12,"FAIL":0,"BLOCKED":0,"SKIP":0,"NOT APPLICABLE":2}},"verdict":"{verdict}"}}"#
            ),
        )
        .unwrap();
    }

    fn passed(range: std::ops::RangeInclusive<u32>) -> Vec<(u32, Outcome)> {
        range.map(|n| (n, Outcome::Passed)).collect()
    }

    #[test]
    fn a_word_a_later_goatbuilder_wrote_is_shown_as_written_and_named_so() {
        let d = tempfile::tempdir().unwrap();
        let mut ph = passed(1..=13);
        ph.push((14, Outcome::Later("deferred".into())));
        journal(d.path(), &ph, "deferred");
        let p = peering_at("xab", "dev", "alpha", &folders(d.path()), at("2026-11-09T14:00:00Z"));
        assert_eq!(p.phases.last().unwrap().outcome, "deferred");
        assert_eq!(p.phases.last().unwrap().note, Some(LATER_NOTE));
        let page = table(&StatusDoc { schema: SCHEMA, peerings: vec![p] });
        assert!(
            page.contains("  Phases:    1-13 passed, 14 deferred (written by a later goatbuilder)\n"),
            "{page}"
        );
    }

    #[test]
    fn a_peering_reads_its_journal_and_evidence() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("agreement.json"), b"{\"spec\":\"x\"}").unwrap();
        let mut ph = passed(1..=9);
        ph.push((10, Outcome::Stopped));
        journal(d.path(), &ph, "stopped");
        let mut st = State::load(d.path(), "xab").unwrap();
        st.confirm("r", 5, "operator-step", "flag", "sha256:i", Map::new());
        st.own("r", "udp-port", "edge-1", "51821", "claimed");
        st.save(d.path()).unwrap();
        evidence(
            d.path(),
            &agreement_digest(&d.path().join("agreement.json")),
            "PASS",
        );
        let p = peering_at(
            "xab",
            "dev",
            "alpha",
            &folders(d.path()),
            at("2026-11-09T14:00:00Z"),
        );
        assert_eq!(p.journal, "ok");
        assert_eq!(p.last_run.as_ref().unwrap().outcome, "stopped");
        let a = p.acceptance.as_ref().unwrap();
        assert_eq!(
            (a.verdict.as_str(), a.agreement.as_str(), a.age_seconds),
            ("PASS", "current", Some(3 * 3600))
        );
        assert!(
            p.next.contains("operator step of phase 10") && p.next.contains("--from 11"),
            "{}",
            p.next
        );
        assert!(
            p.next.contains(&format!("--work {}", d.path().display())),
            "aimed at the same folder: {}",
            p.next
        );
        let page = table(&StatusDoc {
            schema: SCHEMA,
            peerings: vec![p],
        });
        assert!(page.starts_with("PEERING  ENV  LAST RUN"), "{page}");
        assert!(page.contains("PASS 3h ago"), "{page}");
        assert!(
            page.contains("  Phases:    1-9 passed, 10 stopped\n"),
            "{page}"
        );
        assert!(page.contains("  Phase 10:  why\n"), "{page}");
        assert!(
            page.contains("  Holds:     udp-port 51821 on edge-1 (claimed)\n"),
            "{page}"
        );
        assert!(
            page.contains("  Checks:    NOT APPLICABLE 2, PASS 12\n"),
            "{page}"
        );
    }

    #[test]
    fn the_advice_follows_the_last_run_not_the_highest_phase() {
        let d = tempfile::tempdir().unwrap();
        let mut ph = passed(1..=4);
        ph.push((5, Outcome::Stopped));
        journal(d.path(), &ph, "stopped");
        // A later run fails at phase 2.
        journal(
            d.path(),
            &[(1, Outcome::Passed), (2, Outcome::Failed)],
            "failed",
        );
        let p = peering("xab", "dev", "alpha", &folders(d.path()));
        assert!(
            p.next.starts_with("fix the cause") && p.next.contains("--from 2"),
            "{}",
            p.next
        );
        let page = table(&StatusDoc {
            schema: SCHEMA,
            peerings: vec![p],
        });
        assert!(
            page.contains("  Phase 2:   why\n") && !page.contains("Phase 5:"),
            "{page}"
        );
    }

    #[test]
    fn the_advice_for_each_state_a_journal_can_be_in() {
        let next = |phases: &[(u32, Outcome)], outcome: &str, ev: Option<&str>| {
            let d = tempfile::tempdir().unwrap();
            std::fs::write(d.path().join("agreement.json"), b"{}").unwrap();
            journal(d.path(), phases, outcome);
            if let Some(v) = ev {
                evidence(
                    d.path(),
                    &agreement_digest(&d.path().join("agreement.json")),
                    v,
                );
            }
            peering("xab", "dev", "alpha", &folders(d.path())).next
        };
        let n = next(&[], "failed", None);
        assert!(
            n.starts_with("goatbuilder federate xab") && !n.contains("--from"),
            "failed before any phase: {n}"
        );
        assert!(next(&passed(1..=9), "passed", None).ends_with("--from 10"));
        assert!(next(&passed(1..=14), "passed", Some("PASS")).starts_with("nothing"));
        assert!(
            next(&passed(1..=14), "passed", Some("FAIL")).starts_with("fix what acceptance found")
        );
        assert!(next(&passed(1..=14), "passed", Some("INCOMPLETE"))
            .starts_with("fix what acceptance found"));
        assert!(next(&passed(1..=14), "passed", None).ends_with("--from 14"));
    }

    #[test]
    fn evidence_from_another_agreement_is_stale() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("agreement.json"), b"{}").unwrap();
        journal(d.path(), &passed(1..=14), "passed");
        evidence(d.path(), "sha256:0000", "PASS");
        let p = peering("xab", "dev", "alpha", &folders(d.path()));
        assert_eq!(p.acceptance.as_ref().unwrap().agreement, "stale");
        assert!(p.next.starts_with("run acceptance again"), "{}", p.next);
        let page = table(&StatusDoc {
            schema: SCHEMA,
            peerings: vec![p],
        });
        assert!(
            page.contains("(stale)") && page.contains("run acceptance again"),
            "{page}"
        );
    }

    #[test]
    fn a_run_with_no_end_is_killed_or_in_progress_by_its_process() {
        let d = tempfile::tempdir().unwrap();
        let mut st = State::new("xab");
        let run = st.open_run("federate", 1, 99, false, "");
        st.log(&run, 6, "start", "");
        st.save(d.path()).unwrap();
        let p = peering("xab", "dev", "alpha", &folders(d.path()));
        assert_eq!(p.last_run.unwrap().outcome, "killed", "no lock file");
        assert!(
            p.next.contains("--resume") && p.next.contains("killed in phase 6"),
            "{}",
            p.next
        );
        std::fs::write(
            d.path().join(".lock"),
            format!("pid {}\n", std::process::id()),
        )
        .unwrap();
        let p = peering("xab", "dev", "alpha", &folders(d.path()));
        assert_eq!(p.last_run.unwrap().outcome, "in progress");
        assert!(p.next.starts_with("wait"), "{}", p.next);
        std::fs::write(d.path().join(".lock"), "pid 4294967295\n").unwrap();
        assert_eq!(
            peering("xab", "dev", "alpha", &folders(d.path()))
                .last_run
                .unwrap()
                .outcome,
            "killed"
        );
    }

    #[test]
    fn a_damaged_journal_or_evidence_is_shown_not_hidden() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("state.json"), "{broken").unwrap();
        std::fs::create_dir_all(d.path().join("evidence/acceptance")).unwrap();
        std::fs::write(
            d.path().join("evidence/acceptance/result.json"),
            b"\xff\xfe{",
        )
        .unwrap();
        let p = peering("xab", "dev", "alpha", &folders(d.path()));
        assert!(
            p.journal.starts_with("unreadable") && p.journal.contains("GB-182"),
            "{}",
            p.journal
        );
        assert!(p
            .acceptance
            .as_ref()
            .unwrap()
            .verdict
            .starts_with("unreadable"));
        assert!(
            p.next.starts_with("move ") && p.next.contains("state.json aside"),
            "{}",
            p.next
        );
        assert_eq!(
            std::fs::read_to_string(d.path().join("state.json")).unwrap(),
            "{broken",
            "read-only"
        );
        assert!(!d.path().join(".lock").exists(), "creates nothing");
    }

    #[test]
    fn the_folders_follow_the_runs_own_rule() {
        assert_eq!(
            folders_for("xab", true, Some("/w")).work,
            PathBuf::from("/w")
        );
        if crate::federate::runenv::var_os("FED_EVIDENCE_DIR").is_none() {
            assert_eq!(
                folders_for("xab", true, Some("/w")).evidence,
                PathBuf::from("/w/evidence/acceptance")
            );
        }
        if crate::federate::runenv::var_os("FED_WORK").is_none()
            && crate::federate::runenv::var_os("FED_WORK_ROOT").is_none()
        {
            assert_eq!(
                folders_for("xab", true, None).work,
                PathBuf::from("/var/lib/goat-federation/xab")
            );
            assert_eq!(
                folders_for("xab", false, None).work,
                PathBuf::from("/var/lib/goat-federation/xab")
            );
        }
    }
}
