//! `status` and `deploy` on a fixture (`--mock <dir>`): the first verbs that
//! exercise the converger interface end to end, and the clause tests of
//! goatbuilder-conformance.md that run on them (C1–C10). The real
//! convergers land behind the same calls, each behind the parity gate; until
//! then both verbs refuse a run without `--mock` and name the script that does
//! the job today.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use crate::config::{self, Config};
use crate::converger::{Converger, Outcome, Scripted};
use crate::{Check, Report, EXIT_NOT_CONVERGED, EXIT_REFUSED};

/// `declared/pins.yaml` in the fixture: component → environment → ref.
#[derive(Debug, Deserialize)]
struct Declared {
    /// The declaration's revision in its source of record (C3); absent is 0.
    #[serde(default)]
    revision: u64,
    components: BTreeMap<String, BTreeMap<String, Pin>>,
}

#[derive(Debug, Deserialize)]
struct Pin {
    #[serde(rename = "ref")]
    reference: String,
}

pub struct Fixture {
    pub root: PathBuf,
    pub config: Config,
    declared: Declared,
    pub converger: Scripted,
}

impl Fixture {
    pub fn open(root: &Path) -> Result<Self> {
        let config = config::load(&root.join("goatbuilder.yaml"))?;
        let mut converger = Scripted::open(root)?;
        converger.set_kinds(config.environments.iter().map(|e| (e.name.clone(), e.converger.clone())).collect());
        let p = root.join("declared/pins.yaml");
        let declared: Declared = serde_yaml::from_str(
            &std::fs::read_to_string(&p).with_context(|| format!("{}", p.display()))?,
        )
        .with_context(|| format!("{}", p.display()))?;
        Ok(Fixture { root: root.to_path_buf(), config, declared, converger })
    }

    /// C3: the local declaration against the fixture's source of record
    /// (`source/pins.yaml`, when the fixture has one). Behind: fast-forwarded,
    /// or refused under `--frozen`. Ahead or diverged: refused — a declaration
    /// that is not in the source of record is never applied.
    fn freshen(&mut self, frozen: bool) -> Result<Option<Check>> {
        let src = self.root.join("source/pins.yaml");
        let Ok(text) = std::fs::read_to_string(&src) else { return Ok(None) };
        let source: Declared = serde_yaml::from_str(&text).with_context(|| format!("{}", src.display()))?;
        let local = self.root.join("declared/pins.yaml");
        let (have, want) = (self.declared.revision, source.revision);
        if have == want {
            if std::fs::read_to_string(&local)? == text {
                return Ok(None);
            }
            return Err(goat_errors::coded("GB-046", format!("declared/pins.yaml and source/pins.yaml are both at revision {have} and differ; the local declaration is not the source of record's")).into());
        }
        if have > want {
            return Err(goat_errors::coded("GB-046", format!("declared/pins.yaml is at revision {have}, ahead of the source of record at {want}; a declaration that is not in the source of record is never applied")).into());
        }
        if frozen {
            return Err(goat_errors::coded("GB-046", format!("declared/pins.yaml is at revision {have}, behind the source of record at {want}, and --frozen refuses to fast-forward: cp source/pins.yaml declared/pins.yaml, or run without --frozen")).into());
        }
        std::fs::write(&local, &text).with_context(|| format!("{}", local.display()))?;
        self.declared = source;
        Ok(Some(ok("declaration/source", format!("fast-forwarded declared/pins.yaml from revision {have} to {want} (the source of record)"))))
    }

    fn environment(&self, env: &str) -> Result<()> {
        if self.config.environments.iter().any(|e| e.name == env) {
            Ok(())
        } else {
            Err(anyhow!(
                "environment '{env}' is not declared in {} (declared: {})",
                self.root.join("goatbuilder.yaml").display(),
                self.config.environments.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join(", ")
            ))
        }
    }

    /// The declared ref for a component on an environment (C1: digest-pinned).
    pub fn declared_ref(&self, env: &str, component: &str) -> Result<String> {
        if !self.config.components.contains_key(component) {
            return Err(goat_errors::coded("GB-001", format!("component '{component}' is not declared in goatbuilder.yaml")).into());
        }
        let r = self
            .declared
            .components
            .get(component)
            .and_then(|m| m.get(env))
            .map(|p| p.reference.clone())
            .ok_or_else(|| goat_errors::coded("GB-024", format!("{component} has no declaration for {env} in declared/pins.yaml")))?;
        Ok(r)
    }

    fn components_on(&self, env: &str, only: Option<&str>) -> Vec<String> {
        self.config
            .components
            .keys()
            .filter(|c| only.map(|o| o == c.as_str()).unwrap_or(true))
            .filter(|c| self.declared.components.get(*c).map(|m| m.contains_key(env)).unwrap_or(false))
            .cloned()
            .collect()
    }
}

/// C4: every declaration names who made it; refused before anything changes.
pub fn attributed(actor: &str) -> Result<()> {
    if actor.trim().is_empty() {
        return Err(goat_errors::coded("GB-047", "a declaration names who made it; pass --ack \"<your name>\" (C4)").into());
    }
    Ok(())
}

fn immutable(reference: &str) -> bool {
    reference.contains("@sha256:")
}

/// `status`: declared against recorded, per machine, reading only (C8).
pub fn status(fixture: &Fixture, env: &str, only: Option<&str>) -> Result<Report> {
    fixture.environment(env)?;
    let mut checks = Vec::new();
    // A sited fixture scopes each component to the class that carries it.
    let scope = crate::standup::class_scope(&fixture.root);
    for component in fixture.components_on(env, only) {
        let declared = fixture.declared_ref(env, &component)?;
        let machines: Vec<String> = match scope.as_ref().and_then(|s| s.get(&component)) {
            Some(m) => m.clone(),
            None => fixture.converger.machines(env),
        };
        for machine in machines {
            let name = format!("{env}/{machine}/{component}");
            match fixture.converger.read_record(env, &machine, &component) {
                Ok(Some(r)) if r.reference == declared => checks.push(ok(&name, format!("converged on {}", short(&declared)))),
                Ok(Some(r)) => checks.push(not_done(&name, format!("pending: record names {}, declared {}", short(&r.reference), short(&declared)))),
                Ok(None) => checks.push(not_done(&name, format!("pending: no record yet; declared {}", short(&declared)))),
                Err(e) => checks.push(not_done(&name, format!("unknown: {e}"))),
            }
        }
    }
    if checks.is_empty() {
        return Err(anyhow!("nothing declared for {env}{}", only.map(|o| format!(" and component {o}")).unwrap_or_default()));
    }
    Ok(Report::new("status", checks, EXIT_NOT_CONVERGED))
}

/// A row the machines table owns: one component on one machine, in a state the
/// verb defines, or the publish line that goes above the table. One predicate
/// answers for both the table and the steps the caller prints beside it, so no
/// row can fall out of both (output standard, rule 2: nothing is dropped).
pub(crate) fn machine_row(c: &Check) -> bool {
    let parts: Vec<&str> = c.name.split('/').collect();
    if parts.len() != 3 {
        return false;
    }
    if parts[2] == "publish" {
        return true;
    }
    // A machine that did not reach its state is a failure, and a failure reads
    // as what went wrong where, not as a row in a table of states.
    c.ok && ["converged on", "pending", "unknown", "same: ", "DRIFT"].iter().any(|w| c.detail.starts_with(w))
}

