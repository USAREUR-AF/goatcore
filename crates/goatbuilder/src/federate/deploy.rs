//! `goatbuilder federate deploy <partner>` and `goatbuilder federate test
//! <partner>`: two stages of a peering, each run alone.
//!
//! `deploy` executes an approved plan through the existing plays. `apply`
//! stays a renderer; this is the separate, named step that changes hosts.
//!
//! 1. The plan is `<work>/plan.json` (or `--plan`), written by
//!    `goatbuilder federate plan` (plan.rs, `goat-federation-plan/v1`). It is
//!    read with `plan::read`, which recomputes its hash and refuses a plan
//!    that does not match it (GB-201).
//! 2. Approval binds to that hash: `--approve-plan <hash>`, or a person at a
//!    terminal types `yes` after reading the plan and its hash. `--yes` never
//!    approves a plan. Without either, nothing runs (GB-186).
//! 3. One run, under the peering's lock, then, before anything changes: the
//!    plan is read again and refused if its hash is not the one approved
//!    (GB-187); `plan::recheck_live_prerequisites` reruns the live
//!    prerequisites and compares every input section with a plan built now,
//!    and nothing changes unless it allows the change (GB-188: changed inputs
//!    need a new plan, a saved plan is not proof the environment stayed the
//!    same); when the run starts after phase 10, phase 10 must be recorded as
//!    done for what is rendered now (GB-190); the plan's named resources are
//!    claimed in the run journals of this machine (lock.rs, GB-181); the
//!    approval is journaled, bound to the hash. Then phases 10 to 13 through
//!    the existing plays, with the plan's inputs compared again before each
//!    of 11, 12 and 13. A refusal is journaled as a `refused` run.
//!
//! Phase 10 is still the by-hand admission on our core unless confirmation 2
//! is asked for: the first deploy stops there, and `deploy <partner> --from
//! 11` (with the same approval) declares it done and carries on. A later
//! deploy skips phase 10 while that declaration holds and its inputs are
//! unchanged; phases 11 to 13 always run (each play is idempotent).
//!
//! `test` runs phase 14, acceptance, and nothing else.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::lock::Want;
use super::plan::{self as fplan, Plan};
use super::prompt::{self, groups, Ask};
use super::run::Surface;
use super::{federate_with, Options};
use crate::Report;

/// The phases `deploy` runs: the ones that change hosts. And `test`:
/// acceptance.
pub const DEPLOY_FROM: u32 = 10;
pub const DEPLOY_TO: u32 = 13;
pub const TEST: u32 = 14;

/// An approved plan, as the run checks it again under the lock.
#[derive(Debug, Clone)]
pub struct Approved {
    pub repo: PathBuf,
    pub peering: String,
    pub path: PathBuf,
    pub hash: String,
    /// `terminal` or `flag`.
    pub how: String,
    /// The plan's resources that have a value, as claims (scope = where).
    pub resources: Vec<Want>,
}

fn coded(code: &str, msg: String) -> anyhow::Error {
    goat_errors::coded(code, msg).into()
}

/// The plan's resources with a value, as host-level claims.
pub fn claims(p: &Plan) -> Vec<Want> {
    p.resources
        .iter()
        .filter_map(|r| r.name.as_ref().map(|n| Want::new(&r.kind, &r.on, n)))
        .collect()
}

/// What the person approving reads.
pub fn summary(p: &Plan, path: &Path) -> String {
    let mut s = format!(
        "\nThe plan to deploy: {}\n  peering:    {} on {} ({})\n  verdict:    {}\n",
        path.display(),
        p.peering,
        p.environment,
        p.site,
        p.verdict
    );
    if let Some(a) = &p.agreement {
        s.push_str(&format!("  agreement:  {} ({})\n", a.digest, a.transport));
    }
    if !p.rendered.is_empty() {
        s.push_str("  rendered (each compared again before anything changes):\n");
        for f in &p.rendered {
            s.push_str(&format!("    {}\n", f.path));
        }
    }
    if !p.resources.is_empty() {
        s.push_str("  takes:\n");
        for r in &p.resources {
            s.push_str(&format!(
                "    {} {} on {}\n",
                r.kind,
                r.name.as_deref().unwrap_or("(not declared)"),
                r.on
            ));
        }
    }
    if !p.restarts.is_empty() {
        s.push_str("  restarts:\n");
        for r in &p.restarts {
            s.push_str(&format!(
                "    phase {}: {} ({})\n",
                r.phase, r.what, r.required
            ));
        }
    }
    for r in p.readiness.iter().filter(|r| r.outcome != "PASS") {
        if r.detail.is_empty() {
            s.push_str(&format!("  readiness:  {} {}\n", r.check, r.outcome));
        } else {
            s.push_str(&format!("  readiness:  {} {}: {}\n", r.check, r.outcome, r.detail));
        }
    }
    if !p.conflicts.is_empty() {
        s.push_str(&format!(
            "  conflicts with other peerings: {}\n",
            p.conflicts.len()
        ));
    }
    s.push_str(&format!(
        "  plan hash:  {}\n              {}\n",
        p.hash,
        groups(&p.hash)
    ));
    s
}

