//! A release component (fleet-pins `artifact_type: release`, gate J23) in
//! `deploy`, `verify` and `rollback`: its files are fetched and verified on the
//! operator's machine (release_file.rs), the `release-file` role installs the
//! host's own architecture's file, and the file a host's `current` names is
//! held to the pin. `ops/scripts/deploy-component.sh` takes the fetch and the
//! verdict on a host's file from the hidden `release-file` verb here, so both
//! avenues say the same words.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};

use crate::push;
use crate::release_file::{self, Declared, Fetched, Pin, Source};
use crate::runner::Cmd;
use crate::update::signature::TrustRoot;
use crate::{Check, Report, EXIT_NOT_CONVERGED, EXIT_OK, EXIT_REFUSED};

/// A release that cannot be fetched or verified.
pub(crate) const CODE: &str = "GB-381";
/// What is not built for a release component.
pub(crate) const NOT_BUILT: &str = "GB-379";
/// The play of a release component that declares none of its own.
pub(crate) const PLAY: &str = "release-file-deploy.yml";
/// The play of one whose archive carries its own (`release.play`).
pub(crate) const ARCHIVE_PLAY: &str = "release-archive-play.yml";
/// The role's `release_file_root`.
const ROOT: &str = "/opt/goat/releases";
/// A directory of `<owner>/<repo>/<tag>/<file>` read instead of GitHub's
/// release downloads.
pub(crate) const MIRROR_ENV: &str = "GOATBUILDER_RELEASE_MIRROR";

/// A release component's declaration.
pub(crate) struct Component {
    pub name: String,
    pub ledger: String,
    pub declared: Option<Declared>,
}

/// `name` as the pins declare it, when it is a release component. A table that
/// cannot be read, or an unknown name, is the caller's own refusal to make.
pub(crate) fn component(repo: &Path, name: &str) -> Option<Component> {
    let table = crate::pins::load_table(&repo.join("ops/pins/fleet-pins.yaml")).ok()?;
    of(table.components.get(name)?, name)
}

fn of(spec: &Value, name: &str) -> Option<Component> {
    let s = |k: &str| spec.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    if s("artifact_type") != "release" {
        return None;
    }
    let ledger = Some(s("ledger_component")).filter(|l| !l.is_empty()).unwrap_or_else(|| name.to_string());
    Some(Component { name: name.into(), ledger, declared: spec.get("release").and_then(Declared::from_spec) })
}

/// A name that is safe as a path segment on a host: the role's own rule.
fn safe_name(n: &str) -> bool {
    regex::Regex::new(r"^[a-z0-9][a-z0-9._-]*$").unwrap().is_match(n)
}

/// One pin's verified files, one per architecture the release has.
#[derive(Debug)]
pub(crate) struct Release {
    pub files: BTreeMap<String, Fetched>,
    /// The file of variables the play takes (`-e @<file>`).
    pub vars: PathBuf,
    /// The archive's own play, when the component declares one.
    pub play: Option<PathBuf>,
}

impl Release {
    /// What both avenues print once a release is verified.
    pub(crate) fn line(&self) -> String {
        let files: Vec<String> = self.files.iter().map(|(a, f)| format!("{} ({a}) sha256 {}", f.name, f.sha256)).collect();
        let signed = self.files.values().next().map(|f| format!("signed by {}, log index {}", f.signed_by, f.log_index)).unwrap_or_default();
        format!("release verified: {}; {signed}", files.join(", "))
    }
}

/// Where verified files are kept on the operator's machine.
fn store(repo: &Path) -> PathBuf {
    Path::new(&crate::settings::state_dir_abs(repo)).join("releases")
}

/// A play out of the archive stands beside one checkout's variables and roles, so
/// the file that names it is that checkout's own.
fn vars_name(repo: &Path, c: &Component) -> String {
    match &c.declared {
        Some(d) if d.play.is_some() => format!("deploy-vars-{}.json", crate::release_play::here(repo)),
        _ => "deploy-vars.json".into(),
    }
}

/// The file of variables the play takes for `reference`.
pub(crate) fn vars_path(repo: &Path, c: &Component, reference: &str) -> Option<PathBuf> {
    let pin = Pin::parse(reference)?;
    Some(release_file::kept_dir(&store(repo), &c.name, &pin).join(vars_name(repo, c)))
}