/// A ref as a person reads it: its tag, or the start of its digest if it has none.
fn first_word(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or(s)
}

pub(crate) fn version_of(reference: &str) -> String {
    let (image, digest) = reference.split_once('@').unwrap_or((reference, ""));
    let last = image.rsplit('/').next().unwrap_or(image);
    match last.split_once(':') {
        Some((_, tag)) if !tag.is_empty() => tag.to_string(),
        _ => digest.trim_end_matches('\u{2026}').chars().take(19).collect(),
    }
}

/// The table a person reads: one line per component on a machine, in the three
/// states the verb defines (docs/design/goatbuilder-output.md rule 3). Pending
/// and unknown are states, not failures, so neither prints as `FAIL`.
/// A deploy, verify or rollback whose rows are a publish and the machines that
/// converged: the same table `status` prints, since it answers the same question.
/// `None` for a run that reports anything else (a real run's preflight, transport
/// and playbook rows, or a refusal), which reads as its steps instead.
pub fn machines_page(report: &Report, color: bool) -> Option<String> {
    // The table counts the machines it holds ("2 of 2 converged"), and a machine
    // that failed is not one of them. So a run with any failure in it reads as
    // its steps instead, where a failure names what went wrong and where: a
    // headline that counts only the machines that worked is a headline that
    // lies by arithmetic.
    if report.checks.iter().any(|c| !c.ok) {
        return None;
    }
    let (mut rows, mut published, mut component) = (Vec::new(), None, String::new());
    for c in &report.checks {
        if !machine_row(c) {
            continue;
        }
        let parts: Vec<&str> = c.name.split('/').collect();
        if parts[2] == "publish" {
            component = parts[1].to_string();
            published = Some(c.detail.clone());
            continue;
        }
        rows.push(c.clone());
    }
    if rows.is_empty() {
        return None;
    }
    let env = rows[0].name.split('/').next().unwrap_or("").to_string();
    let title = if component.is_empty() { env.clone() } else { format!("{component} on {env}") };
    let verb = match report.verb {
        "rollback" => "Rollback:",
        "verify" => "Verify:",
        _ => "Deploy:",
    };
    let mut out = state_page(&Report::new(report.verb, rows, report.exit), verb, &title, ["converged", "pending", "unknown"], color);
    if let Some(p) = published {
        // "serial 1: <ref> by <who>" reads as one line above the table.
        let version = p.split_once(": ").map(|(_, r)| version_of(r.split(" by ").next().unwrap_or(r))).unwrap_or_default();
        let serial = p.split_whitespace().nth(1).unwrap_or("").trim_end_matches(':').to_string();
        let who = p.split(" by ").nth(1).unwrap_or("").to_string();
        let line = match (serial.is_empty(), who.is_empty()) {
            (false, false) => format!("Published {version} as record {serial}, by {who}."),
            (false, true) => format!("Published {version} as record {serial}."),
            _ => format!("Published {version}."),
        };
        out = format!("{}\n\n{}", out.trim_end(), line);
    }
    Some(out)
}

pub fn status_page(report: &Report, env: &str, color: bool) -> String {
    state_page(report, "Status:", env, ["converged", "pending", "unknown"], color)
}

/// `diff` as the same table: drift is a state, not a failure. `None` when a row
/// isn't one a machine and component can name: the plain table then prints it.
pub fn diff_page(report: &Report, color: bool) -> Option<String> {
    let known = |d: &str| d.starts_with("same: ") || d.starts_with("unknown: ") || d.starts_with("pending: ") || d.starts_with("not provisioned") || d.starts_with("no ledger coverage") || d.starts_with("DRIFT: image same") || (d.starts_with("DRIFT: running ") && d.contains(" \u{2014} main declares "));
    // A real run adds how it read the hosts and its own totals: the page counts them.
    let rows: Vec<Check> = report.checks.iter().filter(|c| !c.name.ends_with("/transport") && !c.name.ends_with("/diff")).cloned().collect();
    if rows.is_empty() || !rows.iter().all(|c| c.name.matches('/').count() == 2 && known(&c.detail)) {
        return None;
    }
    let env = rows[0].name.split('/').next().unwrap_or("").to_string();
    Some(state_page(&Report::new("diff", rows, report.exit), "Diff:", &env, ["same", "drift", "unknown"], color))
}