/// Bind approval to the plan's hash: a flag that names exactly that hash, or
/// a person at a terminal typing `yes`. Never `--yes`, never silence.
pub fn approve(
    repo: &Path,
    p: &Plan,
    path: &Path,
    flag: Option<&str>,
    ask: &mut dyn Ask,
) -> Result<Approved> {
    let how = match flag {
        Some(v) if prompt::same_value(v, &p.hash) => "flag",
        Some(v) => {
            return Err(coded(
                "GB-186",
                format!("deploy: --approve-plan {v} is not the hash of the plan in {} ({}); read the plan again and approve the hash it has now", path.display(), p.hash),
            ))
        }
        None if ask.interactive() => {
            ask.say(&summary(p, path));
            let a = ask.ask("Type yes to approve exactly this plan and deploy it; anything else stops:");
            if !prompt::is_yes(&a) {
                return Err(coded("GB-186", "deploy: the plan was not approved; nothing changed".into()));
            }
            "terminal"
        }
        None => {
            return Err(coded(
                "GB-186",
                format!("deploy: the plan needs approval and there is no terminal to ask at; read {} and pass --approve-plan {} (--yes never approves a plan)", path.display(), p.hash),
            ))
        }
    };
    Ok(Approved {
        repo: repo.to_path_buf(),
        peering: p.peering.clone(),
        path: path.to_path_buf(),
        hash: p.hash.clone(),
        how: how.into(),
        resources: claims(p),
    })
}

/// The saved plan is still the approved one.
fn same_plan(a: &Approved) -> Result<()> {
    let saved = fplan::read(&a.path)?;
    if saved.hash != a.hash {
        return Err(coded("GB-187", format!("deploy: the plan {} changed after it was approved (approved {}, now {}); nothing more runs", a.path.display(), a.hash, saved.hash)));
    }
    Ok(())
}

/// Immediately before the first change: the plan is the approved one and the
/// planner's live recheck allows the change.
pub fn recheck(a: &Approved) -> Result<String> {
    same_plan(a)?;
    let mut r = fplan::recheck_live_prerequisites(&a.repo, &a.path, &a.peering)?;
    // A later run of a plan deploy has already run part of: the sections are
    // compared with what deploy's last phase left, not with the plan.
    if let Some(before) = baseline(a) {
        let (_, now, _) = fplan::inputs_now(&a.repo, &a.path, &a.peering, None)?;
        r.changed = fplan::changed_sections(&before, &now);
    }
    if r.plan_hash != a.hash {
        return Err(coded(
            "GB-187",
            format!(
                "deploy: the recheck read plan {}, not the approved {}",
                r.plan_hash, a.hash
            ),
        ));
    }
    if !r.allows_change() {
        let mut why: Vec<String> = r.unready.clone();
        why.extend(
            r.changed
                .iter()
                .map(|c| format!("{c} changed since the plan")),
        );
        why.extend(
            r.live
                .iter()
                .filter(|l| !r.holds(l))
                .map(|l| format!("{} {}", l.check, l.outcome)),
        );
        for c in fplan::LIVE_CHECKS {
            if !r.live.iter().any(|l| l.check == *c) {
                why.push(format!("{c} was not answered"));
            }
        }
        return Err(coded(
            "GB-188",
            format!(
                "deploy: the plan no longer holds: {}; make a new plan and approve it",
                why.join("; ")
            ),
        ));
    }
    Ok(format!(
        "plan {} rechecked live: every prerequisite holds and no input changed",
        a.hash
    ))
}

/// Before each phase that changes a host: the plan is the approved one and
/// no section it was made from changed (the planner's `recheck_inputs`, no
/// live reads; the run's own export of phase 1's inputs keeps each value's
/// derived source, so it is not a change).
pub fn inputs_hold(a: &Approved) -> Result<()> {
    inputs_hold_since(a, None)
}

