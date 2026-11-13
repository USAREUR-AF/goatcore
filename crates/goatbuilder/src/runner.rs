//! The one door every child process goes through (docs/design/goatbuilder-legibility.md,
//! step 2). Nothing else in goatbuilder calls `Command::new`; a test holds every other
//! file to that (tests/legibility_one_door.rs).
//!
//! [`Cmd`] builds a command the way `std::process::Command` does, and runs it three
//! ways: [`Cmd::status`] streams the child's output while it runs, [`Cmd::output`]
//! captures it, [`Cmd::spawn`] starts it in the background. Every way:
//!
//! - logs the command under `-v` (crate::trace);
//! - inside a run ([`begin`]), prints the frame on stderr BEFORE the child can
//!   print anything (the phase, what it is for, the exact command, the host, the log
//!   file), copies the child's output to a per-command log under
//!   `<state>/runs/<id>/logs/` while still streaming it, prints the outcome and the
//!   duration, and keeps a [`CommandEntry`] that the run record writes;
//! - outside a run (read-only verbs, unit tests), does exactly what `Command` does.
//!
//! The frame for a phase that succeeds is three lines:
//!
//! ```text
//! Phase 5 · mgmt-stack: site-bootstrap up to the account seed
//!   $ cd ops/ansible && ../scripts/with-aws default -- ...   (on alpha_hosts; log ~/.goat/state/runs/…/logs/003-5.log)
//!   ok in 12s
//! ```
//!
//! A command outside a phase (a probe such as the fresh-source check or a `git`
//! read) prints nothing when it succeeds: it is in the record, and logged under
//! `-v`. When it fails it prints one line, `· FAILED: <command>: exit 2 in 0.1s; log …`.
//!
//! To write the log while streaming, the child's stdout and stderr are pipes the
//! runner copies to the terminal and the log; stdin stays inherited, so a prompt still
//! works. A child therefore does not see a terminal on its output. With
//! `GOATBUILDER_CHILD_TTY=1` a streamed command inherits the terminal as before and
//! no log is written for it (the record says so).

// Step 2a builds the door; step 2b moves every call site to it and step 4 writes
// the record, which use the rest of this module. Removed with 2b.
#![allow(dead_code)]

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;

/// One process the tool started, as the run record keeps it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CommandEntry {
    /// The phase it ran in; `None` for a probe outside any phase.
    pub phase: Option<String>,
    /// The line as a shell reads it.
    pub command: String,
    pub argv: Vec<String>,
    /// Where it acts; `None` is this machine.
    pub host: Option<String>,
    /// The log of its output; `None` when it was not captured (see `log_note`).
    pub log: Option<String>,
    /// Why there is no log, when there is none.
    pub log_note: Option<String>,
    pub started: String,
    pub ended: Option<String>,
    /// Exit code; `None` while running, killed by a signal, or failed to start.
    pub exit: Option<i32>,
    pub secs: Option<f64>,
}

/// The phase the next commands belong to.
#[derive(Debug, Clone)]
struct Phase {
    id: String,
    name: String,
    purpose: String,
    /// Whether its header is printed yet.
    shown: bool,
}

/// One phase as the run record keeps it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PhaseEntry {
    pub id: String,
    pub name: String,
    pub purpose: String,
    pub started: String,
    pub ended: Option<String>,
    /// The first non-zero exit of its commands, else 0; `None` when it ran no
    /// command, or one that did not start.
    pub exit: Option<i32>,
    pub commands: usize,
}

/// A run as it ends: what the run record writes.
#[derive(Debug, Clone)]
pub struct Finished {
    pub dir: PathBuf,
    pub verb: String,
    pub args: Vec<String>,
    pub tree_commit: Option<String>,
    pub started: String,
    pub phases: Vec<PhaseEntry>,
    pub commands: Vec<CommandEntry>,
    pub inputs: serde_json::Value,
}