fn state_page(report: &Report, title: &str, env: &str, states: [&'static str; 3], color: bool) -> String {
    struct Row { component: String, machine: String, state: &'static str, running: String, declared: String }
    let rows: Vec<Row> = report
        .checks
        .iter()
        .map(|c| {
            let mut parts = c.name.splitn(3, '/');
            let _env = parts.next();
            let machine = parts.next().unwrap_or("").to_string();
            let component = parts.next().unwrap_or("").to_string();
            let d = c.detail.as_str();
            let (state, running, declared) = if let Some(rest) = d.strip_prefix("converged on ") {
                ("converged", version_of(rest), version_of(rest))
            } else if let Some(rest) = d.strip_prefix("same: ") {
                let v = version_of(first_word(rest));
                ("same", v.clone(), v)
            } else if let Some((run, dec)) = d.strip_prefix("DRIFT: running ").and_then(|r| r.split_once(" \u{2014} main declares ")) {
                // The same tag on two digests is a real difference: show the digests.
                let (a, b) = (version_of(run), version_of(first_word(dec)));
                if a == b {
                    let digest = |r: &str| r.split_once('@').map_or(r, |(_, d)| d).trim_end_matches('\u{2026}').to_string();
                    ("drift", digest(run), digest(first_word(dec)))
                } else {
                    ("drift", a, b)
                }
            } else if let Some(rest) = d.strip_prefix("DRIFT: image same (") {
                let (image, why) = rest.split_once("), but ").unwrap_or((rest, ""));
                ("drift", why.split(" (row ").next().unwrap_or(why).to_string(), version_of(image))
            } else if d.starts_with("not provisioned") || d.starts_with("no ledger coverage") {
                let dec = d.split_once("main declares ").map_or(String::new(), |(_, v)| version_of(first_word(v)));
                ("none", if d.starts_with("not") { "not provisioned" } else { "no ledger" }.to_string(), dec)
            } else if let Some((why, dec)) = d.strip_prefix("unknown: ").and_then(|r| r.split_once("; main declares ")) {
                let why = why.strip_prefix("host ").unwrap_or(why);
                let why = if why.starts_with("no ledger") { "no ledger" } else if why.starts_with("the shared ledger") { "other image" } else { why };
                ("unknown", why.to_string(), version_of(first_word(dec)))
            } else if let Some(rest) = d.strip_prefix("pending: record names ") {
                let (rec, dec) = rest.split_once(", declared ").unwrap_or((rest, ""));
                // The same tag on two digests is a real difference: show the digests.
                let (a, b) = (version_of(rec), version_of(dec));
                if a == b {
                    let digest = |r: &str| r.split_once('@').map_or(r, |(_, d)| d).trim_end_matches('\u{2026}').to_string();
                    ("pending", digest(rec), digest(dec))
                } else {
                    ("pending", a, b)
                }
            } else if let Some(rest) = d.strip_prefix("pending: no record yet; declared ") {
                ("pending", "none".to_string(), version_of(rest))
            } else {
                let why = d.strip_prefix("unknown: ").unwrap_or(d);
                let why = why.strip_prefix(&format!("{machine}: ")).unwrap_or(why);
                let why = if why == "record unreadable" { "unreadable" } else { why };
                ("unknown", why.to_string(), String::new())
            };
            Row { component, machine, state, running, declared }
        })
        .collect();
    // An unreadable machine is still declared: fill the column from its component.
    let declared_of = |component: &str| rows.iter().find(|r| r.component == component && !r.declared.is_empty()).map(|r| r.declared.clone()).unwrap_or_default();
    let declared: Vec<String> = rows.iter().map(|r| if r.declared.is_empty() { declared_of(&r.component) } else { r.declared.clone() }).collect();
    let paint = |code: &str, t: &str| if color { format!("\x1b[{code}m{t}\x1b[0m") } else { t.to_string() };
    let bold = |t: &str| paint("1", t);
    let count = |s: &str| rows.iter().filter(|r| r.state == s).count();
    let distinct = |f: &dyn Fn(&Row) -> &str| rows.iter().map(f).collect::<std::collections::BTreeSet<_>>().len();
    let (components, machines) = (distinct(&|r| &r.component), distinct(&|r| &r.machine));
    let plural = |n: usize, word: &str| format!("{} {word}{}", bold(&n.to_string()), if n == 1 { "" } else { "s" });
    // A row that is in none of the three states (a machine nothing has been
    // provisioned on, a component with no ledger coverage) is still a row: it
    // is named, so the numbers add up to the number of rows.
    let others = rows.len() - count(states[0]) - count(states[1]) - count(states[2]);
    let fourth = if others > 0 { format!(", {} in neither", bold(&others.to_string())) } else { String::new() };
    let mut out = format!(
        "{} {env}\n\n{} on {}. {} of {} {}, {} {}, {} {}{fourth}.\n\n",
        bold(title),
        plural(components, "component"),
        plural(machines, "machine"),
        bold(&count(states[0]).to_string()),
        bold(&rows.len().to_string()),
        states[0],
        bold(&count(states[1]).to_string()),
        states[1],
        bold(&count(states[2]).to_string()),
        states[2],
    );
    let w = |head: &str, f: &dyn Fn(&Row) -> usize| rows.iter().map(f).max().unwrap_or(0).max(head.len());
    let wc = w("COMPONENT", &|r| r.component.chars().count());
    let wm = w("MACHINE", &|r| r.machine.chars().count());
    let wr = w("RUNNING", &|r| r.running.chars().count());
    out.push_str(&bold(&format!("{:<wc$}  {:<wm$}  {:<9}  {:<wr$}  DECLARED", "COMPONENT", "MACHINE", "STATE", "RUNNING")));
    for (r, declared) in rows.iter().zip(&declared) {
        let code = match r.state {
            "converged" | "same" => "32",
            "pending" | "drift" => "33",
            _ => "31",
        };
        let state = paint(code, &format!("{:<9}", r.state));
        let line = format!("{:<wc$}  {:<wm$}  {state}  {:<wr$}  {declared}", r.component, r.machine, r.running);
        out.push('\n');
        out.push_str(line.trim_end());
    }
    // A table of machines that do not match, and nothing to do about it, is a
    // dead end. The next command is part of the answer.
    for line in next_steps(&rows.iter().map(|r| (r.state, r.component.as_str())).collect::<Vec<_>>(), states[0], env) {
        out.push_str(&format!("\n\n{}", crate::tutor::fill(&line, "", "  ").trim_end()));
    }
    out
}

/// What to run about rows that are not in the verb's first state: putting the
/// declared version onto them, and reading a machine that did not answer.
/// Empty when every row is in that state, since then there is nothing to do.
fn next_steps(rows: &[(&str, &str)], good: &str, env: &str) -> Vec<String> {
    // `none` is a machine nothing is declared to run on: nothing to deploy there.
    let off: Vec<&(&str, &str)> = rows.iter().filter(|(s, _)| *s != good && *s != "none").collect();
    if off.is_empty() {
        return Vec::new();
    }
    // Rows name a goatnet with hyphens; `--env` takes its key.
    let env = env.replace('-', "_");
    let mut out = Vec::new();
    let named: std::collections::BTreeSet<&str> = off.iter().filter(|(s, _)| *s != "unknown").map(|(_, c)| *c).filter(|c| !c.is_empty()).collect();
    match named.len() {
        0 => {}
        // A real deploy takes one component at a time.
        1 => out.push(format!("To put the version main declares onto them: goatbuilder deploy --env {env} --component {}", named.iter().next().unwrap())),
        _ => out.push(format!("To put the version main declares onto them, deploy each of {} with: goatbuilder deploy --env {env} --component <component>", named.iter().copied().collect::<Vec<_>>().join(", "))),
    }
    if off.iter().any(|(s, _)| *s == "unknown") {
        out.push(format!("A machine that did not answer is unknown, never converged. To find out why: goatbuilder doctor --env {env}"));
    }
    out
}

/// `deploy`: publish the declared ref, observe once, read every record back.
/// Done means every machine's record names the ref (C7); a second run over a
/// converged environment changes nothing (C5).
pub fn deploy(fixture: &mut Fixture, env: &str, only: Option<&str>, actor: &str, frozen: bool) -> Result<Report> {
    fixture.environment(env)?;
    attributed(actor)?;
    let mut checks: Vec<Check> = fixture.freshen(frozen)?.into_iter().collect();
    // C2 before anything is published: the log is intact and in order.
    fixture.converger.publications()?;
    let components = fixture.components_on(env, only);
    if components.is_empty() {
        return Err(anyhow!("nothing declared for {env}{}", only.map(|o| format!(" and component {o}")).unwrap_or_default()));
    }
    for component in components {
        let declared = fixture.declared_ref(env, &component)?;
        if !immutable(&declared) {
            checks.push(refused(
                &format!("{env}/{component}/declaration"),
                format!("{declared} is a mutable tag; a deploy declares a digest (C1)"),
                "pin the declaration to <image>@sha256:<digest> in declared/pins.yaml",
            ));
            continue;
        }
        let serial = fixture.converger.publish(env, &component, &declared, actor)?;
        checks.push(ok(&format!("{env}/{component}/publish"), format!("serial {serial}: {} by {actor}", short(&declared))));
        let observed = fixture.converger.observe(env, &component, &declared)?;
        for (machine, outcome) in observed {
            let name = format!("{env}/{machine}/{component}");
            // Done is what the RECORD says, never what the observation claimed (C7).
            match fixture.converger.read_record(env, &machine, &component) {
                Ok(Some(r)) if r.reference == declared => checks.push(ok(&name, format!("converged on {}", short(&declared)))),
                Ok(_) => checks.push(not_done(&name, format!("pending ({})", match outcome { Outcome::Pending => "not yet converged", Outcome::Converged => "observation says converged but the record does not", Outcome::Unknown => "unknown" }))),
                Err(e) => checks.push(not_done(&name, format!("unknown: {e}"))),
            }
        }
    }
    let exit = if checks.iter().any(|c| c.fix.is_some()) { EXIT_REFUSED } else { EXIT_NOT_CONVERGED };
    Ok(Report::new("deploy", checks, exit))
}

/// `deploy --dry-run` on a fixture: what a deploy would publish, and to which
/// machines, with nothing published and no record written.
pub fn deploy_plan(fixture: &Fixture, env: &str, only: Option<&str>) -> Result<Report> {
    fixture.environment(env)?;
    let components = fixture.components_on(env, only);
    if components.is_empty() {
        return Err(anyhow!("nothing declared for {env}{}", only.map(|o| format!(" and component {o}")).unwrap_or_default()));
    }
    let machines = fixture.converger.machines(env);
    let mut checks = Vec::new();
    for component in components {
        let declared = fixture.declared_ref(env, &component)?;
        if !immutable(&declared) {
            checks.push(refused(
                &format!("{env}/{component}/declaration"),
                format!("{declared} is a mutable tag; a deploy declares a digest (C1)"),
                "pin the declaration to <image>@sha256:<digest> in declared/pins.yaml",
            ));
            continue;
        }
        checks.push(ok(
            &format!("{env}/{component}/plan"),
            format!("DRY: would publish {} as serial {} and observe {} machine(s): {}", short(&declared), fixture.converger.last_serial() + 1, machines.len(), machines.join(" ")),
        ));
    }
    let exit = if checks.iter().any(|c| c.fix.is_some()) { EXIT_REFUSED } else { crate::EXIT_OK };
    let mut r = Report::new("deploy", checks, exit);
    r.exit = exit;
    r.ok = exit == crate::EXIT_OK;
    Ok(r)
}

/// `declared/artifacts.yaml` in a fixture: what a component IS, which decides
/// whether a rollback is safe (ADR 1060). Absent means `image`.
#[derive(Debug, Deserialize, Default)]
struct Artifact {
    #[serde(default)]
    artifact_type: Option<String>,
    #[serde(default)]
    reversible: Option<bool>,
}

/// Exit for a rollback refused as unsafe, the same code `rollback.sh` and the
/// host path use (data, cert, forward-only, reversible: false).
pub const EXIT_REFUSE_UNSAFE: i32 = 3;

/// `rollback` on a fixture (contract C9): roll a component back N steps by
/// DECLARING the earlier ref — a new declaration that travels the same path as
/// any deploy (publish, converge, verify every record) — never by reaching
/// into a machine. The timeline is the records' history then the publications,
/// newest first, each ref once. A data, cert or forward-only artifact, or one
/// marked `reversible: false`, is refused with exit 3 before anything changes.
pub fn rollback(fixture: &mut Fixture, env: &str, component: &str, n: u32, actor: &str, dry_run: bool) -> Result<Report> {
    fixture.environment(env)?;
    if !dry_run {
        attributed(actor)?;
    }
    if !(1..=7).contains(&n) {
        return Err(anyhow!("N must be 1..7 (got: {n})"));
    }
    let current = fixture.declared_ref(env, component)?;
    let artifacts: BTreeMap<String, Artifact> = match std::fs::read_to_string(fixture.root.join("declared/artifacts.yaml")) {
        Ok(t) => serde_yaml::from_str(&t).context("declared/artifacts.yaml")?,
        Err(_) => BTreeMap::new(),
    };
    let a = artifacts.get(component);
    let kind = a.and_then(|a| a.artifact_type.clone()).unwrap_or_else(|| "image".into());
    let reversible = a.and_then(|a| a.reversible).unwrap_or(true);
    // `reversible` decides only for an image; data, cert and forward-only are
    // refused whatever it says, so printing it there contradicted the refusal.
    let what = if kind == "image" { format!("artifact_type=image reversible={reversible}") } else { format!("artifact_type={kind}") };
    let mut checks = vec![ok(&format!("{env}/{component}/artifact"), format!("{what} (declared/artifacts.yaml)"))];
    let unsafe_reason = match kind.as_str() {
        "data" => Some(format!("'{component}' is artifact_type=data. A ref swap cannot undo committed data; restore from its backup instead.")),
        "cert" => Some(format!("'{component}' is artifact_type=cert. Certificates are re-minted, not reverted.")),
        "forward-only" => Some(format!("'{component}' is artifact_type=forward-only — not reversible.")),
        _ if !reversible => Some(format!("'{component}' is marked reversible: false.")),
        _ => None,
    };
    if let Some(why) = unsafe_reason {
        checks.push(Check { name: format!("{env}/{component}/rollback"), ok: false, detail: format!("REFUSING: {why}"), fix: None });
        let mut r = Report::new("rollback", checks, EXIT_REFUSE_UNSAFE);
        r.exit = EXIT_REFUSE_UNSAFE;
        r.ok = false;
        return Ok(r);
    }
    // The timeline, oldest first: every record line for this component on this
    // environment, then every publication of it; then newest first, each ref once.
    let mut timeline: Vec<String> = Vec::new();
    for machine in fixture.converger.machines(env) {
        let path = fixture.root.join("records").join(env).join(format!("{machine}.jsonl"));
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.lines() {
                if let Ok(r) = serde_json::from_str::<crate::converger::Record>(line) {
                    if r.component == component && !timeline.contains(&r.reference) {
                        timeline.push(r.reference);
                    }
                }
            }
        }
    }
    if let Ok(text) = std::fs::read_to_string(fixture.root.join("declared/publications.jsonl")) {
        for line in text.lines() {
            if let Ok(p) = serde_json::from_str::<crate::converger::Publication>(line) {
                if p.env == env && p.component == component {
                    timeline.retain(|r| r != &p.reference);
                    timeline.push(p.reference);
                }
            }
        }
    }
    timeline.retain(|r| r != &current);
    timeline.push(current.clone());
    let newest_first: Vec<String> = timeline.into_iter().rev().collect();
    let Some(target) = newest_first.get(n as usize).cloned() else {
        checks.push(Check {
            name: format!("{env}/{component}/rollback"),
            ok: false,
            detail: format!("only {} earlier ref(s) in the history; cannot go back {n}", newest_first.len().saturating_sub(1)),
            fix: Some(format!("goatbuilder --mock <fixture> rollback {component} 1 --env {env} --dry-run")),
        });
        return Ok(Report::new("rollback", checks, EXIT_REFUSED));
    };
    checks.push(ok(&format!("{env}/{component}/history"), format!("newest first: {}", newest_first.iter().map(|r| short(r)).collect::<Vec<_>>().join(" ← "))));
    if dry_run {
        checks.push(ok(&format!("{env}/{component}/rollback"), format!("DRY: would declare {} (back {n} from {}) and converge it like any deploy", short(&target), short(&current))));
        let mut r = Report::new("rollback", checks, crate::EXIT_OK);
        r.exit = crate::EXIT_OK;
        r.ok = true;
        return Ok(r);
    }
    // A new declaration at the earlier ref (C9), in the source of record: the
    // fixture's source/ when it has one (C3, a new revision), and the local copy.
    fixture.freshen(false)?;
    let pins = fixture.root.join("declared/pins.yaml");
    let mut doc: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&pins)?)?;
    doc["components"][component][env]["ref"] = serde_yaml::Value::String(target.clone());
    let source = fixture.root.join("source/pins.yaml");
    if source.exists() {
        doc["revision"] = serde_yaml::Value::from(fixture.declared.revision + 1);
        std::fs::write(&source, serde_yaml::to_string(&doc)?)?;
    }
    std::fs::write(&pins, serde_yaml::to_string(&doc)?)?;
    *fixture = Fixture::open(&fixture.root.clone())?;
    checks.push(ok(&format!("{env}/{component}/declare"), format!("declared {} in declared/pins.yaml (was {})", short(&target), short(&current))));
    let mut r = deploy(fixture, env, Some(component), actor, false)?;
    checks.append(&mut r.checks);
    let converged = checks.iter().all(|c| c.ok);
    let exit = if converged { crate::EXIT_OK } else { EXIT_NOT_CONVERGED };
    let mut out = Report::new("rollback", checks, exit);
    out.exit = exit;
    out.ok = converged;
    Ok(out)
}

