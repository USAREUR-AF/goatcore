//! `goatbuilder preflight <component> --env <env>` — the pre-rollout readiness
//! and reversibility check (ADR 1211, docs/design/goatbuilder-rollout-preflight.md).
//!
//! `terraform plan` answers "what will change?"; this answers the question an
//! operator asks the moment before they commit a rollout: "if this goes wrong,
//! how cleanly can we undo it?" It prints THREE explicit facts — never a single
//! editorial "risk" word — and the readiness half the other verbs already prove:
//!
//!   State:    stateless | stateful            does the rollout create/migrate data
//!   Rollback: pin-revert | restore-required | none
//!   Scope:    single | coupled                part of a graduation set (ADR 1120)
//!
//! The three facts are DECLARED per component, one-fact-one-home, beside the
//! artifact type in `ops/pins/fleet-pins.yaml` (`owns_state`, `rollback`,
//! `restore_from`, `coupled_with`); coupling also derives from an OPEN
//! graduation set the component is in (ADR 1125). FAIL CLOSED: an undeclared
//! component reads `stateful · rollback: none (undeclared)`, never a hopeful
//! guess, and that reads not-clean (exit 3), so the gap is visible.
//!
//! It MEASURES, it does not assume (honesty-graded like `assay`): the declared
//! ref is digest-pinned and, when a registry probe can run, pullable; `pins
//! --check` is clean; the deploy-time secrets gate is satisfied; the checkout
//! is fresh (C3); the component's end state is declared (ADR 1206). For a
//! `pin-revert` it runs `rollback --dry-run` to confirm the flip would execute;
//! for `restore-required` it reads the restore pointer and the remaining
//! retention window. What it cannot check reads UNKNOWN, never a false green.
//!
//! `deploy` runs this as its first phase (the prep phase) and prints the
//! readout; a rollout graded `none` or `restore-required` then needs an
//! explicit `--ack`, and the accepted facts are written into the deploy ledger
//! row. It does NOT hard-block — a stateful cutover stays possible, knowingly.
//! The script avenue is `ops/scripts/deploy-component.sh --preflight` (ADR 1201).
//!
//! Exit 0 ready and cleanly reversible; 3 when a readiness check fails, a fact
//! is undeclared (fail-closed), a declared rollback contradicts the artifact
//! type, or a validation could not answer (GB-076); 1 when it cannot run at all
//! (GB-069).

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::{Check, Report, EXIT_NOT_CONVERGED};

/// The allowed values of the declared `rollback:` fact.
pub const ROLLBACK_KINDS: &[&str] = &["pin-revert", "restore-required", "none"];

/// A component's declared rollout facts, plus the artifact type the deploy
/// contract already records — the ground truth a hopeful `rollback:` is checked
/// against. Filled from `fleet-pins.yaml` on real hosts, from a fixture's
/// `declared/artifacts.yaml` on a mock; the grading is the same either way.
#[derive(Debug, Clone, Default)]
pub struct Declaration {
    pub component: String,
    /// `image|config|acl|data|cert|forward-only` (ADR 1060); default `image`.
    pub artifact_type: String,
    /// The ledger's `reversible` flag where the declaration carries one.
    pub reversible: Option<bool>,
    /// Declared: does the rollout own persistent data it creates or migrates.
    pub owns_state: Option<bool>,
    /// Declared: `pin-revert` | `restore-required` | `none`.
    pub rollback: Option<String>,
    /// Declared for `restore-required`: where undo restores from, and for how long.
    pub restore_from: Option<RestoreFrom>,
    /// Declared coupling (other components this one cannot roll back without).
    pub coupled_with: Vec<String>,
    /// Coupling derived from an OPEN graduation set: (set name, member components).
    pub graduation_set: Option<(String, Vec<String>)>,
}

/// The restore pointer and its retention window, for a `restore-required` undo.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RestoreFrom {
    /// A human pointer a reader can act on (a pgbackrest stanza, an archive path).
    pub pointer: String,
    /// How many days the restore source is kept.
    pub retention_days: Option<i64>,
    /// When the window started (the cutover or backup date), `YYYY-MM-DD`.
    pub as_of: Option<String>,
}

impl RestoreFrom {
    /// Days left in the window as of `today` (`YYYY-MM-DD`), when both the
    /// retention and the start date are known; `None` when it cannot be computed.
    pub fn remaining_days(&self, today: &str) -> Option<i64> {
        let days = self.retention_days?;
        let start = self.as_of.as_deref()?;
        Some(days - days_between(start, today)?)
    }
}

