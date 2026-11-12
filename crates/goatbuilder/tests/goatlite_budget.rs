//! Gate row G2: the first run of a goatlite does not grow
//! (docs/design/goatlite-growth.md §1 and §8).
//!
//! A goatlite's first run is one file of four facts and one command. Growing a
//! goatlite into a full environment happens later and at growth time, so
//! nothing may be added to the first run to prepare for it. This test pins
//! today's first-run budget as a ceiling:
//!
//! - the facts the learner must supply (the schema's `lite.required`), and a
//!   file of exactly those facts validates;
//! - the stops where the run waits for the learner (`MANUAL` steps);
//! - the lite LAN standup plan: its phases, its numbered steps, every titled
//!   row (the two unnumbered ones by name), its command rows, and the shell
//!   command separators inside them, so work appended to an existing step counts;
//! - `docs/tutorials/00-goatlite.md` before the goatnet works (the assay of its
//!   §7): numbered sections, every heading at any level, fenced blocks and their
//!   lines, and inline code spans.
//!
//! What it cannot see: a prompt a play or script asks during a real run (the dry
//! run does not run them), and the pages the tutorial sends the learner to.
//!
//! A count that drops is welcome: lower the ceiling in the same change. A count
//! that rises is a decision about the first run, not a side effect: it changes
//! the budget in goatlite-growth.md §8 and here together, with the reason.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The first-run budget as measured on 2026-11-12 at the commit this test
/// landed on. Each is a ceiling.
const REQUIRED_FACTS: &[&str] = &["name", "host", "ssh_user", "admin"];
const MAX_MANUAL_STOPS: usize = 1;
const MAX_PHASES: usize = 12;
const MAX_STEPS: usize = 36;
/// Every row the plan titles `=== … ===`: the numbered steps and the two
/// image-pull rows.
const MAX_TITLED_ROWS: usize = 38;
const TITLED_UNNUMBERED: &[&str] = &[
    "=== Image pull [A ∥] every pinned image for pasture_hosts, pulled on its hosts in parallel (joined before phase 5) ===",
    "=== join the image pull before phase 5 ===",
];
const MAX_COMMAND_ROWS: usize = 33;
/// Shell command separators (`&&`, `||`, `;`, `|`) across every command row: work
/// appended inside an existing step raises it.
const MAX_COMMAND_SEPARATORS: usize = 59;
const MAX_TUTORIAL_SECTIONS: usize = 7;
const MAX_TUTORIAL_HEADINGS: usize = 18;
const MAX_TUTORIAL_BLOCKS: usize = 36;
const MAX_TUTORIAL_LINES: usize = 75;
const MAX_TUTORIAL_INLINE_CODE: usize = 159;

/// The heading that follows "it works" in tutorial 00: everything before it is
/// the way to a working goatnet.
const AFTER_IT_WORKS: &str = "\n## 8. Use it";

const WHY: &str =
    "goatlite's first run may not grow (gate row G2, docs/design/goatlite-growth.md §1): \
prepare for growth at growth time. If this change is a deliberate decision about the first run, \
change the budget in goatlite-growth.md §8 and in this test together, with the reason";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn fixture() -> PathBuf {
    repo().join("ops/goatbuilder/fixtures/goatnet-from-scratch/00-goatlite/goatbuilder.yaml")
}

/// The learner's file: the lite fixture's `schema`, `preset` and `lite:` block,
/// and nothing else.
fn four_fact_file() -> String {
    let fx: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    let mut doc = serde_yaml::Mapping::new();
    for k in ["schema", "preset", "lite"] {
        doc.insert(
            k.into(),
            fx.get(k)
                .unwrap_or_else(|| panic!("the lite fixture has no {k}"))
                .clone(),
        );
    }
    serde_yaml::to_string(&doc).unwrap()
}

fn goatbuilder(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(args)
        .env("GOAT_MESH_PROBE", "0")
        .output()
        .expect("run goatbuilder")
}