pub fn short(reference: &str) -> String {
    match reference.find("@sha256:") {
        Some(i) => format!("{}@sha256:{}…", &reference[..i], &reference[i + 8..(i + 20).min(reference.len())]),
        None => reference.to_string(),
    }
}

fn ok(name: &str, detail: String) -> Check {
    Check { name: name.to_string(), ok: true, detail, fix: None }
}

fn not_done(name: &str, detail: String) -> Check {
    Check { name: name.to_string(), ok: false, detail, fix: None }
}

fn refused(name: &str, detail: String, fix: &str) -> Check {
    Check { name: name.to_string(), ok: false, detail, fix: Some(fix.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EXIT_NOT_CONVERGED, EXIT_OK, EXIT_REFUSED};

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let dst = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy_dir(&e.path(), &dst);
            } else {
                std::fs::copy(e.path(), dst).unwrap();
            }
        }
    }

    /// A throwaway copy of the basic fixture: the tests write records into it.
    fn fixture(tag: &str) -> crate::testdir::Held<Fixture> {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/fixtures/basic");
        let dst = crate::testdir::path(&format!("goatbuilder-fixture-{tag}-"));
        copy_dir(&src, &dst);
        let f = Fixture::open(&dst).unwrap();
        crate::testdir::Held::with(dst, f)
    }

    fn tree_hash(root: &Path) -> Vec<(String, String)> {
        let mut out = Vec::new();
        fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, root, out);
                } else {
                    out.push((p.strip_prefix(root).unwrap().display().to_string(), std::fs::read_to_string(&p).unwrap_or_default()));
                }
            }
        }
        walk(root, root, &mut out);
        out.sort();
        out
    }

    /// C7: one machine never, one unreadable, three converge — exit 3, and
    /// neither the pending nor the unknown machine is ever reported done.
    #[test]
    fn c7_pending_and_unknown_are_never_done() {
        let mut f = fixture("c7");
        let r = deploy(&mut f, "dev", Some("portal"), "A Person", false).unwrap();
        assert_eq!(r.exit, EXIT_NOT_CONVERGED);
        let by: BTreeMap<_, _> = r.checks.iter().map(|c| (c.name.clone(), c)).collect();
        assert!(by["dev/dev-core-a/portal"].ok);
        assert!(by["dev/dev-core-b/portal"].ok);
        assert!(!by["dev/dev-obs-a/portal"].ok, "after:2 is pending on the first run");
        assert!(!by["dev/dev-obs-b/portal"].ok, "never stays pending");
        assert!(by["dev/dev-edge-a/portal"].detail.starts_with("unknown"), "{}", by["dev/dev-edge-a/portal"].detail);
        let s = status(&f, "dev", Some("portal")).unwrap();
        assert_eq!(s.checks.iter().filter(|c| c.ok).count(), 2);
        assert_eq!(s.checks.iter().filter(|c| c.detail.starts_with("unknown")).count(), 1);
        assert_eq!(s.exit, EXIT_NOT_CONVERGED);
    }

    /// C5: a second run over converged machines writes nothing new; a machine
    /// that converges "after 2" is picked up by the second run (resumable).
    #[test]
    fn c5_idempotent_and_resumable() {
        let mut f = fixture("c5");
        deploy(&mut f, "dev", Some("portal"), "A Person", false).unwrap();
        let rows_a = std::fs::read_to_string(f.root.join("records/dev/dev-core-a.jsonl")).unwrap();
        let r2 = deploy(&mut f, "dev", Some("portal"), "A Person", false).unwrap();
        assert_eq!(rows_a, std::fs::read_to_string(f.root.join("records/dev/dev-core-a.jsonl")).unwrap(), "converged machine untouched");
        let by: BTreeMap<_, _> = r2.checks.iter().map(|c| (c.name.clone(), c)).collect();
        assert!(by["dev/dev-obs-a/portal"].ok, "after:2 converges on the second run");
        assert_eq!(r2.exit, EXIT_NOT_CONVERGED, "never + unreadable still hold the verdict");
        let p = fixture("c5-preprod");
        let mut p = p;
        let r = deploy(&mut p, "preprod", Some("portal"), "A Person", false).unwrap();
        assert_eq!(r.exit, EXIT_OK, "{:?}", r.checks);
        let again = deploy(&mut p, "preprod", Some("portal"), "A Person", false).unwrap();
        assert_eq!(again.exit, EXIT_OK);
        assert_eq!(std::fs::read_to_string(p.root.join("records/preprod/preprod-core-a.jsonl")).unwrap().lines().count(), 1, "already on the declared ref: no new row");
    }

    /// C1: a mutable tag in the declaration is refused before anything is published.
    #[test]
    fn c1_mutable_tag_is_refused() {
        let mut f = fixture("c1");
        let r = deploy(&mut f, "dev", Some("netvis"), "A Person", false).unwrap();
        assert_eq!(r.exit, EXIT_REFUSED);
        assert!(r.checks[0].fix.is_some());
        assert!(!f.root.join("declared/publications.jsonl").exists(), "nothing published");
    }

    /// C8: status reads; the fixture is byte-identical before and after.
    #[test]
    fn c8_status_modifies_nothing() {
        let mut f = fixture("c8");
        deploy(&mut f, "dev", Some("portal"), "A Person", false).unwrap();
        let before = tree_hash(&f.root);
        let s = status(&f, "dev", None).unwrap();
        assert!(!s.checks.is_empty());
        assert_eq!(before, tree_hash(&f.root));
    }

    /// C10: the JSON report carries its schema version and the typed exit.
    #[test]
    fn c10_json_carries_schema_and_exit() {
        let f = fixture("c10");
        let s = status(&f, "dev", Some("portal")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&crate::output::render(&s, crate::OutputFormat::Json, crate::output::ColorChoice::Never)).unwrap();
        assert_eq!(v["schema"], "goatbuilder.output/v1");
        assert_eq!(v["verb"], "status");
        assert_eq!(v["exit"], EXIT_NOT_CONVERGED);
    }

    #[test]
    fn undeclared_environment_is_an_error() {
        let f = fixture("env");
        assert!(status(&f, "prod", None).is_err());
    }

    fn tutor_fixture(tag: &str) -> crate::testdir::Held<Fixture> {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/fixtures/tutor");
        let dst = crate::testdir::path(&format!("goatbuilder-tutor-fixture-{tag}-"));
        copy_dir(&src, &dst);
        let f = Fixture::open(&dst).unwrap();
        crate::testdir::Held::with(dst, f)
    }

    /// `deploy --dry-run` on a fixture decides and publishes nothing: no
    /// publication, no record, the tree byte for byte as before.
    #[test]
    fn deploy_dry_run_on_a_fixture_changes_nothing() {
        let f = tutor_fixture("dry");
        let before = tree_hash(&f.root);
        let r = deploy_plan(&f, "preprod", Some("portal")).unwrap();
        assert_eq!(r.exit, EXIT_OK);
        assert!(r.checks.iter().any(|c| c.detail.starts_with("DRY: would publish")));
        assert_eq!(tree_hash(&f.root), before);
    }

    /// C9: a rollback is a NEW declaration at the earlier ref that travels the
    /// same path as a deploy — the declaration changes, a later serial is
    /// published, every record names the earlier ref under it.
    #[test]
    fn c9_rollback_declares_the_earlier_ref_and_converges_it() {
        let mut f = tutor_fixture("c9");
        let v14 = f.declared_ref("preprod", "portal").unwrap();
        assert_eq!(deploy(&mut f, "preprod", Some("portal"), "A Person", false).unwrap().exit, EXIT_OK);
        let r = rollback(&mut f, "preprod", "portal", 1, "A Person", false).unwrap();
        assert_eq!(r.exit, EXIT_OK, "{:?}", r.checks.iter().filter(|c| !c.ok).map(|c| &c.detail).collect::<Vec<_>>());
        let back = f.declared_ref("preprod", "portal").unwrap();
        assert!(back.contains("portal:v1.3.2@"), "{back}");
        for m in ["preprod-core-a", "preprod-core-b"] {
            let rec = f.converger.read_record("preprod", m, "portal").unwrap().unwrap();
            assert_eq!(rec.reference, back);
            assert_eq!(rec.serial, 2, "the earlier ref under a newer ordering");
        }
        // And forward again is the same mechanism: back 1 from v1.3.2 is v1.4.0.
        assert_eq!(rollback(&mut f, "preprod", "portal", 1, "A Person", false).unwrap().exit, EXIT_OK);
        assert_eq!(f.declared_ref("preprod", "portal").unwrap(), v14);
    }

    /// C9's refusal: a data artifact is refused with exit 3 before anything
    /// changes; a dry run of a safe rollback changes nothing either.
    #[test]
    fn c9_unsafe_rollback_is_refused_and_a_dry_run_changes_nothing() {
        let mut f = tutor_fixture("c9-refuse");
        let before = tree_hash(&f.root);
        let r = rollback(&mut f, "preprod", "db-schema", 1, "A Person", false).unwrap();
        assert_eq!(r.exit, EXIT_REFUSE_UNSAFE);
        assert!(r.checks.iter().any(|c| c.detail.starts_with("REFUSING: 'db-schema' is artifact_type=data")));
        let r = rollback(&mut f, "preprod", "portal", 1, "A Person", true).unwrap();
        assert_eq!(r.exit, EXIT_OK);
        assert_eq!(tree_hash(&f.root), before);
        // Further back than the history goes is refused with the command that shows it.
        let r = rollback(&mut f, "preprod", "portal", 5, "A Person", false).unwrap();
        assert_eq!(r.exit, EXIT_REFUSED);
        assert!(r.checks.iter().any(|c| c.detail.contains("cannot go back 5")));
        let _ = EXIT_NOT_CONVERGED;
    }

    fn set_ref(f: &Fixture, env: &str, component: &str, reference: &str) -> Fixture {
        let pins = f.root.join("declared/pins.yaml");
        let mut doc: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&pins).unwrap()).unwrap();
        doc["components"][component][env]["ref"] = serde_yaml::Value::String(reference.into());
        std::fs::write(&pins, serde_yaml::to_string(&doc).unwrap()).unwrap();
        Fixture::open(&f.root).unwrap()
    }

    fn code(e: &anyhow::Error) -> String {
        e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.clone()).unwrap_or_default()
    }

    const V132: &str = "ghcr.io/example-org/portal:v1.3.2@sha256:9f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c5b4a39281706f5e4d3c2b1a0";
    const V140: &str = "ghcr.io/example-org/portal:v1.4.0@sha256:0b2a6c1d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b";

    /// C2: a tampered publication and a replayed older serial are both refused,
    /// exit 1, before anything is published; a NEW serial at an old digest is
    /// accepted (that is a revert).
    #[test]
    fn c2_signed_and_ordered() {
        // A new serial at an old digest: v1.3.2 (1), v1.4.0 (2), v1.3.2 again (3).
        let mut f = fixture("c2");
        assert_eq!(deploy(&mut f, "preprod", Some("portal"), "A Person", false).unwrap().exit, EXIT_OK);
        let mut f = set_ref(&f, "preprod", "portal", V140);
        assert_eq!(deploy(&mut f, "preprod", Some("portal"), "A Person", false).unwrap().exit, EXIT_OK);
        let mut f = set_ref(&f, "preprod", "portal", V132);
        assert_eq!(deploy(&mut f, "preprod", Some("portal"), "A Person", false).unwrap().exit, EXIT_OK);
        let rec = f.converger.read_record("preprod", "preprod-core-a", "portal").unwrap().unwrap();
        assert_eq!((rec.reference.as_str(), rec.serial), (V132, 3), "the revert is accepted under a newer serial");

        // A replay: serial 2 appended after serial 3 was accepted, correctly sealed.
        let log = f.root.join("declared/publications.jsonl");
        let accepted = std::fs::read_to_string(&log).unwrap();
        let replay = crate::converger::Publication { serial: 2, env: "preprod".into(), component: "portal".into(), reference: V140.into(), actor: "A Person".into(), ts: "epoch+0d00000s".into(), seal: String::new() }.sealed();
        std::fs::write(&log, format!("{accepted}{}\n", serde_json::to_string(&replay).unwrap())).unwrap();
        let e = deploy(&mut f, "preprod", Some("portal"), "A Person", false).unwrap_err();
        assert_eq!(code(&e), "GB-045");
        assert!(e.to_string().contains("serial 2 is older than last accepted 3"), "{e}");
        assert_eq!(goat_errors::explain("GB-045").and_then(|x| x.exit), Some(EXIT_REFUSED));

        // A tampered row: the accepted serial 2's ref rewritten in place.
        let tampered = accepted.replacen("portal:v1.4.0", "portal:v9.9.9", 1);
        assert_ne!(tampered, accepted);
        std::fs::write(&log, &tampered).unwrap();
        let e = deploy(&mut f, "preprod", Some("portal"), "A Person", false).unwrap_err();
        assert_eq!(code(&e), "GB-045");
        assert!(e.to_string().contains("signature"), "{e}");
        assert_eq!(std::fs::read_to_string(&log).unwrap(), tampered, "nothing published after a refusal");
    }

    /// C3: a local declaration behind the source of record is fast-forwarded and
    /// applied — never applied from the stale copy; with --frozen it is refused;
    /// a local copy ahead of the source of record is always refused.
    #[test]
    fn c3_fresh_source() {
        let f = fixture("c3");
        let pins = std::fs::read_to_string(f.root.join("declared/pins.yaml")).unwrap();
        std::fs::create_dir_all(f.root.join("source")).unwrap();
        std::fs::write(f.root.join("source/pins.yaml"), format!("revision: 2\n{}", pins.replace(V132, V140))).unwrap();
        let mut f = Fixture::open(&f.root).unwrap();

        let e = deploy(&mut f, "preprod", Some("portal"), "A Person", true).unwrap_err();
        assert_eq!(code(&e), "GB-046");
        assert!(e.to_string().contains("--frozen") && e.to_string().contains("cp source/pins.yaml declared/pins.yaml"), "{e}");
        assert!(!f.root.join("declared/publications.jsonl").exists(), "nothing published from the stale copy");

        let r = deploy(&mut f, "preprod", Some("portal"), "A Person", false).unwrap();
        assert_eq!(r.exit, EXIT_OK, "{:?}", r.checks);
        assert!(r.checks[0].detail.contains("fast-forwarded declared/pins.yaml from revision 0 to 2"), "{}", r.checks[0].detail);
        assert_eq!(f.converger.read_record("preprod", "preprod-core-a", "portal").unwrap().unwrap().reference, V140, "the source of record's ref, never the stale copy's");
        assert_eq!(f.converger.publications().unwrap().iter().map(|p| p.reference.as_str()).collect::<Vec<_>>(), [V140]);

        // Ahead of the source of record: refused, frozen or not.
        let local = std::fs::read_to_string(f.root.join("declared/pins.yaml")).unwrap();
        std::fs::write(f.root.join("declared/pins.yaml"), local.replace("revision: 2", "revision: 3")).unwrap();
        let mut f = Fixture::open(&f.root).unwrap();
        let e = deploy(&mut f, "preprod", Some("portal"), "A Person", false).unwrap_err();
        assert_eq!(code(&e), "GB-046");
        assert!(e.to_string().contains("ahead of the source of record"), "{e}");

        // A rollback writes its declaration into the source of record first.
        std::fs::write(f.root.join("declared/pins.yaml"), &local).unwrap();
        let mut f = Fixture::open(&f.root).unwrap();
        assert_eq!(rollback(&mut f, "preprod", "portal", 1, "A Person", false).unwrap().exit, EXIT_OK);
        let source = std::fs::read_to_string(f.root.join("source/pins.yaml")).unwrap();
        assert!(source.contains("revision: 3") && source.contains(V132), "{source}");
        assert_eq!(source, std::fs::read_to_string(f.root.join("declared/pins.yaml")).unwrap());
    }

    /// C4: the declaration and every record it converges carry the actor; a
    /// deploy, a rollback or a standup that names no actor is refused, exit 1,
    /// before anything is published.
    #[test]
    fn c4_attributed() {
        // dev-core-a records v1.3.2; the deploy of v1.4.0 writes a new row.
        let mut f = fixture("c4");
        deploy(&mut f, "dev", Some("portal"), "A Person", false).unwrap();
        let p = f.converger.publications().unwrap();
        assert_eq!(p[0].actor, "A Person");
        let rec = f.converger.read_record("dev", "dev-core-a", "portal").unwrap().unwrap();
        assert_eq!((rec.reference.as_str(), rec.actor.as_str()), (V140, "A Person"));

        let mut g = fixture("c4-none");
        for e in [
            deploy(&mut g, "preprod", Some("portal"), "", false).unwrap_err(),
            deploy(&mut g, "preprod", Some("portal"), "  ", false).unwrap_err(),
            rollback(&mut g, "preprod", "portal", 1, "", false).unwrap_err(),
        ] {
            assert_eq!(code(&e), "GB-047");
            assert!(e.to_string().contains("--ack"), "{e}");
        }
        assert!(!g.root.join("declared/publications.jsonl").exists(), "nothing published");
        assert_eq!(goat_errors::explain("GB-047").and_then(|x| x.exit), Some(EXIT_REFUSED));
        // A dry run declares nothing, so it needs no actor.
        assert_eq!(rollback(&mut g, "preprod", "portal", 1, "", true).unwrap().exit, EXIT_REFUSED, "basic preprod has one ref: nothing to go back to");
    }

    /// C6: a deploy opens only its own environment's transport; under a pull
    /// agent it opens none at all.
    #[test]
    fn c6_least_privilege() {
        let mut f = fixture("c6");
        deploy(&mut f, "dev", Some("portal"), "A Person", false).unwrap();
        let log = std::fs::read_to_string(f.root.join("state/transports.jsonl")).unwrap();
        let opened: Vec<serde_json::Value> = log.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert!(!opened.is_empty());
        assert!(opened.iter().all(|o| o["env"] == "dev" && o["transport"] == "push-ansible"), "{log}");

        let g = fixture("c6-pull");
        let cfg = g.root.join("goatbuilder.yaml");
        let text = std::fs::read_to_string(&cfg).unwrap();
        std::fs::write(&cfg, text.replacen("- name: dev\n    converger: push-ansible", "- name: dev\n    converger: pull-agent", 1)).unwrap();
        let mut g = Fixture::open(&g.root).unwrap();
        let r = deploy(&mut g, "dev", Some("portal"), "A Person", false).unwrap();
        assert!(r.checks.iter().any(|c| c.ok && c.detail.starts_with("converged on")), "the pull agents still converge");
        assert!(!g.root.join("state/transports.jsonl").exists(), "a pull agent's machines fetch; nothing is opened to them");
    }
}