struct Run {
    dir: PathBuf,
    seq: u32,
    phase: Option<Phase>,
    commands: Vec<CommandEntry>,
    verb: String,
    args: Vec<String>,
    tree_commit: Option<String>,
    started: String,
    phases: Vec<PhaseEntry>,
    /// The index in `commands` where the open phase began.
    phase_from: usize,
    inputs: serde_json::Value,
    /// Unit tests share one process: a run belongs to the test thread that
    /// began it, so another test's command never joins it. A real run is the
    /// whole process, its worker threads included.
    #[cfg(test)]
    owner: std::thread::ThreadId,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Close the open phase entry, if any: its end, its exit, its command count.
fn close_phase(r: &mut Run) {
    if let Some(p) = r.phases.last_mut().filter(|p| p.ended.is_none()) {
        let cmds = &r.commands[r.phase_from..];
        p.ended = Some(now());
        p.commands = cmds.len();
        p.exit = if cmds.is_empty()
            || cmds
                .iter()
                .any(|c| c.exit.is_none() && c.log_note.as_deref() == Some("did not start"))
        {
            None
        } else {
            Some(
                cmds.iter()
                    .filter_map(|c| c.exit)
                    .find(|e| *e != 0)
                    .unwrap_or(0),
            )
        };
    }
}

static RUN: Mutex<Option<Run>> = Mutex::new(None);
/// Where the frame goes: stderr, or a file a test reads.
static SINK: Mutex<Option<PathBuf>> = Mutex::new(None);

fn lock_run() -> std::sync::MutexGuard<'static, Option<Run>> {
    RUN.lock().unwrap_or_else(|e| e.into_inner())
}

fn say(line: &str) {
    let sink = SINK.lock().unwrap_or_else(|e| e.into_inner());
    match sink.as_ref() {
        Some(path) => {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(f, "{line}");
            }
        }
        None => {
            let _ = writeln!(std::io::stderr(), "{line}");
        }
    }
}

/// Start a run: commands from now on are framed, logged and recorded under
/// `<state_dir>/runs/<id>/`. The directory is made when the first command runs, so
/// a run that starts nothing leaves nothing behind. Returns the run's directory.
pub fn begin(
    state_dir: &Path,
    verb: &str,
    args: Vec<String>,
    tree_commit: Option<String>,
) -> PathBuf {
    let id = format!(
        "{}-{verb}-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
        std::process::id()
    );
    let dir = state_dir.join("runs").join(id);
    *lock_run() = Some(Run {
        dir: dir.clone(),
        seq: 0,
        phase: None,
        commands: Vec::new(),
        verb: verb.to_string(),
        args,
        tree_commit,
        started: now(),
        phases: Vec::new(),
        phase_from: 0,
        inputs: serde_json::Value::Array(Vec::new()),
        #[cfg(test)]
        owner: std::thread::current().id(),
    });
    dir
}

/// End the run; its commands, in the order they started.
/// End the run: everything the run record writes, or `None` outside a run.
pub fn end() -> Option<Finished> {
    let mut r = lock_run().take()?;
    close_phase(&mut r);
    Some(Finished {
        dir: r.dir,
        verb: r.verb,
        args: r.args,
        tree_commit: r.tree_commit,
        started: r.started,
        phases: r.phases,
        commands: r.commands,
        inputs: r.inputs,
    })
}

/// The run's inputs, each with its source (docs/design/goatbuilder-legibility.md
/// §"Run record"), as the verb that knows them gives them. Never a secret value.
pub fn set_inputs(inputs: serde_json::Value) {
    if let Some(r) = lock_run().as_mut() {
        r.inputs = inputs;
    }
}

/// The commands of the run so far.
pub fn commands() -> Vec<CommandEntry> {
    lock_run()
        .as_ref()
        .map(|r| r.commands.clone())
        .unwrap_or_default()
}

/// The commands that follow belong to phase `id` (`"5"`, `"image-pull"`), named
/// `name`, for `purpose`. Its header prints with its first command.
pub fn phase(id: &str, name: &str, purpose: &str) {
    if let Some(r) = lock_run().as_mut() {
        close_phase(r);
        r.phase_from = r.commands.len();
        r.phases.push(PhaseEntry {
            id: id.to_string(),
            name: name.to_string(),
            purpose: purpose.to_string(),
            started: now(),
            ended: None,
            exit: None,
            commands: 0,
        });
        r.phase = Some(Phase {
            id: id.to_string(),
            name: name.to_string(),
            purpose: purpose.to_string(),
            shown: false,
        });
    }
}

/// The commands that follow belong to no phase.
pub fn no_phase() {
    if let Some(r) = lock_run().as_mut() {
        close_phase(r);
        r.phase = None;
    }
}

/// A command, built as `std::process::Command` builds one, run through this door.
#[derive(Debug)]
pub struct Cmd {
    inner: Command,
    host: Option<String>,
    stdout_set: bool,
    stderr_set: bool,
    /// The file the caller sends both streams to: then it is this command's log.
    own_log: Option<PathBuf>,
}

impl Cmd {
    pub fn new<S: AsRef<OsStr>>(program: S) -> Cmd {
        Cmd {
            inner: Command::new(program),
            host: None,
            own_log: None,
            stdout_set: false,
            stderr_set: false,
        }
    }

