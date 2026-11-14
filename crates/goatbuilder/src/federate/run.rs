//! One real run of `goatbuilder federate`, recorded in the peering's journal
//! (state.rs) under the per-peering lock (lock.rs), and, with `--resume`,
//! resumed from what is actually there.
//!
//! Every real run is recorded: the run, each phase attempt and outcome, and a
//! by-hand phase declared done with `--from`. Recording changes no phase.
//!
//! Resuming (the default; `--no-resume` turns it off) skips a phase only
//! when all of these hold:
//!
//! - the phase is one of 3, 5, 6, 9 or 10: it leaves local files that are
//!   a pure product of what it reads (6 the agreement, 9 the rendered files),
//!   or it is a by-hand phase (5, 10), or it is phase 3, whose output is our
//!   half as already sent to the partner (re-measuring it makes a new half
//!   that needs a new exchange, so that is asked for with `--from 3`; its
//!   digest covers the command, the signing key's bytes and, when a key
//!   rotation is recorded, `<work>/rotation.json`);
//! - its journal record says it passed;
//! - the digest of its command and of the files it reads, measured now, is
//!   the recorded one; and the digest of the files it left, measured now, is
//!   the recorded one (a missing or edited output is a change);
//! - no earlier phase of that kind had to run again. The first one that does
//!   is where the run starts, and every such phase after it runs too.
//!
//! Everything else always runs: the gates (1, 14) are live checks, the lint
//! and verify phases (4, 7) read again what is there, minting (8) reuses each
//! leaf that validates (its name, the CA it chains to, at least a day of
//! validity) and issues a missing one; a leaf that fails validation, one about
//! to expire included, stops phase 8 until it is replaced with `--rotate` (a
//! new leaf changes phase 10's inputs, so the by-hand review stops again),
//! and the phases that change hosts (2, 11, 12, 13) run their idempotent plays
//! again, which is how the actual state of a host is inspected and converged. So a resumed run
//! never trusts a phase number, and never skips work whose result it cannot
//! see.
//!
//! A by-hand phase is recorded as done only by an explicit act: `--from N`
//! when the journal's last record of phase N-1 is the stop there and the
//! phase still reads what it read when it stopped, which is what the stop
//! tells the operator to type. A `--from N` for any other reason
//! (a retry after phase N failed) records nothing about N-1. That is recorded as an `operator-step` confirmation bound to what the
//! phase read at that moment; when those inputs change, the phase stops again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::Result;

use super::admit;
use super::confirm;
use super::deploy::{self, Approved};
use super::lock::{self, PeeringLock};
use super::prompt::{Ask, Nobody};
use super::state::{self, Outcome, State};
use super::Phase;
use crate::Check;

/// What a run is asked to do beyond running the phases.
#[derive(Default)]
pub struct Surface {
    /// Confirmation 1 (confirm.rs): `--confirm` and the values read aloud.
    pub values: confirm::Values,
    /// Who answers questions: the terminal, or nobody.
    pub ask: Option<Box<dyn Ask>>,
    /// Confirmation 2 (admit.rs): `--approve-admission`, `--review-in-pr`.
    pub admission: admit::Admission,
    /// `--resume`: skip phases found done (see above).
    pub resume: bool,
    /// A stage runs exactly these phases (deploy: 10 to 13; test: 14)
    /// instead of `--from` to `--to`.
    pub only: Option<BTreeSet<u32>>,
    /// `federate` (the default), `deploy` or `test`, as the journal names it.
    pub stage: &'static str,
    /// The approved plan a deploy executes (deploy.rs).
    pub plan: Option<Approved>,
    /// The command that runs this stage again, when it is not
    /// `goatbuilder federate <partner>` (deploy carries its approval).
    pub again: Option<String>,
    /// A plain run with a person at the terminal: at phase 10 it hands over
    /// to plan, deploy and test (`deploy::whole`) instead of stopping for the
    /// by-hand admission.
    pub hand_off_at_10: bool,
    /// Set by the run when it stopped at phase 10 to hand over.
    pub handed_off: bool,
    /// `rotate <partner> leaves`: phase 8 replaces every leaf (rotation.rs).
    pub rotate_leaves: bool,
}

