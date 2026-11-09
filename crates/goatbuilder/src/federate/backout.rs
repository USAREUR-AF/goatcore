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
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use crate::config;
use crate::{Check, Report, EXIT_NOT_CONVERGED, EXIT_OK, EXIT_REFUSED};

/// The subcommand word. `goatbuilder federate backout <partner>`; a partner can
/// never be named this (config.rs RESERVED_PEERING_NAMES).
pub const VERB: &str = "backout";

/// What `goatbuilder federate <subcommand>` takes: one of the six federation
/// verbs, as `goat federate` has them, or `backout`.
#[derive(clap::Subcommand, Debug)]
pub enum FederateSub {
    #[command(flatten)]
    Verb(goat_federation::verbs::FederateCmd),
    /// Back out ONE peering: survey its hosts, confirm, remove its tunnel,
    /// SPIRE federation, edge router, republisher, forwarder mapping and core
    /// admission, and leave every other peering and the core as they were.
    #[command(name = VERB)]
    Backout(BackoutArgs),
}

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

/// A path or group name that may sit inside single quotes in a `bash -c` line.
fn plain(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/:@+,~=-".contains(c))
}

fn env_or(name: &str, default: String) -> String {
    std::env::var(name)
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
    })
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
    for (k, v) in extra {
        e.push_str(&format!(" -e {k}='{v}'"));
    }
    format!(
        "cd '{r}/ops/ansible' && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws {aws} -- ../scripts/ansible-preflight.sh -i inventory/peering-edges.ini --limit '{}:{}:peering_forwarders' {e} peering-backout.yml",
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
            let generated = text.starts_with("# GENERATED by `goat federate apply`");
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
    match Command::new("python3")
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
    Ok(Command::new("bash")
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
    let t = target(o)?;
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
    let apply_cmd = play_cmd(repo, &t, &[("peering_backout_apply", "true".into())]);
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
    if !run_bash(repo, &survey_cmd)? {
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
    let recs = read_records(&survey_dir)?;
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
    let (slot_word, slot_ok, slot_said) =
        slot_recheck(repo, env.as_deref(), s.core_candidate.is_some());
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
    if !run_bash(repo, &apply_cmd)? {
        checks.push(row("remove", false, "the removal play failed partway (GB-225); run the back-out again: every step is idempotent".into(), Some(format!("goatbuilder federate backout {}", t.partner))));
        let mut r = Report::new("federate-backout", checks, EXIT_REFUSED);
        r.exit = EXIT_NOT_CONVERGED;
        r.printed = true;
        return Ok(r);
    }

    // 4 + 5 the core
    if let Some(digest) = &s.core_candidate {
        if !slot_ok {
            checks.push(row("slot-recheck", false, format!("{slot_word}: {slot_said} (GB-223)"), Some("regenerate and commit the slot list, then run the back-out again; the edge side is already removed".into())));
            exit = EXIT_NOT_CONVERGED;
        } else {
            checks.push(row(
                "slot-recheck",
                true,
                format!("{slot_word}: {slot_said}"),
                None,
            ));
            let core_cmd = play_cmd(
                repo,
                &t,
                &[
                    ("peering_backout_core_apply", "true".into()),
                    ("peering_backout_core_expect", digest.clone()),
                ],
            );
            checks.push(row("cmd", true, core_cmd.clone(), None));
            if !run_bash(repo, &core_cmd)? {
                checks.push(row("core", false, "the core router was not restarted on the reviewed candidate (GB-224): its configuration changed since the survey, or the restart failed".into(), Some(format!("goatbuilder federate backout {} --dry-run, review the new candidate, then run it again", t.partner))));
                exit = EXIT_NOT_CONVERGED;
            } else {
                checks.push(row(
                    "core",
                    true,
                    format!("core router restarted on {digest}"),
                    None,
                ));
            }
        }
    }

    // 6 directory
    if let Some((_, args)) = directory(repo, o, &t, false) {
        let out = Command::new("python3")
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
    if rec_ours == Some(true) {
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
    let after_ok = run_bash(repo, &survey_cmd)?;
    let after = read_records(&survey_dir).unwrap_or_default();
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
        if std::env::var("FED_SLOT_DB").is_err()
            && std::env::var("FED_SLOT_REGISTRY_STORE").is_err()
        {
            let (w, ok, said) = slot_recheck(d.path(), Some("dev"), true);
            assert_eq!((w.as_str(), ok), ("BLOCKED", false), "{said}");
        }
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