    pub fn arg<S: AsRef<OsStr>>(&mut self, a: S) -> &mut Cmd {
        self.inner.arg(a);
        self
    }

    pub fn args<I, S>(&mut self, a: I) -> &mut Cmd
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.inner.args(a);
        self
    }

    pub fn env<K: AsRef<OsStr>, V: AsRef<OsStr>>(&mut self, k: K, v: V) -> &mut Cmd {
        self.inner.env(k, v);
        self
    }

    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Cmd
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.inner.envs(vars);
        self
    }

    pub fn env_remove<K: AsRef<OsStr>>(&mut self, k: K) -> &mut Cmd {
        self.inner.env_remove(k);
        self
    }

    pub fn env_clear(&mut self) -> &mut Cmd {
        self.inner.env_clear();
        self
    }

    pub fn current_dir<P: AsRef<Path>>(&mut self, d: P) -> &mut Cmd {
        self.inner.current_dir(d);
        self
    }

    pub fn stdin<T: Into<Stdio>>(&mut self, s: T) -> &mut Cmd {
        self.inner.stdin(s);
        self
    }

    /// The caller's own stdout: it is then neither streamed nor logged by the runner.
    pub fn stdout<T: Into<Stdio>>(&mut self, s: T) -> &mut Cmd {
        self.stdout_set = true;
        self.inner.stdout(s);
        self
    }

    /// The caller's own stderr: it is then neither streamed nor logged by the runner.
    pub fn stderr<T: Into<Stdio>>(&mut self, s: T) -> &mut Cmd {
        self.stderr_set = true;
        self.inner.stderr(s);
        self
    }

    /// The caller sends stdout and stderr to the file at `path` itself (a
    /// parallel step's own log): the record and the frame name that file as the
    /// command's log, and the runner copies nothing.
    pub fn logged_at<P: AsRef<Path>>(&mut self, path: P) -> &mut Cmd {
        self.own_log = Some(path.as_ref().to_path_buf());
        self
    }

    /// Where the command acts, for the frame and the record (`alpha_hosts`, `operator@192.0.2.10`).
    pub fn host<S: Into<String>>(&mut self, h: S) -> &mut Cmd {
        self.host = Some(h.into());
        self
    }

    pub fn get_program(&self) -> &OsStr {
        self.inner.get_program()
    }

    pub fn get_args(&self) -> std::process::CommandArgs<'_> {
        self.inner.get_args()
    }

    /// The argv, as strings.
    pub fn argv(&self) -> Vec<String> {
        std::iter::once(self.inner.get_program())
            .chain(self.inner.get_args())
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// The line as a shell reads it: a `bash -c <script>` is its script; otherwise
    /// each word quoted only when it needs it.
    pub fn line(&self) -> String {
        line_of(&self.argv())
    }

    /// The line as it runs from `base`, the checkout a person pastes it in: a
    /// command with a working directory is `(cd <dir> && <line>)`, its
    /// directory relative to `base` when it is under it, and no `cd` when it is
    /// `base` itself.
    pub fn line_from(&self, base: &Path) -> String {
        match self.inner.get_current_dir() {
            Some(d) => {
                let dir = d.strip_prefix(base).unwrap_or(d).display().to_string();
                if dir.is_empty() {
                    // It runs in the checkout itself: no cd to print.
                    self.line()
                } else {
                    format!("(cd {} && {})", quote(&dir), self.line())
                }
            }
            None => self.line(),
        }
    }

    /// Run it, streaming its output, and wait.
    pub fn status(&mut self) -> std::io::Result<ExitStatus> {
        crate::trace::cmd(&self.inner);
        let Some(mut entry) = self.open(true) else {
            return self.inner.status();
        };
        let t0 = Instant::now();
        // open() decided whether the command keeps the terminal; such an entry
        // has no log, and says why.
        let status = if keeps_terminal(&entry.command).is_some() {
            self.inner.status()
        } else {
            self.streamed(&entry)
        };
        self.close(&mut entry, t0, status.as_ref().ok().map(|s| s.code()));
        status
    }

    /// Run it, capturing its output, and wait.
    pub fn output(&mut self) -> std::io::Result<Output> {
        crate::trace::cmd(&self.inner);
        let Some(mut entry) = self.open(false) else {
            return self.inner.output();
        };
        let t0 = Instant::now();
        // The log exists before the command starts, so a command that cannot
        // start still names a file that is there.
        let _ = entry.log.as_deref().and_then(log_file);
        let out = self.inner.output();
        if let (Ok(o), Some(log)) = (&out, &entry.log) {
            if let Some(mut f) = log_file(log) {
                let _ = f.write_all(&o.stdout);
                let _ = f.write_all(&o.stderr);
            }
        }
        self.close(&mut entry, t0, out.as_ref().ok().map(|o| o.status.code()));
        out
    }

    /// Start it in the background. Its output is streamed and logged as with
    /// [`Cmd::status`]; the record's end and exit stay empty, because the runner does
    /// not wait for it.
    pub fn spawn(&mut self) -> std::io::Result<Child> {
        crate::trace::cmd(&self.inner);
        let Some(mut entry) = self.open(false) else {
            return self.inner.spawn();
        };
        let log = entry
            .log
            .as_deref()
            .and_then(log_file)
            .map(|f| Arc::new(Mutex::new(f)));
        if !self.stdout_set {
            self.inner.stdout(Stdio::piped());
        }
        if !self.stderr_set {
            self.inner.stderr(Stdio::piped());
        }
        let child = self.inner.spawn();
        match child {
            Ok(mut c) => {
                if let Some(o) = c.stdout.take() {
                    copy_in_background(o, log.clone(), stdout_to_stderr());
                }
                if let Some(e) = c.stderr.take() {
                    copy_in_background(e, log, true);
                }
                entry.log_note = Some("started in the background; its end is not recorded".into());
                self.record(entry);
                Ok(c)
            }
            Err(e) => {
                entry.exit = None;
                entry.log_note = Some(format!("did not start: {e}"));
                self.record(entry);
                Err(e)
            }
        }
    }

    /// The entry for this command, and its frame printed; `None` outside a run.
    fn open(&self, streamed: bool) -> Option<CommandEntry> {
        let mut guard = lock_run();
        let run = guard.as_mut()?;
        #[cfg(test)]
        if run.owner != std::thread::current().id() {
            return None;
        }
        run.seq += 1;
        let phase_id = run.phase.as_ref().map(|p| p.id.clone());
        let log_name = format!(
            "{:03}-{}.log",
            run.seq,
            phase_id
                .as_deref()
                .unwrap_or("probe")
                .replace(['/', ' '], "-")
        );
        let log = run.dir.join("logs").join(log_name);
        let line = self.line();
        let terminal = if streamed {
            keeps_terminal(&line)
        } else {
            None
        };
        let where_ = if terminal.is_some() {
            "no log: the command keeps the terminal".to_string()
        } else if let Some(own) = &self.own_log {
            format!("log {}", shown(own))
        } else if self.stdout_set && self.stderr_set {
            "no log: the caller's own streams".to_string()
        } else {
            format!("log {}", shown(&log))
        };
        let at = match &self.host {
            Some(h) => format!("on {h}; {where_}"),
            None => where_,
        };
        // A probe (a command outside any phase) is in the record and under -v
        // only; it prints a line of its own when it fails (`close`).
        if let Some(p) = run.phase.as_mut() {
            if !p.shown {
                p.shown = true;
                let purpose = if p.purpose.is_empty() {
                    String::new()
                } else {
                    format!(": {}", p.purpose)
                };
                say(&format!("Phase {} \u{b7} {}{purpose}", p.id, p.name));
            }
            say(&format!("  $ {line}   ({at})"));
        }
        // A stream the caller set is the caller's own: the runner neither copies
        // nor logs it, and the record says so rather than naming an empty log.
        let (log, log_note) = match (self.stdout_set, self.stderr_set) {
            _ if terminal.is_some() => (None, terminal),
            _ if self.own_log.is_some() => (
                self.own_log.clone(),
                Some("the caller writes this log".to_string()),
            ),
            (true, true) => (
                None,
                Some("not captured: stdout and stderr are the caller's own".to_string()),
            ),
            (true, false) => (
                Some(log),
                Some("stdout is the caller's own; the log holds stderr".to_string()),
            ),
            (false, true) => (
                Some(log),
                Some("stderr is the caller's own; the log holds stdout".to_string()),
            ),
            (false, false) => (Some(log), None),
        };
        Some(CommandEntry {
            phase: phase_id,
            command: line,
            argv: self.argv(),
            host: self.host.clone(),
            log: log.map(|l| l.display().to_string()),
            log_note,
            started: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ended: None,
            exit: None,
            secs: None,
        })
    }

    fn close(&self, entry: &mut CommandEntry, t0: Instant, code: Option<Option<i32>>) {
        let secs = t0.elapsed().as_secs_f64();
        entry.ended = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        entry.secs = Some((secs * 10.0).round() / 10.0);
        entry.exit = code.flatten();
        let in_phase = entry.phase.is_some();
        let took = human(secs);
        let log = entry
            .log
            .as_deref()
            .map(|l| format!("; log {}", shown(Path::new(l))))
            .unwrap_or_default();
        // A probe has no frame above it, so its failure line names it.
        let what = if in_phase {
            "  FAILED".to_string()
        } else {
            format!("\u{b7} FAILED: {}", entry.command)
        };
        match code {
            Some(Some(0)) if in_phase => say(&format!("  ok in {took}")),
            Some(Some(0)) => {}
            // Exit 3 is the contract's "not converged" (C10): a phase that is
            // waiting or incomplete, not one that broke.
            Some(Some(3)) if in_phase => say(&format!("  not converged (exit 3) in {took}{log}")),
            Some(Some(n)) => say(&format!("{what}: exit {n} in {took}{log}")),
            Some(None) => say(&format!("{what}: killed by a signal after {took}{log}")),
            None => {
                entry.log_note = Some("did not start".into());
                say(&format!("{what}: did not start{log}"));
            }
        }
        self.record(entry.clone());
    }

    fn record(&self, entry: CommandEntry) {
        if let Some(r) = lock_run().as_mut() {
            r.commands.push(entry);
        }
    }

    /// `status`, with the output piped through the runner to the terminal and the log.
    fn streamed(&mut self, entry: &CommandEntry) -> std::io::Result<ExitStatus> {
        let log = entry
            .log
            .as_deref()
            .and_then(log_file)
            .map(|f| Arc::new(Mutex::new(f)));
        if !self.stdout_set {
            self.inner.stdout(Stdio::piped());
        }
        if !self.stderr_set {
            self.inner.stderr(Stdio::piped());
        }
        let mut child = self.inner.spawn()?;
        let mut threads = Vec::new();
        if let Some(o) = child.stdout.take() {
            threads.push(copy_in_background(o, log.clone(), stdout_to_stderr()));
        }
        if let Some(e) = child.stderr.take() {
            threads.push(copy_in_background(e, log, true));
        }
        let status = child.wait();
        for t in threads {
            let _ = t.join();
        }
        status
    }
}