/// `b - a` in whole days for two `YYYY-MM-DD` dates, by the proleptic Gregorian
/// day number (no date crate; the same civil arithmetic push.rs uses).
fn days_between(a: &str, b: &str) -> Option<i64> {
    Some(day_number(b)? - day_number(a)?)
}

fn day_number(d: &str) -> Option<i64> {
    let mut it = d.splitn(3, '-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let day: i64 = it.next()?.trim().parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    // days_from_civil (Howard Hinnant), epoch 1970-01-01 = 0.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// The three facts, graded. The `*_clean` flags say whether the fact is a
/// clean, declared, self-consistent one (true) or a fail-closed gap / lie (false).
#[derive(Debug, Clone, Serialize)]
pub struct Facts {
    pub state: String,
    pub state_clean: bool,
    pub rollback: String,
    pub rollback_clean: bool,
    pub scope: String,
    pub scope_clean: bool,
}

/// Grade the three facts from a declaration alone — pure, no I/O. The caller
/// adds the measured readiness and validation rows around these.
pub fn grade_facts(d: &Declaration) -> Facts {
    let at = if d.artifact_type.is_empty() { "image" } else { d.artifact_type.as_str() };

    // State. Declared wins; an undeclared component fails closed to stateful.
    // A `data` artifact is stateful whatever a hopeful flag says.
    let (state, state_clean) = match d.owns_state {
        Some(true) => ("stateful".to_string(), true),
        Some(false) if at == "data" => ("stateful (declared stateless, but artifact_type=data)".to_string(), false),
        Some(false) => ("stateless".to_string(), true),
        None if at == "data" => ("stateful".to_string(), true),
        None => ("stateful (undeclared — fail closed)".to_string(), false),
    };

    // Rollback. Declared wins, then checked against the artifact type and the
    // ledger's reversible flag (ADR 1127: pin-revert is the safe class; a
    // schema/identity/ACL/data migration is not).
    let safe_pin_class = matches!(at, "image" | "config") && d.reversible != Some(false);
    let (rollback, rollback_clean) = match d.rollback.as_deref() {
        Some("pin-revert") if !safe_pin_class => (
            format!("pin-revert (declared, but artifact_type={at}{} — a ref swap cannot undo this)", if d.reversible == Some(false) { ", reversible=false" } else { "" }),
            false,
        ),
        Some("pin-revert") => ("pin-revert".to_string(), true),
        Some("restore-required") => match &d.restore_from {
            Some(r) if !r.pointer.trim().is_empty() => (format!("restore-required: {}", restore_summary(r)), true),
            _ => ("restore-required (no restore_from pointer declared)".to_string(), false),
        },
        Some("none") => ("none".to_string(), true),
        Some(other) => (format!("{other} (not one of {})", ROLLBACK_KINDS.join(", ")), false),
        None => ("none (undeclared — fail closed)".to_string(), false),
    };

    // Scope. Declared coupling or an open graduation set makes it coupled.
    let (scope, scope_clean) = match (&d.graduation_set, d.coupled_with.is_empty()) {
        (Some((name, members)), _) => {
            let others: Vec<&String> = members.iter().filter(|m| **m != d.component).collect();
            (format!("coupled — graduation set '{name}' with {}", join_or_none(&others)), true)
        }
        (None, false) => (format!("coupled — declared with {}", d.coupled_with.join(", ")), true),
        (None, true) => ("single".to_string(), true),
    };

    Facts { state, state_clean, rollback, rollback_clean, scope, scope_clean }
}

fn join_or_none<S: AsRef<str>>(xs: &[S]) -> String {
    if xs.is_empty() {
        "no other members".to_string()
    } else {
        xs.iter().map(|x| x.as_ref()).collect::<Vec<_>>().join(", ")
    }
}

fn restore_summary(r: &RestoreFrom) -> String {
    let window = match (r.retention_days, &r.as_of) {
        (Some(days), Some(_)) => format!(" ({days}-day window)"),
        (Some(days), None) => format!(" ({days}-day retention; start date not declared)"),
        (None, _) => String::new(),
    };
    format!("{}{window}", r.pointer)
}

/// Turn the graded facts into the three readout rows and the one summary line.
/// `verdict_ok` is appended by the caller after the readiness rows; here we only
/// carry the facts themselves and whether each is clean.
pub fn fact_rows(f: &Facts) -> Vec<Check> {
    vec![
        row("preflight/state", f.state_clean, format!("State: {}", f.state), fix_for_state(f)),
        row("preflight/rollback", f.rollback_clean, format!("Rollback: {}", f.rollback), fix_for_rollback(f)),
        row("preflight/scope", f.scope_clean, format!("Scope: {}", f.scope), None),
    ]
}

fn fix_for_state(f: &Facts) -> Option<String> {
    (!f.state_clean).then(|| "declare owns_state: true|false for this component in ops/pins/fleet-pins.yaml".to_string())
}

fn fix_for_rollback(f: &Facts) -> Option<String> {
    (!f.rollback_clean).then(|| "declare rollback: pin-revert|restore-required|none (and restore_from: for restore-required) in ops/pins/fleet-pins.yaml".to_string())
}

fn row(name: &str, ok: bool, detail: String, fix: Option<String>) -> Check {
    Check { name: name.to_string(), ok, detail, fix }
}

/// The one-line readout `deploy` prints and `preflight`'s summary carries:
/// `<state> · <rollback> · <scope>`, the three facts and nothing editorial.
pub fn readout_line(f: &Facts) -> String {
    format!("{} · rollback: {} · {}", short_state(&f.state), f.rollback, f.scope)
}

fn short_state(state: &str) -> String {
    // The summary leads with the bare state word; the caveat stays on the row.
    state.split_whitespace().next().unwrap_or(state).to_string()
}

/// Whether a rollout needs an explicit `--ack` on deploy. A DECLARED `none`,
/// `restore-required`, or a declared rollback that contradicts the artifact
/// type is a risk the ledger must show was knowingly accepted (ADR 1211). A
/// clean `pin-revert` needs none. An UNDECLARED component is not gated here —
/// the gap is a fail-closed not-clean row in the `preflight` verb, but gating
/// every not-yet-declared deploy on an ack would break automation that cannot
/// pass one; declaring the fact is the path, and `preflight` makes it visible.
pub fn needs_ack(f: &Facts) -> Option<&'static str> {
    let r = &f.rollback;
    if r.contains("undeclared") {
        return None;
    }
    if f.rollback_clean && r == "pin-revert" {
        return None;
    }
    if r.starts_with("restore-required") {
        Some("restore-required")
    } else if r.starts_with("none") {
        Some("none")
    } else {
        // A declared pin-revert that contradicts the artifact type: the real undo
        // is not a ref swap, so it is acknowledged as a non-reversible rollout.
        Some("not cleanly reversible")
    }
}

/// The accepted-risk object written into the deploy ledger row, as compact JSON,
/// so a reader of the ledger sees what was knowingly accepted and by whom.
pub fn accepted_risk_json(f: &Facts, acked_by: &str) -> String {
    serde_json::json!({
        "state": short_state(&f.state),
        "rollback": f.rollback,
        "scope": f.scope.split_whitespace().next().unwrap_or(&f.scope),
        "accepted_by": acked_by,
    })
    .to_string()
}

/// Assemble the full report from the facts, the readiness rows and the
/// validation rows. The verdict: not-clean (exit 3) when any row is not ok;
/// clean (exit 0) otherwise. The summary row is always last and always printed.
pub fn assemble(component: &str, env: &str, facts: &Facts, readiness: Vec<Check>, validation: Vec<Check>) -> Report {
    let mut checks = fact_rows(facts);
    checks.extend(readiness);
    checks.extend(validation);
    let clean = checks.iter().all(|c| c.ok);
    let verdict = if clean {
        format!("{component} on {env}: ready — {}", readout_line(facts))
    } else {
        format!("{component} on {env}: NOT clean — {} (see rows above) (GB-076)", readout_line(facts))
    };
    checks.push(row("preflight", clean, verdict, (!clean).then(|| "resolve each not-ok row above; an undeclared fact is a fail-closed gap, not a green".to_string())));
    let mut r = Report::new("preflight", checks, EXIT_NOT_CONVERGED);
    r.ok = clean;
    r.exit = if clean { crate::EXIT_OK } else { EXIT_NOT_CONVERGED };
    r
}

// --------------------------------------------------------------------------
// Real-host path: read the declaration from fleet-pins + graduation sets, and
// measure against this repository and the goatnet's hosts.
// --------------------------------------------------------------------------

pub struct Options<'a> {
    pub component: &'a str,
    pub goatnet: &'a str,
    pub site: Option<&'a str>,
}

