//! The run journal of one peering: `<work>/state.json`.
//!
//! Three things are kept apart, as the plan-and-state design asks:
//!
//! - intent: the `peerings:` declaration and the verified agreement, which
//!   live in the deployment's configuration and `<work>/agreement.json`;
//! - progress: this file. It is versioned (`schema`) and holds every run, every
//!   phase attempt, the last outcome of each phase with the digest of what it
//!   read and what it left, every value a person confirmed (and how), and the
//!   resources this peering owns on shared hosts;
//! - evidence: `<work>/evidence/`, written by acceptance. This file only points
//!   at it.
//!
//! The journal never decides on its own that a phase is done. A phase can be
//! skipped on a resumed run only when its record says it passed AND what it
//! read and what it left, measured again now, are what it recorded (run.rs).
//!
//! The file is written whole, to a temporary file (`.state.json.<pid>.tmp`)
//! and then renamed, so a run killed mid-write leaves the previous journal,
//! never half of one. The temporary file it leaves is never read as the
//! journal, and the next run removes it once it holds the peering lock
//! (`sweep_stale_temp`). A journal
//! that does not parse, or one written by a newer schema, is refused (GB-182),
//! never overwritten: it is the only account of what ran.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA: &str = "goatbuilder.federate.state/v1";
pub const FILE: &str = "state.json";
/// The journal keeps this many most recent attempts; older ones are dropped.
/// Runs, confirmations, owned resources and each phase's last record are kept.
const ATTEMPTS_MAX: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The phase finished and its outputs are in place.
    Passed,
    /// The phase ran and failed; the run stopped there.
    Failed,
    /// The phase is waiting for a person: an operator step, or a confirmation
    /// not given yet.
    Stopped,
    /// A word a later goatbuilder wrote, kept as written. It is never a
    /// pass, so a resumed run runs the phase again, and a rewrite keeps it.
    Later(String),
}

impl Outcome {
    pub fn word(&self) -> &str {
        match self {
            Outcome::Passed => "passed",
            Outcome::Failed => "failed",
            Outcome::Stopped => "stopped",
            Outcome::Later(w) => w,
        }
    }

    /// Whether this build knows the word.
    pub fn known(&self) -> bool {
        !matches!(self, Outcome::Later(_))
    }
}

impl Serialize for Outcome {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.word())
    }
}

impl<'de> Deserialize<'de> for Outcome {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = String::deserialize(d)?;
        Ok(match w.as_str() {
            "passed" => Outcome::Passed,
            "failed" => Outcome::Failed,
            "stopped" => Outcome::Stopped,
            _ => Outcome::Later(w),
        })
    }
}

/// Fields a later goatbuilder wrote that this build does not know: kept, and
/// written back unchanged.
pub type Extra = BTreeMap<String, serde_json::Value>;

/// One invocation that executed phases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEntry {
    /// `run-<UTC time>-<pid>`; unique on one machine.
    pub id: String,
    /// `federate` (the whole procedure), `deploy` or `test`.
    pub stage: String,
    pub started: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ended: String,
    pub from: u32,
    pub to: u32,
    /// Whether this run skipped phases it found done (`--resume`).
    #[serde(default)]
    pub resume: bool,
    /// The plan this run executed, by hash, when it executed one (deploy).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plan: String,
    /// `passed`, `failed`, `stopped`, or empty while running (or when the
    /// run was killed: an empty `ended` on an older run means exactly that).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub outcome: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// One phase attempt, or one event inside a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub at: String,
    pub run: String,
    pub phase: u32,
    /// `start`, `passed`, `failed`, `stopped`, `skipped`, `confirmed`,
    /// `claimed` or `released`.
    pub event: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// The last outcome of one phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseRecord {
    pub outcome: Outcome,
    /// `sha256:` over the phase's command and the files it reads.
    pub inputs: String,
    /// `sha256:` over the files it leaves; empty when it leaves none.
    #[serde(default)]
    pub outputs: String,
    /// When this outcome was recorded (UTC, RFC 3339).
    pub at: String,
    pub run: String,
    #[serde(default)]
    pub attempts: u32,
    /// One line on why it stopped or failed, with its catalog code.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A value a person confirmed, how, and what it is bound to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Confirmation {
    /// `operator-step` (a by-hand phase declared done with `--from`),
    /// `plan`, `keys`, `agreement` or `admission`.
    pub kind: String,
    pub phase: u32,
    /// `terminal` (typed by a person) or `flag` (given on the command line).
    pub how: String,
    pub at: String,
    pub run: String,
    /// The digest this confirmation is valid for: the phase's inputs, the
    /// plan hash, or the admission digest. Anything else is a new question.
    pub bound_to: String,
    /// The values confirmed (keys, fingerprints, digests).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A resource on a shared host that this peering owns, so a second peering
