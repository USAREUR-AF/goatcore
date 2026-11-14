//! Two drivers for one peering (docs/design/federation-two-drivers.md): an
//! edge driver that reaches only the edge host and holds no CA key, and a
//! controller that reaches the core and holds the CA key, the plan, the
//! approvals and the journal. A peering opts in with `drivers: two` on its
//! `peerings:` entry; absent, or `one`, one machine drives every phase.
//!
//! Built here: each phase's side, and the controller's stop at phases 1 and 2,
//! the two where the generic hand-off object is the contract's, with that
//! object in the run's `-o json` report (member `handoff`) and the same argvs
//! in its text. A two-driver run at any later phase that needs the edge (3,
//! 11 to 14) is refused before acting (GB-267), because its object differs
//! (phase 3's result is the edge half, 11 to 13 carry files, 14 binds by id)
//! and is not built.
//!
//! Built for phase 1: `import` ([`import`]) checks the edge result against
//! the confirm code the edge driver printed, places it under
//! `<work>/handoff/1/` and journals its sha256 as an `operator-step` on phase
//! 1; the controller's `--from 1` then hashes it again ([`check_imported`]),
//! runs phase 1's core half (preflight `--side core`) and joins the two sides'
//! results ([`join_phase_1`]). The edge driver's run of phase 1 (`--side edge
//! --from 1 --to 1 --expect-commit <commit>`, [`edge_run`]) refuses a
//! checkout at another commit or with changes, and a carried file
//! (`--expect <file>=<sha256>`) that is missing or differs (GB-278); it runs
//! preflight's edge checks into `<work>/handoff/1/edge.json` and prints the
//! way back ([`way_back`]) with the confirm code. Not built: the edge
//! driver's run, the import and core half of phase 2 and later, and `status`
//! showing the object.

use crate::Check;
use std::path::Path;

/// The run is waiting on the other driver: one code for every hand-off.
pub const WAITING: &str = "GB-265";
/// The controller's checkout has changes, so no hand-off can pin it.
pub const DIRTY: &str = "GB-266";
/// This part of a two-driver peering is not built yet.
pub const NOT_BUILT: &str = "GB-267";

/// Whether the controller's stop at phase `n` gives the contract's object:
/// phases 1 and 2 only, where nothing is carried and the result is the edge's
/// `handoff/<n>/edge.json`.
pub fn hands_off(n: u32) -> bool {
    matches!(n, 1 | 2)
}

const ONE_DRIVER: &str = "run the peering with one driver for now: remove drivers: two from its peerings: entry, from a machine that reaches both the edge host and the core host";

/// The first phase of a run (`runs(n)` says whether the run holds phase n)
/// that needs the edge.
fn first_edge_phase(runs: impl Fn(u32) -> bool) -> Option<u32> {
    (1..=14).find(|&n| runs(n) && matches!(side_of(n), Side::Edge | Side::Both))
}

/// A two-driver run whose first phase that needs the edge is after 2 is
/// refused before it acts: before a deploy's live recheck, its approval or
/// phase 10's admission on our core, which would otherwise run and leave the
/// admission half done when the run reached 11. A run whose first such phase
/// is 1 or 2 stops there with the hand-off object instead, so it never
/// reaches a later one.
pub fn refused_before_running(runs: impl Fn(u32) -> bool) -> Option<anyhow::Error> {
    let n = first_edge_phase(runs).filter(|&n| !hands_off(n))?;
    Some(
        goat_errors::coded(
            NOT_BUILT,
            format!("phase {n} needs the edge, and its hand-off is not built yet: a two-driver peering (drivers: two) hands off only at phases 1 and 2 (docs/design/federation-two-drivers.md). Refused before this run acted: nothing of phase {n} ran, and no phase before it in this run. {ONE_DRIVER}"),
        )
        .into(),
    )
}

/// The refusal of a later phase that needs the edge, met inside a run. Not
/// reached while `refused_before_running` gates the run; if it is, the
/// phases before it in this run did run, and its rows above say which.
pub fn later_phase_not_built(n: u32) -> Check {
    Check {
        name: "handoff".into(),
        ok: false,
        detail: format!("{NOT_BUILT}: phase {n} needs the edge, and its hand-off is not built yet: a two-driver peering (drivers: two) hands off only at phases 1 and 2 (docs/design/federation-two-drivers.md). Nothing of phase {n} ran; the phases of this run before it did (the rows above)"),
        fix: Some(ONE_DRIVER.into()),
    }
}

