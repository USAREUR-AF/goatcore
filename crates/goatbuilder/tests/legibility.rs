//! Contract clause C11, legibility: for every action goatbuilder takes, its
//! command, its inputs with their sources and its reason can be read from the
//! tool before, during and after the run. This is the conformance test that
//! proves it (docs/design/goatbuilder-legibility.md).
//!
//! Each verb family (standup, join-node, deploy, federate) runs on a fixture
//! with a recorder in place of the real commands, and four checks are made:
//!
//!   (a) every process the tool started is in the run record, with its command
//!       and its log path;
//!   (b) every value in a command line has an entry in the run record's
//!       `inputs`, with a source;
//!   (c) the commands a dry run prints at a terminal are the commands the real
//!       run executes, in order;
//!   (d) every phase that can run has a phase-catalog entry, and every document
//!       path the tool prints exists.
//!
//! Most of these fail today. A check that fails today is listed, by check and
//! verb, in `tests/legibility-known-failing.txt`, so CI stays green while the
//! features are built; the pull request that builds a feature removes its
//! lines. A listed check that passes fails the test, so the list only shrinks.
//! An unlisted check that fails, fails the test.
//!
//! The recorder. The tool's PATH is one directory in which every name (every
//! program on this machine's PATH, and the ones a host needs that this machine
//! may lack) is a link to one recorder script, so nothing the tool starts by
//! name escapes it; every script under the fixture checkout's ops/scripts is the
//! same recorder.
//! Each invocation writes its parent pid and its argv to a file in a record
//! directory and appends that file's name to an order file, so the processes
//! are read back in the order they started. Only processes whose parent is the
//! goatbuilder process count: those are the processes the tool started. For
//! standup, join-node and deploy the recorder stands in for the program and
//! exits 0, because their commands reach hosts. For federate it records and
//! then runs the real program of that name (a name with none fails loudly),
//! because the stand-in repository's own stubs
//! (tests/federate_world) produce the files the next phase reads; nothing in
//! that repository reaches a host.
//!
//! The terminal. Check (c) reads the dry run as a person sees it at a terminal,
//! where several verbs render a plan page instead of every row. The dry run
//! runs under `script(1)`, which gives it a pseudo-terminal.
//!
//! The run record is the interface step 4 builds: `<state>/runs/<id>/run.json`,
//! looked for anywhere under the test's own temporary root. Its fields, as this
//! test reads them, are in docs/design/goatbuilder-legibility.md §"Run record".

mod federate_world;
use federate_world::test_support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

// --- the checks and the known-failing list ------------------------------------

const CHECKS: [char; 4] = ['a', 'b', 'c', 'd'];
const VERBS: [&str; 4] = ["standup", "join-node", "deploy", "federate"];
const KNOWN_FAILING: &str = "tests/legibility-known-failing.txt";

/// The phase catalog step 6 adds (schema `goat-phases/v1`).
const PHASE_CATALOG: &str = "ops/phases/catalog.yaml";

/// The fields a phase-catalog entry carries, each non-empty.
const CATALOG_FIELDS: [&str; 7] = [
    "what", "why", "changes", "verify", "runs", "by_hand", "read",
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_goatbuilder")
}

/// `(check, verb)` pairs listed as failing today. A line is
/// `<check> <verb> <reason>`; `#` starts a comment.
fn known_failing() -> Result<BTreeSet<(char, String)>, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(KNOWN_FAILING);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = BTreeSet::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        let (check, verb) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
        let reason = words.collect::<Vec<_>>().join(" ");
        let c = check
            .chars()
            .next()
            .filter(|c| check.len() == 1 && CHECKS.contains(c));
        let Some(c) = c else {
            return Err(format!(
                "{KNOWN_FAILING}:{}: `{check}` is not a check (a, b, c or d)",
                i + 1
            ));
        };
        if !VERBS.contains(&verb) {
            return Err(format!(
                "{KNOWN_FAILING}:{}: `{verb}` is not a verb family ({})",
                i + 1,
                VERBS.join(", ")
            ));
        }
        if reason.is_empty() {
            return Err(format!(
                "{KNOWN_FAILING}:{}: `{check} {verb}` gives no reason",
                i + 1
            ));
        }
        if !out.insert((c, verb.to_string())) {
            return Err(format!(
                "{KNOWN_FAILING}:{}: `{check} {verb}` is listed twice",
                i + 1
            ));
        }
    }
    Ok(out)
}

/// Hold one verb's four results against the known-failing list.
fn judge(verb: &str, results: Vec<(char, Result<(), String>)>) {
    let known = known_failing().unwrap_or_else(|e| panic!("{e}"));
    let show_all = std::env::var_os("GB_LEGIBILITY_DETAIL").is_some();
    let mut problems = Vec::new();
    for (check, result) in results {
        let listed = known.contains(&(check, verb.to_string()));
        match (listed, result) {
            (true, Err(why)) => {
                let shown = if show_all { why.clone() } else { why.lines().next().unwrap_or("").to_string() };
                eprintln!("C11({check}) {verb}: known failing: {shown}");
            }
            (true, Ok(())) => problems.push(format!(
                "C11({check}) {verb} passes now: remove the line `{check} {verb}` from crates/goatbuilder/{KNOWN_FAILING}"
            )),
            (false, Ok(())) => eprintln!("C11({check}) {verb}: passes"),
            (false, Err(why)) => problems.push(format!("C11({check}) {verb} fails:\n{why}")),
        }
    }
    assert!(problems.is_empty(), "\n{}\n", problems.join("\n\n"));
}

// --- the recorder -------------------------------------------------------------

/// Programs the tool may start by name, recorded even where this machine has none.
/// Every other name on this machine's PATH gets a recorder too.
const PROGRAMS: [&str; 17] = [
    "sh",
    "bash",
    "python3",
    "ssh",
    "scp",
    "ansible",
    "ansible-playbook",
    "ansible-inventory",
    "netbird",
    "git",
    "go",
    "sudo",
    "aws",
    "aws-vault",
    "docker",
    "curl",
    "id",
];

/// One process the recorder saw.
#[derive(Debug, Clone)]
struct Proc {
    ppid: u32,
    program: String,
    args: Vec<String>,
}