#[cfg(test)]
mod status_page_tests {
    use super::*;

    fn row(name: &str, detail: &str) -> Check {
        // Every state these verbs define is a state, not a failure: converged,
        // pending and unknown are all ok rows, as the real verbs build them. A
        // test that wants a failure builds that row itself.
        Check { name: name.into(), ok: true, detail: detail.into(), fix: None }
    }

    fn page() -> String {
        let r = Report::new("status", vec![
            row("dev/a/portal", "converged on ghcr.io/o/portal:v2@sha256:3c3c3c3c3c3c\u{2026}"),
            row("dev/b/portal", "pending: record names ghcr.io/o/portal:v1@sha256:9f9f9f9f9f9f\u{2026}, declared ghcr.io/o/portal:v2@sha256:3c3c3c3c3c3c\u{2026}"),
            row("dev/c/portal", "pending: no record yet; declared ghcr.io/o/portal:v2@sha256:3c3c3c3c3c3c\u{2026}"),
            row("dev/d/portal", "unknown: d: record unreadable"),
        ], EXIT_NOT_CONVERGED);
        status_page(&r, "dev", false)
    }

    #[test]
    fn pending_and_unknown_are_states_in_a_table() {
        let p = page();
        assert!(!p.contains("FAIL") && !p.contains("ghcr.io"), "{p}");
        assert!(p.contains("1 component on 4 machines. 1 of 4 converged, 2 pending, 1 unknown."), "{p}");
        let cols = |l: &str| l.split_whitespace().map(String::from).collect::<Vec<_>>();
        let lines: Vec<&str> = p.lines().collect();
        assert_eq!(cols(lines[4]), ["COMPONENT", "MACHINE", "STATE", "RUNNING", "DECLARED"]);
        assert_eq!(cols(lines[5]), ["portal", "a", "converged", "v2", "v2"]);
        assert_eq!(cols(lines[6]), ["portal", "b", "pending", "v1", "v2"]);
        assert_eq!(cols(lines[7]), ["portal", "c", "pending", "none", "v2"]);
        assert_eq!(cols(lines[8]), ["portal", "d", "unknown", "unreadable", "v2"]);
    }

