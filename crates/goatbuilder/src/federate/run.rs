//! One real run of `goatbuilder federate`, recorded in the peering's journal
//! (state.rs) under the per-peering lock (lock.rs), and, with `--resume`,
//! resumed from what is actually there.
//!
//! Every real run is recorded: the run, each phase attempt and outcome, and a
//! by-hand phase declared done with `--from`. Recording changes no phase.
//!
//! `--resume` (off unless given) skips a phase only when all of these hold:
//!
//! - the phase is one of 3, 5, 6, 9 or 10: it leaves local files that are
//!   a pure product of what it reads (6 the agreement, 9 the rendered files),
//!   or it is a by-hand phase (5, 10), or it is phase 3, whose output is our
//!   half as already sent to the partner (re-measuring it makes a new half
//!   that needs a new exchange, so that is asked for with `--from 3`; its
//!   digest covers the command and the signing key's bytes);
//! - its journal record says it passed;
//! - the digest of its command and of the files it reads, measured now, is
//!   the recorded one; and the digest of the files it left, measured now, is
//!   the recorded one (a missing or edited output is a change);
//! - no earlier phase of that kind had to run again. The first one that does
//!   is where the run starts, and every such phase after it runs too.
//!
//! Everything else always runs: the gates (1, 14) are live checks, the lint
//! and verify phases (4, 7) read again what is there, minting (8) checks the
//! leaves' expiry and the CA they chain to and renews them when needed (a
//! renewed leaf changes phase 10's inputs, so the by-hand review stops again),
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

use super::lock::PeeringLock;
use super::state::{self, Outcome, State};
use super::Phase;
use crate::Check;

/// What a run is asked to do beyond running the phases.
#[derive(Default)]
pub struct Surface {
    /// `--resume`: skip phases found done (see above).
    pub resume: bool,
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
}

fn env_or(name: &str, default: String) -> String {
    std::env::var(name)
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
            pki: w.join("pki"),
            signing: std::env::var("FED_SIGNING_KEY")
                .ok()
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            work: w,
        }
    }

    /// What phase 8 minted: every file in pki/ except the partner's CA,
    /// which phase 10 places there.
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
            3 => (self.signing.iter().cloned().collect(), vec![o]),
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
        stage: &str,
        s: &Surface,
    ) -> Result<(Run, Vec<Check>)> {
        let paths = Paths::new(work);
        let lock = PeeringLock::take(&paths.work, with)?;
        let mut state = State::load(&paths.work, with)?;
        let resume = s.resume && from <= 1;
        let id = state.open_run(stage, from, to, resume, "");
        let mut rows = Vec::new();
        if s.resume && from > 1 {
            rows.push(row(
                "resume",
                format!("--resume skips nothing when --from names the start; phases {from} to {to} all run"),
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

        let mut skip = BTreeSet::new();
        if resume {
            for p in phases.iter().filter(|p| p.n >= from && p.n <= to) {
                if !skippable(p) {
                    continue;
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
        state.save(&paths.work)?;
        Ok((
            Run {
                _lock: lock,
                state,
                paths,
                id,
                skip,
                finished: false,
                save_errors: Vec::new(),
            },
            rows,
        ))
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
        self.state
            .record(&id, p.n, outcome, inputs, outputs, detail);
        self.save();
    }

    /// Close the run in the journal with its outcome word. Returns a failed
    /// row for each journal write that did not land, so `-o json` shows it.
    pub(super) fn finish(&mut self, outcome: &str) -> Vec<Check> {
        let id = self.id.clone();
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
            "federate",
            &Surface { resume },
        )
        .unwrap()
        .0
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
        let (r, rows) = Run::begin(
            "xab",
            &w,
            &phases(),
            11,
            99,
            "federate",
            &Surface::default(),
        )
        .unwrap();
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
        let e = Run::begin("xab", &w, &phases(), 1, 99, "federate", &Surface::default())
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