impl Proc {
    /// The command line as a shell would read it: the `-c` script for a shell,
    /// the planned line for standup's phase wrapper, else program and arguments.
    fn line(&self) -> String {
        if matches!(self.program.as_str(), "bash" | "sh") {
            if let Some(i) = self.args.iter().position(|a| a == "-c") {
                if let Some(script) = self.args.get(i + 1) {
                    return script.clone();
                }
            }
        }
        if self.program == "standup-phase-run" && self.args.len() == 1 {
            return self.args[0].clone();
        }
        std::iter::once(self.program.clone())
            .chain(self.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The recorder. It records the invocation, then, by `GB_REC_MODE`, exits 0
/// (`stub`) or runs the real program of the same name from `GB_REC_PATH` (`pass`);
/// a `pass` with no real program fails loudly. When the program and its first
/// argument are `GB_REC_SAY_FOR`, or its arguments contain `GB_REC_SAY_WHEN`,
/// it first prints `GB_REC_SAY` on stdout, as a child that talks would. Its own tools come from
/// `GB_REC_PATH`, since the tool's PATH holds only recorders.
const RECORDER: &str = r#"#!/bin/sh
# goatbuilder C11 recorder (crates/goatbuilder/tests/legibility.rs)
PATH="${GB_REC_PATH:?}"; export PATH
d="${GB_REC_DIR:?}"
name=${0##*/}
f=$(mktemp "$d/p.XXXXXX") || exit 97
{
  printf '%s %s
' "$PPID" "$$"
  printf '%s' "$name" | od -An -tx1 | tr -d ' 
'; printf '
'
  for a in "$@"; do printf '%s' "$a" | od -An -tx1 | tr -d ' 
'; printf '
'; done
} > "$f"
printf '%s
' "${f##*/}" >> "$d/order"
if [ -n "${GB_REC_SAY_FOR:-}" ] && [ "$name ${1:-}" = "$GB_REC_SAY_FOR" ]; then
  printf '%s
' "${GB_REC_SAY:?}"
fi
if [ -n "${GB_REC_SAY_WHEN:-}" ]; then
  case " $* " in *"$GB_REC_SAY_WHEN"*) printf '%s
' "${GB_REC_SAY:?}" ;; esac
fi
if [ "${GB_REC_MODE:-stub}" = pass ]; then
  ifs=$IFS; IFS=:
  for dir in $GB_REC_PATH; do
    if [ "$dir" != "${GB_REC_SHIMS:-}" ] && [ -f "$dir/$name" ] && [ -x "$dir/$name" ]; then
      IFS=$ifs; exec "$dir/$name" "$@"
    fi
  done
  IFS=$ifs
  echo "C11 recorder: no real $name on PATH" >&2
  exit 127
fi
exit 0
"#;

fn exe(p: &Path, text: &str) {
    test_support::executable(p, text);
}

/// This machine's PATH, as the recorder runs real programs from it.
fn real_path() -> String {
    std::env::var("PATH").unwrap_or_default()
}

/// The program `name` on this machine's PATH (for the test's own use).
fn real(name: &str) -> PathBuf {
    std::env::split_paths(&real_path())
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("{name} is needed by the C11 test and is not on PATH"))
}

/// The recorder's PATH directory: a link to the recorder for every program
/// name on this machine's PATH and every name in [`PROGRAMS`]. The tool's PATH
/// is this directory alone, so every program it starts by name is recorded.
fn install_shims(root: &Path) -> PathBuf {
    let script = root.join("recorder.sh");
    exe(&script, RECORDER);
    let dir = root.join("shims");
    std::fs::create_dir_all(&dir).unwrap();
    let mut names: BTreeSet<String> = PROGRAMS.iter().map(|s| s.to_string()).collect();
    for d in std::env::split_paths(&real_path()) {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            use std::os::unix::fs::PermissionsExt;
            // Follow a symlink: most of a PATH is links (sh, env, awk, ...).
            let ok = std::fs::metadata(e.path())
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
            if ok {
                names.insert(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    for n in names {
        let _ = std::os::unix::fs::symlink(&script, dir.join(&n));
    }
    dir
}

/// The environment that puts the tool behind the recorder.
fn behind_recorder(c: &mut Command, shims: &Path, mode: &str, rec: &Path) {
    c.env("PATH", shims)
        .env("GB_REC_PATH", real_path())
        .env("GB_REC_SHIMS", shims)
        .env("GB_REC_MODE", mode)
        .env("GB_REC_DIR", rec);
}

fn unhex(s: &str) -> String {
    let bytes: Vec<u8> = (0..s.len() / 2)
        .filter_map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok())
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Every process recorded in `dir` whose parent is `parent`, in start order.
/// A shell given one simple command execs it in place, so the same process
/// can be recorded twice (as `bash -c ...`, then as the program); the first
/// record of a pid is the one the tool started.
fn recorded(dir: &Path, parent: u32) -> Vec<Proc> {
    let order = std::fs::read_to_string(dir.join("order")).unwrap_or_default();
    let mut seen = BTreeSet::new();
    order
        .lines()
        .filter_map(|name| std::fs::read_to_string(dir.join(name)).ok())
        .filter_map(|text| {
            let mut lines = text.lines();
            let (ppid, pid) = lines.next()?.trim().split_once(' ')?;
            let (ppid, pid): (u32, u32) = (ppid.parse().ok()?, pid.parse().ok()?);
            let program = unhex(lines.next()?);
            let args = lines.map(unhex).collect();
            Some((
                pid,
                Proc {
                    ppid,
                    program,
                    args,
                },
            ))
        })
        .filter(|(pid, p)| p.ppid == parent && seen.insert(*pid))
        .map(|(_, p)| p)
        .collect()
}

/// Run `cmd` with the recorder writing to `rec`; the output and the processes
/// goatbuilder itself started.
fn run_recorded(mut cmd: Command, rec: &Path) -> (Output, Vec<Proc>) {
    std::fs::create_dir_all(rec).unwrap();
    cmd.env("GB_REC_DIR", rec)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn().expect("start goatbuilder");
    let pid = child.id();
    let out = child.wait_with_output().expect("goatbuilder ran");
    (out, recorded(rec, pid))
}

/// Where the recorder writes for a run whose processes nobody reads.
fn elsewhere(root: &Path) -> PathBuf {
    let d = root.join("rec/elsewhere");
    std::fs::create_dir_all(&d).unwrap();
    d
}

// --- the terminal -------------------------------------------------------------

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `cmd` run under a pseudo-terminal, as a person runs it; what it printed.
fn at_a_terminal(cmd: &Command) -> Result<String, String> {
    let mut words = vec![cmd.get_program().to_string_lossy().into_owned()];
    words.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));
    // By its path: the tool's PATH holds only recorders.
    let mut t = Command::new(real("script"));
    if cfg!(target_os = "macos") {
        t.args(["-q", "/dev/null"]).args(&words);
    } else {
        let line = words
            .iter()
            .map(|w| sh_quote(w))
            .collect::<Vec<_>>()
            .join(" ");
        t.args(["-qec", &line, "/dev/null"]);
    }
    t.env_clear();
    for (k, v) in cmd.get_envs() {
        if let Some(v) = v {
            t.env(k, v);
        }
    }
    t.env("SHELL", "/bin/sh");
    if let Some(d) = cmd.get_current_dir() {
        t.current_dir(d);
    }
    let out = t
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("script(1) is needed to give the dry run a terminal: {e}"))?;
    // script(1) hands back the dry run's own exit code (`-e` on Linux; the BSD
    // script does by default). A dry run that fails is a harness failure.
    if !out.status.success() {
        return Err(format!(
            "the terminal dry run `{}` exited {:?}:\n{}{}",
            words[1..].join(" "),
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let ansi = regex::Regex::new("\x1b\\[[0-9;]*[A-Za-z]").unwrap();
    let page = ansi
        .replace_all(&String::from_utf8_lossy(&out.stdout), "")
        .replace('\r', "");
    if std::env::var_os("GB_LEGIBILITY_DETAIL").is_some() {
        eprintln!(
            "--- the terminal dry run: {}\n{page}\n---",
            words[1..].join(" ")
        );
    }
    Ok(page)
}

// --- the worlds ---------------------------------------------------------------

/// A checkout with no hosts: the standup fixture's declaration as the
/// checkout's config, every ops/scripts entry a recorder, and the few files a
/// deploy of one component reads. The recorder stands in for every program.
struct Hollow {
    root: tempfile::TempDir,
    repo: PathBuf,
    shims: PathBuf,
}

/// The digest-pinned ref the standup fixture declares for portal on dev.
const PORTAL_REF: &str = "ghcr.io/example-org/portal:v1.4.0@sha256:3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c";

fn hollow() -> Hollow {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    let repo = r.join("repo");
    let fixture = self::repo().join("ops/goatbuilder/fixtures/standup");
    let mut cfg = std::fs::read_to_string(fixture.join("goatbuilder.yaml")).unwrap();
    // join-node reaches a new machine as the project's declared account.
    cfg.push_str("deploy:\n  ssh_user: operator\n");
    exe(&repo.join("ops/goatbuilder/goatbuilder.yaml"), &cfg);
    let declared = std::fs::read_to_string(fixture.join("declared/pins.yaml")).unwrap();
    std::fs::create_dir_all(repo.join("ops/goatbuilder/declared")).unwrap();
    std::fs::write(repo.join("ops/goatbuilder/declared/pins.yaml"), &declared).unwrap();
    std::fs::copy(
        fixture.join("converger.yaml"),
        repo.join("ops/goatbuilder/converger.yaml"),
    )
    .unwrap();
    assert!(
        declared.contains(PORTAL_REF),
        "the standup fixture no longer declares {PORTAL_REF}"
    );
    // Every script the checkout carries is the recorder, so a script started by
    // its path is recorded and does nothing.
    for e in std::fs::read_dir(self::repo().join("ops/scripts")).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_file() {
            exe(&repo.join("ops/scripts").join(e.file_name()), RECORDER);
        }
    }
    // deploy portal --env dev: the goatnet, the play, the inventory, the pin.
    exe(
        &repo.join("ops/pins/fleet-pins.yaml"),
        &format!("goatnets:\n  dev:\n    members: [dev_hosts]\ncomponents:\n  portal:\n    var: portal_image\n    fleet: \"{PORTAL_REF}\"\n"),
    );
    exe(
        &repo.join("ops/ansible/portal-deploy.yml"),
        "- name: portal\n  hosts: \"{{ target }}\"\n  tasks: []\n",
    );
    exe(
        &repo.join("ops/ansible/inventory/mgmt-hosts.ini"),
        "[dev_hosts]\ndev-core-a ansible_host=192.0.2.11\n",
    );
    exe(
        &repo.join("ops/ansible/group_vars/dev_hosts/pins.yml"),
        &format!("portal_image: \"{PORTAL_REF}\"\n"),
    );
    std::fs::create_dir_all(r.join("home")).unwrap();
    std::fs::create_dir_all(r.join("tmp")).unwrap();
    let shims = install_shims(r);
    Hollow { root, repo, shims }
}

impl Hollow {
    fn command(&self, args: &[&str]) -> Command {
        let r = self.root.path();
        let mut c = Command::new(bin());
        c.args(args).env_clear();
        // A run outside run_recorded (the dry runs) records in `elsewhere`, unread.
        behind_recorder(&mut c, &self.shims, "stub", &elsewhere(r));
        c.env("HOME", r.join("home"))
            .env("TMPDIR", r.join("tmp"))
            .env("GOAT_MESH_PROBE", "0")
            // On macOS a long-running verb starts `caffeinate -i -w <pid>` to
            // keep the operator's machine awake (src/awake.rs). The recorder
            // shadows every name on this machine's PATH, so a macOS run would
            // record that process too: it is no command of the plan, and it
            // starts on macOS only, so every OS records the same processes.
            .env("GOAT_NO_CAFFEINATE", "1")
            .env("GOAT_NO_FUN", "1")
            .env("GOAT_LANG", "en")
            .env("NO_COLOR", "1")
            .current_dir(r);
        c
    }

    fn repo(&self) -> &str {
        self.repo.to_str().unwrap()
    }
}

// --- what the tool printed and recorded ---------------------------------------

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// Every `runs/<id>/run.json` under `root` whose verb is `verb`.
fn run_records(root: &Path, verb: &str) -> Vec<serde_json::Value> {
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.file_name().is_some_and(|n| n == "run.json")
                && p.parent()
                    .and_then(|x| x.parent())
                    .and_then(|x| x.file_name())
                    .is_some_and(|n| n == "runs")
            {
                out.push(p);
            }
        }
    }
    let mut found = Vec::new();
    walk(root, &mut found);
    found
        .iter()
        .map(|p| {
            let text =
                std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            serde_json::from_str::<serde_json::Value>(&text)
                .unwrap_or_else(|e| panic!("{} is not JSON: {e}", p.display()))
        })
        .filter(|v| v["verb"] == verb)
        .collect()
}

fn strs(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Whitespace, quoting and line continuations folded away, so a command
/// reflowed for a terminal compares equal to the line that ran.
fn norm(s: &str) -> String {
    s.replace("\\\n", " ")
        .replace(['\'', '"'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn tokens(line: &str) -> Vec<String> {
    shlex::split(line).unwrap_or_else(|| line.split_whitespace().map(String::from).collect())
}

/// (a) every process the tool started is in a run record, with its command and
/// a log path that exists.
fn check_a(procs: &[Proc], records: &[serde_json::Value]) -> Result<(), String> {
    if records.is_empty() {
        return Err(format!(
            "no run record: no runs/<id>/run.json was written ({} processes started)",
            procs.len()
        ));
    }
    let commands: Vec<&serde_json::Value> = records
        .iter()
        .flat_map(|r| r["commands"].as_array().into_iter().flatten())
        .collect();
    let mut missing = Vec::new();
    for p in procs {
        let mut argv = vec![p.program.clone()];
        argv.extend(p.args.iter().cloned());
        let hit = commands.iter().find(|c| {
            let rec_argv = strs(&c["argv"]);
            let same_argv = !rec_argv.is_empty()
                && rec_argv.len() == argv.len()
                && Path::new(&rec_argv[0])
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .as_deref()
                    == Some(argv[0].as_str())
                && rec_argv[1..] == argv[1..];
            same_argv
                || c["command"]
                    .as_str()
                    .is_some_and(|s| norm(s) == norm(&p.line()))
        });
        match hit {
            None => missing.push(format!("not in the run record: {}", p.line())),
            Some(c) => match c["log"].as_str() {
                Some(l) if Path::new(l).is_file() => {}
                Some(l) => missing.push(format!("its log {l} does not exist: {}", p.line())),
                // No log is allowed only for a program on the reviewed interactive
                // list, and only when the entry says so; anything else must log.
                None => {
                    let note = c["log_note"].as_str().unwrap_or("");
                    let listed = interactive_program(&p.line());
                    if !(note.starts_with("not captured (interactive)") && listed.is_some()) {
                        missing.push(format!(
                            "no log path (note: {note:?}; on the interactive list: {}): {}",
                            listed.as_deref().unwrap_or("no"),
                            p.line()
                        ));
                    }
                }
            },
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} of {} processes:\n  {}",
            missing.len(),
            procs.len(),
            missing.join("\n  ")
        ))
    }
}

/// The reviewed list of programs that keep the terminal (src/runner.rs reads the
/// same file).
const INTERACTIVE: &str = "tests/legibility-interactive.txt";

/// The listed program `line` names, if any: its words, by file name, against
/// the list.
fn interactive_program(line: &str) -> Option<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(INTERACTIVE);
    let list = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    interactive_in(&list, line)
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
        .filter_map(|l| l.split_whitespace().next())
        .find(|name| words.contains(name))
        .map(String::from)
}

/// Options whose next word is a value.
const VALUE_FLAGS: [&str; 16] = [
    "-l",
    "--limit",
    "-i",
    "--inventory",
    "--site",
    "--profile",
    "--tags",
    "--skip-tags",
    "--repo",
    "--address",
    "--node",
    "--class",
    "--env",
    "--component",
    "--out",
    "--with",
];

/// The values in one command line, by a fixed rule: the right side of every
/// `-e name=value` and `--flag=value`, and the word after each option in
/// [`VALUE_FLAGS`]. A value that reads the environment (`$(printenv NAME)`,
/// `$NAME`) is named by the variable it reads.
fn values(line: &str) -> Vec<Value> {
    // A word the shell ends with `;` (`... mesh.ini; fi`) carries the value without it.
    let t: Vec<String> = tokens(line)
        .into_iter()
        .map(|w| w.trim_end_matches(';').to_string())
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        let w = &t[i];
        if w == "-e" && i + 1 < t.len() {
            let x = &t[i + 1];
            if let Some((_, v)) = x.split_once('=') {
                out.push(Value::of(v));
            } else if let Some(file) = x.strip_prefix('@') {
                // `-e @<file>`: the vars file is a value too.
                out.push(Value::of(file));
            }
            i += 2;
            continue;
        }
        if let Some(kv) = w.strip_prefix("-e ") {
            if let Some((_, v)) = kv.split_once('=') {
                out.push(Value::of(v));
            }
        } else if w.starts_with("--") && w.contains('=') {
            out.push(Value::of(w.split_once('=').unwrap().1));
        } else if (VALUE_FLAGS.contains(&w.as_str()) || w.ends_with("with-aws")) && i + 1 < t.len()
        {
            // `with-aws <profile> -- ...`: the profile is positional, and a value.
            out.push(Value::of(&t[i + 1]));
            i += 1;
        }
        i += 1;
    }
    out
}

#[derive(Debug)]
enum Value {
    Literal(String),
    FromEnv(String),
}

impl Value {
    fn of(v: &str) -> Value {
        let v = v.trim();
        let env = v
            .strip_prefix("$(printenv ")
            .and_then(|r| r.strip_suffix(')'))
            .or_else(|| v.strip_prefix("${").and_then(|r| r.strip_suffix('}')))
            .or_else(|| {
                v.strip_prefix('$')
                    .filter(|r| r.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            });
        // A play reading the environment itself: `{{ lookup("env", "NAME") }}`.
        let lookup =
            regex::Regex::new(r#"lookup\(\s*['"]env['"]\s*,\s*['"]([A-Za-z_][A-Za-z0-9_]*)['"]"#)
                .unwrap();
        match (env, lookup.captures(v)) {
            (Some(name), _) => Value::FromEnv(name.trim().to_string()),
            (None, Some(c)) => Value::FromEnv(c[1].to_string()),
            (None, None) => Value::Literal(v.to_string()),
        }
    }
}

/// (b) every value in a command line has an entry in `inputs` with a source.
fn check_b(procs: &[Proc], records: &[serde_json::Value]) -> Result<(), String> {
    if records.is_empty() {
        return Err("no run record, so no `inputs`: no value has a recorded source".to_string());
    }
    let inputs: Vec<&serde_json::Value> = records
        .iter()
        .flat_map(|r| r["inputs"].as_array().into_iter().flatten())
        .collect();
    let sourced =
        |i: &&serde_json::Value| i["source"].as_str().is_some_and(|s| !s.trim().is_empty());
    let mut missing = BTreeSet::new();
    for p in procs {
        for v in values(&p.line()) {
            let ok = match &v {
                Value::Literal(s) => inputs
                    .iter()
                    .any(|i| sourced(i) && i["value"].as_str() == Some(s.as_str())),
                Value::FromEnv(n) => inputs
                    .iter()
                    .any(|i| sourced(i) && i["name"].as_str() == Some(n.as_str())),
            };
            if !ok {
                missing.insert(format!("{v:?}"));
            }
        }
    }
    // A secret's value is never recorded: its entry shows the source only.
    for i in &inputs {
        if i["secret"] == true && !i["value"].is_null() {
            missing.insert(format!("{} is secret and its value is recorded", i["name"]));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} values with no sourced input: {}",
            missing.len(),
            missing.into_iter().collect::<Vec<_>>().join(", ")
        ))
    }
}

/// (c) each command the real run executed is printed by the terminal dry run,
/// in the order it ran.
fn check_c(procs: &[Proc], dry_terminal: &str, parallel: &BTreeSet<String>) -> Result<(), String> {
    let page = norm(dry_terminal);
    let mut at = 0;
    let mut missing = Vec::new();
    for p in procs {
        let want = norm(&p.line());
        // A command of a step that runs beside the others (`∥`, standup's image
        // pull) starts when that step starts, which races the steps around it:
        // it must be printed, but not in order. Only a command that IS one of
        // that step's commands is held this loosely; any other command keeps
        // its order, even when its text also appears inside the parallel one.
        if parallel.contains(&want) {
            if !page.contains(&want) {
                missing.push(format!("not printed: {}", p.line()));
            }
            continue;
        }
        // A program started by its path is recorded by its file name, which
        // the printed path ends with, so a plain substring match holds.
        match page[at..].find(&want) {
            Some(i) => at += i + want.len(),
            None if page.contains(&want) => missing.push(format!(
                "printed, but not in the order it ran: {}",
                p.line()
            )),
            None => missing.push(format!("not printed: {}", p.line())),
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} of {} commands that ran:\n  {}",
            missing.len(),
            procs.len(),
            missing.join("\n  ")
        ))
    }
}