/// The side of the peering a phase needs (the contract's table, "Where each
/// phase needs to run, today").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Only the edge host.
    Edge,
    /// The core host or the CA key.
    Core,
    /// Both: its edge half runs on the edge driver, its core half on the controller.
    Both,
    /// Neither host: local work, on whichever machine runs it.
    Either,
}

/// The side phase `n` needs.
pub fn side_of(n: u32) -> Side {
    match n {
        1 | 2 | 3 | 12 | 13 | 14 => Side::Both,
        11 => Side::Edge,
        8 | 10 => Side::Core,
        _ => Side::Either,
    }
}

/// Whether a peering's `drivers:` declares two drivers. Absent is one.
pub fn two(drivers: Option<&str>) -> bool {
    drivers == Some("two")
}

/// The flags the hand-off object's edge `command` carries beside `--side
/// edge`: the edge driver's run ([`edge_run`]).
#[derive(clap::Args, Debug, Default, Clone)]
pub struct EdgeRunArgs {
    /// With --side edge: the commit the edge driver's checkout must be at,
    /// as a hand-off object names it. The run refuses a checkout at another
    /// commit or with changes (GB-278).
    #[arg(long, value_name = "COMMIT")]
    pub expect_commit: Option<String>,
    /// With --side edge: a file the controller produced, under the edge
    /// driver's <work> or its checkout, and the sha256 the hand-off names.
    /// The run refuses a file that is not there or has another digest (GB-278).
    #[arg(long, value_name = "FILE=SHA256")]
    pub expect: Vec<String>,
}

/// The edge driver's checkout or carried files are not the controller's.
pub const NOT_THE_CONTROLLERS: &str = "GB-278";

/// The phases whose edge driver's run is built.
pub fn edge_half_built(n: u32) -> bool {
    n == 1
}

/// What the edge driver's run carries into the phase loop: the commit its
/// checkout was checked at, and the result file its phase writes.
#[derive(Clone, Debug)]
pub struct EdgeRun {
    pub commit: String,
    pub result: std::path::PathBuf,
}

fn not_the_controllers(why: String) -> anyhow::Error {
    goat_errors::coded(
        NOT_THE_CONTROLLERS,
        format!("{why}; nothing ran on the edge"),
    )
    .into()
}

/// The checks before the edge driver runs phase `from` (= `to`) of a
/// two-driver peering: one phase, built, with the controller's commit
/// checked out clean and every carried file as the hand-off names it. A
/// carried file is read under `work` when it is there, else in the checkout.
pub fn edge_run(
    repo: &Path,
    a: &EdgeRunArgs,
    from: u32,
    to: u32,
    work: &Path,
) -> anyhow::Result<EdgeRun> {
    if from != to {
        return Err(goat_errors::coded("GB-189", format!("--side edge runs one phase, as a hand-off names it: --from N --to N (given --from {from} --to {to})")).into());
    }
    if !edge_half_built(from) {
        return Err(goat_errors::coded(NOT_BUILT, format!("--side edge: the edge driver's run of phase {from} is not built yet; only phase 1's is (docs/design/federation-two-drivers.md); run the peering with one driver for now")).into());
    }
    let Some(expect) = a.expect_commit.as_deref() else {
        return Err(not_the_controllers("--side edge needs --expect-commit <commit>, the commit the controller's hand-off names: without it the edge driver cannot know its checkout is the controller's".into()));
    };
    let commit = match checkout(repo) {
        Ok(c) => c,
        Err(Checkout::NotGit) => return Err(not_the_controllers(format!("{} is not a git checkout, so it cannot be shown to be the controller's commit {expect}", repo.display()))),
        Err(Checkout::NoCommit) => return Err(not_the_controllers(format!("{} has no commit yet; the controller's hand-off names {expect}", repo.display()))),
        Err(Checkout::Changed(files)) => return Err(not_the_controllers(format!("the edge driver's checkout has changes, so it is not the controller's commit {expect}: {}", files.join(", ")))),
    };
    if !commit.eq_ignore_ascii_case(expect.trim()) {
        return Err(not_the_controllers(format!("the edge driver's checkout is at {commit}, and the controller's hand-off names {expect}: check out that commit, then run the same command again")));
    }
    for e in &a.expect {
        let Some((file, want)) = e.split_once('=') else {
            return Err(goat_errors::coded(
                "GB-189",
                format!("--expect takes <file>=<sha256>, as a hand-off names it, not '{e}'"),
            )
            .into());
        };
        let rel = Path::new(file);
        if file.is_empty()
            || rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(not_the_controllers(format!("--expect {file}: a carried file is a path under <work> or the checkout, as the hand-off names it")));
        }
        let want = want.trim().trim_start_matches("sha256:").to_lowercase();
        let at = [work.join(rel), repo.join(rel)]
            .into_iter()
            .find(|p| p.is_file());
        let Some(at) = at else {
            return Err(not_the_controllers(format!("the carried file {file} is neither under {} nor in the checkout: copy it from the controller's <work> to the same path here", work.display())));
        };
        let bytes = std::fs::read(&at).map_err(|e| {
            not_the_controllers(format!(
                "the carried file {} cannot be read ({e})",
                at.display()
            ))
        })?;
        let got = hex_sha256(&bytes);
        if got != want {
            return Err(not_the_controllers(format!("the carried file {} has sha256 {got}, and the hand-off names {want}: copy it from the controller again", at.display())));
        }
    }
    Ok(EdgeRun {
        commit,
        result: work.join(result_of(from)),
    })
}