/// Fetch and verify `c`'s release at `reference`, every architecture's file,
/// and write the play's variables beside them. GB-381 otherwise, with no host
/// touched. Files an earlier run kept are verified again before the network is
/// asked.
pub(crate) fn fetch(repo: &Path, c: &Component, reference: &str) -> Result<Release> {
    let bad = |check: &str, why: &str| -> anyhow::Error {
        goat_errors::coded(CODE, format!("refused before any host: {}'s release {reference}: {why} (check: {check})", c.name)).into()
    };
    let Some(pin) = Pin::parse(reference) else {
        return Err(bad("pin", "the pin is not <owner>/<repository>@<tag>@sha256:<digest of its checksums file>"));
    };
    let Some(declared) = &c.declared else {
        return Err(bad("declaration", "the component's `release:` does not name its checksums file, files and signer"));
    };
    if !safe_name(&c.name) || !safe_name(&c.ledger) {
        return Err(bad("declaration", "the component's name and ledger_component must be lowercase letters, digits, dots, dashes and underscores: they name a directory on each host"));
    }
    let root = TrustRoot::resolve().map_err(|e| release_file::refused(&c.name, &pin, "trust-root", e))?;
    let store = store(repo);
    let kept = Source::Kept(release_file::kept_dir(&store, &c.name, &pin));
    let source = match std::env::var_os(MIRROR_ENV).filter(|v| !v.is_empty()) {
        Some(d) => Source::Dir(PathBuf::from(d)),
        None => Source::github(),
    };
    let mut files = BTreeMap::new();
    for arch in declared.files.keys() {
        let get = |s: &Source| release_file::fetch(&c.name, &pin, declared, arch, s, &root, &store);
        let f = match get(&kept) {
            Ok(f) => f,
            Err(_) => get(&source)?,
        };
        files.insert(arch.clone(), f);
    }
    let mut play = None;
    if let Some(p) = &declared.play {
        let bad = |why: String| release_file::refused(&c.name, &pin, "play", why);
        let mut all = files.values();
        let Some(first) = all.next() else { return Err(bad("it declares no file".into())) };
        if all.any(|f| f.sha256 != first.sha256) {
            return Err(bad("its architectures have different files, so its play has no one archive".into()));
        }
        let state = crate::settings::state_dir_abs(repo);
        play = Some(crate::release_play::unpack(repo, Path::new(&state), &c.name, first, p).map_err(bad)?);
    }
    let vars = release_file::kept_dir(&store, &c.name, &pin).join(vars_name(repo, c));
    debug_assert_eq!(vars_path(repo, c, reference).as_ref(), Some(&vars));
    let mut doc = json!({
        "release_file_component": c.name,
        "release_file_ledger": c.ledger,
        "release_file_ref": pin.text(),
        "release_file_files": files.iter().map(|(a, f)| (a.clone(), json!({"src": f.path, "name": f.name, "sha256": f.sha256}))).collect::<BTreeMap<_, _>>(),
    });
    if let Some(p) = &play {
        doc["release_play"] = json!(p);
    }
    let tmp = vars.with_extension(format!("{}.partial", std::process::id()));
    std::fs::write(&tmp, format!("{doc:#}\n"))
        .and_then(|_| std::fs::rename(&tmp, &vars))
        .map_err(|e| release_file::refused(&c.name, &pin, "store", format!("{} cannot be written: {e}", vars.display())))?;
    Ok(Release { files, vars, play })
}

/// A refusal as both avenues print it: its code, then its words.
pub(crate) fn refusal(e: &anyhow::Error) -> String {
    format!("{CODE} {e:#}")
}

/// The same, where a run reads hosts and refuses nothing (`verify`).
pub(crate) fn not_compared(e: &anyhow::Error) -> String {
    let text = format!("{e:#}");
    format!("{CODE} {}", text.strip_prefix("refused before any host: ").unwrap_or(&text))
}

/// The ad-hoc `ansible` words that read the file each host's `current` names:
/// `GBFILE <uname -m> <sha256>  <path>`, or `GBFILE none`.
pub(crate) fn file_read_args(inv: &[String], group: &str, component: &str) -> Vec<String> {
    let current = format!("{ROOT}/{component}/current");
    let mut args = inv.to_vec();
    args.push(group.to_string());
    args.extend([
        "-m".into(),
        "shell".into(),
        "-a".into(),
        format!("readlink -e {current} >/dev/null && echo GBFILE $(uname -m) $(sha256sum $(readlink -e {current})) || echo GBFILE none"),
        "-o".into(),
    ]);
    args
}