/// on the same host never takes it, and backing this peering out knows what
/// is its own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Owned {
    /// `wg-interface`, `udp-port`, `tcp-port`, `link-subnet`, `cert-dir`, ...
    pub kind: String,
    /// The host or inventory group the resource lives on.
    pub scope: String,
    pub value: String,
    /// `claimed` (reserved before a mutation), `deployed`, or `released`.
    pub status: String,
    pub run: String,
    pub at: String,
    #[serde(flatten)]
    pub extra: Extra,
}

#[cfg_attr(not(test), allow(dead_code))]
impl Owned {
    /// Whether two resources are the same thing on the same host.
    pub fn same(&self, kind: &str, scope: &str, value: &str) -> bool {
        self.kind == kind && self.scope == scope && self.value == value
    }

    pub fn held(&self) -> bool {
        self.status != "released"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub schema: String,
    pub peering: String,
    pub updated: String,
    #[serde(default)]
    pub runs: Vec<RunEntry>,
    /// The last outcome of each phase, by number.
    #[serde(default)]
    pub phases: BTreeMap<u32, PhaseRecord>,
    #[serde(default)]
    pub confirmations: Vec<Confirmation>,
    #[serde(default)]
    pub owned: Vec<Owned>,
    #[serde(default)]
    pub attempts: Vec<Attempt>,
    /// The goatbuilder that wrote this file last (`goatbuilder <version>`),
    /// so a build that cannot read it can say which one did.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub written_by: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `written_by` after a save: the latest goatbuilder that has written the
/// journal. A later one that wrote it before stays named, so the journal still
/// says which build's words it may hold; otherwise this build.
fn latest_writer(recorded: &str, ours: &str) -> String {
    let version = |w: &str| -> Option<Vec<u64>> {
        let v = w.strip_prefix("goatbuilder ")?.split(['-', '+']).next()?;
        v.split('.').map(|n| n.parse().ok()).collect()
    };
    let mine = format!("goatbuilder {ours}");
    match (version(recorded), version(&mine)) {
        (Some(r), Some(m)) if r > m => recorded.to_string(),
        _ => mine,
    }
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

impl State {
    pub fn new(peering: &str) -> Self {
        State {
            schema: SCHEMA.into(),
            peering: peering.into(),
            updated: now(),
            runs: Vec::new(),
            phases: BTreeMap::new(),
            confirmations: Vec::new(),
            owned: Vec::new(),
            attempts: Vec::new(),
            written_by: String::new(),
            extra: Extra::new(),
        }
    }

    pub fn path(work: &Path) -> PathBuf {
        work.join(FILE)
    }

    /// The journal in `work`, or `None` when there is none.
    pub fn read(work: &Path) -> Result<Option<Self>> {
        let p = Self::path(work);
        let text = match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(goat_errors::coded(
                    "GB-182",
                    format!("{}: cannot read the run journal ({e})", p.display()),
                )
                .into())
            }
        };
        let parse = |e: serde_json::Error| {
            goat_errors::coded(
                "GB-182",
                format!("{}: the run journal does not parse ({e})", p.display()),
            )
        };
        // The schema first: a later schema is refused by name, whatever its
        // shape. Within v1, a value or field this build does not know is kept.
        let v: serde_json::Value = serde_json::from_str(&text).map_err(parse)?;
        let schema = v["schema"].as_str().unwrap_or_default();
        if schema != SCHEMA {
            let by = v["written_by"]
                .as_str()
                .filter(|w| !w.is_empty())
                .map(|w| format!(", written by {w}"))
                .unwrap_or_default();
            return Err(goat_errors::coded(
                "GB-182",
                format!(
                    "{}: the run journal is {schema}{by}, and this goatbuilder reads {SCHEMA}: run that goatbuilder, or a later one, on this peering",
                    p.display()
                ),
            )
            .into());
        }
        let s: State = serde_json::from_value(v).map_err(parse)?;
        Ok(Some(s))
    }

    /// The journal of `peering` in `work`, or a fresh one when there is none.
    pub fn load(work: &Path, peering: &str) -> Result<Self> {
        let Some(s) = Self::read(work)? else {
            return Ok(State::new(peering));
        };
        if s.peering != peering {
            return Err(goat_errors::coded(
                "GB-182",
                format!(
                    "{}: the run journal is for peering '{}', not '{peering}'; each peering needs its own working directory",
                    Self::path(work).display(),
                    s.peering
                ),
            )
            .into());
        }
        Ok(s)
    }

    /// Write the journal whole: to a temporary file, then rename. Called only
    /// by a run that holds the peering lock (lock.rs).
    pub fn save(&mut self, work: &Path) -> Result<()> {
        self.updated = now();
        self.written_by = latest_writer(&self.written_by, env!("CARGO_PKG_VERSION"));
        if self.attempts.len() > ATTEMPTS_MAX {
            let drop = self.attempts.len() - ATTEMPTS_MAX;
            self.attempts.drain(..drop);
        }
        let io = |what: String, e: std::io::Error| -> anyhow::Error {
            goat_errors::coded("GB-184", format!("{what}: {e}")).into()
        };
        std::fs::create_dir_all(work).map_err(|e| {
            io(
                format!("cannot create the working folder {}", work.display()),
                e,
            )
        })?;
        let p = Self::path(work);
        let tmp = work.join(temp_name(std::process::id()));
        let mut f = File::create(&tmp)
            .map_err(|e| io(format!("cannot write the run journal {}", tmp.display()), e))?;
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        f.write_all(text.as_bytes())
            .map_err(|e| io(format!("cannot write the run journal {}", tmp.display()), e))?;
        f.sync_all().ok();
        std::fs::rename(&tmp, &p).map_err(|e| {
            io(
                format!("cannot rename {} to {}", tmp.display(), p.display()),
                e,
            )
        })?;
        Ok(())
    }

    /// Open a run; returns its id.
    pub fn open_run(
        &mut self,
        stage: &str,
        from: u32,
        to: u32,
        resume: bool,
        plan: &str,
    ) -> String {
        let base = format!(
            "run-{}-{}",
            chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
            std::process::id()
        );
        let mut id = base.clone();
        let mut k = 1;
        while self.runs.iter().any(|r| r.id == id) {
            k += 1;
            id = format!("{base}-{k}");
        }
        self.runs.push(RunEntry {
            id: id.clone(),
            stage: stage.into(),
            started: now(),
            ended: String::new(),
            from,
            to,
            resume,
            plan: plan.into(),
            outcome: String::new(),
            extra: Extra::new(),
        });
        id
    }

    pub fn close_run(&mut self, run: &str, outcome: &str) {
        if let Some(r) = self.runs.iter_mut().find(|r| r.id == run) {
            r.ended = now();
            r.outcome = outcome.into();
        }
    }

    pub fn log(&mut self, run: &str, phase: u32, event: &str, detail: impl Into<String>) {
        self.attempts.push(Attempt {
            at: now(),
            run: run.into(),
            phase,
            event: event.into(),
            detail: detail.into(),
            extra: Extra::new(),
        });
    }

    pub fn record(
        &mut self,
        run: &str,
        phase: u32,
        outcome: Outcome,
        inputs: String,
        outputs: String,
        detail: impl Into<String>,
    ) {
        let detail = detail.into();
        let attempts = self.phases.get(&phase).map(|r| r.attempts).unwrap_or(0) + 1;
        self.log(run, phase, outcome.word(), detail.clone());
        self.phases.insert(
            phase,
            PhaseRecord {
                outcome,
                inputs,
                outputs,
                at: now(),
                run: run.into(),
                attempts,
                detail,
                extra: Extra::new(),
            },
        );
    }

    /// Record a confirmation, replacing an earlier one of the same kind and
    /// phase: the newest answer is the one in force.
    pub fn confirm(
        &mut self,
        run: &str,
        phase: u32,
        kind: &str,
        how: &str,
        bound_to: &str,
        values: BTreeMap<String, String>,
    ) {
        self.confirmations
            .retain(|c| !(c.kind == kind && c.phase == phase));
        self.confirmations.push(Confirmation {
            kind: kind.into(),
            phase,
            how: how.into(),
            at: now(),
            run: run.into(),
            bound_to: bound_to.into(),
            values,
            extra: Extra::new(),
        });
        self.log(
            run,
            phase,
            "confirmed",
            format!("{kind} ({how}) for {bound_to}"),
        );
    }

    /// The confirmation of `kind` at `phase` that is bound to `bound_to`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn confirmed(&self, kind: &str, phase: u32, bound_to: &str) -> Option<&Confirmation> {
        self.confirmations
            .iter()
            .find(|c| c.kind == kind && c.phase == phase && c.bound_to == bound_to)
    }