    /// A deploy that published and watched machines converge is a table; a run
    /// that reports anything else keeps its steps.
    #[test]
    fn a_deploy_of_machines_is_the_status_table() {
        let d = "3c".repeat(32);
        let r = Report::new("deploy", vec![
            row("preprod/portal/publish", &format!("serial 1: ghcr.io/o/portal:v1.4.0@sha256:{d} by A Learner")),
            row("preprod/core-a/portal", &format!("converged on ghcr.io/o/portal:v1.4.0@sha256:{d}")),
            row("preprod/core-b/portal", &format!("converged on ghcr.io/o/portal:v1.4.0@sha256:{d}")),
        ], crate::EXIT_OK);
        let p = machines_page(&r, false).unwrap();
        assert!(p.starts_with("Deploy: portal on preprod"), "{p}");
        assert!(p.contains("2 of 2 converged"), "{p}");
        assert_eq!(p.lines().find(|l| l.starts_with("portal")).unwrap().split_whitespace().collect::<Vec<_>>(), ["portal", "core-a", "converged", "v1.4.0", "v1.4.0"]);
        assert!(p.trim_end().ends_with("Published v1.4.0 as record 1, by A Learner."), "{p}");
        // A run with no machine to report is not a table of machines: a refusal
        // reads as what it refused.
        let refused = Report::new("rollback", vec![
            row("preprod/db-schema/artifact", "artifact_type=data"),
            Check { name: "preprod/db-schema/rollback".into(), ok: false, detail: "REFUSING: data".into(), fix: None },
        ], EXIT_NOT_CONVERGED);
        assert!(machines_page(&refused, false).is_none());
        // A machine that failed is not a row in a table of states: the predicate
        // hands it to the steps, which name it...
        let hurt = Check { name: "zagros-dev/h/portal".into(), ok: false, detail: "converged on nothing".into(), fix: None };
        assert!(!machine_row(&hurt), "a failed machine belongs to the steps");
        // A real run's own rows are left to the caller; its machines still table.
        let real = Report::new("deploy", vec![row("zagros-dev/preflight", "prep: stateful"), row("zagros-dev/h/portal", "converged on ghcr.io/o/p:v9@sha256:aa")], crate::EXIT_OK);
        let t = machines_page(&real, false).unwrap();
        assert!(t.contains(" h ") && t.contains("v9") && !t.contains("prep:"), "{t}");
        // ...and the whole run then reads as steps, so nothing reports "1 of 1
        // converged" over a machine that did not.
        let mixed = Report::new("deploy", vec![row("zagros-dev/h/portal", "converged on ghcr.io/o/p:v9@sha256:aa"), hurt], EXIT_NOT_CONVERGED);
        assert!(machines_page(&mixed, false).is_none(), "a failure is not a table of states");
    }

