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
//! - the phases, and the steps inside them, of the lite LAN standup plan;
//! - the steps of `docs/tutorials/00-goatlite.md` before the goatnet works
//!   (the assay of its §7).
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
const MAX_TUTORIAL_SECTIONS: usize = 7;
const MAX_TUTORIAL_BLOCKS: usize = 36;
const MAX_TUTORIAL_LINES: usize = 75;

/// The heading that follows "it works" in tutorial 00: everything before it is
/// the way to a working goatnet.
const AFTER_IT_WORKS: &str = "\n## 8. Use it";

const WHY: &str = "goatlite's first run may not grow (gate row G2, docs/design/goatlite-growth.md §1): \
prepare for growth at growth time. If this change is a deliberate decision about the first run, \
change the budget in goatlite-growth.md §8 and in this test together, with the reason";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

fn fixture() -> PathBuf {
    repo().join("ops/goatbuilder/fixtures/goatnet-from-scratch/00-goatlite/goatbuilder.yaml")
}

/// The learner's file: the lite fixture's `schema`, `preset` and `lite:` block,
/// and nothing else.
fn four_fact_file() -> String {
    let fx: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    let mut doc = serde_yaml::Mapping::new();
    for k in ["schema", "preset", "lite"] {
        doc.insert(k.into(), fx.get(k).unwrap_or_else(|| panic!("the lite fixture has no {k}")).clone());
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
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo().join("ops/goatbuilder/schema/goatbuilder-v1.schema.json")).unwrap()).unwrap();
    let required: Vec<&str> = schema["properties"]["lite"]["required"]
        .as_array()
        .expect("the schema's lite block lists what it requires")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(required, REQUIRED_FACTS, "{WHY}");
    let lite = serde_yaml::from_str::<serde_yaml::Value>(&four_fact_file()).unwrap();
    let keys: Vec<String> = lite["lite"].as_mapping().unwrap().keys().map(|k| k.as_str().unwrap().to_string()).collect();
    assert_eq!(keys, REQUIRED_FACTS, "the lite fixture declares exactly the required facts");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("goatlite.yaml");
    std::fs::write(&path, four_fact_file()).unwrap();
    let o = goatbuilder(&["validate", path.to_str().unwrap()]);
    assert!(o.status.success(), "a file of the four facts validates: {}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
}

/// The phase lines of a standup plan: `(phase number, kind)` for each step.
fn steps(plan: &serde_json::Value) -> Vec<(u32, String)> {
    plan["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] == "phase")
        .filter_map(|c| {
            let d = c["detail"].as_str()?.strip_prefix("=== Phase ")?;
            let (n, rest) = d.split_once(' ')?;
            let kind = rest.strip_prefix('[')?.split_once(']')?.0;
            Some((n.parse().ok()?, kind.to_string()))
        })
        .collect()
}

#[test]
fn the_first_run_gains_no_phase_step_or_stop() {
    // A checkout holding only the learner's file: the dry run prints the whole
    // lite plan from it and writes nothing.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("goatlite.yaml"), four_fact_file()).unwrap();
    let o = goatbuilder(&["-o", "json", "standup", "-C", dir.path().to_str().unwrap(), "pasture", "--dry-run"]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(o.status.success(), "the lite dry run plans: {out}{}", String::from_utf8_lossy(&o.stderr));
    let plan: serde_json::Value = serde_json::from_str(&out).expect("the dry run's JSON");
    assert!(out.contains("profile=lan"), "a goatlite's first run is a LAN run: {out}");
    let steps = steps(&plan);
    let mut phases: Vec<u32> = steps.iter().map(|s| s.0).collect();
    phases.dedup();
    let manual = steps.iter().filter(|s| s.1 == "MANUAL").count();
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(entries, vec![std::ffi::OsString::from("goatlite.yaml")], "a dry run writes nothing");

    let measured = format!("phases {phases:?} ({}), steps {}, MANUAL stops {manual}", phases.len(), steps.len());
    assert!(phases.len() <= MAX_PHASES, "{measured}: more than {MAX_PHASES} phases. {WHY}");
    assert!(steps.len() <= MAX_STEPS, "{measured}: more than {MAX_STEPS} steps. {WHY}");
    assert!(manual <= MAX_MANUAL_STOPS, "{measured}: more than {MAX_MANUAL_STOPS} stop(s) for the learner. {WHY}");
    // Guard against a parse that silently counts nothing.
    assert!(steps.len() >= 10 && phases.contains(&1) && phases.contains(&14), "the plan was read: {measured}");
}

/// How much of tutorial 00 comes before the goatnet works: numbered sections,
/// fenced blocks, and the non-comment lines inside them.
fn tutorial_budget(text: &str) -> (usize, usize, usize) {
    let before = &text[..text.find(AFTER_IT_WORKS).expect("tutorial 00 has its '## 8. Use it' section")];
    let sections = before.lines().filter(|l| l.strip_prefix("## ").is_some_and(|r| r.split_once(". ").is_some_and(|(n, _)| n.parse::<u32>().is_ok()))).count();
    let (mut fenced, mut blocks, mut lines) = (false, 0, 0);
    for l in before.lines() {
        if l.trim_start().starts_with("```") {
            fenced = !fenced;
            blocks += usize::from(fenced);
        } else if fenced && !l.trim().is_empty() && !l.trim_start().starts_with('#') {
            lines += 1;
        }
    }
    (sections, blocks, lines)
}

#[test]
fn tutorial_00_gains_no_step_before_the_goatnet_works() {
    let text = std::fs::read_to_string(repo().join("docs/tutorials/00-goatlite.md")).unwrap();
    let (sections, blocks, lines) = tutorial_budget(&text);
    let measured = format!("before '## 8. Use it': {sections} sections, {blocks} command blocks, {lines} lines in them");
    assert!(sections <= MAX_TUTORIAL_SECTIONS, "{measured}. {WHY}");
    assert!(blocks <= MAX_TUTORIAL_BLOCKS, "{measured}. {WHY}");
    assert!(lines <= MAX_TUTORIAL_LINES, "{measured}. {WHY}");

    // The negative control: one more command before "it works" is counted.
    let grown = text.replacen(AFTER_IT_WORKS, "\n```text\ngoatbuilder prepare-for-growth pasture\n```\n\n## 8. Use it", 1);
    assert_eq!(tutorial_budget(&grown), (sections, blocks + 1, lines + 1));
    // A command after it is not.
    let after = text.replacen(AFTER_IT_WORKS, "\n## 8. Use it\n\n```text\ngoat health\n```\n", 1);
    assert_eq!(tutorial_budget(&after), (sections, blocks, lines));
}