    /// Mark a resource as this peering's, or update its status.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn own(&mut self, run: &str, kind: &str, scope: &str, value: &str, status: &str) {
        match self.owned.iter_mut().find(|o| o.same(kind, scope, value)) {
            Some(o) => {
                o.status = status.into();
                o.run = run.into();
                o.at = now();
            }
            None => self.owned.push(Owned {
                kind: kind.into(),
                scope: scope.into(),
                value: value.into(),
                status: status.into(),
                run: run.into(),
                at: now(),
                extra: Extra::new(),
            }),
        }
        self.log(run, 0, status, format!("{kind} {value} on {scope}"));
    }
}

/// The temporary file `save` writes before its rename: `.state.json.<pid>.tmp`.
fn temp_name(pid: u32) -> String {
    format!(".{FILE}.{pid}.tmp")
}

/// Whether `name` is a temporary file `save` writes: `.state.json.<digits>.tmp`.
fn is_temp(name: &str) -> bool {
    name.strip_prefix(&format!(".{FILE}."))
        .and_then(|rest| rest.strip_suffix(".tmp"))
        .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
}

/// Remove what interrupted journal writes left in `work`, and return the
/// names removed.
///
/// `save` writes `.state.json.<pid>.tmp` and renames it over the journal; a
/// run killed between the two (kill -9, a lost host) leaves the temporary
/// file behind. Only `save` creates one, and only a run that holds the
/// peering lock calls `save`; the kernel drops that lock when its holder
/// dies. Called with the lock held, so every such file seen here belongs to
/// a run that is no longer alive, whatever its process id, and none is a
/// write in progress. The journal, the lock file and every other name are
/// never touched. A symlink of that name is unlinked, never followed; a
/// directory of that name is not something `save` writes and is left alone.
/// A file that cannot be removed stops the run (GB-184).
pub fn sweep_stale_temp(work: &Path) -> Result<Vec<String>> {
    let io = |what: String, e: std::io::Error| -> anyhow::Error {
        goat_errors::coded("GB-184", format!("{what}: {e}")).into()
    };
    let entries = std::fs::read_dir(work).map_err(|e| {
        io(
            format!("cannot read the working folder {}", work.display()),
            e,
        )
    })?;
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| is_temp(n))
        .collect();
    names.sort();
    let mut removed = Vec::new();
    for name in names {
        let p = work.join(&name);
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.is_dir() => continue,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io(format!("cannot inspect {}", p.display()), e)),
        }
        std::fs::remove_file(&p).map_err(|e| {
            io(
                format!(
                    "cannot remove {}, a journal write left by an interrupted run",
                    p.display()
                ),
                e,
            )
        })?;
        removed.push(name);
    }
    Ok(removed)
}