pub(crate) fn file_read_cmd(repo: &Path, inv: &[String], group: &str, component: &str) -> Cmd {
    push::ansible_cmd(repo, "ansible", &file_read_args(inv, group, component))
}

/// What a host's installed file says: is it the pin's, and the words both
/// avenues print.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum File {
    Pinned(String),
    /// Another file, or none.
    Off(String),
    /// The host's file was not read.
    Unread(String),
}

/// Judge one line of [`file_read_args`]'s output against the pin's files,
/// (name, SHA-256) by architecture. `None`: the host gave no line.
pub(crate) fn judge(files: &BTreeMap<String, (String, String)>, line: Option<&str>) -> File {
    let Some(read) = line.and_then(|l| l.split_once("GBFILE ")).map(|x| x.1.trim()) else {
        return File::Unread("the installed file was not read on the host".into());
    };
    let w: Vec<&str> = read.split_whitespace().collect();
    let [machine, sha, ..] = w[..] else {
        return File::Off(format!("no file is installed ({ROOT}/<component>/current names none)"));
    };
    let arch = match machine {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    };
    match files.get(arch) {
        None => File::Off(format!("the host is {arch}, and the release has no file for it")),
        Some((name, want)) if want == sha => File::Pinned(format!("installed file {name} is the pin's (sha256 {sha})")),
        Some((name, want)) => File::Off(format!("the installed file is sha256 {sha}, and the pin's {name} is {want}")),
    }
}

impl Release {
    pub(crate) fn shas(&self) -> BTreeMap<String, (String, String)> {
        self.files.iter().map(|(a, f)| (a.clone(), (f.name.clone(), f.sha256.clone()))).collect()
    }
}

/// The hidden `release-file` verb, which deploy-component.sh calls.
#[derive(clap::Subcommand, Debug)]
pub enum ReleaseFileCmd {
    /// Fetch and verify a component's release at a pin; print one JSON object:
    /// `line` and `vars`, or `refused` (exit 1). No host is touched.
    Fetch {
        component: String,
        #[arg(long = "pin", value_name = "PIN")]
        pin: String,
    },
    /// The same, printing only the play's variables file on stdout and the rest
    /// on stderr, for a join-node phase.
    Vars {
        component: String,
        #[arg(long = "pin", value_name = "PIN")]
        pin: String,
    },
    /// Judge a host's `GBFILE` line, read from stdin, against a vars file.
    /// Exit 0 the pin's file, 1 not, 3 not read; its words on stdout.
    Judge {
        #[arg(long, value_name = "FILE")]
        vars: PathBuf,
    },
}

pub fn run(repo: &Path, cmd: &ReleaseFileCmd) -> i32 {
    match cmd {
        ReleaseFileCmd::Fetch { component: name, pin } => {
            let got = match component(repo, name) {
                Some(c) => fetch(repo, &c, pin),
                None => Err(goat_errors::coded(CODE, format!("refused before any host: {name} is not a release component of ops/pins/fleet-pins.yaml (check: declaration)")).into()),
            };
            match got {
                Ok(r) => {
                    println!("{}", json!({"line": r.line(), "vars": r.vars, "play": r.play}));
                    EXIT_OK
                }
                Err(e) => {
                    println!("{}", json!({"refused": refusal(&e), "not_compared": not_compared(&e)}));
                    EXIT_REFUSED
                }
            }
        }
        ReleaseFileCmd::Vars { component: name, pin } => {
            let got = match component(repo, name) {
                Some(c) => fetch(repo, &c, pin),
                None => Err(goat_errors::coded(CODE, format!("refused before any host: {name} is not a release component of ops/pins/fleet-pins.yaml (check: declaration)")).into()),
            };
            match got {
                Ok(r) => {
                    eprintln!(">> {name}: {}", r.line());
                    println!("{}", r.vars.display());
                    EXIT_OK
                }
                Err(e) => {
                    eprintln!("{}", refusal(&e));
                    EXIT_REFUSED
                }
            }
        }
        ReleaseFileCmd::Judge { vars } => {
            let mut line = String::new();
            let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut line);
            let files = std::fs::read_to_string(vars).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).map(|v| files_of(&v)).unwrap_or_default();
            let (exit, text) = match judge(&files, Some(line.as_str())) {
                File::Pinned(t) => (EXIT_OK, t),
                File::Off(t) => (EXIT_REFUSED, t),
                File::Unread(t) => (EXIT_NOT_CONVERGED, t),
            };
            println!("{text}");
            exit
        }
    }
}