impl Surface {
    /// The person to ask, or nobody.
    pub fn asker(&mut self) -> &mut dyn Ask {
        self.ask.get_or_insert_with(|| Box::new(Nobody)).as_mut()
    }

    /// Whether phase `n` is in this run: one of a stage's phases, or else
    /// between `--from` and `--to`.
    pub fn runs(&self, n: u32, from: u32, to: u32) -> bool {
        match &self.only {
            Some(set) => set.contains(&n),
            None => n >= from && n <= to,
        }
    }
}

/// The files one peering's run reads and writes, resolved the way `plan()`
/// resolves them.
pub struct Paths {
    pub work: PathBuf,
    pub ours: PathBuf,
    pub counterparty: PathBuf,
    pub keyring: PathBuf,
    pub their_ca: PathBuf,
    pub agreement: PathBuf,
    pub applied: PathBuf,
    pub pki: PathBuf,
    /// The seed that signs our package (FED_SIGNING_KEY), read by phase 3.
    pub signing: Option<PathBuf>,
    /// `<work>/rotation.json`: a key rotation asked for on this peering
    /// (docs/design/federation-key-rotation.md §5). Read by phase 3 when it
    /// exists, so a new key is measured into a new package.
    pub rotation: PathBuf,
}

fn env_or(name: &str, default: String) -> String {
    crate::federate::runenv::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or(default)
}

impl Paths {
    pub fn new(work: &str) -> Self {
        let w = PathBuf::from(work);
        Paths {
            ours: w.join("ours.canonical.json"),
            counterparty: PathBuf::from(env_or(
                "FED_COUNTERPARTY",
                format!("{work}/theirs.canonical.json"),
            )),
            keyring: PathBuf::from(env_or("FED_KEYRING", format!("{work}/keys.txt"))),
            their_ca: w.join("theirs-ca.pem"),
            agreement: w.join("agreement.json"),
            applied: w.join("applied"),
            rotation: w.join("rotation.json"),
            pki: w.join("pki"),
            signing: crate::federate::runenv::var("FED_SIGNING_KEY")
                .ok()
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            work: w,
        }
    }

    /// What phase 8 minted: every file in pki/ except a partner CA kept
    /// there by hand.
    fn minted(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&self.pki)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_file() && p.file_name().is_some_and(|f| f != "partner-ca.pem"))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        if v.is_empty() {
            v.push(self.pki.join("core.pem"));
        }
        v
    }

    /// The local files a phase reads, and the local files it leaves.
    pub fn io(&self, n: u32) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let (o, t, k, ca, a, ap) = (
            self.ours.clone(),
            self.counterparty.clone(),
            self.keyring.clone(),
            self.their_ca.clone(),
            self.agreement.clone(),
            self.applied.clone(),
        );
        match n {
            // The rotation record only when there is one: a journal written
            // before it existed keeps its digest, so an upgrade reruns nothing.
            3 => (
                self.signing
                    .iter()
                    .cloned()
                    .chain(Some(self.rotation.clone()).filter(|r| r.exists()))
                    .collect(),
                vec![o],
            ),
            4 => (vec![o], vec![]),
            5 => (vec![o, t, ca], vec![k]),
            6 => (vec![o, t, k], vec![a]),
            7 => (vec![a], vec![]),
            8 => (vec![a], self.minted()),
            9 => (vec![a], vec![ap]),
            10 => {
                let mut i = vec![a, ap, ca];
                i.extend(self.minted());
                (i, vec![])
            }
            11 | 12 => {
                let mut i = vec![a, ap];
                i.extend(self.minted());
                (i, vec![])
            }
            13 | 14 => (vec![a], vec![]),
            _ => (vec![], vec![]),
        }
    }
}

fn row(name: &str, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        ok: true,
        detail: detail.into(),
        fix: None,
    }
}

/// Whether a resumed run may skip this phase when it finds it done.
fn skippable(p: &Phase) -> bool {
    p.kind == "MANUAL" || (p.kind == "T" && matches!(p.n, 3 | 6 | 9))
}

/// What a deploy did before the run began: the peering's lock, taken first,
/// and the planner's live recheck, run under it before the phase-1 inputs are
/// exported (an exported input would read as changed).
pub struct Prelude {
    pub lock: PeeringLock,
    pub rechecked: String,
}

