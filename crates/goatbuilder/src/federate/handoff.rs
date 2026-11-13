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
//! and is not built; so is `--side edge`. Not built either: `import`, each
//! phase's core half against an imported result, and `status` showing the
//! object. A two-driver peering therefore stops at its first phase that needs
//! the edge and goes no further.

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

/// The refusal of `--side edge` while the edge driver's run is not built.
pub fn edge_side_not_built() -> anyhow::Error {
    goat_errors::coded(
        NOT_BUILT,
        "--side edge: the edge driver's run of a two-driver peering is not built yet (docs/design/federation-two-drivers.md); run the peering with one driver for now",
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
    let refuse = |detail: String| {
        Check {
        name: "handoff".into(),
        ok: false,
        detail,
        fix: Some("commit every file named, then run the same command again: the hand-off pins the edge driver's checkout to the controller's commit".into()),
    }
    };
    let Some(status) = git(&["status", "--porcelain", "--untracked-files=all"]) else {
        return Err(refuse(format!("{DIRTY}: {} is not a git checkout, so no hand-off can name the commit the edge driver must be at", repo.display())));
    };
    let files: Vec<&str> = status.lines().filter_map(|l| l.get(3..)).collect();
    if !files.is_empty() {
        return Err(refuse(format!("{DIRTY}: the controller's checkout has changes, so no hand-off is given: the edge driver could not be pinned to them. Commit each: {}", files.join(", "))));
    }
    match git(&["rev-parse", "HEAD"]) {
        Some(c) if !c.trim().is_empty() => Ok(c.trim().to_string()),
        _ => Err(refuse(format!(
            "{DIRTY}: {} has no commit yet",
            repo.display()
        ))),
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
            detail: "the edge driver's run, `import` and the core half against an imported result are not built yet (docs/design/federation-two-drivers.md, What has to be built); this stop is".into(),
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
