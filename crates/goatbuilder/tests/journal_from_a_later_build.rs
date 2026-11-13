//! A run journal written by a later goatbuilder is read, not refused
//! (the promise for the files goatbuilder writes and reads back, ADR 1228's
//! proposed fifth surface; gate rows A1 and E3).
//!
//! The released 0.5.1 refuses a journal holding an outcome it does not know
//! (GB-182) and strands the peering: tests/upgrade_update.rs,
//! `e3_back_on_0_5_1_a_journal_from_a_later_build_is_refused_and_kept`. This is
//! its twin against the build under test, which must keep the rule:
//!
//! - a value written by a later version is shown as written, never refused;
//! - on resume an unknown outcome counts as not passed, so the phase runs
//!   again, never skipped;
//! - a rewrite keeps the values and fields it does not know;
//! - only a journal of another schema version is refused (GB-182) and kept.
//!
//! `deferred` is an outcome no build knows (`waiting` becomes known in 0.6.0,
//! so it would stop testing this); `written_by` and `session` are fields no
//! build writes. The same journal is kept in the compatibility baseline:
//! ops/goatbuilder/compat/documents/fixtures/journal/later/state.json.

mod federate_world;

use federate_world::{phases_run, rows, says, world, World};

const LATER: &str = "deferred";

/// A world whose peering ran to the end, then its journal as a later build
/// leaves it: phases 3 and 14 and the last run `deferred`, and fields this
/// build does not write.
fn journal_from_a_later_build() -> World {
    let w = world();
    std::fs::write(w.work.join("keys.txt"), "xaa=QUJD\nxab=QUJE\n").unwrap();
    w.run(&["hello-ally"]);
    w.run(&["hello-ally", "--from", "6"]);
    let (out, _) = w.run(&["hello-ally", "--from", "11"]);
    assert!(out.status.success(), "{}", says(&out));
    let path = w.work.join("state.json");
    let mut v = w.state();
    later(&mut v);
    std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    w
}

/// The edits a later goatbuilder's journal carries (the kept fixture has them).
fn later(v: &mut serde_json::Value) {
    v["written_by"] = serde_json::json!("goatbuilder 9.9.0");
    v["phases"]["3"]["outcome"] = serde_json::json!(LATER);
    v["phases"]["14"]["outcome"] = serde_json::json!(LATER);
    v["phases"]["14"]["detail"] = serde_json::json!("GB-199: waiting for xab's half of acceptance");
    v["phases"]["14"]["session"] = serde_json::json!("ses-1");
    let last = v["runs"].as_array().unwrap().len() - 1;
    v["runs"][last]["outcome"] = serde_json::json!(LATER);
}

#[test]
fn status_shows_an_outcome_from_a_later_build_as_written() {
    let w = journal_from_a_later_build();
    let (out, _) = w.run(&["status", "hello-ally"]);
    let text = says(&out);
    assert!(
        !text.contains("GB-182"),
        "status refused the journal: {text}"
    );
    assert!(
        text.contains(LATER) && text.contains("later goatbuilder"),
        "status names the value as written, and that a later build wrote it: {text}"
    );
}

/// Phase 3 is a phase resume skips when it passed and nothing changed; holding
/// an outcome this build does not know, it runs again. (A phase 3 that runs
/// again is a new half, so the run then stops at phase 5 for a new exchange, as
/// every resumed run does; phase 14, a gate that always runs, cannot show skip
/// from run, and `a_rewrite_keeps_what_it_does_not_know` runs it again.)
#[test]
fn resume_runs_a_phase_whose_outcome_it_does_not_know() {
    let w = journal_from_a_later_build();
    w.clear_log();
    let (out, v) = w.run(&["hello-ally"]);
    assert!(
        out.status.success(),
        "resume refused the journal: {}",
        says(&out)
    );
    let ran = phases_run(&v);
    assert!(
        ran.contains(&3),
        "phase 3 runs again: ran {ran:?}, skipped {:?}",
        rows(&v, "skipped")
    );
    let st = w.state();
    assert_eq!(st["phases"]["3"]["outcome"], "passed", "{st:#}");
}

#[test]
fn a_rewrite_keeps_what_it_does_not_know() {
    let w = journal_from_a_later_build();
    let before = w.state();
    // A run that stops before phase 14 rewrites the journal and leaves 14 alone.
    let (out, _) = w.run(&["hello-ally", "--to", "13"]);
    assert!(out.status.success(), "{}", says(&out));
    let st = w.state();
    assert_eq!(st["written_by"], "goatbuilder 9.9.0", "{st:#}");
    assert_eq!(
        st["phases"]["14"], before["phases"]["14"],
        "an entry the run did not touch is kept as written"
    );
    let last = before["runs"].as_array().unwrap().len() - 1;
    assert_eq!(st["runs"][last]["outcome"], LATER, "{st:#}");
    // Then phase 14 runs, and only its own record changes.
    let (out, _) = w.run(&["hello-ally", "--from", "14"]);
    assert!(out.status.success(), "{}", says(&out));
    let st = w.state();
    assert_eq!(st["phases"]["14"]["outcome"], "passed");
    assert_eq!(st["written_by"], "goatbuilder 9.9.0", "{st:#}");
}

#[test]
fn only_another_schema_version_is_refused_naming_its_writer_and_kept() {
    let w = journal_from_a_later_build();
    let path = w.work.join("state.json");
    let mut v = w.state();
    v["schema"] = serde_json::json!("goatbuilder.federate.state/v2");
    v["written_by"] = serde_json::json!("goatbuilder 0.9.0");
    std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    let kept = std::fs::read(&path).unwrap();
    let (out, _) = w.run(&["hello-ally"]);
    let text = says(&out);
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("GB-182") && text.contains("0.9.0"), "{text}");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        kept,
        "a v2 journal is kept byte for byte"
    );
}