/// [`inputs_hold`], compared with `since` when deploy has already run a
/// phase: the plan's sections as they read right after deploy's last phase.
/// What deploy's own phases changed (the admission's certificates, the
/// rendered files a play leaves) is then expected; anything changed since
/// that phase ended is refused.
pub fn inputs_hold_since(a: &Approved, since: Option<&Plan>) -> Result<()> {
    same_plan(a)?;
    let changed = match since {
        None => fplan::recheck_inputs(&a.repo, &a.path, &a.peering, None)?.1,
        Some(before) => {
            let (_, now, _) = fplan::inputs_now(&a.repo, &a.path, &a.peering, None)?;
            fplan::changed_sections(before, &now)
        }
    };
    if !changed.is_empty() {
        return Err(coded(
            "GB-188",
            if since.is_some() {
                format!("deploy: what the plan was made from has changed since deploy's last phase: {}; make a new plan and approve it", changed.join(", "))
            } else {
                format!("deploy: what the plan was made from has changed: {}; make a new plan and approve it", changed.join(", "))
            },
        ));
    }
    Ok(())
}

/// Where deploy keeps the plan's sections as they read right after its last
/// phase passed, bound to the approved plan's hash: `<work>/plan/deployed.json`.
fn baseline_path(a: &Approved) -> PathBuf {
    a.path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("plan/deployed.json")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Baseline {
    plan_hash: String,
    sections: Plan,
}

/// The sections deploy's own last phase left, for this approved plan; None
/// before deploy has passed a phase of it.
pub fn baseline(a: &Approved) -> Option<Plan> {
    let b: Baseline =
        serde_json::from_str(&std::fs::read_to_string(baseline_path(a)).ok()?).ok()?;
    (b.plan_hash == a.hash).then_some(b.sections)
}

/// Record the plan's sections as they read now, after a phase deploy ran
/// passed: what that phase changed is deploy's own, and the next phase (in
/// this run or a later run of the same plan) is compared with it.
pub fn record_baseline(a: &Approved) -> Result<()> {
    let (_, now, _) = fplan::inputs_now(&a.repo, &a.path, &a.peering, None)?;
    let p = baseline_path(a);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(&Baseline {
            plan_hash: a.hash.clone(),
            sections: now,
        })?,
    )?;
    std::fs::rename(&tmp, &p).with_context(|| format!("{}", p.display()))?;
    Ok(())
}

/// The command that runs deploy again with the same approval.
fn again(o: &Options, hash: &str, plan_flag: Option<&Path>) -> String {
    let mut c = format!("goatbuilder federate deploy {}", o.with);
    for (flag, v) in [
        ("--env", o.env),
        ("--as", o.r#as),
        ("--edge-group", o.edge_group),
        ("--work", o.work),
    ] {
        if let Some(v) = v {
            c.push_str(&format!(" {flag} {v}"));
        }
    }
    if let Some(p) = plan_flag {
        c.push_str(&format!(" --plan {}", p.display()));
    }
    c.push_str(&format!(" --approve-plan {hash}"));
    c
}

/// `goatbuilder federate deploy <partner>`. `o.from` is 1 (the default) or
/// 10 for the whole deploy, or 11 once the admission is done by hand.
pub fn deploy(
    repo: &Path,
    o: &Options,
    plan_flag: Option<&Path>,
    approve_flag: Option<&str>,
    ask: &mut dyn Ask,
    admission: super::Admission,
    confirm: bool,
) -> Result<Report> {
    let from = match o.from {
        1 | DEPLOY_FROM => DEPLOY_FROM,
        11 => 11,
        n => {
            return Err(coded(
                "GB-189",
                format!("deploy: --from {n} is not a deploy start: deploy runs phase 10 (the admission on our core) then 11 to 13; pass --from 11 once the admission is done by hand"),
            ))
        }
    };
    // A two-driver peering reaches 11 on the edge: refused before the plan is
    // read, approved or rechecked, and before phase 10 admits on our core.
    super::refuse_two_drivers_before(repo, o, |n| (from..=DEPLOY_TO).contains(&n))?;
    let work = super::work_dir(o);
    let path = plan_flag
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new(&work).join("plan.json"));
    let p = fplan::read(&path)?;
    if p.peering != o.with {
        return Err(coded(
            "GB-187",
            format!(
                "deploy: the plan {} is for peering '{}', not '{}'",
                path.display(),
                p.peering,
                o.with
            ),
        ));
    }
    let only: BTreeSet<u32> = (from..=DEPLOY_TO).collect();
    let again = Some(again(o, &p.hash, plan_flag));
    if o.dry_run || o.mock.is_some() {
        // Nothing runs, so nobody is asked; a hash that is given is still
        // checked, and so is whether the plan's inputs still hold.
        let a = approve(
            repo,
            &p,
            &path,
            Some(approve_flag.unwrap_or(&p.hash)),
            &mut prompt::Nobody,
        )?;
        if o.mock.is_none() {
            inputs_hold(&a)?;
        }
        let mut s = Surface {
            resume: true,
            only: Some(only),
            stage: "deploy",
            again,
            ..Default::default()
        };
        return federate_with(
            repo,
            &Options {
                from,
                to: DEPLOY_TO,
                table: false,
                ..*o
            },
            &mut s,
        );
    }
    let approved = approve(repo, &p, &path, approve_flag, ask)?;
    // Phase 10 asks the same person, at the same terminal, when --confirm.
    let mut s = Surface {
        resume: true,
        only: Some(only),
        stage: "deploy",
        plan: Some(approved),
        again,
        admission,
        values: super::Values {
            ask: confirm,
            ..Default::default()
        },
        ask: Some(Box::new(super::prompt::Terminal)),
        ..Default::default()
    };
    federate_with(
        repo,
        &Options {
            from,
            to: DEPLOY_TO,
            ..*o
        },
        &mut s,
    )
}