#[test]
fn the_learner_supplies_four_facts_and_no_more() {
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo().join("ops/goatbuilder/schema/goatbuilder-v1.schema.json"))
            .unwrap(),
    )
    .unwrap();
    let required: Vec<&str> = schema["properties"]["lite"]["required"]
        .as_array()
        .expect("the schema's lite block lists what it requires")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(required, REQUIRED_FACTS, "{WHY}");
    let lite = serde_yaml::from_str::<serde_yaml::Value>(&four_fact_file()).unwrap();
    let keys: Vec<String> = lite["lite"]
        .as_mapping()
        .unwrap()
        .keys()
        .map(|k| k.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        keys, REQUIRED_FACTS,
        "the lite fixture declares exactly the required facts"
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("goatlite.yaml");
    std::fs::write(&path, four_fact_file()).unwrap();
    let o = goatbuilder(&["validate", path.to_str().unwrap()]);
    assert!(
        o.status.success(),
        "a file of the four facts validates: {}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
}

/// The phase lines of a standup plan: `(phase number, kind)` for each step.
fn steps(plan: &serde_json::Value) -> Vec<(u32, String)> {
    titled(plan)
        .iter()
        .filter_map(|d| {
            let d = d.strip_prefix("=== Phase ")?;
            let (n, rest) = d.split_once(' ')?;
            let kind = rest.strip_prefix('[')?.split_once(']')?.0;
            Some((n.parse().ok()?, kind.to_string()))
        })
        .collect()
}

/// Every titled row of the plan (`=== … ===`), numbered or not.
fn titled(plan: &serde_json::Value) -> Vec<String> {
    rows(plan, "phase")
        .into_iter()
        .filter(|d| d.starts_with("=== "))
        .collect()
}

fn rows(plan: &serde_json::Value, name: &str) -> Vec<String> {
    plan["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] == name)
        .filter_map(|c| c["detail"].as_str().map(String::from))
        .collect()
}

/// The shell command separators in a command: `&&`, `||`, `;` and `|`.
fn separators(cmd: &str) -> usize {
    let (mut n, b) = (0, cmd.as_bytes());
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1)) {
            (b'&', Some(b'&')) | (b'|', Some(b'|')) => {
                n += 1;
                i += 2;
            }
            (b';', _) | (b'|', _) => {
                n += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    n
}

/// What the budget measures in a plan.
#[derive(Debug, PartialEq)]
struct PlanBudget {
    phases: usize,
    steps: usize,
    manual: usize,
    titled: usize,
    command_rows: usize,
    separators: usize,
}

fn plan_budget(plan: &serde_json::Value) -> PlanBudget {
    let steps = steps(plan);
    let mut phases: Vec<u32> = steps.iter().map(|s| s.0).collect();
    phases.dedup();
    let cmds = rows(plan, "cmd");
    PlanBudget {
        phases: phases.len(),
        steps: steps.len(),
        manual: steps.iter().filter(|s| s.1 == "MANUAL").count(),
        titled: titled(plan).len(),
        command_rows: cmds.len(),
        separators: cmds.iter().map(|c| separators(c)).sum(),
    }
}

#[test]
fn the_first_run_gains_no_phase_step_command_or_stop() {
    // A checkout holding only the learner's file: the dry run prints the whole
    // lite plan from it and writes nothing.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("goatlite.yaml"), four_fact_file()).unwrap();
    let o = goatbuilder(&[
        "-o",
        "json",
        "standup",
        "-C",
        dir.path().to_str().unwrap(),
        "pasture",
        "--dry-run",
    ]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "the lite dry run plans: {out}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let plan: serde_json::Value = serde_json::from_str(&out).expect("the dry run's JSON");
    assert!(
        out.contains("profile=lan"),
        "a goatlite's first run is a LAN run: {out}"
    );
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        entries,
        vec![std::ffi::OsString::from("goatlite.yaml")],
        "a dry run writes nothing"
    );

    let b = plan_budget(&plan);
    let m = format!("{b:?}");
    assert!(
        b.phases <= MAX_PHASES,
        "{m}: more than {MAX_PHASES} phases. {WHY}"
    );
    assert!(
        b.steps <= MAX_STEPS,
        "{m}: more than {MAX_STEPS} steps. {WHY}"
    );
    assert!(
        b.manual <= MAX_MANUAL_STOPS,
        "{m}: more than {MAX_MANUAL_STOPS} stop(s) for the learner. {WHY}"
    );
    assert!(
        b.titled <= MAX_TITLED_ROWS,
        "{m}: more than {MAX_TITLED_ROWS} titled rows. {WHY}"
    );
    assert!(
        b.command_rows <= MAX_COMMAND_ROWS,
        "{m}: more than {MAX_COMMAND_ROWS} commands. {WHY}"
    );
    assert!(b.separators <= MAX_COMMAND_SEPARATORS, "{m}: more than {MAX_COMMAND_SEPARATORS} command separators (work appended inside a step). {WHY}");
    let unnumbered: Vec<String> = titled(&plan)
        .into_iter()
        .filter(|t| !t.starts_with("=== Phase "))
        .collect();
    assert!(
        unnumbered
            .iter()
            .all(|t| TITLED_UNNUMBERED.contains(&t.as_str())),
        "{unnumbered:?}: a titled row that is not a numbered phase. {WHY}"
    );
    // Guard against a parse that silently counts nothing.
    assert!(
        b.steps >= 10 && b.command_rows >= 10 && b.separators >= 10,
        "the plan was read: {m}"
    );

    // The negative controls, on this plan: each way first-run work could slip in
    // raises a count.
    let mut grown = plan.clone();
    let checks = grown["checks"].as_array_mut().unwrap();
    let store = checks
        .iter()
        .position(|c| {
            c["name"] == "cmd"
                && c["detail"]
                    .as_str()
                    .is_some_and(|d| d.contains("posture.py' init"))
        })
        .expect("the lite secrets-store step");
    let appended = format!(
        "{} && provision-growth-key pasture",
        checks[store]["detail"].as_str().unwrap()
    );
    checks[store]["detail"] = serde_json::Value::from(appended);
    assert_eq!(
        plan_budget(&grown).separators,
        b.separators + 1,
        "work appended to a step is counted"
    );
    let mut titled_more = plan.clone();
    titled_more["checks"].as_array_mut().unwrap().push(
        serde_json::json!({"name": "phase", "ok": true, "detail": "=== prepare for growth ==="}),
    );
    assert_eq!(
        plan_budget(&titled_more).titled,
        b.titled + 1,
        "a titled row that is not a numbered phase is counted"
    );
}

/// How much of tutorial 00 comes before the goatnet works.
#[derive(Debug, PartialEq, Clone, Copy)]
struct TutorialBudget {
    /// Numbered `## N.` sections.
    sections: usize,
    /// Every heading, numbered or not, at any level.
    headings: usize,
    /// Fenced blocks (indented fences included) and the non-comment lines in them.
    blocks: usize,
    lines: usize,
    /// Inline code spans outside the fenced blocks.
    inline_code: usize,
}

fn tutorial_budget(text: &str) -> TutorialBudget {
    let before = &text[..text
        .find(AFTER_IT_WORKS)
        .expect("tutorial 00 has its '## 8. Use it' section")];
    let mut b = TutorialBudget {
        sections: 0,
        headings: 0,
        blocks: 0,
        lines: 0,
        inline_code: 0,
    };
    let mut fenced = false;
    for l in before.lines() {
        if l.trim_start().starts_with("```") {
            fenced = !fenced;
            b.blocks += usize::from(fenced);
            continue;
        }
        if fenced {
            b.lines += usize::from(!l.trim().is_empty() && !l.trim_start().starts_with('#'));
            continue;
        }
        let hashes = l.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && l[hashes..].starts_with(' ') {
            b.headings += 1;
            b.sections += usize::from(
                hashes == 2
                    && l[3..]
                        .split_once(". ")
                        .is_some_and(|(n, _)| n.parse::<u32>().is_ok()),
            );
        }
        b.inline_code += l.split('`').count().saturating_sub(1) / 2;
    }
    b
}

#[test]
fn tutorial_00_gains_no_step_before_the_goatnet_works() {
    let text = std::fs::read_to_string(repo().join("docs/tutorials/00-goatlite.md")).unwrap();
    let b = tutorial_budget(&text);
    let m = format!("before '## 8. Use it': {b:?}");
    assert!(b.sections <= MAX_TUTORIAL_SECTIONS, "{m}. {WHY}");
    assert!(b.headings <= MAX_TUTORIAL_HEADINGS, "{m}. {WHY}");
    assert!(b.blocks <= MAX_TUTORIAL_BLOCKS, "{m}. {WHY}");
    assert!(b.lines <= MAX_TUTORIAL_LINES, "{m}. {WHY}");
    assert!(b.inline_code <= MAX_TUTORIAL_INLINE_CODE, "{m}. {WHY}");

    // The negative controls: each way a step could be added before "it works" is counted.
    let block = text.replacen(
        AFTER_IT_WORKS,
        "\n```text\ngoatbuilder prepare-for-growth pasture\n```\n\n## 8. Use it",
        1,
    );
    assert_eq!(
        tutorial_budget(&block),
        TutorialBudget {
            blocks: b.blocks + 1,
            lines: b.lines + 1,
            ..b
        }
    );
    let first = text.find("\n## 1.").expect("tutorial 00 has its §1");
    let mut unnumbered = text.clone();
    unnumbered.insert_str(
        first,
        "\n## Before you begin\n\nCreate an account and run `goat-account create`.\n",
    );
    assert_eq!(
        tutorial_budget(&unnumbered),
        TutorialBudget {
            headings: b.headings + 1,
            inline_code: b.inline_code + 1,
            ..b
        }
    );
    let mut sub = text.clone();
    sub.insert_str(first, "\n### One more thing\n");
    assert_eq!(tutorial_budget(&sub).headings, b.headings + 1);
    // A step after it is not counted.
    let after = text.replacen(
        AFTER_IT_WORKS,
        "\n## 8. Use it\n\n```text\ngoat health\n```\n",
        1,
    );
    assert_eq!(tutorial_budget(&after), b);
}