/// `GOATBUILDER_CHILD_TTY=1`. A unit test sets its answer on its own thread
/// (`CHILD_TTY_IN_TEST`) rather than in the process environment, which every
/// test shares.
fn child_tty() -> bool {
    #[cfg(test)]
    if let Some(v) = CHILD_TTY_IN_TEST.with(|c| c.get()) {
        return v;
    }
    std::env::var_os("GOATBUILDER_CHILD_TTY").is_some_and(|v| v == "1")
}

#[cfg(test)]
thread_local! {
    static CHILD_TTY_IN_TEST: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Why a streamed command whose line is `line` keeps the terminal, when it does:
/// a program on the interactive list, or `GOATBUILDER_CHILD_TTY=1`.
fn keeps_terminal(line: &str) -> Option<String> {
    if let Some(why) = interactive(line) {
        return Some(format!("not captured (interactive): {why}"));
    }
    if child_tty() {
        return Some(
            "GOATBUILDER_CHILD_TTY=1: the command had the terminal; nothing was captured".into(),
        );
    }
    None
}

/// The programs that keep the terminal (tests/legibility-interactive.txt): one
/// `<program> <why>` per line.
const INTERACTIVE: &str = include_str!("../tests/legibility-interactive.txt");

/// Why the command whose line is `line` keeps the terminal, when it names a
/// program on the interactive list.
pub fn interactive(line: &str) -> Option<String> {
    interactive_in(INTERACTIVE, line)
}

fn interactive_in(list: &str, line: &str) -> Option<String> {
    let words: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || ";&|()'\"`".contains(c))
        .filter(|w| !w.is_empty())
        .map(|w| w.rsplit('/').next().unwrap_or(w))
        .collect();
    list.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .find_map(|l| {
            let (name, why) = l.split_once(char::is_whitespace).unwrap_or((l, ""));
            words.contains(&name).then(|| why.trim().to_string())
        })
}