fn files_of(vars: &Value) -> BTreeMap<String, (String, String)> {
    let s = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    vars.get("release_file_files")
        .and_then(|f| f.as_object())
        .map(|m| m.iter().map(|(a, v)| (a.clone(), (s(v, "name"), s(v, "sha256")))).collect())
        .unwrap_or_default()
}

// --- a machine's own build -------------------------------------------------

/// A release component a target type installs, on one machine's group.
pub(crate) enum Pinned {
    Yes { pin: String, play: PathBuf, target_flag: String },
    /// Why there is no pin, in words that go with the type's own skip reason.
    No(String),
}

/// Whether `name` has a release pin for `group` on `goatnet`. A component that
/// is there but could not be installed (it is an image, its play is missing,
/// its pin is not a release pin) is an error, never a skip.
pub(crate) fn pinned(repo: &Path, name: &str, goatnet: &str, group: &str) -> Result<Pinned> {
    let path = repo.join("ops/pins/fleet-pins.yaml");
    if !path.is_file() {
        return Ok(Pinned::No(format!("this checkout has no ops/pins/fleet-pins.yaml, so {name} has no pin")));
    }
    let table = crate::pins::load_table(&path)?;
    let Some(spec) = table.components.get(name) else {
        return Ok(Pinned::No(format!("ops/pins/fleet-pins.yaml declares no component {name}")));
    };
    if of(spec, name).is_none() {
        return Err(goat_errors::coded(CODE, format!("refused before any host: {name} is in ops/pins/fleet-pins.yaml and is not a release component (artifact_type: release) (check: declaration)")).into());
    }
    let d = push::declared(repo, name, goatnet, None)?;
    if d.skipped.iter().any(|g| g == group) {
        return Ok(Pinned::No(format!("ops/pins/fleet-pins.yaml declares {name} absent on {group}")));
    }
    if !d.members.iter().any(|g| g == group) {
        return Ok(Pinned::No(format!("{group} is not a member group of the goatnet {goatnet} in ops/pins/fleet-pins.yaml, so {name} has no pin for it")));
    }
    let Some(pin) = push::want(repo, group, &d.var) else {
        return Ok(Pinned::No(format!("ops/ansible/group_vars/{group}/pins.yml has no {} (declare {name}'s pin, then: goatbuilder pins)", d.var)));
    };
    if Pin::parse(&pin).is_none() {
        return Err(goat_errors::coded(CODE, format!("refused before any host: {name}'s release {pin}: the pin is not <owner>/<repository>@<tag>@sha256:<digest of its checksums file> (check: pin)")).into());
    }
    let r = push::resolve(repo, name, goatnet, None)?;
    Ok(Pinned::Yes { pin, play: r.play, target_flag: r.target_flag })
}

/// [`pinned`] on the goatnet that has `group` as a member.
pub(crate) fn pinned_anywhere(repo: &Path, name: &str, group: &str) -> Result<Pinned> {
    let path = repo.join("ops/pins/fleet-pins.yaml");
    let net = if path.is_file() {
        let t = crate::pins::load_table(&path)?;
        let has = |g: &Value| g.get("members").and_then(|m| m.as_array()).is_some_and(|m| m.iter().any(|x| x.as_str() == Some(group)));
        t.goatnets.iter().find(|(_, g)| has(g)).map(|(n, _)| n.clone())
    } else {
        None
    };
    match net {
        Some(n) => pinned(repo, name, &n, group),
        // No goatnet has the group: `pinned` says what is missing, in order.
        None => match pinned(repo, name, "", group) {
            Err(e) if e.downcast_ref::<goat_errors::Coded>().is_some_and(|c| c.code == "GB-002") => Ok(Pinned::No(format!("{group} is not a member group of any goatnet in ops/pins/fleet-pins.yaml, so {name} has no pin for it"))),
            other => other,
        },
    }
}

// --- rollback ---------------------------------------------------------------