/// Read a component's declared facts from `ops/pins/fleet-pins.yaml` and the
/// OPEN graduation sets. The artifact type and reversible flag come from the
/// same deploy contract (ADR 1060); coupling from ADR 1125.
pub fn declaration(repo: &Path, component: &str) -> Result<Declaration> {
    let text = std::fs::read_to_string(repo.join("ops/pins/fleet-pins.yaml"))
        .map_err(|e| goat_errors::coded("GB-069", format!("cannot read ops/pins/fleet-pins.yaml: {e}")))?;
    let doc: serde_yaml::Value = serde_yaml::from_str(&text)
        .map_err(|e| goat_errors::coded("GB-069", format!("ops/pins/fleet-pins.yaml did not parse: {e}")))?;
    let comp = doc
        .get("components")
        .and_then(|c| c.get(component))
        .ok_or_else(|| goat_errors::coded("GB-069", format!("unknown component {component:?} in ops/pins/fleet-pins.yaml")))?;

    let s = |k: &str| comp.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let b = |k: &str| comp.get(k).and_then(|v| v.as_bool());
    let restore_from = comp.get("restore_from").map(|r| RestoreFrom {
        pointer: r.get("pointer").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        retention_days: r.get("retention_days").and_then(|v| v.as_i64()),
        as_of: r.get("as_of").and_then(|v| v.as_str()).map(str::to_string),
    });
    let coupled_with = comp
        .get("coupled_with")
        .and_then(|v| v.as_sequence())
        .map(|seq| seq.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let sets = crate::promote::Tree::new(repo).load_sets();
    let graduation_set = crate::promote::open_set_for(&sets, component).map(|rec| {
        let name = rec.doc.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
        let members = rec
            .doc
            .get("members")
            .and_then(|m| m.as_array())
            .map(|a| a.iter().filter_map(|m| m.get("component").and_then(|c| c.as_str()).map(String::from)).collect())
            .unwrap_or_default();
        (name, members)
    });

    Ok(Declaration {
        component: component.to_string(),
        artifact_type: s("artifact_type").unwrap_or_else(|| "image".into()),
        reversible: b("reversible"),
        owns_state: b("owns_state"),
        rollback: s("rollback"),
        restore_from,
        coupled_with,
        graduation_set,
    })
}

/// Today as `YYYY-MM-DD` (UTC), or the override in `GOAT_PREFLIGHT_TODAY` (tests).
fn today() -> String {
    if let Ok(t) = std::env::var("GOAT_PREFLIGHT_TODAY") {
        if !t.is_empty() {
            return t;
        }
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The readiness half, reusing the checks the other verbs already prove.
fn readiness(repo: &Path, o: &Options) -> Vec<Check> {
    let mut checks = Vec::new();

    // Candidate ref: digest-pinned (always checkable) and, when a registry
    // probe can run, pullable. The declared ref per member comes from the plan.
    match crate::push::plan(repo, &crate::push::Options { component: o.component, goatnet: o.goatnet, only_group: None, dry_run: true, verify_only: false, ack: "" }) {
        Ok(p) => {
            let refs: Vec<String> = p.wants.values().cloned().collect();
            if refs.is_empty() {
                checks.push(unknown("preflight/candidate-ref", format!("no declared ref for {} on {} in group_vars — nothing to roll out", o.component, p.name)));
            } else {
                for w in &refs {
                    checks.push(ref_check("preflight/candidate-ref", w));
                }
            }
            // Deploy-time secrets gate (#2011), reused from the plan.
            if p.secrets_missing.is_empty() {
                checks.push(ok("preflight/secrets", format!("the deploy-time secrets gate is satisfied ({} variable(s))", p.secrets.len())));
            } else {
                checks.push(fail("preflight/secrets", format!("NOT SET: {} — the play would refuse on the host", p.secrets_missing.join(" ")), Some(crate::secrets::hint(repo, &p.secrets_missing))));
            }
        }
        Err(e) => checks.push(unknown("preflight/candidate-ref", format!("could not resolve the rollout from the pins: {e}"))),
    }

    // pins --check clean.
    match crate::pins::run(repo, true) {
        Ok(r) if r.ok => checks.push(ok("preflight/pins", "pins --check is clean (no drift, every intent signed)".into())),
        Ok(_) => checks.push(fail("preflight/pins", "pins --check reports drift or an unsigned/foreign pin".into(), Some("goatbuilder pins --check".into()))),
        Err(e) => checks.push(unknown("preflight/pins", format!("pins --check could not run: {e}"))),
    }

    // Fresh source (C3).
    match crate::fresh::check(repo) {
        Ok(f) => checks.push(f.check),
        Err(e) => checks.push(unknown("preflight/fresh-source", format!("the fresh-source check could not run: {e}"))),
    }

    // End state declared (ADR 1206): the component's end state is in the catalog.
    checks.push(endstate_presence(repo, o.component));

    checks
}

/// Is the component's end state declared (ADR 1206)? A service in the endstate
/// catalog whose `ledger` names it, or a service named for it. A component with
/// no end state is a coverage gap, not a green.
fn endstate_presence(repo: &Path, component: &str) -> Check {
    let path = repo.join("ops/goatbuilder/endstate/catalog.yaml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return unknown("preflight/endstate", format!("cannot read {}", path.display()));
    };
    let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
        return unknown("preflight/endstate", "ops/goatbuilder/endstate/catalog.yaml did not parse".into());
    };
    let services = doc.get("services").and_then(|s| s.as_mapping());
    let declared = services
        .map(|m| {
            m.iter().any(|(name, svc)| {
                name.as_str() == Some(component) || svc.get("ledger").and_then(|l| l.as_str()) == Some(component)
            })
        })
        .unwrap_or(false);
    if declared {
        ok("preflight/endstate", format!("end state declared for {component} (ADR 1206)"))
    } else {
        fail(
            "preflight/endstate",
            format!("no end state declared for {component} in ops/goatbuilder/endstate/catalog.yaml (ADR 1206)"),
            Some("add the component's service row to ops/goatbuilder/endstate/catalog.yaml".into()),
        )
    }
}

/// Validate the declared rollback fact against the live state, honesty-graded.
fn validation(repo: &Path, o: &Options, d: &Declaration, f: &Facts) -> Vec<Check> {
    let mut checks = Vec::new();
    match f.rollback.as_str() {
        "pin-revert" => {
            // The flip would execute: rollback --dry-run decides it without
            // touching a host. A refusal here contradicts the declared fact.
            match crate::rollback::roll(repo, o.component, 1, true, o.site) {
                Ok(rep) if rep.exit == crate::EXIT_OK => checks.push(ok("preflight/rollback-validated", "rollback --dry-run confirms the pin flip would execute".into())),
                Ok(rep) => checks.push(fail(
                    "preflight/rollback-validated",
                    format!("rollback --dry-run refused the flip (exit {}): the declared pin-revert is not what the ledger would do", rep.exit),
                    Some(format!("goatbuilder rollback {} 1 --dry-run{}", o.component, o.site.map(|s| format!(" --site {s}")).unwrap_or_default())),
                )),
                Err(e) => checks.push(unknown("preflight/rollback-validated", format!("rollback --dry-run could not run (no reachable ledger?): {e}"))),
            }
        }
        r if r.starts_with("restore-required") => {
            checks.push(restore_window_check(d, &today()));
        }
        _ => {}
    }
    checks
}

/// For `restore-required`: the restore pointer and the remaining retention
/// window. In-window is graded from the declared retention and start date;
/// existence of the archive reads UNKNOWN (it is off-box and not probed here).
fn restore_window_check(d: &Declaration, today: &str) -> Check {
    let Some(r) = &d.restore_from else {
        return fail("preflight/restore-window", "restore-required but no restore_from declared".into(), None);
    };
    match r.remaining_days(today) {
        Some(days) if days > 0 => ok("preflight/restore-window", format!("restore from {} — {days} day(s) left in the window (existence off-box: verify before deploy)", r.pointer)),
        Some(days) => fail(
            "preflight/restore-window",
            format!("restore from {} — the window has EXPIRED ({} day(s) past retention): there is no in-window restore source", r.pointer, -days),
            Some("confirm a live backup/archive and update restore_from.as_of before this rollout".into()),
        ),
        None => unknown("preflight/restore-window", format!("restore from {} — retention window UNKNOWN (declare retention_days and as_of in restore_from to grade it)", r.pointer)),
    }
}

/// A ref check: digest-pinned always, pullable when a probe can run.
fn ref_check(name: &str, reference: &str) -> Check {
    let rendered = crate::push::render_ref(reference);
    if !rendered.contains("@sha256:") {
        return fail(name, format!("{} is not digest-pinned (C1)", crate::verbs::short(&rendered)), Some("pin it to <image>@sha256:<digest> in the pins".into()));
    }
    match pull_probe(&rendered) {
        Some(true) => ok(name, format!("{} is digest-pinned and pullable", crate::verbs::short(&rendered))),
        Some(false) => fail(name, format!("{} is digest-pinned but NOT pullable from here", crate::verbs::short(&rendered)), Some("check the ref was built and pushed, and the registry is reachable".into())),
        None => unknown(name, format!("{} is digest-pinned; pullability UNKNOWN (no registry probe here)", crate::verbs::short(&rendered))),
    }
}

/// Probe whether a ref is pullable with `docker manifest inspect`, when docker
/// is present and `GOAT_PREFLIGHT_PULL` is not `0`. Returns None (UNKNOWN) when
/// no probe can run — CI and the fixtures take that path.
fn pull_probe(reference: &str) -> Option<bool> {
    if std::env::var("GOAT_PREFLIGHT_PULL").map(|v| v == "0").unwrap_or(false) {
        return None;
    }
    let docker = crate::runner::Cmd::new("docker")
        .args(["manifest", "inspect", reference])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match docker {
        Ok(s) => Some(s.success()),
        Err(_) => None, // docker not installed: UNKNOWN, never a false fail.
    }
}

fn ok(name: &str, detail: String) -> Check {
    Check { name: name.to_string(), ok: true, detail, fix: None }
}
fn fail(name: &str, detail: String, fix: Option<String>) -> Check {
    Check { name: name.to_string(), ok: false, detail, fix }
}
/// An UNKNOWN is not a green (assay's rule): it is not ok, and its detail leads
/// with the word so a reader never mistakes it for a pass.
fn unknown(name: &str, detail: String) -> Check {
    Check { name: name.to_string(), ok: false, detail: format!("UNKNOWN — {detail}"), fix: None }
}

/// `goatbuilder preflight <component> --env <env>` on real hosts.
pub fn run(repo: &Path, o: &Options) -> Result<Report> {
    let d = declaration(repo, o.component)?;
    let facts = grade_facts(&d);
    let readiness = readiness(repo, o);
    let validation = validation(repo, o, &d, &facts);
    Ok(assemble(o.component, o.goatnet, &facts, readiness, validation))
}

// --------------------------------------------------------------------------
// Fixture path (`--mock <dir>`): the declaration from `declared/artifacts.yaml`,
// the candidate ref from `declared/pins.yaml`, and the pin-revert validation
// through the fixture rollback. The repo-level readiness checks (pins --check,
// fresh source, the secrets gate, the end-state catalog) are not a fixture's to
// answer, so a mock readout carries the facts, the ref and the rollback
// validation — enough to exercise the grading end to end (C-cases in the tests).
// --------------------------------------------------------------------------

/// A component row of a fixture's `declared/artifacts.yaml` — a superset of the
/// shape `verbs.rs` reads for rollback, with the preflight facts added.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct MockArtifact {
    #[serde(default)]
    artifact_type: Option<String>,
    #[serde(default)]
    reversible: Option<bool>,
    #[serde(default)]
    owns_state: Option<bool>,
    #[serde(default)]
    rollback: Option<String>,
    #[serde(default)]
    restore_from: Option<MockRestoreFrom>,
    #[serde(default)]
    coupled_with: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct MockRestoreFrom {
    #[serde(default)]
    pointer: String,
    #[serde(default)]
    retention_days: Option<i64>,
    #[serde(default)]
    as_of: Option<String>,
}

pub fn run_mock(mut fixture: crate::verbs::Fixture, env: &str, component: &str) -> Result<Report> {
    let artifacts: std::collections::BTreeMap<String, MockArtifact> =
        match std::fs::read_to_string(fixture.root.join("declared/artifacts.yaml")) {
            Ok(t) => serde_yaml::from_str(&t).map_err(|e| goat_errors::coded("GB-069", format!("declared/artifacts.yaml did not parse: {e}")))?,
            Err(_) => std::collections::BTreeMap::new(),
        };
    let a = artifacts.get(component).cloned().unwrap_or_default();
    let d = Declaration {
        component: component.to_string(),
        artifact_type: a.artifact_type.unwrap_or_else(|| "image".into()),
        reversible: a.reversible,
        owns_state: a.owns_state,
        rollback: a.rollback,
        restore_from: a.restore_from.map(|r| RestoreFrom { pointer: r.pointer, retention_days: r.retention_days, as_of: r.as_of }),
        coupled_with: a.coupled_with,
        graduation_set: None,
    };
    let facts = grade_facts(&d);

    let mut readiness = Vec::new();
    // In a fixture there is no registry to probe, so the candidate ref is graded
    // on being digest-pinned (C1) alone; pullability is a real-host concern.
    match fixture.declared_ref(env, component) {
        Ok(reference) if reference.contains("@sha256:") => readiness.push(ok("preflight/candidate-ref", format!("{} is digest-pinned", crate::verbs::short(&reference)))),
        Ok(reference) => readiness.push(fail("preflight/candidate-ref", format!("{} is not digest-pinned (C1)", crate::verbs::short(&reference)), Some("pin it to <image>@sha256:<digest>".into()))),
        Err(e) => readiness.push(unknown("preflight/candidate-ref", format!("no declared ref for {component} on {env}: {e}"))),
    }

    let mut validation = Vec::new();
    if facts.rollback == "pin-revert" {
        match crate::verbs::rollback(&mut fixture, env, component, 1, "", true) {
            Ok(rep) if rep.exit == crate::EXIT_OK => validation.push(ok("preflight/rollback-validated", "rollback --dry-run confirms the earlier ref would be declared and converged".into())),
            Ok(rep) => validation.push(fail(
                "preflight/rollback-validated",
                format!("rollback --dry-run did not confirm a clean flip (exit {})", rep.exit),
                Some(format!("goatbuilder --mock <fixture> rollback {component} 1 --env {env} --dry-run")),
            )),
            Err(e) => validation.push(unknown("preflight/rollback-validated", format!("rollback --dry-run could not run: {e}"))),
        }
    } else if facts.rollback.starts_with("restore-required") {
        validation.push(restore_window_check(&d, &today()));
    }

    Ok(assemble(component, env, &facts, readiness, validation))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(component: &str) -> Declaration {
        Declaration { component: component.into(), artifact_type: "image".into(), ..Default::default() }
    }

    #[test]
    fn stateless_pin_revert_single_is_clean_and_reads_the_three_facts() {
        let mut d = decl("netvis");
        d.owns_state = Some(false);
        d.rollback = Some("pin-revert".into());
        let f = grade_facts(&d);
        assert_eq!(f.state, "stateless");
        assert_eq!(f.rollback, "pin-revert");
        assert_eq!(f.scope, "single");
        assert!(f.state_clean && f.rollback_clean && f.scope_clean);
        assert_eq!(readout_line(&f), "stateless · rollback: pin-revert · single");
        assert!(needs_ack(&f).is_none());
    }

    #[test]
    fn undeclared_fails_closed_to_stateful_none() {
        // The whole point: an undeclared component never reads a hopeful green.
        let f = grade_facts(&decl("mystery"));
        assert!(f.state.starts_with("stateful") && !f.state_clean);
        assert!(f.rollback.starts_with("none") && f.rollback.contains("undeclared") && !f.rollback_clean);
        assert_eq!(f.scope, "single");
        // Undeclared is surfaced by the preflight verb (not-clean), but it does
        // not gate every not-yet-declared deploy on an ack.
        assert!(needs_ack(&f).is_none());
    }

    #[test]
    fn the_ack_gate_fires_for_declared_risk_but_not_for_a_clean_pin_revert() {
        let mut d = decl("x");
        d.owns_state = Some(false);
        d.rollback = Some("pin-revert".into());
        assert!(needs_ack(&grade_facts(&d)).is_none(), "a clean pin-revert needs no ack");
        d.rollback = Some("none".into());
        assert_eq!(needs_ack(&grade_facts(&d)), Some("none"), "a declared none is acknowledged");
        d.rollback = Some("restore-required".into());
        d.restore_from = Some(RestoreFrom { pointer: "p".into(), retention_days: Some(28), as_of: Some("2026-10-01".into()) });
        assert_eq!(needs_ack(&grade_facts(&d)), Some("restore-required"));
        // A declared pin-revert that contradicts the artifact type is acknowledged.
        let mut bad = decl("db");
        bad.artifact_type = "data".into();
        bad.rollback = Some("pin-revert".into());
        assert_eq!(needs_ack(&grade_facts(&bad)), Some("not cleanly reversible"));
    }

    #[test]
    fn restore_required_carries_the_pointer_and_window() {
        let mut d = decl("id-admin-console");
        d.owns_state = Some(true);
        d.rollback = Some("restore-required".into());
        d.restore_from = Some(RestoreFrom {
            pointer: "SQLite archive /var/lib/goat/id-admin-console/pre-pg".into(),
            retention_days: Some(28),
            as_of: Some("2026-10-29".into()),
        });
        let f = grade_facts(&d);
        assert_eq!(f.state, "stateful");
        assert!(f.rollback.starts_with("restore-required: SQLite archive") && f.rollback.contains("28-day window"));
        assert!(f.rollback_clean);
        assert_eq!(needs_ack(&f), Some("restore-required"));
    }

    #[test]
    fn restore_required_without_a_pointer_is_not_clean() {
        let mut d = decl("x");
        d.rollback = Some("restore-required".into());
        let f = grade_facts(&d);
        assert!(f.rollback.contains("no restore_from pointer") && !f.rollback_clean);
    }

    #[test]
    fn a_pin_revert_declared_over_a_data_artifact_is_a_lie_and_not_clean() {
        let mut d = decl("db");
        d.artifact_type = "data".into();
        d.rollback = Some("pin-revert".into());
        let f = grade_facts(&d);
        assert!(!f.rollback_clean, "a ref swap cannot undo a data migration");
        assert!(f.rollback.contains("artifact_type=data"));
        // And the state is stateful whatever a hopeful owns_state says.
        d.owns_state = Some(false);
        assert!(!grade_facts(&d).state_clean);
    }

    #[test]
    fn reversible_false_defeats_a_declared_pin_revert() {
        let mut d = decl("forward");
        d.rollback = Some("pin-revert".into());
        d.reversible = Some(false);
        let f = grade_facts(&d);
        assert!(!f.rollback_clean && f.rollback.contains("reversible=false"));
    }

    #[test]
    fn declared_coupling_and_graduation_set_both_read_coupled() {
        let mut d = decl("consistency-console");
        d.rollback = Some("pin-revert".into());
        d.owns_state = Some(false);
        d.coupled_with = vec!["consistency-health".into()];
        let f = grade_facts(&d);
        assert!(f.scope.starts_with("coupled — declared with consistency-health"));

        d.coupled_with.clear();
        d.graduation_set = Some(("consistency-rj1".into(), vec!["consistency-console".into(), "consistency-health".into()]));
        let f = grade_facts(&d);
        assert!(f.scope.contains("graduation set 'consistency-rj1'") && f.scope.contains("consistency-health"));
        assert!(!f.scope.contains("consistency-console"), "the component itself is not listed as its own coupling");
    }

    #[test]
    fn the_retention_window_arithmetic_counts_days() {
        let r = RestoreFrom { pointer: "p".into(), retention_days: Some(28), as_of: Some("2026-10-01".into()) };
        assert_eq!(r.remaining_days("2026-10-01"), Some(28));
        assert_eq!(r.remaining_days("2026-10-29"), Some(0));
        assert_eq!(r.remaining_days("2026-10-30"), Some(-1));
        assert_eq!(r.remaining_days("2026-11-01"), Some(-3));
        // Across a month and a leap-ish boundary.
        let r = RestoreFrom { pointer: "p".into(), retention_days: Some(90), as_of: Some("2024-02-01".into()) };
        assert_eq!(r.remaining_days("2024-03-01"), Some(61)); // Feb 2024 has 29 days
        assert_eq!(RestoreFrom { pointer: "p".into(), retention_days: None, as_of: Some("2026-01-01".into()) }.remaining_days("2026-01-02"), None);
    }

    #[test]
    fn assemble_is_clean_only_when_every_row_is_ok() {
        let mut d = decl("netvis");
        d.owns_state = Some(false);
        d.rollback = Some("pin-revert".into());
        let f = grade_facts(&d);
        let clean = assemble("netvis", "zagros-dev", &f, vec![ok("preflight/pins", "clean".into())], vec![]);
        assert_eq!(clean.exit, crate::EXIT_OK);
        assert!(clean.checks.last().unwrap().detail.contains("ready"));

        let dirty = assemble("netvis", "zagros-dev", &f, vec![fail("preflight/pins", "drift".into(), None)], vec![]);
        assert_eq!(dirty.exit, EXIT_NOT_CONVERGED);
        assert!(dirty.checks.last().unwrap().detail.contains("NOT clean") && dirty.checks.last().unwrap().detail.contains("GB-076"));
    }

    #[test]
    fn an_unknown_row_is_never_green() {
        let u = unknown("preflight/candidate-ref", "no registry probe".into());
        assert!(!u.ok && u.detail.starts_with("UNKNOWN —"));
    }

    #[test]
    fn accepted_risk_json_names_the_facts_and_the_actor() {
        let mut d = decl("id-admin-console");
        d.owns_state = Some(true);
        d.rollback = Some("restore-required".into());
        d.restore_from = Some(RestoreFrom { pointer: "SQLite archive".into(), retention_days: Some(28), as_of: Some("2026-10-29".into()) });
        let f = grade_facts(&d);
        let v: serde_json::Value = serde_json::from_str(&accepted_risk_json(&f, "A Operator")).unwrap();
        assert_eq!(v["state"], "stateful");
        assert_eq!(v["accepted_by"], "A Operator");
        assert!(v["rollback"].as_str().unwrap().starts_with("restore-required"));
    }

    /// The declaration reader finds a real component's facts and derives the
    /// real graduation-set coupling from ops/release/sets (consistency-rj1).
    #[test]
    fn reads_a_real_components_declaration_and_coupling() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let d = declaration(&repo, "netvis").expect("netvis is a real component");
        assert_eq!(d.component, "netvis");
        assert!(!d.artifact_type.is_empty());
        // consistency-console sits in the open graduation set consistency-rj1.
        let c = declaration(&repo, "consistency-console").expect("real component");
        if let Some((name, members)) = &c.graduation_set {
            assert_eq!(name, "consistency-rj1");
            assert!(members.iter().any(|m| m == "consistency-health"));
        }
        let f = grade_facts(&c);
        assert!(f.scope.starts_with("coupled"), "consistency-console is coupled: {}", f.scope);
    }

    #[test]
    fn an_unknown_component_is_refused_to_run() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let e = declaration(&repo, "no-such-component-anywhere").unwrap_err();
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().unwrap().code, "GB-069");
    }
}