/// Where a child's stdout goes as it streams inside a run. Under `-o json` or
/// `-o yaml` stdout is the report a script parses, so a child's output goes to
/// stderr (and to its log, as always); at a terminal or `-o table` it stays on
/// stdout. Outside a run, and for a command that keeps the terminal, the child
/// inherits stdout as `Command` gives it.
fn stdout_to_stderr() -> bool {
    crate::output::machine_output()
}

/// Copy a child's stream to our stdout (or stderr) and to the log, as it arrives.
fn copy_in_background<R: Read + Send + 'static>(
    mut from: R,
    log: Option<Arc<Mutex<std::fs::File>>>,
    to_stderr: bool,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            let n = match from.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if to_stderr {
                let mut e = std::io::stderr();
                let _ = e.write_all(&buf[..n]);
                let _ = e.flush();
            } else {
                let mut o = std::io::stdout();
                let _ = o.write_all(&buf[..n]);
                let _ = o.flush();
            }
            if let Some(l) = &log {
                if let Ok(mut f) = l.lock() {
                    let _ = f.write_all(&buf[..n]);
                }
            }
        }
    })
}

/// The log file, made with its directory; readable by its owner only, since a
/// tool's output can carry anything.
fn log_file(path: &str) -> Option<std::fs::File> {
    let p = Path::new(path);
    std::fs::create_dir_all(p.parent()?).ok()?;
    let mut o = std::fs::OpenOptions::new();
    o.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(p).ok()
}