impl Prelude {
    /// Take the lock and run the recheck; a refusal is journaled as a
    /// `refused` run of this peering.
    pub fn deploy(with: &str, work: &str, plan: &Approved, from: u32, to: u32) -> Result<Prelude> {
        let paths = Paths::new(work);
        let lock = PeeringLock::take(&paths.work, with)?;
        match deploy::recheck(plan) {
            Ok(rechecked) => Ok(Prelude { lock, rechecked }),
            Err(e) => {
                if let Ok(mut state) = State::load(&paths.work, with) {
                    let id = state.open_run("deploy", from, to, false, &plan.hash);
                    state.log(&id, 0, "refused", format!("{e:#}"));
                    state.close_run(&id, "refused");
                    let _ = state.save(&paths.work);
                }
                Err(e)
            }
        }
    }
}

/// One real run: the lock, the journal, and what it skips.
pub(super) struct Run {
    _lock: PeeringLock,
    pub state: State,
    pub paths: Paths,
    pub id: String,
    skip: BTreeSet<u32>,
    finished: bool,
    /// Journal writes that failed during the run, shown when it ends.
    save_errors: Vec<String>,
    /// The approved plan a deploy executes: checked again before each phase
    /// that changes a host; its claimed resources become `deployed` when the
    /// run passes.
    plan: Option<Approved>,
}