/// The goatnets a release component runs on, as the pins declare them.
fn goatnets(repo: &Path, component: &str) -> Vec<String> {
    let Ok(table) = crate::pins::load_table(&repo.join("ops/pins/fleet-pins.yaml")) else { return Vec::new() };
    let mut v: Vec<String> = table
        .goatnets
        .keys()
        .filter(|g| push::declared(repo, component, g, None).map(|d| !d.members.is_empty()).unwrap_or(false))
        .cloned()
        .collect();
    v.sort();
    v
}

/// The one goatnet a rollback is for: `--site`, or the only one the component
/// is on. GB-055 when there are more.
fn one_goatnet(repo: &Path, component: &str, site: Option<&str>) -> Result<String> {
    if let Some(s) = site {
        return Ok(s.replace('-', "_"));
    }
    let nets = goatnets(repo, component);
    match &nets[..] {
        [one] => Ok(one.clone()),
        _ => {
            let names: Vec<String> = nets.iter().map(|g| g.replace('_', "-")).collect();
            Err(goat_errors::coded(
                "GB-055",
                format!(
                    "'{component}' is deployed on {} goatnets ({}); a rollback with no --site would flip every one, prod included. Re-run with --site <{}>.",
                    names.len(),
                    names.join(", "),
                    names.join("|")
                ),
            )
            .into())
        }
    }
}

/// What `rollback` was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Roll { n: u32, dry_run: bool },
    List,
    Status,
}

fn row(detail: String) -> Check {
    Check { name: "rollback".into(), ok: true, detail, fix: None }
}

fn bad_row(detail: String) -> Check {
    Check { name: "rollback".into(), ok: false, detail, fix: None }
}

/// GB-379: steps beyond the one file a host keeps.
pub(crate) fn steps_refusal(component: &str, n: u32) -> anyhow::Error {
    goat_errors::coded(
        NOT_BUILT,
        format!("{component} is a release component (artifact_type: release): a host keeps the one file before the one in use, so it rolls back 1 step, not {n}. Deploy the earlier pin instead"),
    )
    .into()
}