/// `sha256:` over labeled parts, each length-prefixed so two different
/// splits of the same bytes never give the same digest.
pub fn digest(parts: &[(&str, &[u8])]) -> String {
    let mut h = Sha256::new();
    for (label, bytes) in parts {
        h.update((label.len() as u64).to_be_bytes());
        h.update(label.as_bytes());
        h.update((bytes.len() as u64).to_be_bytes());
        h.update(bytes);
    }
    format!("sha256:{:x}", h.finalize())
}

/// One file or directory as digest parts. A file is its mode and bytes; a
/// directory is every entry under it in path order; a symbolic link is the
/// path it points at (never followed, so a loop cannot recurse); an absent
/// path is the word `absent` and an unreadable one `unreadable:<kind>`.
fn path_parts(p: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    let label = p.display().to_string();
    let meta = match std::fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            out.push((label, b"absent".to_vec()));
            return;
        }
        Err(e) => {
            out.push((label, format!("unreadable:{:?}", e.kind()).into_bytes()));
            return;
        }
    };
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(p)
            .map(|t| t.display().to_string())
            .unwrap_or_default();
        out.push((label, format!("symlink:{target}").into_bytes()));
    } else if meta.is_dir() {
        let mut entries: Vec<PathBuf> = match std::fs::read_dir(p) {
            Ok(rd) => rd.flatten().map(|e| e.path()).collect(),
            Err(e) => {
                out.push((label, format!("unreadable:{:?}", e.kind()).into_bytes()));
                return;
            }
        };
        entries.sort();
        if entries.is_empty() {
            out.push((format!("{label}/"), b"empty".to_vec()));
        }
        for e in entries {
            path_parts(&e, out);
        }
    } else {
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode()
        };
        #[cfg(not(unix))]
        let mode = 0u32;
        match std::fs::read(p) {
            Ok(b) => {
                out.push((format!("{label}#mode"), mode.to_be_bytes().to_vec()));
                out.push((label, b));
            }
            Err(e) => out.push((label, format!("unreadable:{:?}", e.kind()).into_bytes())),
        }
    }
}