impl Run {
    /// Take the lock, read the journal, open a run in it, record a by-hand
    /// phase declared done by `--from`, and work out what `--resume` skips.
    pub(super) fn begin(
        with: &str,
        work: &str,
        phases: &[Phase],
        from: u32,
        to: u32,
        s: &Surface,
        pre: Option<Prelude>,
    ) -> Result<(Run, Vec<Check>)> {
        let stage = if s.stage.is_empty() {
            "federate"
        } else {
            s.stage
        };
        let paths = Paths::new(work);
        let (lock, rechecked) = match pre {
            Some(p) => (p.lock, p.rechecked),
            None => (PeeringLock::take(&paths.work, with)?, String::new()),
        };
        let mut rows: Vec<Check> = lock
            .removed
            .iter()
            .map(|name| {
                row(
                    "journal",
                    format!("[removed] {name}: a journal write left by an interrupted run"),
                )
            })
            .collect();
        let mut state = State::load(&paths.work, with)?;
        let resume = s.resume && (from <= 1 || s.only.is_some());
        let plan_hash = s.plan.as_ref().map(|p| p.hash.clone()).unwrap_or_default();
        let id = state.open_run(stage, from, to, resume, &plan_hash);
        if s.resume && from > 1 && s.only.is_none() {
            rows.push(row(
                "resume",
                format!("--from names the start, so nothing is skipped: phases {from} to {to} all run"),
            ));
        }

        // `--from N` when the journal's last word on by-hand phase N-1 is the
        // stop there: the operator says that phase is done. Recorded, bound
        // to what it read.
        // ... and only while that phase still reads what it read when it
        // stopped: what the operator acted on is what gets recorded.
        if let Some(prev) = phases
            .iter()
            .find(|p| p.n + 1 == from && p.kind == "MANUAL")
        {
            let inputs = Self::inputs(&paths, prev);
            match state.phases.get(&prev.n) {
                Some(r) if r.outcome == Outcome::Stopped && r.inputs == inputs => {
                    let outputs = Self::outputs(&paths, prev.n);
                    state.confirm(
                        &id,
                        prev.n,
                        "operator-step",
                        "flag",
                        &inputs,
                        BTreeMap::from([("from".to_string(), from.to_string())]),
                    );
                    state.record(
                        &id,
                        prev.n,
                        Outcome::Passed,
                        inputs,
                        outputs,
                        format!("done by hand, declared with --from {from}"),
                    );
                    rows.push(row(
                        "journal",
                        format!("phase {} recorded as done by hand (--from {from})", prev.n),
                    ));
                }
                Some(r) if r.outcome == Outcome::Stopped => rows.push(row(
                    "journal",
                    format!(
                        "phase {} is not recorded as done: what it reads changed after it stopped, so the step done by hand may not match; run phase {} again to review what is there now",
                        prev.n, prev.n
                    ),
                )),
                _ => {}
            }
        }

        if let Some(plan) = &s.plan {
            // Under the lock, before anything changes: the plan is the one
            // approved, its inputs are unchanged, the by-hand admission is
            // done when this run does not include it, and nothing another
            // peering holds is taken. A refusal is journaled as `refused`.
            let checked = (|| -> Result<Vec<Check>> {
                let _ = plan;
                if !s.runs(deploy::DEPLOY_FROM, from, to) {
                    let p10 = phases.iter().find(|p| p.n == deploy::DEPLOY_FROM);
                    let done = p10.is_some_and(|p| {
                        state.phases.get(&p.n).is_some_and(|r| {
                            r.outcome == Outcome::Passed && r.inputs == Self::inputs(&paths, p)
                        })
                    });
                    if !done {
                        return Err(goat_errors::coded(
                            "GB-190",
                            "deploy: phase 10, admitting the edge on our core, is not recorded as done for what is rendered now; run deploy without --from so it stops at phase 10, do the admission, then run deploy --from 11",
                        )
                        .into());
                    }
                }
                let mut out = vec![row("recheck", rechecked)];
                if plan.resources.is_empty() {
                    out.push(row("allocation", "the plan names no resources with a value yet, so there was nothing to claim in this machine's journals"));
                } else {
                    let _guard = lock::claim(
                        &lock::work_root(&paths.work),
                        &paths.work,
                        &mut state,
                        &id,
                        &plan.resources,
                    )?;
                    out.push(row(
                        "allocation",
                        format!(
                            "claimed {} resources in this machine's run journals; no other peering's journal holds them",
                            plan.resources.len()
                        ),
                    ));
                }
                Ok(out)
            })();
            match checked {
                Ok(r) => rows.extend(r),
                Err(e) => {
                    state.log(&id, 0, "refused", format!("{e:#}"));
                    state.close_run(&id, "refused");
                    let _ = state.save(&paths.work);
                    return Err(e);
                }
            }
            state.confirm(
                &id,
                0,
                "plan",
                &plan.how,
                &plan.hash,
                BTreeMap::from([("path".to_string(), plan.path.display().to_string())]),
            );
            rows.push(row(
                "approval",
                format!("plan {} approved ({})", plan.hash, plan.how),
            ));
        }

        let mut skip = BTreeSet::new();
        if resume {
            for p in phases.iter().filter(|p| s.runs(p.n, from, to)) {
                if !skippable(p) {
                    continue;
                }
                // Key values given on the command line are a new answer to
                // phase 5: it runs, and so does everything that reads its keyring.
                if p.n == 5
                    && (s.values.partner_key.is_some() || s.values.partner_ca_fingerprint.is_some())
                {
                    break;
                }
                let done = state.phases.get(&p.n).is_some_and(|r| {
                    r.outcome == Outcome::Passed
                        && r.inputs == Self::inputs(&paths, p)
                        && r.outputs == Self::outputs(&paths, p.n)
                });
                if !done {
                    break;
                }
                skip.insert(p.n);
            }
            let detail = if skip.is_empty() {
                "resuming: nothing is unchanged since it passed, so every phase runs".to_string()
            } else {
                let list: Vec<String> = skip.iter().map(u32::to_string).collect();
                format!(
                    "resuming: phases {} are unchanged since they passed and are not run again; every other phase runs ({})",
                    list.join(", "),
                    State::path(&paths.work).display()
                )
            };
            rows.push(row("resume", detail));
        }
        state.log(&id, from, "start", format!("phases {from} to {to}"));
        let mut run = Run {
            _lock: lock,
            state,
            paths,
            id,
            skip,
            finished: false,
            save_errors: Vec::new(),
            plan: s.plan.clone(),
        };
        // The run exists from here, so a failed write is shown when it ends
        // and an early return closes it.
        run.save();
        Ok((run, rows))
    }

    fn inputs(paths: &Paths, p: &Phase) -> String {
        state::digest_of(&p.cmd, &paths.io(p.n).0)
    }

    fn outputs(paths: &Paths, n: u32) -> String {
        state::outputs_of(&paths.io(n).1)
    }

    pub(super) fn skips(&self, p: &Phase) -> bool {
        self.skip.contains(&p.n)
    }