/// `rollback` of a release component: every host of one goatnet switches
/// `current` to `previous`, by the release-file role, with no fetch.
pub(crate) fn rollback(repo: &Path, c: &Component, site: Option<&str>, mode: Mode) -> Result<Report> {
    if let Mode::Roll { n, .. } = mode {
        if !(1..=7).contains(&n) {
            return Err(goat_errors::coded("GB-246", format!("N must be 1..7 (got: {n})")).into());
        }
        if n != 1 {
            return Err(steps_refusal(&c.name, n));
        }
    }
    let goatnet = one_goatnet(repo, &c.name, site)?;
    let name = goatnet.replace('_', "-");
    let dry_run = matches!(mode, Mode::Roll { dry_run: true, .. });
    let mut checks = Vec::new();
    let r = push::resolve(repo, &c.name, &goatnet, None)?;
    if matches!(mode, Mode::Roll { .. }) {
        // A rollback is a push, and reads this checkout.
        crate::converger::refuse_pull_agent(repo, &goatnet)?;
    }
    let mut trunk = String::new();
    if matches!(mode, Mode::Roll { dry_run: false, .. }) {
        let f = crate::fresh::check(repo)?;
        let refused = f.refused();
        trunk = f.trunk.clone();
        checks.push(f.check);
        if refused {
            return Ok(Report::new("rollback", checks, EXIT_REFUSED));
        }
    }
    let t = push::transport(repo, &name, &r.members, dry_run)?;
    let inv = push::inventory_args(&t);
    if mode == Mode::List || mode == Mode::Status {
        return read(repo, c, &name, &r, &inv, mode);
    }
    checks.push(row(format!("rolling {} (artifact_type=release) to the previous file on every host of {name}", c.name)));
    checks.push(row(format!("groups [{}] via {}", r.members.join(" "), t.detail)));
    let mut bad = false;
    let mut moved = 0usize;
    for g in &r.members {
        let extra = [
            format!("release_file_component={}", c.name),
            format!("release_file_ledger={}", c.ledger),
            "release_file_rollback=true".to_string(),
        ];
        let mut extra = extra.to_vec();
        if c.declared.as_ref().is_some_and(|d| d.play.is_some()) {
            // Its play is in the archive, the pin's: verified again first.
            let Some(pin) = push::want(repo, g, &r.var) else {
                return Err(goat_errors::coded(CODE, format!("refused before any host: {}'s play is in its release, and {g} has no pin for it (check: pin)", c.name)).into());
            };
            if let Some(p) = fetch(repo, c, &pin)?.play {
                // As JSON: a path is one value, whatever it holds.
                extra.push(json!({"release_play": p}).to_string());
            }
        }
        let args = push::deploy_args(repo, &inv, &r.play, &r.target_flag, g, None, &extra);
        let mut cmd = push::ansible_cmd(repo, "preflight", &args);
        if dry_run {
            checks.push(row(format!("{g}: DRY: {}", cmd.line_from(repo))));
            checks.push(row(format!("{g}: DRY: would read /var/lib/goat/{}/deploy-history.jsonl on {g} and require a rollback row", c.ledger)));
            continue;
        }
        // The rows before the play: a rollback row that was there already is not
        // this run's.
        let (before, _) = push::read_tails_status(repo, &inv, g, &c.ledger)?;
        crate::runner::phase(&format!("rollback:{g}"), &format!("the play on {g}"), "");
        if !trunk.is_empty() {
            cmd.env(crate::fresh::TRUNK_ENV, &trunk);
        }
        // The ladder judges what is deployed; this goes back to what ran before.
        cmd.env(crate::ladder::CHECKED_ENV, "1");
        let out = cmd.output().map_err(|e| goat_errors::coded("GB-320", format!("could not start ansible-preflight.sh: {e}")))?;
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        let recap: Vec<&str> = text.lines().skip_while(|l| !l.contains("PLAY RECAP")).skip(1).take(8).filter(|l| l.contains("ok=")).collect();
        if recap.is_empty() && text.contains("skipping: no hosts matched") {
            checks.push(row(format!("{g}: no host of this group in the loaded inventories (declared, not built?) — nothing rolled back, skipping")));
            continue;
        }
        for l in &recap {
            checks.push(row(format!("{g}: {}", l.trim())));
        }
        if !out.status.success() || recap.is_empty() {
            bad = true;
            let fatal: Vec<String> = text.lines().filter(|l| l.contains("fatal:") || l.contains("REFUSE")).take(3).map(|l| l.chars().take(300).collect()).collect();
            checks.push(bad_row(format!("{g}: rollback FAILED: {}", fatal.join(" | "))));
        }
        // Each host's own ledger says whether it moved, a failed play included.
        let (rows, status) = push::read_tails_status(repo, &inv, g, &c.ledger)?;
        let (expected, _) = push::expected_hosts(repo, &t, g, &r.write_role).unwrap_or_default();
        let mut answered = std::collections::HashSet::new();
        for line in rows.lines().filter(|l| !l.trim().is_empty()) {
            let host = line.split_whitespace().next().unwrap_or("").to_string();
            answered.insert(host.clone());
            let f = |k: &str| push::ledger_row_field(line, k).unwrap_or_default();
            if f("action") == "rollback" && before.lines().any(|b| b == line) {
                bad = true;
                checks.push(bad_row(format!("{host}: NOT rolled back — this run wrote no ledger row; its newest is an earlier rollback ({})", f("ts"))));
            } else if f("action") == "rollback" {
                moved += 1;
                checks.push(row(format!("{host}: rolled back: {} -> {}", crate::inspect::short(&f("prior_ref")), crate::inspect::short(&f("ref")))));
            } else {
                bad = true;
                let tail: String = line.split_once(">>").map(|x| x.1).unwrap_or(line).trim().chars().take(140).collect();
                checks.push(bad_row(format!("{host}: NOT rolled back — ledger tail: {tail}")));
            }
        }
        if let Some(code) = status {
            bad = true;
            checks.push(bad_row(format!("{g}: ledger read FAILED (exit {code})")));
        }
        for h in expected.iter().filter(|h| !answered.contains(*h)) {
            bad = true;
            checks.push(bad_row(format!("{h}: NOT rolled back — no ledger row came back from this host")));
        }
    }
    if !dry_run && moved == 0 && !bad {
        bad = true;
        checks.push(bad_row(format!("{} on {goatnet}: nothing was rolled back — no group had a host", c.name)));
    }
    let verdict = match (dry_run, bad) {
        (true, _) => format!("{} on {goatnet}: DRY: nothing was read or changed", c.name),
        (false, true) => format!("{} on {goatnet}: NOT rolled back everywhere (see above)", c.name),
        (false, false) => format!("{} on {goatnet}: every host is on the file before; the pin still names the one it left", c.name),
    };
    checks.push(if bad { bad_row(verdict) } else { row(verdict) });
    Ok(Report::new("rollback", checks, EXIT_NOT_CONVERGED))
}