/// The edge driver's way back from phase `n`: the object (`run_on`
/// `controller`, no `command`) with the result it wrote, its sha256 and the
/// confirm code, which `import` carries filled in; and the text, which names
/// the same argvs.
pub fn way_back(with: &str, n: u32, e: &EdgeRun) -> anyhow::Result<Stop> {
    let bytes = std::fs::read(&e.result).map_err(|err| {
        anyhow::anyhow!(
            "the edge result {} was not written ({err})",
            e.result.display()
        )
    })?;
    let sha = hex_sha256(&bytes);
    let code = sha.as_bytes()[..CODE_DIGITS]
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let words = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let phase = n.to_string();
    let result = result_of(n);
    let import = words(&[
        "goatbuilder",
        "federate",
        "import",
        with,
        &phase,
        "{result}",
        "--sha256",
        &code,
    ]);
    let then = words(&["goatbuilder", "federate", with, "--from", &phase]);
    let object = serde_json::json!({
        "phase": n,
        "code": WAITING,
        "run_on": "controller",
        "command": null,
        "commit": e.commit,
        "carry": [],
        "result": result,
        "result_path": e.result.display().to_string(),
        "result_sha256": format!("sha256:{sha}"),
        "confirm_code": code,
        "import": import,
        "then": then,
    });
    let text = format!(
        "{WAITING}: phase {n}'s edge half is done: the run is waiting on the controller.\nThe confirm code: {code}\nBring {} to the controller; never send this code with the file. There, run:\n  {}\nthen:\n  {}",
        e.result.display(),
        shell_line(&import),
        shell_line(&then),
    );
    Ok(Stop {
        object,
        rows: vec![Check {
            name: "handoff".into(),
            ok: true,
            detail: text,
            fix: None,
        }],
    })
}

/// `goatbuilder federate import <partner> <phase> <result> --sha256 <code>`:
/// the hand-off object's `import`. Built for phase 1 ([`import`]); a later
/// phase's import is refused as not built (GB-267).
#[derive(clap::Args, Debug, Clone)]
pub struct ImportArgs {
    /// The peering name (the counterparty, as declared).
    pub with: String,
    /// The phase whose edge result this is.
    pub phase: u32,
    /// The result file the edge driver wrote.
    pub result: std::path::PathBuf,
    /// The confirm code the edge driver printed (the result's sha256, or its
    /// first digits).
    #[arg(long, value_name = "CODE")]
    pub sha256: String,
    /// The working directory (default `$FED_WORK`, else /var/lib/goat-federation/<peering>).
    #[arg(long)]
    pub work: Option<String>,
}

/// The edge result cannot be imported (gate row B10).
pub const UNIMPORTABLE: &str = "GB-277";

/// The phases whose import and core half are built.
pub fn core_half_built(n: u32) -> bool {
    n == 1
}

/// The result a phase-`n` hand-off names, under the writer's `<work>`.
pub fn result_of(n: u32) -> String {
    format!("handoff/{n}/edge.json")
}