    #[test]
    fn diff_is_the_same_table_and_falls_back_on_other_rows() {
        let r = Report::new("diff", vec![
            row("dev/a/portal", "same: ghcr.io/o/portal:v2@sha256:3c"),
            row("dev/b/portal", "DRIFT: running ghcr.io/o/portal:v1@sha256:9f \u{2014} main declares ghcr.io/o/portal:v2@sha256:3c"),
        ], EXIT_NOT_CONVERGED);
        let p = diff_page(&r, false).unwrap();
        assert!(p.starts_with("Diff: dev") && !p.contains("FAIL"), "{p}");
        assert!(p.contains("1 of 2 same, 1 drift, 0 unknown."), "{p}");
        let other = Report::new("diff", vec![row("dev/transport", "read via mesh")], EXIT_NOT_CONVERGED);
        assert!(diff_page(&other, false).is_none());
    }

    /// A real run's rows, which the tutor's practice copy doesn't produce.
    #[test]
    fn a_real_diff_prints_the_same_table() {
        let r = Report::new("diff", vec![
            row("zagros-dev/transport", "read via mesh"),
            row("zagros-dev/h1/netvis", "same: ghcr.io/o/netvis:v2@sha256:3c (row deploy 2026-11-01T10:00:00Z)"),
            row("zagros-dev/h2/netvis", "DRIFT: running ghcr.io/o/netvis:v1@sha256:9f \u{2014} main declares ghcr.io/o/netvis:v2@sha256:3c (row deploy 2026-11-01T10:00:00Z)"),
            row("zagros-dev/h3/netvis", "unknown: host unreachable; main declares ghcr.io/o/netvis:v2@sha256:3c"),
            row("zagros-dev/h4/netvis", "not provisioned (absent); main declares ghcr.io/o/netvis:v2@sha256:3c"),
            row("zagros-dev/diff", "1 same, 1 drift, 1 unknown across 1 component(s)"),
        ], EXIT_NOT_CONVERGED);
        let p = diff_page(&r, false).unwrap();
        let cols = |m: &str| p.lines().find(|l| l.contains(m)).unwrap().split_whitespace().map(String::from).collect::<Vec<_>>();
        assert_eq!(cols(" h1 "), ["netvis", "h1", "same", "v2", "v2"]);
        assert_eq!(cols(" h2 "), ["netvis", "h2", "drift", "v1", "v2"]);
        assert_eq!(cols(" h3 "), ["netvis", "h3", "unknown", "unreachable", "v2"]);
        assert_eq!(cols(" h4 "), ["netvis", "h4", "none", "not", "provisioned", "v2"]);
        // Four rows, and every one of them in the count: the fourth state is
        // named rather than left out of the arithmetic.
        assert!(p.contains("1 of 4 same, 1 drift, 1 unknown, 1 in neither.") && !p.contains("FAIL") && !p.contains("transport"), "{p}");
        // And the page says what to do about the three that do not match.
        assert!(p.contains("To put the version main declares onto them: goatbuilder deploy") && p.contains("--component netvis"), "{p}");
        assert!(p.contains("did not answer is unknown") && p.contains("goatbuilder doctor --env zagros_dev"), "{p}");
    }