/// `--list` and `--status`: each host's ledger, or the file it runs.
fn read(repo: &Path, c: &Component, name: &str, r: &push::Resolved, inv: &[String], mode: Mode) -> Result<Report> {
    let mut checks = vec![row(match mode {
        Mode::List => format!("deploy history for {} (newest first):", c.name),
        _ => format!("{} — artifact_type=release reversible=true (one step: the file before the one in use)", c.name),
    })];
    let current = format!("{ROOT}/{}/current", c.name);
    let words = match mode {
        Mode::List => format!("tac /var/lib/goat/{}/deploy-history.jsonl 2>/dev/null || true", c.ledger),
        _ => format!("cat $(dirname $(readlink -e {current}) 2>/dev/null)/.ref 2>/dev/null || echo none"),
    };
    let head = regex::Regex::new(r"^(\S+) \| ([A-Z!]+)").unwrap();
    for g in &r.members {
        let mut args = inv.to_vec();
        args.extend([g.clone(), "-m".into(), "shell".into(), "-a".into(), words.clone()]);
        let out = push::ansible_cmd(repo, "ansible", &args).output().map_err(|e| goat_errors::coded("GB-320", format!("could not start ansible: {e}")))?;
        for l in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(h) = head.captures(l) {
                checks.push(if &h[2] == "CHANGED" || &h[2] == "SUCCESS" {
                    row(format!("  (on-host @ {}, goatnet {name})", &h[1]))
                } else {
                    bad_row(format!("  {}: <unreachable>", &h[1]))
                });
            } else if mode == Mode::Status {
                if !l.trim().is_empty() {
                    checks.push(row(format!("  {}: {}", c.name, l.trim())));
                }
            } else if let Ok(v) = serde_json::from_str::<Value>(l) {
                checks.push(row(format!("{}  action={}", crate::rollback::format_row(&v), v.get("action").and_then(|a| a.as_str()).unwrap_or("deploy"))));
            }
        }
    }
    Ok(Report::new("rollback", checks, EXIT_NOT_CONVERGED))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> BTreeMap<String, (String, String)> {
        [("amd64".to_string(), ("tool-amd64".to_string(), "aa".to_string()))].into()
    }

    #[test]
    fn a_hosts_file_is_held_to_its_own_architectures_file_of_the_pin() {
        let j = |l: &str| judge(&files(), Some(l));
        let line = |x: &str| format!("host-a | CHANGED | rc=0 | (stdout) GBFILE {x}");
        assert_eq!(j(&line("x86_64 aa /opt/goat/releases/tool/aa/tool-amd64")), File::Pinned("installed file tool-amd64 is the pin's (sha256 aa)".into()));
        assert_eq!(j(&line("x86_64 bb /x")), File::Off("the installed file is sha256 bb, and the pin's tool-amd64 is aa".into()));
        assert_eq!(j(&line("aarch64 aa /x")), File::Off("the host is arm64, and the release has no file for it".into()));
        assert!(matches!(j(&line("none")), File::Off(t) if t.starts_with("no file is installed")));
        for unread in [None, Some("host-a | UNREACHABLE! => {}"), Some("")] {
            assert!(matches!(judge(&files(), unread), File::Unread(_)), "{unread:?}");
        }
    }

    #[test]
    fn only_a_release_component_is_one_and_its_ledger_defaults_to_its_name() {
        assert!(of(&json!({"artifact_type": "image"}), "tool").is_none());
        assert!(of(&json!({}), "tool").is_none());
        let c = of(&json!({"artifact_type": "release", "release": {"checksums": "SHA256SUMS", "files": {"amd64": "t"}, "signer": "s"}}), "tool").unwrap();
        assert_eq!((c.ledger.as_str(), c.declared.is_some()), ("tool", true));
        let c = of(&json!({"artifact_type": "release", "ledger_component": "led"}), "tool").unwrap();
        assert_eq!((c.ledger.as_str(), c.declared.is_none()), ("led", true));
    }

    #[test]
    fn a_name_that_leaves_its_directory_is_not_safe() {
        assert!(safe_name("goat-rope.v2"));
        for n in ["", "../x", "a b", "a;b", ".x", "A"] {
            assert!(!safe_name(n), "{n}");
        }
    }
}
