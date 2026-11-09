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

use anyhow::Result;

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
        s.push_str(&format!("  readiness:  {} {}\n", r.check, r.outcome));
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
    let r = fplan::recheck_live_prerequisites(&a.repo, &a.path, &a.peering)?;
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
                .filter(|l| l.outcome != "PASS" && l.outcome != "NOT APPLICABLE")
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
    same_plan(a)?;
    let (_, changed, _) = fplan::recheck_inputs(&a.repo, &a.path, &a.peering, None)?;
    if !changed.is_empty() {
        return Err(coded(
            "GB-188",
            format!("deploy: what the plan was made from has changed: {}; make a new plan and approve it", changed.join(", ")),
        ));
    }
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
    let mut s = Surface {
        resume: true,
        only: Some(only),
        stage: "deploy",
        plan: Some(approved),
        again,
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

/// `goatbuilder federate test <partner>`: acceptance alone.
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