    pub(super) fn skipped(&mut self, p: &Phase) -> Check {
        let at = self
            .state
            .phases
            .get(&p.n)
            .map(|r| r.at.clone())
            .unwrap_or_default();
        let id = self.id.clone();
        self.state
            .log(&id, p.n, "skipped", "unchanged since it passed");
        row(
            "skipped",
            format!(
                "phase {} unchanged since it passed at {at}; not run again",
                p.n
            ),
        )
    }

    /// A deploy checks its plan again just before each phase that changes a
    /// host (10 to 13): a change since the run began refuses the phase.
    pub(super) fn guard(&self, p: &Phase) -> Option<Check> {
        let plan = self.plan.as_ref()?;
        if p.n < deploy::DEPLOY_FROM {
            return None;
        }
        deploy::inputs_hold_since(plan, deploy::baseline(plan).as_ref()).err().map(|e| Check {
            name: "failed".into(),
            ok: false,
            detail: format!("{e:#}"),
            fix: Some(
                "make a new plan from what is there now, approve its hash, and deploy again".into(),
            ),
        })
    }

    /// A phase is about to run: journaled first, so a run killed during it
    /// shows which phase it was in.
    pub(super) fn start(&mut self, p: &Phase) {
        let id = self.id.clone();
        self.state.log(&id, p.n, "start", "");
        self.save();
    }

    pub(super) fn end(&mut self, p: &Phase, outcome: Outcome, detail: &str) {
        let inputs = Self::inputs(&self.paths, p);
        let outputs = Self::outputs(&self.paths, p.n);
        let id = self.id.clone();
        let passed = outcome == Outcome::Passed;
        self.state
            .record(&id, p.n, outcome, inputs, outputs, detail);
        self.save();
        // What deploy's own phase changed is expected; the next phase is
        // compared with what this one left.
        if let (true, Some(plan)) = (passed && p.n >= deploy::DEPLOY_FROM, self.plan.as_ref()) {
            if let Err(e) = deploy::record_baseline(plan) {
                self.save_errors.push(format!("{e:#}"));
            }
        }
    }

    /// Close the run in the journal with its outcome word. Returns a failed
    /// row for each journal write that did not land, so `-o json` shows it.
    pub(super) fn finish(&mut self, outcome: &str) -> Vec<Check> {
        let id = self.id.clone();
        if let Some(plan) = self.plan.as_ref().filter(|_| outcome == "passed") {
            // What the plan names is deployed; what this peering held before
            // and the plan no longer names is let go.
            let held: Vec<(String, String, String)> = self
                .state
                .owned
                .iter()
                .filter(|o| o.held())
                .map(|o| (o.kind.clone(), o.scope.clone(), o.value.clone()))
                .collect();
            for (k, sc, v) in held {
                let named = plan
                    .resources
                    .iter()
                    .any(|w| w.kind == k && w.scope == sc && w.value == v);
                self.state.own(
                    &id,
                    &k,
                    &sc,
                    &v,
                    if named { "deployed" } else { "released" },
                );
            }
        }
        self.state.close_run(&id, outcome);
        self.save();
        self.finished = true;
        self.save_errors
            .drain(..)
            .map(|e| Check {
                name: "journal".into(),
                ok: false,
                detail: format!("the run journal was not fully written: {e}"),
                fix: Some(
                    "make the working folder writable; until then --resume cannot rely on this run"
                        .into(),
                ),
            })
            .collect()
    }

    fn save(&mut self) {
        if let Err(e) = self.state.save(&self.paths.work) {
            let e = format!("{e:#}");
            eprintln!("goatbuilder: {e}");
            if !self.save_errors.contains(&e) {
                self.save_errors.push(e);
            }
        }
    }
}

