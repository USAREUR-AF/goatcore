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
//! A command outside a phase (a probe such as the fresh-source check) gets one line,
//! `· <command> (log …)`, and a second only when it fails.
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

struct Run {
    dir: PathBuf,
    seq: u32,
    phase: Option<Phase>,
    commands: Vec<CommandEntry>,
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
pub fn begin(state_dir: &Path, verb: &str) -> PathBuf {
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
    });
    dir
}

/// End the run; its commands, in the order they started.
pub fn end() -> Vec<CommandEntry> {
    lock_run().take().map(|r| r.commands).unwrap_or_default()
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
        r.phase = None;
    }
}

/// A command, built as `std::process::Command` builds one, run through this door.
pub struct Cmd {
    inner: Command,
    host: Option<String>,
    stdout_set: bool,
    stderr_set: bool,
}

impl Cmd {
    pub fn new<S: AsRef<OsStr>>(program: S) -> Cmd {
        Cmd {
            inner: Command::new(program),
            host: None,
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

    /// Run it, streaming its output, and wait.
    pub fn status(&mut self) -> std::io::Result<ExitStatus> {
        crate::trace::cmd(&self.inner);
        let Some(mut entry) = self.open() else {
            return self.inner.status();
        };
        let child_tty = std::env::var_os("GOATBUILDER_CHILD_TTY").is_some_and(|v| v == "1");
        let t0 = Instant::now();
        let status = if child_tty {
            entry.log_note = Some(
                "GOATBUILDER_CHILD_TTY=1: the command had the terminal; nothing was captured"
                    .into(),
            );
            entry.log = None;
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
        let Some(mut entry) = self.open() else {
            return self.inner.output();
        };
        let t0 = Instant::now();
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
        let Some(mut entry) = self.open() else {
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
                    copy_in_background(o, log.clone(), false);
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
    fn open(&self) -> Option<CommandEntry> {
        let mut guard = lock_run();
        let run = guard.as_mut()?;
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
        let at = match &self.host {
            Some(h) => format!("on {h}; log {}", shown(&log)),
            None => format!("log {}", shown(&log)),
        };
        match run.phase.as_mut() {
            Some(p) => {
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
            None => say(&format!("\u{b7} {line}   ({at})")),
        }
        Some(CommandEntry {
            phase: phase_id,
            command: line,
            argv: self.argv(),
            host: self.host.clone(),
            log: Some(log.display().to_string()),
            log_note: None,
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
        match code {
            Some(Some(0)) if in_phase => say(&format!("  ok in {took}")),
            Some(Some(0)) => {}
            Some(Some(n)) => say(&format!("  FAILED exit {n} in {took}{log}")),
            Some(None) => say(&format!("  FAILED killed by a signal after {took}{log}")),
            None => {
                entry.log_note = Some("did not start".into());
                say(&format!("  FAILED: did not start{log}"));
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
            threads.push(copy_in_background(o, log.clone(), false));
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
        let dir = begin(t.path(), "standup");
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
        let cmds = end();
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
        begin(t.path(), "deploy");
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
    fn a_failure_names_its_exit_and_log_and_a_probe_is_one_line() {
        let (_g, _sink) = capture();
        let t = tempfile::tempdir().unwrap();
        begin(t.path(), "deploy");
        Cmd::new("sh").args(["-c", "true"]).output().unwrap();
        phase("3", "playbook", "the play");
        let st = Cmd::new("sh").args(["-c", "exit 4"]).status().unwrap();
        assert_eq!(st.code(), Some(4));
        let f = frame();
        let lines: Vec<&str> = f.lines().collect();
        assert!(lines[0].starts_with("\u{b7} true   (log "), "{f}");
        assert_eq!(lines[1], "Phase 3 \u{b7} playbook: the play");
        assert!(
            lines[3].starts_with("  FAILED exit 4 in ") && lines[3].contains("; log "),
            "{f}"
        );
        let cmds = end();
        assert_eq!(cmds[0].phase, None);
        assert_eq!(cmds[1].exit, Some(4));
    }

    #[test]
    fn a_command_that_does_not_start_is_recorded_as_such() {
        let (_g, _sink) = capture();
        let t = tempfile::tempdir().unwrap();
        begin(t.path(), "deploy");
        assert!(Cmd::new("/nonexistent/program").status().is_err());
        let cmds = end();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].exit, None);
        assert_eq!(cmds[0].log_note.as_deref(), Some("did not start"));
        assert!(frame().contains("FAILED: did not start"));
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