/// The fewest hex digits of the sha256 a confirm code must give: the edge
/// driver prints 32.
const CODE_DIGITS: usize = 32;

fn hex_sha256(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

fn unimportable(why: String) -> anyhow::Error {
    goat_errors::coded(
        UNIMPORTABLE,
        format!("{why}; nothing was placed or recorded"),
    )
    .into()
}

/// A confirm code as typed: its hex digits, lowercased, spaces and dashes
/// between groups dropped. None when it holds anything else.
fn code_digits(code: &str) -> Option<String> {
    let d: String = code
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_lowercase();
    (!d.is_empty() && d.chars().all(|c| c.is_ascii_hexdigit())).then_some(d)
}

/// Whether `doc` is phase 1's edge result for `with`: preflight's result
/// (`goat-federation-preflight/v1`) for the edge side's checks.
fn phase_1_edge_result(doc: &serde_json::Value, with: &str) -> Result<(), String> {
    if doc["schema"] != "goat-federation-preflight/v1" {
        return Err(format!(
            "it is not a phase 1 result (schema {}, not goat-federation-preflight/v1)",
            doc["schema"]
        ));
    }
    if doc["peering"] != with {
        return Err(format!(
            "it is peering {}'s result, not {with}'s",
            doc["peering"]
        ));
    }
    if doc["side"] != "edge" {
        return Err(format!(
            "it holds the {} side's checks, not the edge's",
            doc["side"]
        ));
    }
    if doc["ready"].as_bool().is_none() || !doc["results"].is_array() {
        return Err("it has no ready flag or no results".into());
    }
    Ok(())
}

/// `goatbuilder federate import <with> <phase> <result> --sha256 <code>`: the
/// controller takes in an edge result. The code is the one the edge driver
/// printed: at least the first 32 hex digits of the file's sha256. The file
/// must be this phase's edge result for this peering, and this peering's
/// journal must be waiting on phase `phase` for the edge. Then the file is
/// placed at `<work>/handoff/<phase>/edge.json` and its whole sha256 is
/// journaled as an `operator-step` confirmation on the phase, which the
/// phase's core half reads (`--from <phase>`). Anything else is GB-277, and
/// nothing is placed or recorded.
pub fn import(a: &ImportArgs, work: &Path) -> anyhow::Result<crate::Report> {
    if !core_half_built(a.phase) {
        return Err(import_not_built(a));
    }
    let bytes = std::fs::read(&a.result).map_err(|e| {
        unimportable(format!(
            "import: {} cannot be read ({e})",
            a.result.display()
        ))
    })?;
    let sha = hex_sha256(&bytes);
    let code = code_digits(&a.sha256).ok_or_else(|| {
        unimportable(format!(
            "import: the confirm code '{}' is not hex digits",
            a.sha256
        ))
    })?;
    if code.len() < CODE_DIGITS {
        return Err(unimportable(format!("import: the confirm code gives {} hex digits; the edge driver prints {CODE_DIGITS}, and fewer do not bind the file", code.len())));
    }
    if !sha.starts_with(&code) {
        return Err(unimportable(format!("import: {} is not the file the edge driver confirmed: its sha256 begins {}, and the code given is {}", a.result.display(), &sha[..CODE_DIGITS], &code[..CODE_DIGITS.min(code.len())])));
    }
    let doc: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| unimportable(format!("import: {} is not JSON ({e})", a.result.display())))?;
    phase_1_edge_result(&doc, &a.with).map_err(|why| {
        unimportable(format!(
            "import: {} is not phase {}'s edge result for {}: {why}",
            a.result.display(),
            a.phase,
            a.with
        ))
    })?;
    let _lock = super::lock::PeeringLock::take(work, &a.with)?;
    let Some(mut state) = super::state::State::read(work)? else {
        return Err(unimportable(format!("import: no run of {} is waiting on the edge driver here ({} has no run journal): run goatbuilder federate {} --from {} first", a.with, work.display(), a.with, a.phase)));
    };
    if state.peering != a.with {
        return Err(unimportable(format!(
            "import: {} holds peering {}'s journal, not {}'s",
            work.display(),
            state.peering,
            a.with
        )));
    }
    let waiting = state.phases.get(&a.phase).is_some_and(|p| {
        p.outcome == super::state::Outcome::Stopped && p.detail.starts_with(WAITING)
    });
    if !waiting {
        return Err(unimportable(format!("import: peering {} is not waiting on the edge driver at phase {}: run goatbuilder federate {} --from {} to reach its hand-off first", a.with, a.phase, a.with, a.phase)));
    }
    let rel = result_of(a.phase);
    let dest = work.join(&rel);
    let dir = dest.parent().expect("a hand-off result has a folder");
    std::fs::create_dir_all(dir)
        .map_err(|e| super::lock::io_error(format!("cannot create {}", dir.display()), e))?;
    let tmp = dir.join(format!(".edge.json.{}", std::process::id()));
    std::fs::write(&tmp, &bytes)
        .map_err(|e| super::lock::io_error(format!("cannot write {}", tmp.display()), e))?;
    std::fs::rename(&tmp, &dest)
        .map_err(|e| super::lock::io_error(format!("cannot place {}", dest.display()), e))?;
    let run = state.open_run("import", a.phase, a.phase, false, "");
    let bound = format!("sha256:{sha}");
    let mut values = std::collections::BTreeMap::new();
    values.insert("edge_result".to_string(), rel.clone());
    state.confirm(&run, a.phase, "operator-step", "flag", &bound, values);
    state.close_run(&run, "passed");
    state.save(work)?;
    let rows = vec![
        Check { name: "import".into(), ok: true, detail: format!("phase {}'s edge result for {} is the file the edge driver confirmed ({bound}); placed at {}", a.phase, a.with, dest.display()), fix: None },
        Check { name: "next".into(), ok: true, detail: format!("carry on here: goatbuilder federate {} --from {}", a.with, a.phase), fix: None },
    ];
    Ok(crate::Report::new(
        "federate-import",
        rows,
        crate::EXIT_REFUSED,
    ))
}