    #[test]
    fn the_same_tag_on_two_digests_shows_the_digests() {
        let r = Report::new("status", vec![row("dev/a/p", "pending: record names ghcr.io/o/p:v1@sha256:9f9f, declared ghcr.io/o/p:v1@sha256:3c3c")], EXIT_NOT_CONVERGED);
        let p = status_page(&r, "dev", false);
        assert!(p.contains("sha256:9f9f") && p.contains("sha256:3c3c"), "{p}");
    }

    #[test]
    fn a_retagged_image_drifts_by_digest_and_only_a_declared_row_gets_a_next_step() {
        let r = Report::new("diff", vec![row("zagros-dev/h1/p", "DRIFT: running ghcr.io/o/p:v1@sha256:9f \u{2014} main declares ghcr.io/o/p:v1@sha256:3c")], EXIT_NOT_CONVERGED);
        let p = diff_page(&r, false).unwrap();
        assert!(p.contains("sha256:9f") && p.contains("sha256:3c"), "{p}");
        assert!(p.split_whitespace().collect::<Vec<_>>().join(" ").contains("goatbuilder deploy --env zagros_dev --component p"), "--env takes the key: {p}");
        let two = next_steps(&[("drift", "a"), ("drift", "b")], "same", "dev");
        assert!(two[0].contains("each of a, b") && two[0].contains("--component <component>"), "{two:?}");
        assert!(next_steps(&[("same", "a"), ("none", "b")], "same", "dev").is_empty());
    }

    #[test]
    fn a_ref_with_no_tag_shows_the_start_of_its_digest() {
        assert_eq!(version_of("ghcr.io/o/p@sha256:7b7b7b7b7b7b\u{2026}"), "sha256:7b7b7b7b7b7b");
        assert_eq!(version_of("localhost:5000/o/p:v9@sha256:7b"), "v9");
    }
}