impl Drop for Run {
    /// A run that returns early with an error is closed as failed. (A killed
    /// run runs no code, so its run keeps an empty end: that is how it reads.)
    fn drop(&mut self) {
        if !self.finished {
            let id = self.id.clone();
            self.state.close_run(&id, "failed");
            let _ = self.state.save(&self.paths.work);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ph(n: u32, kind: &'static str) -> Phase {
        Phase {
            n,
            kind,
            label: format!("phase {n}"),
            cmd: format!("cmd {n}"),
            verb: None,
        }
    }

    fn phases() -> Vec<Phase> {
        vec![
            ph(1, "GATE"),
            ph(2, "A"),
            ph(3, "T"),
            ph(4, "T"),
            ph(5, "MANUAL"),
            ph(6, "T"),
            ph(7, "T"),
            ph(8, "T"),
            ph(9, "T"),
            ph(10, "MANUAL"),
            ph(11, "A"),
            ph(12, "A"),
            ph(13, "A"),
            ph(14, "GATE"),
        ]
    }

    /// Record a phase as passed now, the way a run would.
    fn pass(work: &str, n: u32) {
        let paths = Paths::new(work);
        let mut st = State::load(&paths.work, "xab").unwrap();
        let p = phases().into_iter().find(|p| p.n == n).unwrap();
        st.record(
            "r0",
            n,
            Outcome::Passed,
            Run::inputs(&paths, &p),
            Run::outputs(&paths, n),
            "",
        );
        st.save(&paths.work).unwrap();
    }

    fn write_outputs(work: &std::path::Path) {
        std::fs::write(work.join("ours.canonical.json"), "{}").unwrap();
        std::fs::write(work.join("keys.txt"), "xab=QUJD\n").unwrap();
        std::fs::write(work.join("agreement.json"), "{\"a\":1}").unwrap();
        std::fs::create_dir_all(work.join("pki")).unwrap();
        std::fs::write(work.join("pki/core.pem"), "c").unwrap();
        std::fs::create_dir_all(work.join("applied")).unwrap();
        std::fs::write(work.join("applied/x"), "r").unwrap();
    }

    fn begin(work: &str, from: u32, resume: bool) -> Run {
        Run::begin(
            "xab",
            work,
            &phases(),
            from,
            99,
            &Surface {
                resume,
                ..Default::default()
            },
            None,
        )
        .unwrap()
        .0
    }

    #[test]
    fn a_newer_journal_write_a_killed_run_left_is_never_read_and_is_removed() {
        // The journal records phase 3 as passed. A run killed mid-write left
        // a newer, complete journal in `.state.json.<pid>.tmp` that records
        // more phases as passed. A resumed run reads only `state.json`: it
        // skips phase 3 alone, and the temporary file is gone.
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        write_outputs(d.path());
        pass(&w, 3);
        let journal = std::fs::read(d.path().join("state.json")).unwrap();
        for n in [5, 6, 8, 9, 10] {
            pass(&w, n);
        }
        let newer = std::fs::read(d.path().join("state.json")).unwrap();
        std::fs::write(d.path().join("state.json"), &journal).unwrap();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let stale = format!(".state.json.{}.tmp", child.id());
        child.wait().unwrap();
        std::fs::write(d.path().join(&stale), &newer).unwrap();

        let (r, rows) = Run::begin(
            "xab",
            &w,
            &phases(),
            1,
            99,
            &Surface {
                resume: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let skipped: Vec<u32> = phases()
            .iter()
            .filter(|p| r.skips(p))
            .map(|p| p.n)
            .collect();
        assert_eq!(skipped, vec![3], "only what state.json records is skipped");
        assert!(!d.path().join(&stale).exists(), "{stale} is still there");
        // The run says what it removed, the way phase 8 reports a stale
        // `.mint-*` directory.
        assert!(
            rows.iter().any(|c| c.name == "journal"
                && c.detail
                    == format!("[removed] {stale}: a journal write left by an interrupted run")),
            "{:?}",
            rows.iter().map(|c| &c.detail).collect::<Vec<_>>()
        );
    }

    #[test]
    fn without_resume_nothing_is_skipped_and_every_run_is_recorded() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        write_outputs(d.path());
        for n in [3, 5, 6, 8, 9, 10] {
            pass(&w, n);
        }
        let mut r = begin(&w, 1, false);
        assert!(phases().iter().all(|p| !r.skips(p)));
        assert!(r.finish("passed").is_empty());
        drop(r);
        let st = State::load(d.path(), "xab").unwrap();
        assert_eq!(st.runs.len(), 1);
        assert!(!st.runs[0].resume);
        assert_eq!(st.runs[0].outcome, "passed");
    }

    #[test]
    fn resume_skips_only_what_it_can_see_is_unchanged() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        write_outputs(d.path());
        for n in [3, 5, 6, 8, 9, 10] {
            pass(&w, n);
        }
        let r = begin(&w, 1, true);
        let skipped: Vec<u32> = phases()
            .iter()
            .filter(|p| r.skips(p))
            .map(|p| p.n)
            .collect();
        assert_eq!(
            skipped,
            [3, 5, 6, 9, 10],
            "checks, minting and plays always run"
        );
        drop(r);

        // An edited agreement: compose's output changed, so 6 runs and so
        // does every later phase that leaves files.
        std::fs::write(d.path().join("agreement.json"), "{\"a\":2}").unwrap();
        let r = begin(&w, 1, true);
        let skipped: Vec<u32> = phases()
            .iter()
            .filter(|p| r.skips(p))
            .map(|p| p.n)
            .collect();
        assert_eq!(skipped, [3, 5]);
        drop(r);

        // A removed output is a change too.
        pass(&w, 6);
        std::fs::remove_file(d.path().join("ours.canonical.json")).unwrap();
        let r = begin(&w, 1, true);
        assert!(
            phases().iter().all(|p| !r.skips(p)),
            "phase 3's output is gone: everything runs"
        );
    }

    #[test]
    fn a_recorded_rotation_makes_phase_3_and_every_later_phase_run_again() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        write_outputs(d.path());
        for n in [3, 5, 6, 8, 9, 10] {
            pass(&w, n);
        }
        // No rotation recorded: phase 3 is skipped as before, so a journal
        // written before rotation.json existed keeps its digest.
        let r = begin(&w, 1, true);
        assert!(phases().iter().any(|p| p.n == 3 && r.skips(p)));
        drop(r);
        // A rotation names a new key: our package must be measured again.
        std::fs::write(d.path().join("rotation.json"), r#"{"key":"transport","nonce":"1"}"#).unwrap();
        let r = begin(&w, 1, true);
        assert!(
            phases().iter().all(|p| !r.skips(p)),
            "phase 3 and every phase after it run again"
        );
    }

    #[test]
    fn an_outcome_a_later_goatbuilder_wrote_is_never_skipped() {
        // Phase 6 holds a word this build does not know, with its inputs and
        // outputs unchanged: it is not a pass, so the run starts there.
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        write_outputs(d.path());
        for n in [3, 5, 6, 8, 9, 10] {
            pass(&w, n);
        }
        let paths = Paths::new(&w);
        let mut st = State::load(&paths.work, "xab").unwrap();
        let p6 = phases().into_iter().find(|p| p.n == 6).unwrap();
        st.record("r0", 6, Outcome::Later("deferred".into()), Run::inputs(&paths, &p6), Run::outputs(&paths, 6), "");
        st.save(&paths.work).unwrap();
        let r = begin(&w, 1, true);
        let skipped: Vec<u32> = phases().iter().filter(|p| r.skips(p)).map(|p| p.n).collect();
        assert_eq!(skipped, [3, 5]);
    }

    #[test]
    fn a_failed_phase_is_where_a_resumed_run_starts() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        write_outputs(d.path());
        for n in [3, 5, 6, 8, 9] {
            pass(&w, n);
        }
        let mut st = State::load(d.path(), "xab").unwrap();
        st.record(
            "r1",
            8,
            Outcome::Failed,
            "x".into(),
            "y".into(),
            "GB-183: phase 8 failed with exit 1",
        );
        st.save(d.path()).unwrap();
        let r = begin(&w, 1, true);
        let skipped: Vec<u32> = phases()
            .iter()
            .filter(|p| r.skips(p))
            .map(|p| p.n)
            .collect();
        // 9 reads only the agreement, unchanged, so it is still skipped; 8
        // runs again because its record is a failure.
        assert_eq!(skipped, [3, 5, 6, 9]);
    }

    #[test]
    fn a_by_hand_phase_is_done_only_when_declared_and_while_its_inputs_hold() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        write_outputs(d.path());
        for n in [3, 5, 6, 8, 9] {
            pass(&w, n);
        }
        // Not declared: the resumed run stops at 10 again.
        let r = begin(&w, 1, true);
        assert!(!r.skips(&ph(10, "MANUAL")));
        drop(r);
        // `--from 11` without the stop at 10 in the journal declares nothing.
        drop(begin(&w, 11, false));
        assert!(State::load(d.path(), "xab")
            .unwrap()
            .confirmations
            .is_empty());
        // After the stop, `--from 11` declares it, bound to what it read.
        let mut st = State::load(d.path(), "xab").unwrap();
        st.record(
            "r1",
            10,
            Outcome::Stopped,
            Run::inputs(&Paths::new(&w), &ph(10, "MANUAL")),
            String::new(),
            "waiting for the operator step",
        );
        st.save(d.path()).unwrap();
        drop(begin(&w, 11, false));
        let st = State::load(d.path(), "xab").unwrap();
        let c = st
            .confirmations
            .iter()
            .find(|c| c.phase == 10)
            .expect("recorded");
        assert_eq!((c.kind.as_str(), c.how.as_str()), ("operator-step", "flag"));
        let r = begin(&w, 1, true);
        assert!(r.skips(&ph(10, "MANUAL")));
        drop(r);
        // New rendered files: the by-hand review has to be done again.
        std::fs::write(d.path().join("applied/x"), "changed").unwrap();
        pass(&w, 9);
        let r = begin(&w, 1, true);
        assert!(r.skips(&ph(9, "T")) && !r.skips(&ph(10, "MANUAL")));
    }