/// The edge result imported for phase `n` and the sha256 it was imported at,
/// from the journal's newest `operator-step` on `n` that names one.
pub fn imported(state: &super::state::State, n: u32) -> Option<(String, String)> {
    state
        .confirmations
        .iter()
        .rev()
        .find(|c| c.kind == "operator-step" && c.phase == n)
        .and_then(|c| {
            c.values
                .get("edge_result")
                .map(|f| (f.clone(), c.bound_to.clone()))
        })
}

/// The imported edge result for phase `n`, hashed again before it is read:
/// its document when its sha256 is the one journaled, else the stop.
pub fn check_imported(
    work: &Path,
    n: u32,
    file: &str,
    bound: &str,
) -> Result<serde_json::Value, Check> {
    let path = work.join(file);
    let stop = |why: String| {
        Check {
        name: "handoff".into(),
        ok: false,
        detail: format!("{WAITING}: phase {n}'s imported edge result {why}, so phase {n}'s core half did not run"),
        fix: Some(format!("bring the edge driver's file again and import it: goatbuilder federate import <peering> {n} <file> --sha256 <code>")),
    }
    };
    let bytes = std::fs::read(&path)
        .map_err(|e| stop(format!("{} cannot be read ({e})", path.display())))?;
    let sha = format!("sha256:{}", hex_sha256(&bytes));
    if sha != bound {
        return Err(stop(format!(
            "{} has changed since it was imported (it is {sha}, and {bound} was imported)",
            path.display()
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| stop(format!("{} is not JSON ({e})", path.display())))
}

/// The edge checks that do not pass in a phase 1 edge result, as `check
/// outcome code` words, or nothing when it is ready.
pub fn phase_1_edge_problems(edge: &serde_json::Value) -> Vec<String> {
    let bad = |o: &str| o == "FAIL" || o == "BLOCKED";
    let mut out: Vec<String> = edge["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| bad(r["outcome"].as_str().unwrap_or_default()))
        .map(|r| {
            format!(
                "{} {} {}",
                r["check"].as_str().unwrap_or("?"),
                r["outcome"].as_str().unwrap_or("?"),
                r["code"].as_str().unwrap_or("")
            )
            .trim()
            .to_string()
        })
        .collect();
    if out.is_empty() && edge["ready"] != true {
        out.push("the edge result says it is not ready".into());
    }
    out
}

/// Phase 1's two sides joined into `<work>/preflight/result.json`: the core
/// half's result there and the imported edge result, written as one result
/// for both sides, ready only when both are.
pub fn join_phase_1(work: &Path, edge: &serde_json::Value) -> anyhow::Result<bool> {
    let path = work.join("preflight/result.json");
    let core: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).map_err(|e| {
            anyhow::anyhow!(
                "phase 1's core half left no result at {} ({e})",
                path.display()
            )
        })?)
        .map_err(|e| anyhow::anyhow!("{} is not JSON ({e})", path.display()))?;
    anyhow::ensure!(
        core["side"] == "core",
        "{} holds the {} side's result, not the core half's",
        path.display(),
        core["side"]
    );
    let ready = core["ready"] == true && edge["ready"] == true;
    let tag = |side: &'static str, v: &serde_json::Value| -> Vec<serde_json::Value> {
        v["results"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|mut r| {
                r["side"] = serde_json::Value::from(side);
                r
            })
            .collect()
    };
    let mut results = tag("edge", edge);
    results.extend(tag("core", &core));
    let joined = serde_json::json!({
        "schema": core["schema"],
        "peering": core["peering"],
        "only": [],
        "side": "both",
        "ready": ready,
        "results": results,
        "joined": {"edge": "imported (handoff/1/edge.json)", "core": "this run"},
    });
    let tmp = path.with_extension("json.joining");
    std::fs::write(&tmp, serde_json::to_string_pretty(&joined)? + "\n")?;
    std::fs::rename(&tmp, &path)?;
    Ok(ready)
}

/// The refusal of `import` while it is not built.
pub fn import_not_built(a: &ImportArgs) -> anyhow::Error {
    goat_errors::coded(
        NOT_BUILT,
        format!(
            "import: the controller's import of an edge result is not built yet (docs/design/federation-two-drivers.md); nothing was read or recorded for phase {} of peering {}; run the peering with one driver for now",
            a.phase, a.with
        ),
    )
    .into()
}

/// What the controller prints at a hand-off stop: the object, and the rows
/// of the text a person reads.
pub struct Stop {
    pub object: serde_json::Value,
    pub rows: Vec<Check>,
}

/// The checkout's commit, when it is a git checkout with nothing uncommitted;
/// otherwise the refusal, naming each file with changes.
pub fn clean_commit(repo: &Path) -> Result<String, Check> {
    let refuse = |detail: String| {
        Check {
        name: "handoff".into(),
        ok: false,
        detail,
        fix: Some("commit every file named, then run the same command again: the hand-off pins the edge driver's checkout to the controller's commit".into()),
    }
    };
    match checkout(repo) {
        Ok(c) => Ok(c),
        Err(Checkout::NotGit) => Err(refuse(format!("{DIRTY}: {} is not a git checkout, so no hand-off can name the commit the edge driver must be at", repo.display()))),
        Err(Checkout::Changed(files)) => Err(refuse(format!("{DIRTY}: the controller's checkout has changes, so no hand-off is given: the edge driver could not be pinned to them. Commit each: {}", files.join(", ")))),
        Err(Checkout::NoCommit) => Err(refuse(format!("{DIRTY}: {} has no commit yet", repo.display()))),
    }
}

/// Why a checkout is not one clean commit.
enum Checkout {
    NotGit,
    Changed(Vec<String>),
    NoCommit,
}

/// The checkout's commit, when it is a git checkout with nothing uncommitted.
fn checkout(repo: &Path) -> Result<String, Checkout> {
    let git = |args: &[&str]| {
        crate::runner::Cmd::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
    };
    let Some(status) = git(&["status", "--porcelain", "--untracked-files=all"]) else {
        return Err(Checkout::NotGit);
    };
    let files: Vec<String> = status
        .lines()
        .filter_map(|l| l.get(3..))
        .map(String::from)
        .collect();
    if !files.is_empty() {
        return Err(Checkout::Changed(files));
    }
    match git(&["rev-parse", "HEAD"]) {
        Some(c) if !c.trim().is_empty() => Ok(c.trim().to_string()),
        _ => Err(Checkout::NoCommit),
    }
}

/// The controller's stop at phase `n` of peering `with`, pinned to `commit`:
/// the object of "The hand-off object", for a phase whose edge half (or the
/// whole of an edge phase) runs on the edge driver.
pub fn stop(with: &str, n: u32, commit: &str) -> Stop {
    let words = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let phase = n.to_string();
    let command = words(&[
        "goatbuilder",
        "federate",
        with,
        "--side",
        "edge",
        "--from",
        &phase,
        "--to",
        &phase,
        "--expect-commit",
        commit,
    ]);
    let result = format!("handoff/{n}/edge.json");
    let import = words(&[
        "goatbuilder",
        "federate",
        "import",
        with,
        &phase,
        "{result}",
        "--sha256",
        "{code}",
    ]);
    // A both phase's core half runs at N against the imported result; an edge
    // phase has none, so the controller goes on to the next phase.
    let next = if side_of(n) == Side::Edge {
        (n + 1).to_string()
    } else {
        phase.clone()
    };
    let then = words(&["goatbuilder", "federate", with, "--from", &next]);
    let object = serde_json::json!({
        "phase": n,
        "code": WAITING,
        "run_on": "edge",
        "command": command,
        "commit": commit,
        "carry": [],
        "result": result,
        "import": import,
        "then": then,
    });
    let text = format!(
        "{WAITING}: phase {n} needs the edge: the run is waiting on the edge driver.\nOn the edge driver, with its checkout at {commit}, run:\n  {}\nbring the file it writes ({result}, under its <work>) here, then import it, with the confirm code it prints:\n  {}\nthen carry on here:\n  {}",
        shell_line(&command),
        shell_line(&import),
        shell_line(&then),
    );
    let rows = vec![
        Check { name: "handoff".into(), ok: true, detail: text, fix: None },
        Check {
            name: "note".into(),
            ok: true,
            detail: "phase 1's edge driver's run, import and core half are built; phase 2's and later are not (docs/design/federation-two-drivers.md, What has to be built)".into(),
            fix: None,
        },
    ];
    Stop { object, rows }
}

/// An argv as one line for a POSIX shell: a word of only safe characters as
/// it is, any other in single quotes.
pub fn shell_line(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if !a.is_empty()
                && a.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c))
            {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_phase_has_the_side_the_contract_gives_it() {
        let sides: Vec<(u32, Side)> = (1..=14).map(|n| (n, side_of(n))).collect();
        for (n, s) in sides {
            let want = match n {
                1 | 2 | 3 | 12 | 13 | 14 => Side::Both,
                11 => Side::Edge,
                8 | 10 => Side::Core,
                _ => Side::Either,
            };
            assert_eq!(s, want, "phase {n}");
        }
    }

    #[test]
    fn an_edge_phase_hands_on_to_the_next_phase() {
        let s = stop("xab", 11, "c0ffee");
        assert_eq!(
            s.object["then"],
            serde_json::json!(["goatbuilder", "federate", "xab", "--from", "12"])
        );
        let s = stop("xab", 3, "c0ffee");
        assert_eq!(
            s.object["then"],
            serde_json::json!(["goatbuilder", "federate", "xab", "--from", "3"])
        );
    }

    #[test]
    fn two_drivers_only_when_declared_two() {
        assert!(two(Some("two")));
        assert!(!two(Some("one")));
        assert!(!two(None));
    }

    #[test]
    fn a_word_with_a_token_is_quoted() {
        let line = shell_line(&["import".into(), "{result}".into(), "it's".into()]);
        assert_eq!(line, "import '{result}' 'it'\\''s'");
    }

    #[test]
    fn a_run_is_refused_up_front_when_its_first_edge_phase_is_after_2() {
        let range = |from: u32, to: u32| move |n: u32| n >= from && n <= to;
        // From 1 or 2: the stop hands it off before any later phase.
        assert!(refused_before_running(range(1, 14)).is_none());
        assert!(refused_before_running(range(2, 13)).is_none());
        // 4 to 10 needs no edge (8 and 10 are core): nothing to refuse.
        assert!(refused_before_running(range(4, 10)).is_none());
        // Deploy (10 to 13) and its --from 11 reach 11 on the edge.
        for (from, to, n) in [
            (10, 13, 11),
            (11, 13, 11),
            (3, 3, 3),
            (4, 11, 11),
            (14, 14, 14),
        ] {
            let e = refused_before_running(range(from, to))
                .expect("refused")
                .to_string();
            assert!(
                e.contains(&format!("nothing of phase {n} ran")),
                "{from}..{to}: {e}"
            );
            assert!(!e.contains("Nothing ran"), "{e}");
        }
    }
}