/// `goatbuilder federate <partner>` as the whole procedure, with a person at
/// the terminal: phases 1 to 9 (confirmation 1 at its prompts), then at phase
/// 10 the plan is made and shown, deploy asks to approve its hash and then
/// what our core admits (confirmation 2), runs 10 to 13, and acceptance runs
/// last. A run that does not reach phase 10, or has no one to ask, is the
/// plain run as before. The rows of each stage are kept, in order.
pub fn whole(
    repo: &Path,
    o: &Options,
    s: &mut Surface,
    ask: &mut dyn Ask,
    admission: super::Admission,
    confirm: bool,
) -> Result<Report> {
    let first = federate_with(repo, o, s)?;
    if !s.handed_off || !first.ok {
        return Ok(first);
    }
    let mut checks = first.checks;
    let made = fplan::plan(
        repo,
        &fplan::PlanOptions {
            with: o.with,
            env: o.env,
            r#as: o.r#as,
            edge_group: o.edge_group,
            work: o.work,
            mock: None,
            live: true,
        },
    )?;
    checks.extend(made.checks);
    let deployed = deploy(
        repo,
        &Options {
            from: DEPLOY_FROM,
            to: 99,
            ..*o
        },
        None,
        None,
        ask,
        admission,
        confirm,
    )?;
    let mut exit = deployed.exit;
    let carry_on = deployed.ok && o.to >= TEST;
    checks.extend(deployed.checks);
    if carry_on {
        let tested = test(repo, o)?;
        exit = tested.exit;
        checks.extend(tested.checks);
    }
    let mut r = Report::new("federate", checks, crate::EXIT_REFUSED);
    r.exit = exit;
    r.ok = exit == crate::EXIT_OK;
    Ok(r)
}

/// `goatbuilder federate test <partner>`: acceptance alone.
/// `federate test <partner> --bilateral [<attempt>]`: phase 14 on a session both
/// sides derive (docs/design/federation-acceptance.md §6.3), with this binary as
/// its signer and verifier. Opt-in: without it, phase 14 is as it always was.
pub fn test_bilateral(repo: &Path, o: &Options, attempt: Option<u32>) -> Result<Report> {
    match attempt {
        Some(n) => crate::federate::runenv::set("FED_ACCEPTANCE_ATTEMPT", n.to_string()),
        None => crate::federate::runenv::unset("FED_ACCEPTANCE_ATTEMPT"),
    }
    test(repo, o)
}

pub fn test(repo: &Path, o: &Options) -> Result<Report> {
    let again = Some(format!("goatbuilder federate test {}", o.with));
    let mut s = Surface {
        only: Some(BTreeSet::from([TEST])),
        stage: "test",
        again,
        ..Default::default()
    };
    federate_with(
        repo,
        &Options {
            from: TEST,
            to: TEST,
            ..*o
        },
        &mut s,
    )
}