    #[test]
    fn a_by_hand_phase_whose_inputs_changed_after_the_stop_is_not_declared() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        write_outputs(d.path());
        let mut st = State::load(d.path(), "xab").unwrap();
        let p10 = ph(10, "MANUAL");
        st.record(
            "r1",
            10,
            Outcome::Stopped,
            Run::inputs(&Paths::new(&w), &p10),
            String::new(),
            "waiting",
        );
        st.save(d.path()).unwrap();
        // New rendered files after the stop: the review was of the old ones.
        std::fs::write(d.path().join("applied/x"), "re-rendered").unwrap();
        let (r, rows) =
            Run::begin("xab", &w, &phases(), 11, 99, &Surface::default(), None).unwrap();
        drop(r);
        assert!(
            rows.iter()
                .any(|c| c.detail.contains("not recorded as done")),
            "{rows:?}"
        );
        let st = State::load(d.path(), "xab").unwrap();
        assert_eq!(st.phases[&10].outcome, Outcome::Stopped);
        assert!(st.confirmations.is_empty());
    }

    #[test]
    fn a_retry_after_a_failure_does_not_declare_the_by_hand_phase_before_it() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        write_outputs(d.path());
        let mut st = State::load(d.path(), "xab").unwrap();
        st.record(
            "r1",
            5,
            Outcome::Passed,
            "old".into(),
            "old".into(),
            "done by hand",
        );
        st.record("r1", 6, Outcome::Failed, "x".into(), "y".into(), "GB-183");
        st.save(d.path()).unwrap();
        drop(begin(&w, 6, false));
        let st = State::load(d.path(), "xab").unwrap();
        assert_eq!(
            st.phases[&5].inputs, "old",
            "phase 5's record is left as it was"
        );
        assert!(st.confirmations.is_empty());
    }

    #[test]
    fn a_run_that_returns_early_is_closed_as_failed() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        drop(begin(&w, 1, false));
        let st = State::load(d.path(), "xab").unwrap();
        assert_eq!(st.runs[0].outcome, "failed");
        assert!(!st.runs[0].ended.is_empty());
    }

    #[test]
    fn resume_with_from_starts_where_the_operator_says() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        write_outputs(d.path());
        pass(&w, 3);
        let r = begin(&w, 3, true);
        assert!(
            !r.skips(&ph(3, "T")),
            "--from names the start; nothing is skipped"
        );
    }

    #[test]
    fn two_runs_of_one_peering_never_overlap() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().display().to_string();
        let held = begin(&w, 1, false);
        let e = Run::begin("xab", &w, &phases(), 1, 99, &Surface::default(), None)
            .err()
            .expect("refused while the first run holds the lock");
        assert!(
            e.downcast_ref::<goat_errors::Coded>()
                .is_some_and(|c| c.code == "GB-180"),
            "{e}"
        );
        drop(held);
    }
}