/// A path as a person reads it: `~` for the home directory.
fn shown(p: &Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

fn human(secs: f64) -> String {
    if secs < 10.0 {
        format!("{secs:.1}s")
    } else if secs < 120.0 {
        format!("{}s", secs.round() as u64)
    } else {
        let s = secs.round() as u64;
        format!("{}m{:02}s", s / 60, s % 60)
    }
}

/// The line of an argv as a shell reads it.
pub fn line_of(argv: &[String]) -> String {
    let prog = argv.first().map(|p| {
        Path::new(p)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.clone())
    });
    if matches!(prog.as_deref(), Some("bash" | "sh")) {
        if let Some(i) = argv.iter().position(|a| a == "-c") {
            if let Some(script) = argv.get(i + 1) {
                return script.clone();
            }
        }
    }
    argv.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
}

fn quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+^".contains(c));
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The run state and the frame sink are process-wide; these tests take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn capture() -> (std::sync::MutexGuard<'static, ()>, tempfile::NamedTempFile) {
        let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let f = tempfile::NamedTempFile::new().unwrap();
        *SINK.lock().unwrap() = Some(f.path().to_path_buf());
        (g, f)
    }

    fn frame() -> String {
        let p = SINK.lock().unwrap().take();
        p.map(|p| std::fs::read_to_string(p).unwrap_or_default())
            .unwrap_or_default()
    }

    #[test]
    fn outside_a_run_it_is_a_plain_command() {
        let (_g, _sink) = capture();
        let o = Cmd::new("sh").args(["-c", "echo hi"]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&o.stdout), "hi\n");
        assert_eq!(frame(), "", "no run: no frame");
        assert!(commands().is_empty());
    }

    #[test]
    fn a_phase_prints_its_frame_logs_its_output_and_records_it() {
        let (_g, _sink) = capture();
        let t = tempfile::tempdir().unwrap();
        let dir = begin(t.path(), "standup", vec![], None);
        phase("5", "mgmt-stack", "site-bootstrap up to the account seed");
        let st = Cmd::new("sh")
            .args(["-c", "echo from-the-child; echo to-stderr >&2"])
            .host("alpha_hosts")
            .status()
            .unwrap();
        assert!(st.success());
        let f = frame();
        let lines: Vec<&str> = f.lines().collect();
        assert_eq!(
            lines[0],
            "Phase 5 \u{b7} mgmt-stack: site-bootstrap up to the account seed"
        );
        assert!(
            lines[1]
                .starts_with("  $ echo from-the-child; echo to-stderr >&2   (on alpha_hosts; log "),
            "{f}"
        );
        assert!(lines[2].starts_with("  ok in "), "{f}");
        assert_eq!(lines.len(), 3, "three lines for a phase that succeeds: {f}");
        let cmds = end().unwrap().commands;
        assert_eq!(cmds.len(), 1);
        let c = &cmds[0];
        assert_eq!(c.phase.as_deref(), Some("5"));
        assert_eq!(c.command, "echo from-the-child; echo to-stderr >&2");
        assert_eq!(c.argv[0], "sh");
        assert_eq!(c.exit, Some(0));
        let log = std::fs::read_to_string(c.log.as_ref().unwrap()).unwrap();
        assert!(
            log.contains("from-the-child") && log.contains("to-stderr"),
            "{log}"
        );
        assert!(Path::new(c.log.as_ref().unwrap()).starts_with(dir.join("logs")));
    }

    #[test]
    fn the_frame_is_printed_before_the_child_starts() {
        let (_g, _sink) = capture();
        let t = tempfile::tempdir().unwrap();
        begin(t.path(), "deploy", vec![], None);
        phase("1", "gate", "");
        // The child reads the frame the runner has written so far: it must already be there.
        let marker = t.path().join("seen");
        let st = Cmd::new("sh")
            .arg("-c")
            .arg(format!("touch '{}'", marker.display()))
            .status()
            .unwrap();
        assert!(st.success());
        let f = frame();
        assert!(f.starts_with("Phase 1 \u{b7} gate\n  $ touch "), "{f}");
        end();
    }

    #[test]
    fn a_failure_names_its_exit_and_log_and_a_probe_prints_only_when_it_fails() {
        let (_g, _sink) = capture();
        let t = tempfile::tempdir().unwrap();
        begin(t.path(), "deploy", vec![], None);
        Cmd::new("sh").args(["-c", "true"]).output().unwrap();
        phase("3", "playbook", "the play");
        let st = Cmd::new("sh").args(["-c", "exit 4"]).status().unwrap();
        assert_eq!(st.code(), Some(4));
        no_phase();
        Cmd::new("sh").args(["-c", "exit 2"]).output().unwrap();
        let f = frame();
        let lines: Vec<&str> = f.lines().collect();
        // The probe that passed prints nothing; the phase prints its frame and
        // its failure; the probe that failed prints one line naming itself.
        assert_eq!(lines[0], "Phase 3 \u{b7} playbook: the play", "{f}");
        assert!(lines[1].starts_with("  $ exit 4   (log "), "{f}");
        assert!(
            lines[2].starts_with("  FAILED: exit 4 in ") && lines[2].contains("; log "),
            "{f}"
        );
        assert!(
            lines[3].starts_with("\u{b7} FAILED: exit 2: exit 2 in ")
                && lines[3].contains("; log "),
            "{f}"
        );
        assert_eq!(lines.len(), 4, "{f}");
        let fin = end().unwrap();
        let cmds = fin.commands;
        assert_eq!(cmds[0].phase, None);
        assert_eq!(fin.phases.len(), 1);
        assert_eq!(fin.phases[0].id, "3");
        assert_eq!(
            fin.phases[0].exit,
            Some(4),
            "a phase carries its failing command's exit"
        );
        assert_eq!(fin.phases[0].commands, 1);
        assert_eq!(cmds[1].exit, Some(4));
        assert_eq!(
            cmds.len(),
            3,
            "a probe is in the record whether it printed or not"
        );
    }

    #[test]
    fn a_stream_the_caller_owns_is_not_named_as_a_log() {
        let (_g, _sink) = capture();
        let t = tempfile::tempdir().unwrap();
        begin(t.path(), "release", vec![], None);
        phase("1", "tag", "");
        Cmd::new("sh")
            .args(["-c", "true"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        Cmd::new("sh")
            .args(["-c", "echo e >&2"])
            .stdout(Stdio::null())
            .status()
            .unwrap();
        let f = frame();
        assert!(f.contains("(no log: the caller's own streams)"), "{f}");
        let cmds = end().unwrap().commands;
        assert_eq!(
            cmds[0].log, None,
            "both streams the caller's own: no log is named"
        );
        assert_eq!(
            cmds[0].log_note.as_deref(),
            Some("not captured: stdout and stderr are the caller's own")
        );
        let log = cmds[1].log.as_deref().expect("stderr is still logged");
        assert_eq!(std::fs::read_to_string(log).unwrap(), "e\n");
        assert_eq!(
            cmds[1].log_note.as_deref(),
            Some("stdout is the caller's own; the log holds stderr")
        );
    }

    #[test]
    fn a_command_that_keeps_the_terminal_names_no_log_in_its_frame() {
        let (_g, _sink) = capture();
        let t = tempfile::tempdir().unwrap();
        begin(t.path(), "standup", vec![], None);
        phase("2", "baseline", "");
        // GOATBUILDER_CHILD_TTY=1, on this test's thread only.
        CHILD_TTY_IN_TEST.with(|c| c.set(Some(true)));
        let st = Cmd::new("sh").args(["-c", "true"]).status();
        CHILD_TTY_IN_TEST.with(|c| c.set(None));
        assert!(st.unwrap().success());
        let f = frame();
        assert!(
            !f.contains("log /"),
            "a log is named that is never written: {f}"
        );
        assert!(
            f.contains("(no log: the command keeps the terminal)"),
            "{f}"
        );
        let cmds = end().unwrap().commands;
        assert_eq!(cmds[0].log, None);
    }

    #[test]
    fn a_captured_command_that_cannot_start_names_a_log_that_exists() {
        let (_g, _sink) = capture();
        let t = tempfile::tempdir().unwrap();
        begin(t.path(), "deploy", vec![], None);
        assert!(Cmd::new("/nonexistent/program").output().is_err());
        let cmds = end().unwrap().commands;
        let log = cmds[0].log.as_deref().expect("a log is named");
        assert!(
            Path::new(log).is_file(),
            "the named log {log} does not exist"
        );
    }

    #[test]
    fn a_phase_that_exits_3_reads_not_converged() {
        let (_g, _sink) = capture();
        let t = tempfile::tempdir().unwrap();
        begin(t.path(), "federate", vec![], None);
        phase("14", "acceptance", "");
        Cmd::new("sh").args(["-c", "exit 3"]).status().unwrap();
        let f = frame();
        assert!(
            f.lines()
                .nth(2)
                .is_some_and(|l| l.starts_with("  not converged (exit 3) in ")),
            "{f}"
        );
        assert_eq!(end().unwrap().phases[0].exit, Some(3));
    }

    #[test]
    fn a_command_the_caller_logs_names_the_callers_file() {
        let (_g, _sink) = capture();
        let t = tempfile::tempdir().unwrap();
        begin(t.path(), "standup", vec![], None);
        phase("image-pull", "pull the images", "");
        let own = t.path().join("pull.log");
        let f = std::fs::File::create(&own).unwrap();
        let mut child = Cmd::new("sh")
            .args(["-c", "echo pulled"])
            .stdout(Stdio::from(f.try_clone().unwrap()))
            .stderr(Stdio::from(f))
            .logged_at(&own)
            .spawn()
            .unwrap();
        child.wait().unwrap();
        assert!(frame().contains(&format!("(log {})", shown(&own))), "{}", frame());
        let cmds = end().unwrap().commands;
        assert_eq!(
            cmds[0].log.as_deref(),
            Some(own.display().to_string().as_str())
        );
        assert_eq!(std::fs::read_to_string(&own).unwrap(), "pulled\n");
    }

    #[test]
    fn a_line_from_the_checkout_carries_its_working_directory() {
        let base = Path::new("/srv/checkout");
        let mut c = Cmd::new("../scripts/with-aws");
        c.args(["dev", "--", "ansible", "all"]);
        assert_eq!(c.line_from(base), "../scripts/with-aws dev -- ansible all");
        c.current_dir(base.join("ops/ansible"));
        assert_eq!(c.line_from(base), "(cd ops/ansible && ../scripts/with-aws dev -- ansible all)");
        c.current_dir(base);
        assert_eq!(c.line_from(base), "../scripts/with-aws dev -- ansible all", "the checkout itself needs no cd");
        c.current_dir("/other place");
        assert_eq!(c.line_from(base), "(cd '/other place' && ../scripts/with-aws dev -- ansible all)");
    }

    #[test]
    fn a_command_that_does_not_start_is_recorded_as_such() {
        let (_g, _sink) = capture();
        let t = tempfile::tempdir().unwrap();
        begin(t.path(), "deploy", vec![], None);
        assert!(Cmd::new("/nonexistent/program").status().is_err());
        let cmds = end().unwrap().commands;
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].exit, None);
        assert_eq!(cmds[0].log_note.as_deref(), Some("did not start"));
        assert!(frame().contains("\u{b7} FAILED: /nonexistent/program: did not start"));
    }

    #[test]
    fn a_listed_program_keeps_the_terminal_wherever_its_line_names_it() {
        let list = "# comment\naws-vault prompts for a token by isatty(stderr)\n";
        assert_eq!(
            interactive_in(list, "cd x && ../scripts/aws-vault exec p -- y").as_deref(),
            Some("prompts for a token by isatty(stderr)")
        );
        assert_eq!(
            interactive_in(list, "aws-vault-helper exec"),
            None,
            "a whole program name, not a prefix"
        );
        assert_eq!(interactive_in(list, "echo 'no vault here'"), None);
        assert_eq!(interactive(""), None, "the shipped list is empty");
    }

    #[test]
    fn a_line_reads_as_a_shell_reads_it() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            line_of(&v(&["bash", "-euo", "pipefail", "-c", "a && b"])),
            "a && b"
        );
        assert_eq!(
            line_of(&v(&["ssh", "operator@192.0.2.10", "sudo -n true"])),
            "ssh operator@192.0.2.10 'sudo -n true'"
        );
        assert_eq!(line_of(&v(&["x", "it's"])), "x 'it'\\''s'");
    }
}