/// The digest of a command and the files a phase reads.
pub fn digest_of(cmd: &str, paths: &[PathBuf]) -> String {
    let mut owned: Vec<(String, Vec<u8>)> = vec![("cmd".into(), cmd.as_bytes().to_vec())];
    for p in paths {
        path_parts(p, &mut owned);
    }
    let parts: Vec<(&str, &[u8])> = owned
        .iter()
        .map(|(l, b)| (l.as_str(), b.as_slice()))
        .collect();
    digest(&parts)
}

/// The outputs digest; empty when the phase leaves no local file. An output
/// that is missing makes the digest differ from the recorded one.
pub fn outputs_of(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    digest_of("", paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_journal_round_trips_and_refuses_another_peering() {
        let d = tempfile::tempdir().unwrap();
        let mut s = State::new("xab");
        let run = s.open_run("federate", 1, 99, false, "");
        s.record(
            &run,
            3,
            Outcome::Passed,
            "sha256:a".into(),
            "sha256:b".into(),
            "",
        );
        s.confirm(
            &run,
            10,
            "operator-step",
            "flag",
            "sha256:i",
            BTreeMap::new(),
        );
        s.own(&run, "udp-port", "xab_edge", "51821", "claimed");
        s.close_run(&run, "passed");
        s.save(d.path()).unwrap();
        let back = State::load(d.path(), "xab").unwrap();
        assert_eq!(back.phases[&3].outcome, Outcome::Passed);
        assert_eq!(back.phases[&3].attempts, 1);
        assert_eq!(back.phases[&3].run, run);
        assert!(back.confirmed("operator-step", 10, "sha256:i").is_some());
        assert!(back
            .confirmed("operator-step", 10, "sha256:other")
            .is_none());
        assert_eq!(back.owned.len(), 1);
        assert_eq!(back.runs[0].outcome, "passed");
        let e = State::load(d.path(), "xaa").unwrap_err();
        assert!(
            e.downcast_ref::<goat_errors::Coded>()
                .is_some_and(|c| c.code == "GB-182"),
            "{e}"
        );
    }

    #[test]
    fn an_unreadable_or_newer_journal_is_refused_not_overwritten() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(FILE), "{not json").unwrap();
        let e = State::load(d.path(), "xab").unwrap_err();
        assert!(e.to_string().contains("does not parse"), "{e}");
        assert_eq!(
            std::fs::read_to_string(d.path().join(FILE)).unwrap(),
            "{not json"
        );
        let newer = r#"{"schema":"goatbuilder.federate.state/v9","peering":"xab","updated":""}"#;
        std::fs::write(d.path().join(FILE), newer).unwrap();
        let e = State::load(d.path(), "xab").unwrap_err();
        assert!(e.to_string().contains("this goatbuilder reads"), "{e}");
    }

    /// A v1 journal written by a later goatbuilder: an outcome word and fields
    /// this build does not know.
    const LATER: &str = r#"{
  "schema": "goatbuilder.federate.state/v1",
  "peering": "xab",
  "updated": "2026-11-12T00:00:00Z",
  "written_by": "goatbuilder 0.7.0",
  "runs": [{"id": "run-1", "stage": "federate", "started": "s", "ended": "e", "from": 1, "to": 14, "resume": false, "outcome": "deferred", "budget": 3}],
  "phases": {
    "13": {"outcome": "passed", "inputs": "sha256:i13", "outputs": "", "at": "a", "run": "run-1", "attempts": 1},
    "14": {"outcome": "deferred", "inputs": "sha256:i14", "outputs": "", "at": "a", "run": "run-1", "attempts": 1, "detail": "GB-299: later", "session": "ses-1"}
  },
  "confirmations": [{"kind": "partner-package", "phase": 5, "how": "flag", "at": "a", "run": "run-1", "bound_to": "sha256:p", "witness": "x"}],
  "owned": [{"kind": "udp-port", "scope": "xab_edge", "value": "51821", "status": "parked", "run": "run-1", "at": "a", "host_note": "n"}],
  "attempts": [{"at": "a", "run": "run-1", "phase": 14, "event": "deferred", "why": "w"}]
}"#;

    #[test]
    fn a_journal_from_a_later_goatbuilder_is_read_with_its_unknown_values_as_written() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(FILE), LATER).unwrap();
        let s = State::load(d.path(), "xab").unwrap();
        assert_eq!(s.phases[&13].outcome, Outcome::Passed);
        let o = &s.phases[&14].outcome;
        assert_eq!(o.word(), "deferred");
        assert!(!o.known());
        // An unknown outcome is never a pass.
        assert_ne!(*o, Outcome::Passed);
    }

    #[test]
    fn a_rewrite_keeps_unknown_values_and_fields_untouched() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(FILE), LATER).unwrap();
        let mut s = State::load(d.path(), "xab").unwrap();
        let run = s.open_run("federate", 13, 13, false, "");
        s.record(&run, 13, Outcome::Passed, "sha256:i13".into(), "".into(), "");
        s.close_run(&run, "passed");
        s.save(d.path()).unwrap();
        let before: serde_json::Value = serde_json::from_str(LATER).unwrap();
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(d.path().join(FILE)).unwrap()).unwrap();
        // Phase 14, which this run did not touch, is kept whole.
        assert_eq!(after["phases"]["14"], before["phases"]["14"]);
        // Unknown fields everywhere are kept.
        assert_eq!(after["runs"][0], before["runs"][0]);
        assert_eq!(after["confirmations"][0], before["confirmations"][0]);
        assert_eq!(after["owned"][0], before["owned"][0]);
        assert_eq!(after["attempts"][0], before["attempts"][0]);
        // written_by keeps the latest goatbuilder that wrote the file.
        assert_eq!(after["written_by"], "goatbuilder 0.7.0");
        assert_eq!(latest_writer("", "0.6.0"), "goatbuilder 0.6.0");
        assert_eq!(latest_writer("goatbuilder 0.5.1", "0.6.0"), "goatbuilder 0.6.0");
        assert_eq!(latest_writer("goatbuilder 9.9.0", "0.6.0"), "goatbuilder 9.9.0");
        assert_eq!(latest_writer("goatbuilder 0.10.0-rc.1", "0.6.0"), "goatbuilder 0.10.0-rc.1");
    }

    #[test]
    fn only_a_later_schema_is_refused_and_it_names_the_writer() {
        let d = tempfile::tempdir().unwrap();
        let v2 = r#"{"schema":"goatbuilder.federate.state/v2","peering":"xab","updated":"","written_by":"goatbuilder 0.9.0"}"#;
        std::fs::write(d.path().join(FILE), v2).unwrap();
        let e = State::load(d.path(), "xab").unwrap_err();
        let t = e.to_string();
        assert!(t.contains("goatbuilder 0.9.0") && t.contains("v2"), "{t}");
        assert!(
            e.downcast_ref::<goat_errors::Coded>()
                .is_some_and(|c| c.code == "GB-182"),
            "{e}"
        );
        assert_eq!(std::fs::read_to_string(d.path().join(FILE)).unwrap(), v2);
    }

    #[test]
    fn a_confirmation_is_replaced_and_a_resource_updated_in_place() {
        let mut s = State::new("xab");
        s.confirm(
            "r1",
            10,
            "operator-step",
            "flag",
            "sha256:a",
            BTreeMap::new(),
        );
        s.confirm(
            "r2",
            10,
            "operator-step",
            "flag",
            "sha256:b",
            BTreeMap::new(),
        );
        assert_eq!(s.confirmations.len(), 1);
        assert!(s.confirmed("operator-step", 10, "sha256:b").is_some());
        s.own("r1", "wg-interface", "xab_edge", "wg-fed1", "claimed");
        s.own("r2", "wg-interface", "xab_edge", "wg-fed1", "deployed");
        assert_eq!(s.owned.len(), 1);
        assert_eq!(s.owned[0].status, "deployed");
        assert!(s.owned[0].held());
    }

    #[test]
    fn digests_change_with_inputs_and_with_a_missing_output() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("agreement.json");
        std::fs::write(&f, "{}").unwrap();
        let a = digest_of("cmd", std::slice::from_ref(&f));
        assert_eq!(a, digest_of("cmd", std::slice::from_ref(&f)), "stable");
        assert_ne!(
            a,
            digest_of("cmd2", std::slice::from_ref(&f)),
            "the command is an input"
        );
        std::fs::write(&f, "{ }").unwrap();
        assert_ne!(
            a,
            digest_of("cmd", std::slice::from_ref(&f)),
            "a changed file is a changed input"
        );
        let out = outputs_of(std::slice::from_ref(&f));
        std::fs::remove_file(&f).unwrap();
        assert_ne!(out, outputs_of(&[f]), "a removed output is a change");
        assert_eq!(outputs_of(&[]), "");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_loop_is_hashed_not_followed_and_a_mode_change_is_a_change() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("applied");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
        let key = dir.join("key.pem");
        std::fs::write(&key, "k").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        let a = outputs_of(std::slice::from_ref(&dir));
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_ne!(a, outputs_of(std::slice::from_ref(&dir)));
    }
}