/// The commands of the parallel steps a dry run plans, from its `-o json`
/// rows: each `cmd` row under a `phase` row marked `∥`, up to the next `phase`
/// row, normalized.
fn parallel_commands(json: &serde_json::Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut inside = false;
    for c in json["checks"].as_array().into_iter().flatten() {
        let detail = c["detail"].as_str().unwrap_or("");
        // A standup observability leg is a `leg` row, a step like a phase.
        if c["name"] == "phase" || c["name"] == "leg" {
            inside = detail.contains('\u{2225}');
        } else if inside && c["name"] == "cmd" {
            out.insert(norm(detail));
        }
    }
    out
}

/// The phases a dry run plans, from its `-o json` rows: `=== Phase 5 [A] ...`
/// is phase `5`; a titled step (`=== Image pull [A ∥] ...`) is its title,
/// lowercased and hyphenated.
fn planned_phases(json: &serde_json::Value) -> BTreeSet<String> {
    json["checks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["name"] == "phase" || c["name"] == "leg")
        .filter_map(|c| c["detail"].as_str())
        .filter_map(|d| {
            d.trim()
                .strip_prefix("===")
                .and_then(|r| r.strip_suffix("==="))
        })
        .map(|t| step_id(t.trim()))
        .collect()
}

/// A planned step's catalog id: `Phase 5 [A] ...` is `5`; any other title is
/// the words before its `[kind]` or `(note)`, lowercased and hyphenated, quotes
/// dropped (`Image pull [A ∥] ...` is `image-pull`, `Observability leg "host"
/// [A ∥] ...` is `observability-leg-host`, `join observability leg "host" before
/// phase 12 (waiting)` is `join-observability-leg-host-before-phase-12`).
fn step_id(t: &str) -> String {
    if let Some(n) = t.strip_prefix("Phase ").and_then(|r| r.split(' ').next()) {
        return n.to_string();
    }
    let t = t.split(" [").next().unwrap_or(t);
    let t = t.split(" (").next().unwrap_or(t);
    t.trim()
        .replace('"', "")
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

/// The document paths in what the tool printed, as paths in this repository.
/// A `docs/...md` is checked wherever it appears (`docs/x.md`, `./docs/x.md`,
/// `../docs/x.md`, an absolute path into the checkout, `goatcore/docs/x.md`, this
/// repository's own blob or tree URL), except inside a URL to another
/// repository, which is a link check's job.
fn doc_paths(printed: &str) -> BTreeSet<String> {
    let re =
        regex::Regex::new(r"(?:^|[^A-Za-z0-9_-])(docs/[A-Za-z0-9_./-]*[A-Za-z0-9_-]\.md)").unwrap();
    let own = regex::Regex::new(r"(?i)github\.com/usareur-af/goatcore/(?:blob|tree|raw)/[^/\s]+/$")
        .unwrap();
    re.captures_iter(printed)
        .filter_map(|c| {
            let m = c.get(1).unwrap();
            // The word this path sits in, back to whitespace, a quote or a bracket.
            let start = printed[..m.start()]
                .rfind(|ch: char| ch.is_whitespace() || "'\"()<>`[]".contains(ch))
                .map_or(0, |i| i + 1);
            let before = &printed[start..m.start()];
            let elsewhere = before.contains("://") && !own.is_match(before);
            (!elsewhere).then(|| m.as_str().to_string())
        })
        .collect()
}

/// (d) every phase that can run has a catalog entry, and every document path
/// the tool printed exists.
fn check_d(verb: &str, phases: &BTreeSet<String>, printed: &str) -> Result<(), String> {
    check_d_with(
        verb,
        phases,
        printed,
        std::fs::read_to_string(repo().join(PHASE_CATALOG)).ok(),
    )
}

/// [`check_d`] against a given catalog text (`None`: there is no catalog).
fn check_d_with(
    verb: &str,
    phases: &BTreeSet<String>,
    printed: &str,
    catalog: Option<String>,
) -> Result<(), String> {
    let mut problems = Vec::new();
    if phases.is_empty() {
        problems.push("the dry run planned no phase".to_string());
    }
    match catalog {
        None => problems.push(format!(
            "no phase catalog: {PHASE_CATALOG} does not exist ({} phases of {verb} have no entry)",
            phases.len()
        )),
        Some(text) => {
            let doc: serde_yaml::Value =
                serde_yaml::from_str(&text).map_err(|e| format!("{PHASE_CATALOG}: {e}"))?;
            if doc["schema"].as_str() != Some("goat-phases/v1") {
                problems.push(format!("{PHASE_CATALOG}: `schema` is not goat-phases/v1"));
            }
            let entries: Vec<&serde_yaml::Value> = doc["phases"]
                .as_sequence()
                .map(|s| s.iter().collect())
                .unwrap_or_default();
            for ph in phases {
                let entry = entries.iter().find(|e| {
                    e["verb"].as_str() == Some(verb)
                        && (e["phase"].as_str() == Some(ph.as_str())
                            || e["phase"].as_u64().map(|n| n.to_string()).as_deref()
                                == Some(ph.as_str()))
                });
                match entry {
                    None => problems.push(format!("no catalog entry for {verb} phase {ph}")),
                    Some(e) => {
                        for f in CATALOG_FIELDS {
                            if e[f].as_str().is_none_or(|s| s.trim().is_empty()) {
                                problems.push(format!("{verb} phase {ph}: the entry has no `{f}`"));
                            }
                        }
                        if let Some(r) = e["read"].as_str() {
                            let p = r
                                .split('#')
                                .next()
                                .unwrap_or(r)
                                .split_whitespace()
                                .next()
                                .unwrap_or("");
                            if !repo().join(p).exists() {
                                problems.push(format!(
                                    "{verb} phase {ph}: its `read` {p} does not exist"
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    for p in doc_paths(printed) {
        if !repo().join(&p).is_file() {
            problems.push(format!("the tool prints {p}, which does not exist"));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n  "))
    }
}

/// A run that went somewhere: a harness failure is never a known-failing check.
fn ran(what: &str, o: &Output, procs: &[Proc], codes: &[i32]) {
    let code = o.status.code().unwrap_or(-1);
    assert!(
        codes.contains(&code),
        "{what}: exit {code}, expected one of {codes:?}\n{}",
        text(o)
    );
    assert!(
        !procs.is_empty(),
        "{what}: the recorder saw no process the tool started\n{}",
        text(o)
    );
}

/// A dry run, a plan or a help page that worked: exit 0, or the harness fails.
fn ok(what: &str, o: Output) -> Output {
    assert!(
        o.status.success(),
        "{what}: exit {:?}; a dry run, plan or help page must work\n{}",
        o.status.code(),
        text(&o)
    );
    o
}

fn json(o: &Output) -> serde_json::Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("-o json: {e}\n{}", text(o)))
}

// --- the four verb families ---------------------------------------------------

#[test]
fn c11_standup() {
    let h = hollow();
    let r = h.repo();
    let args = ["standup", "-C", r, "dev", "--site", "alpha"];
    let real: Vec<&str> = args
        .iter()
        .copied()
        .chain(["--ack", "A Person", "--yes"])
        .collect();
    let (out, procs) = run_recorded(h.command(&real), &h.root.path().join("rec/real"));
    // The run stops at the first manual step (phase 11).
    ran("standup", &out, &procs, &[0]);
    let dry: Vec<&str> = args.iter().copied().chain(["--dry-run"]).collect();
    let page = at_a_terminal(&h.command(&dry)).unwrap_or_else(|e| panic!("{e}"));
    let mut plan = vec!["-o", "json"];
    plan.extend(&dry);
    let plan = ok("standup plan", h.command(&plan).output().unwrap());
    let help = ok(
        "standup --help",
        h.command(&["standup", "--help"]).output().unwrap(),
    );
    let printed = [text(&out), page.clone(), text(&plan), text(&help)].concat();
    let records = run_records(h.root.path(), "standup");
    judge(
        "standup",
        vec![
            ('a', check_a(&procs, &records)),
            ('b', check_b(&procs, &records)),
            (
                'c',
                check_c(&procs, &page, &parallel_commands(&json(&plan))),
            ),
            (
                'd',
                check_d("standup", &planned_phases(&json(&plan)), &printed),
            ),
        ],
    );
}

#[test]
fn c11_join_node() {
    let h = hollow();
    let r = h.repo();
    // Phase 1 is manual and the only manual step (gate row H2): the run goes
    // from 2 to the end.
    let args = [
        "join-node",
        "-C",
        r,
        "dev",
        "--site",
        "alpha",
        "--node",
        "node-x",
        "--class",
        "core",
        "--address",
        "192.0.2.10",
        "--from",
        "2",
    ];
    let real: Vec<&str> = args.iter().copied().chain(["--yes"]).collect();
    let (out, procs) = run_recorded(h.command(&real), &h.root.path().join("rec/real"));
    ran("join-node", &out, &procs, &[0]);
    let dry: Vec<&str> = args.iter().copied().chain(["--dry-run"]).collect();
    let page = at_a_terminal(&h.command(&dry)).unwrap_or_else(|e| panic!("{e}"));
    let plan = h
        .command(&[
            "-o",
            "json",
            "join-node",
            "-C",
            r,
            "dev",
            "--site",
            "alpha",
            "--node",
            "node-x",
            "--class",
            "core",
            "--dry-run",
        ])
        .output()
        .unwrap();
    let plan = ok("join-node plan", plan);
    let help = ok(
        "join-node --help",
        h.command(&["join-node", "--help"]).output().unwrap(),
    );
    let printed = [text(&out), page.clone(), text(&plan), text(&help)].concat();
    let records = run_records(h.root.path(), "join-node");
    judge(
        "join-node",
        vec![
            ('a', check_a(&procs, &records)),
            ('b', check_b(&procs, &records)),
            (
                'c',
                check_c(&procs, &page, &parallel_commands(&json(&plan))),
            ),
            (
                'd',
                check_d("join-node", &planned_phases(&json(&plan)), &printed),
            ),
        ],
    );
}

/// deploy has no numbered phases today; these are its steps as push.rs runs
/// them. The phase catalog (step 6) names them, and this list then follows it.
const DEPLOY_STEPS: [&str; 6] = [
    "prep",
    "fresh-source",
    "transport",
    "playbook",
    "deploy",
    "verify",
];

#[test]
fn c11_deploy() {
    let h = hollow();
    let r = h.repo();
    let args = ["deploy", "-C", r, "--env", "dev", "--component", "portal"];
    let real: Vec<&str> = args
        .iter()
        .copied()
        .chain(["--ack", "A Person", "--yes"])
        .collect();
    let (out, procs) = run_recorded(h.command(&real), &h.root.path().join("rec/real"));
    // The recorder prints no play recap, so the deploy reads as not converged (3).
    ran("deploy", &out, &procs, &[0, 3]);
    let dry: Vec<&str> = args.iter().copied().chain(["--dry-run"]).collect();
    let page = at_a_terminal(&h.command(&dry)).unwrap_or_else(|e| panic!("{e}"));
    let mut plan = vec!["-o", "json"];
    plan.extend(&dry);
    let plan = ok("deploy plan", h.command(&plan).output().unwrap());
    // deploy plans no numbered phase; its plan must still be a report.
    assert!(
        json(&plan)["checks"].is_array(),
        "deploy plan: no rows\n{}",
        text(&plan)
    );
    let help = ok(
        "deploy --help",
        h.command(&["deploy", "--help"]).output().unwrap(),
    );
    let printed = [text(&out), page.clone(), text(&plan), text(&help)].concat();
    let records = run_records(h.root.path(), "deploy");
    let steps: BTreeSet<String> = DEPLOY_STEPS.iter().map(|s| s.to_string()).collect();
    judge(
        "deploy",
        vec![
            ('a', check_a(&procs, &records)),
            ('b', check_b(&procs, &records)),
            (
                'c',
                check_c(&procs, &page, &parallel_commands(&json(&plan))),
            ),
            ('d', check_d("deploy", &steps, &printed)),
        ],
    );
}

/// Every world a C11 test runs goatbuilder in opts out of the keep-awake
/// assertion (src/awake.rs), so a macOS run starts no `caffeinate` the run record
/// and the dry run do not hold and the processes are the same on every OS.
#[test]
fn every_c11_world_opts_out_of_keep_awake() {
    let opted_out = |c: &Command| {
        c.get_envs()
            .any(|(k, v)| k == "GOAT_NO_CAFFEINATE" && v.is_some_and(|v| v == "1"))
    };
    assert!(opted_out(&hollow().command(&["status"])), "the hollow world");
    let w = federate_world::world();
    assert!(opted_out(&w.command(&["hello-ally"])), "the federate world");
    assert!(opted_out(&w.command_with(&["-o", "table"], &["hello-ally"])), "the federate world, as a table");
}

/// Each group of a deploy is its own phase in the record, `deploy:<group>`
/// and `verify:<group>`, so `runs diff` sees one group's outcome change while
/// another's stays the same.
#[test]
fn a_two_group_deploy_records_a_phase_per_group() {
    let h = hollow();
    let r = h.repo();
    exe(
        &h.repo.join("ops/pins/fleet-pins.yaml"),
        &format!("goatnets:\n  dev:\n    members: [dev_hosts, dev_edge]\ncomponents:\n  portal:\n    var: portal_image\n    fleet: \"{PORTAL_REF}\"\n"),
    );
    exe(
        &h.repo.join("ops/ansible/inventory/mgmt-hosts.ini"),
        "[dev_hosts]\ndev-core-a ansible_host=192.0.2.11\n\n[dev_edge]\ndev-edge-a ansible_host=192.0.2.12\n",
    );
    exe(
        &h.repo.join("ops/ansible/group_vars/dev_edge/pins.yml"),
        &format!("portal_image: \"{PORTAL_REF}\"\n"),
    );
    let args = ["deploy", "-C", r, "--env", "dev", "--component", "portal", "--ack", "A Person", "--yes"];
    // dev_edge's play prints a clean recap, so its deploy succeeds and its
    // verify runs; dev_hosts's prints none, so its deploy fails.
    let mut cmd = h.command(&args);
    cmd.env("GB_REC_SAY_WHEN", "target=dev_edge")
        .env("GB_REC_SAY", "PLAY RECAP ***\ndev-edge-a : ok=1 changed=0 unreachable=0 failed=0");
    let (out, procs) = run_recorded(cmd, &h.root.path().join("rec/two"));
    ran("deploy, two groups", &out, &procs, &[0, 3]);
    let records = run_records(h.root.path(), "deploy");
    assert_eq!(records.len(), 1, "one run record: {records:?}");
    let ids: Vec<String> = records[0]["phases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["id"].as_str().map(String::from))
        .collect();
    let phase = |id: &str| records[0]["phases"].as_array().unwrap().iter().find(|p| p["id"] == id).cloned();
    for g in ["dev_hosts", "dev_edge"] {
        assert!(ids.contains(&format!("deploy:{g}")), "no phase deploy:{g} in {ids:?}");
        assert!(ids.contains(&format!("verify:{g}")), "no phase verify:{g} in {ids:?}");
    }
    // dev_edge's verify ran: it read the ledger, and gave no skip verdict.
    let edge = phase("verify:dev_edge").unwrap();
    assert!(edge["commands"].as_u64().unwrap_or(0) > 0 && edge["verdict"].is_null(), "verify:dev_edge did not run: {edge}");
    // dev_hosts's deploy failed: its verify is a phase with its verdict.
    let hosts = phase("verify:dev_hosts").unwrap();
    assert_eq!(hosts["verdict"], "not checked: the deploy failed", "{hosts}");
    let unique: BTreeSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "a phase id repeats: {ids:?}");
}

/// A deploy's dry run names its four probes under two headings: the two it
/// ran itself (the secrets posture and the mesh status, whose answers shaped
/// the plan) and the two only a real run starts (the fresh-source check and
/// the ledger read). Each row is the line its builder makes, as `-o json`
/// lists it.
#[test]
fn a_deploy_dry_run_names_its_probes_under_two_headings() {
    let h = hollow();
    let r = h.repo();
    let dry = ["deploy", "-C", r, "--env", "dev", "--component", "portal", "--dry-run"];
    let page = at_a_terminal(&h.command(&dry)).unwrap_or_else(|e| panic!("{e}"));
    let mut plan = vec!["-o", "json"];
    plan.extend(&dry);
    let plan = json(&ok("deploy plan", h.command(&plan).output().unwrap()));
    let rows = |name: &str| -> Vec<String> {
        plan["checks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["name"] == name)
            .filter_map(|c| c["detail"].as_str().map(|d| d.trim().to_string()))
            .collect()
    };
    let (ran, real) = (rows("probe-ran"), rows("probe"));
    assert_eq!(ran.len(), 2, "the dry run's own probes: {ran:?}");
    assert!(ran[0].contains("available") && ran[1].contains("netbird status"), "{ran:?}");
    assert_eq!(real.len(), 2, "the real run's probes: {real:?}");
    assert!(real[0].contains("fresh-source.sh"), "{real:?}");
    assert!(real[1].starts_with("(cd ops/ansible && ") && real[1].contains("deploy-history.jsonl"), "the ledger read, runnable from the checkout: {real:?}");
    // On the page, each row sits under its own heading: after it, and before
    // the next heading of the other kind.
    let page = norm(&page);
    let ran_h = "This dry run ran (its answers shaped the plan above):";
    let real_h = "A real run also runs (each verdict is known only then):";
    let under = |heading: &str, other: &str, row: &str| {
        let row = norm(row);
        page.match_indices(heading).any(|(at, _)| {
            let section = &page[at..];
            let end = section[heading.len()..].find(other).map_or(section.len(), |e| e + heading.len());
            section[..end].contains(&row)
        })
    };
    for row in &ran {
        assert!(under(ran_h, real_h, row), "{row:?} is not under {ran_h:?}:\n{page}");
    }
    for row in &real {
        assert!(under(real_h, ran_h, row), "{row:?} is not under {real_h:?}:\n{page}");
    }
}

#[test]
fn c11_federate() {
    let w = federate_world::world();
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    let root = w.root.path().to_path_buf();
    let shims = install_shims(&root);
    let other = elsewhere(&root);
    let with_shims = |mut c: Command| {
        behind_recorder(&mut c, &shims, "pass", &other);
        c
    };
    // Phases 1 to 5 (5 is manual and stops the run), then 6 to 9.
    let segments: [&[&str]; 2] = [&["hello-ally"], &["hello-ally", "--from", "6", "--to", "9"]];
    let (mut procs, mut outs, mut pages) = (Vec::new(), String::new(), String::new());
    for (i, seg) in segments.iter().enumerate() {
        let (out, p) = run_recorded(
            with_shims(w.command(seg)),
            &root.join(format!("rec/real-{i}")),
        );
        ran(&format!("federate {}", seg.join(" ")), &out, &p, &[0]);
        procs.extend(p);
        outs.push_str(&text(&out));
    }
    for seg in [
        &["hello-ally", "--to", "5", "--dry-run"][..],
        &["hello-ally", "--from", "6", "--to", "9", "--dry-run"],
    ] {
        pages.push_str(
            &at_a_terminal(&with_shims(w.command_with(&["-o", "table"], seg)))
                .unwrap_or_else(|e| panic!("{e}")),
        );
        pages.push('\n');
    }
    let plan_out = ok(
        "federate plan",
        with_shims(w.command(&["hello-ally", "--dry-run"]))
            .output()
            .unwrap(),
    );
    let plan = json(&plan_out);
    let help = ok(
        "federate --help",
        with_shims(w.command_with(&["-o", "table"], &["--help"]))
            .output()
            .unwrap(),
    );
    let printed = [outs, pages.clone(), text(&plan_out), text(&help)].concat();
    let records = run_records(&root, "federate");
    judge(
        "federate",
        vec![
            ('a', check_a(&procs, &records)),
            ('b', check_b(&procs, &records)),
            ('c', check_c(&procs, &pages, &parallel_commands(&plan))),
            ('d', check_d("federate", &planned_phases(&plan), &printed)),
        ],
    );
}

/// The list names only checks and verbs that exist, once each, with a reason.
#[test]
fn c11_known_failing_list_is_well_formed() {
    known_failing().unwrap_or_else(|e| panic!("{e}"));
}

/// Each check can pass: given a run record, a page and a catalog that hold
/// what C11 asks, the check says so. A feature is then judged by the check, not
/// by a check that nothing could satisfy.
#[test]
fn c11_each_check_can_pass_when_the_tool_does_what_it_asks() {
    let t = tempfile::tempdir().unwrap();
    let log = t.path().join("phase-5.log");
    std::fs::write(&log, "").unwrap();
    let line = "cd /r/ops/ansible && ../scripts/with-aws default -- ../scripts/ansible-preflight.sh -i inventory/mgmt-hosts.ini site.yml -e target=alpha_hosts -e pg=\"$(printenv ALPHA_PG_PASS)\" --limit 'node-x'";
    let procs = vec![Proc {
        ppid: 1,
        program: "bash".into(),
        args: vec!["-euo".into(), "pipefail".into(), "-c".into(), line.into()],
    }];
    let record = serde_json::json!({
        "schema": "goatbuilder.run/v1",
        "verb": "standup",
        "commands": [{ "phase": "5", "command": line, "log": log.display().to_string() }],
        "inputs": [
            { "name": "aws_profile", "value": "default", "source": "default: the AWS profile when the project declares none" },
            { "name": "inventory", "value": "inventory/mgmt-hosts.ini", "source": "default" },
            { "name": "target", "value": "alpha_hosts", "source": "derived from --site alpha by <site>_hosts" },
            { "name": "ALPHA_PG_PASS", "value": null, "secret": true, "source": "environment" },
            { "name": "node", "value": "node-x", "source": "argument: --node", "origin": { "kind": "argument", "flag": "--node" } }
        ]
    });
    assert_eq!(check_a(&procs, std::slice::from_ref(&record)), Ok(()));
    assert_eq!(check_b(&procs, std::slice::from_ref(&record)), Ok(()));
    let page = format!(
        "Phase 5 · ansible · mgmt-stack\n  {}\n    {}\n",
        &line[..40],
        &line[40..]
    );
    assert_eq!(check_c(&procs, &page, &BTreeSet::new()), Ok(()));
    let phases: BTreeSet<String> = ["5".to_string()].into();
    let catalog = "schema: goat-phases/v1\nphases:\n  - verb: standup\n    phase: 5\n    what: w\n    why: y\n    changes: c\n    verify: v\n    runs: r\n    by_hand: h\n    read: docs/design/goatbuilder.md\n";
    assert_eq!(
        check_d_with(
            "standup",
            &phases,
            "read docs/design/goatbuilder.md",
            Some(catalog.into())
        ),
        Ok(())
    );

    // The value rule, on the shapes the verbs print.
    let got: Vec<String> = values(
        "if x; then a --inventory inv/mesh.ini; fi; ../scripts/with-aws default -- play.yml \
         -e 'pg={{ lookup(\"env\", \"ZAGROS_PG_PASS\") }}' -e @\"$VF\" -l zagros_b_hosts;",
    )
    .iter()
    .map(|v| format!("{v:?}"))
    .collect();
    assert_eq!(
        got,
        [
            "Literal(\"inv/mesh.ini\")",
            "Literal(\"default\")",
            "FromEnv(\"ZAGROS_PG_PASS\")",
            "FromEnv(\"VF\")",
            "Literal(\"zagros_b_hosts\")"
        ]
    );

    // A parallel step's own command is held to being printed, whenever it
    // started; every other command keeps its order.
    let pg = "Phase 2 · A\n  run-a\nImage pull · A \u{2225} · images\n  pull-x\nPhase 3 · T\n  run-b\njoin the image pull before phase 5\nPhase 5\n  run-c\n";
    let par: BTreeSet<String> = ["pull-x".to_string()].into();
    let pr = |l: &str| Proc {
        ppid: 1,
        program: "bash".into(),
        args: vec!["-c".into(), l.into()],
    };
    let seq = |l: &[&str]| -> Vec<Proc> { l.iter().map(|x| pr(x)).collect() };
    for order in [
        ["run-a", "pull-x", "run-b", "run-c"],
        ["run-a", "run-b", "pull-x", "run-c"],
        ["pull-x", "run-a", "run-b", "run-c"],
    ] {
        assert_eq!(check_c(&seq(&order), pg, &par), Ok(()), "{order:?}");
    }
    assert!(
        check_c(&seq(&["run-a", "run-c", "run-b"]), pg, &par).is_err(),
        "steps outside the parallel step keep their order"
    );
    // A step whose text is part of a parallel command is not exempt by that.
    let pg2 = "Phase 2 · A\n  run-a\nImage pull · A \u{2225} · images\n  pull-x && run-b\nPhase 3 · T\n  run-b\nPhase 5\n  run-c\n";
    let par2: BTreeSet<String> = ["pull-x && run-b".to_string()].into();
    assert!(
        check_c(
            &seq(&["run-a", "pull-x && run-b", "run-c", "run-b"]),
            pg2,
            &par2
        )
        .is_err(),
        "run-b, its own Phase 3 step, run out of order"
    );
    let pg3 = "Phase 1 · GATE\n  netbird status\nPull · A \u{2225} · pull\n  ansible pull && netbird status --check\nPhase 2 · A\n  run-a\n";
    let par3: BTreeSet<String> = ["ansible pull && netbird status --check".to_string()].into();
    assert!(
        check_c(&seq(&["run-a", "netbird status"]), pg3, &par3).is_err(),
        "netbird status inside a longer parallel command, run out of order"
    );
    assert!(
        check_c(&seq(&["missing-x"]), pg, &["missing-x".to_string()].into()).is_err(),
        "a parallel command must still be printed"
    );

    // Standup's observability legs are steps too: `leg` rows, with the
    // catalog's ids, and a parallel leg's command is held as a parallel one.
    let legs = serde_json::json!({ "checks": [
        { "name": "phase", "detail": "=== Phase 11 [MANUAL] first login ===" },
        { "name": "leg", "detail": "=== Observability leg \"host\" [A \u{2225}] the obs host baseline ===" },
        { "name": "cmd", "detail": "run-leg-host" },
        { "name": "leg", "detail": "=== Observability leg \"stack\" [A \u{2225}] the obs stack ===" },
        { "name": "cmd", "detail": "run-leg-stack" },
        { "name": "leg", "detail": "=== join observability leg \"host\" before phase 12 ===" },
        { "name": "leg", "detail": "=== join observability leg \"stack\" before phase 13 (waiting) ===" },
        { "name": "phase", "detail": "=== Phase 12 [A] portal ===" },
        { "name": "cmd", "detail": "run-portal" }
    ]});
    let want: BTreeSet<String> = [
        "11",
        "12",
        "observability-leg-host",
        "observability-leg-stack",
        "join-observability-leg-host-before-phase-12",
        "join-observability-leg-stack-before-phase-13",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(planned_phases(&legs), want);
    let par: BTreeSet<String> = ["run-leg-host".to_string(), "run-leg-stack".to_string()].into();
    assert_eq!(
        parallel_commands(&legs),
        par,
        "a parallel leg's commands, and no other"
    );
    assert_eq!(
        step_id("Image pull [A \u{2225}] every pinned image"),
        "image-pull"
    );
    assert_eq!(
        step_id("join the image pull before phase 5"),
        "join-the-image-pull-before-phase-5"
    );

    // A command with no log passes only when it is on the interactive list and
    // its entry says so; the note alone, or the list alone, is not enough.
    let ssh = Proc {
        ppid: 1,
        program: "ssh".into(),
        args: vec!["operator@192.0.2.10".into(), "true".into()],
    };
    let entry = |note: &str| serde_json::json!({ "verb": "standup", "commands": [{ "command": "ssh operator@192.0.2.10 true", "log": null, "log_note": note }], "inputs": [] });
    assert!(
        check_a(
            std::slice::from_ref(&ssh),
            &[entry("not captured (interactive): x")]
        )
        .is_err(),
        "ssh is not on the shipped list"
    );
    assert!(
        check_a(
            std::slice::from_ref(&ssh),
            &[entry("GOATBUILDER_CHILD_TTY=1")]
        )
        .is_err(),
        "an escape is not a way out of logging"
    );
    assert_eq!(
        interactive_in(
            "ssh needs the terminal for x\n",
            "ssh operator@192.0.2.10 true"
        )
        .as_deref(),
        Some("ssh")
    );

    // A dry run that fails is a harness failure, never a known-failing check.
    assert!(
        at_a_terminal(Command::new(bin()).args(["join-node", "--no-such-flag"])).is_err(),
        "a terminal dry run that exits non-zero"
    );

    // Paths in this repository are checked in every form; a URL to another
    // repository is not.
    let printed = "see docs/a/plain.md, ./docs/a/dot.md, ../docs/a/up.md, /srv/checkout/docs/a/abs.md, \
                   goatcore/docs/a/named.md, https://github.com/usareur-af/goatcore/blob/main/docs/a/blob.md, \
                   (https://github.com/USAREUR-AF/goatcore/tree/v1/docs/a/tree.md) and \
                   https://github.com/usareur-af/goatdocs/blob/main/docs/design/elsewhere.md";
    let want: BTreeSet<String> = ["plain", "dot", "up", "abs", "named", "blob", "tree"]
        .iter()
        .map(|n| format!("docs/a/{n}.md"))
        .collect();
    assert_eq!(doc_paths(printed), want);
    assert!(
        check_d_with(
            "standup",
            &phases,
            "./docs/design/missing.md",
            Some(catalog.into())
        )
        .is_err(),
        "a ./docs path that does not exist"
    );
    assert!(
        check_d_with(
            "standup",
            &phases,
            "https://github.com/usareur-af/goatcore/blob/main/docs/design/missing.md",
            Some(catalog.into())
        )
        .is_err(),
        "this repository's URL to a document that does not exist"
    );
    assert_eq!(
        check_d_with(
            "standup",
            &phases,
            "https://github.com/usareur-af/goatdocs/blob/main/docs/design/missing.md",
            Some(catalog.into())
        ),
        Ok(()),
        "another repository's URL is left to a link check"
    );

    // And each fails on the shape it exists to catch.
    let mut leaked = record.clone();
    leaked["inputs"][2]["value"] = serde_json::json!("hunter2");
    assert!(
        check_b(&procs, std::slice::from_ref(&leaked)).is_err(),
        "a secret's value in the record"
    );
    let bare =
        serde_json::json!({ "verb": "standup", "commands": [{ "command": line }], "inputs": [] });
    assert!(
        check_a(&procs, std::slice::from_ref(&bare)).is_err(),
        "a command with no log path"
    );
    assert!(
        check_b(&procs, std::slice::from_ref(&bare)).is_err(),
        "values with no source"
    );
    assert!(
        check_c(
            &procs,
            "add -o json to see every phase's command",
            &BTreeSet::new()
        )
        .is_err(),
        "a plan that hides its commands"
    );
    assert!(
        check_d_with(
            "standup",
            &phases,
            "docs/design/no-such-document.md",
            Some(catalog.into())
        )
        .is_err(),
        "a printed path that does not exist"
    );
}

/// Under `-o json` stdout is the report alone: what a child prints goes to
/// stderr and its log, never into the document a script parses.
#[test]
fn a_json_run_keeps_its_children_off_stdout() {
    let w = federate_world::world();
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    let root = w.root.path().to_path_buf();
    let shims = install_shims(&root);
    let mut c = w.command(&["hello-ally", "--to", "4"]);
    behind_recorder(&mut c, &shims, "pass", &elsewhere(&root));
    // Every phase's own command (`bash -euo pipefail -c ...`) talks on stdout.
    const SAID: &str = "a phase's command printing on stdout";
    c.env("GB_REC_SAY_FOR", "bash -euo").env("GB_REC_SAY", SAID);
    let (out, procs) = run_recorded(c, &root.join("rec/json"));
    ran("federate --to 4 -o json", &out, &procs, &[0]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let doc: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(doc.is_ok(), "stdout is not one JSON document: {:?}\nstdout:\n{stdout}", doc.err());
    assert!(text(&out).contains(SAID), "the child's line is on neither stream: it was not run");
    assert!(String::from_utf8_lossy(&out.stderr).contains(SAID), "the child's line is not on stderr");
    let logged = run_records(&root, "federate").iter().flat_map(|r| r["commands"].as_array().cloned().unwrap_or_default()).filter_map(|c| c["log"].as_str().and_then(|l| std::fs::read_to_string(l).ok())).any(|l| l.contains(SAID));
    assert!(logged, "the child's line is not in its log");
}
