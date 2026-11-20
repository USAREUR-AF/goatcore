//! Gate D: `status` and `verify` carry each machine's state in `-o json` as data,
//! not only as the first word of a row's `detail` (contract C7).
//!
//! Every `status` and `verify` document has a `machines` list, empty when
//! nothing was graded, with one entry per graded row: `environment`, `host`,
//! `component`, `state` (converged, pending or unknown) and `reason` (a closed
//! set), and `check`, the name of the `checks` row it grades. No other verb's
//! document has the list. The fixtures are the ones A4's kept documents are
//! recorded on (ops/goatbuilder/compat/documents/fixtures/basic).

#[path = "common/machines.rs"]
mod shape;
#[path = "common/test_home.rs"]
mod test_home;
use shape::machines;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

const FIXTURE: &str = "ops/goatbuilder/compat/documents/fixtures/basic";

fn repo() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run(args: &[&str]) -> (i32, serde_json::Value, String) {
    let o = test_home::isolate(&mut Command::new(env!("CARGO_BIN_EXE_goatbuilder")))
        .current_dir(repo())
        .args(["-o", "json"])
        .args(args)
        .env("GOAT_MESH_PROBE", "0")
        .env("GOAT_DOOR_OPEN", "0")
        .env("GOAT_EXPLAIN", "0")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let doc = serde_json::from_slice(&o.stdout).unwrap_or(serde_json::Value::Null);
    (o.status.code().unwrap_or(-1), doc, text)
}

fn want(rows: &[(&str, &str, &str, &str)]) -> BTreeMap<(String, String), (String, String)> {
    rows.iter()
        .map(|(h, c, s, r)| {
            (
                (h.to_string(), c.to_string()),
                (s.to_string(), r.to_string()),
            )
        })
        .collect()
}

#[test]
fn status_on_the_fixture_lists_every_machine_with_its_state_and_reason() {
    let (code, doc, text) = run(&["status", "-C", ".", "--mock", FIXTURE, "--env", "dev"]);
    assert_eq!(code, 3, "{text}");
    let got = machines(&doc, &text);
    let mut rows = vec![];
    for c in ["netvis", "portal"] {
        for h in ["dev-core-b", "dev-obs-a", "dev-obs-b"] {
            rows.push((h, c, "pending", "no-record"));
        }
        rows.push(("dev-edge-a", c, "unknown", "unreadable"));
    }
    rows.push(("dev-core-a", "netvis", "pending", "no-record"));
    rows.push(("dev-core-a", "portal", "pending", "other-ref"));
    assert_eq!(got, want(&rows), "{text}");
    for m in doc["machines"].as_array().unwrap() {
        assert_eq!(m["environment"], "dev");
        assert_eq!(
            m["check"].as_str().unwrap(),
            format!(
                "dev/{}/{}",
                m["host"].as_str().unwrap(),
                m["component"].as_str().unwrap()
            )
        );
    }
    // One entry per machine row, none left out.
    assert_eq!(
        doc["machines"].as_array().unwrap().len(),
        doc["checks"].as_array().unwrap().len(),
        "{text}"
    );
}

#[test]
fn a_converged_environment_lists_its_machines_as_converged() {
    let (code, doc, text) = run(&["status", "-C", ".", "--mock", FIXTURE, "--env", "preprod"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(
        machines(&doc, &text),
        want(&[("preprod-core-a", "portal", "converged", "declared")]),
        "{text}"
    );
}

#[test]
fn verify_on_the_fixture_lists_the_component_s_machines() {
    let (code, doc, text) = run(&[
        "verify", "-C", ".", "--mock", FIXTURE, "--env", "dev", "portal",
    ]);
    assert_eq!(code, 3, "{text}");
    let got = machines(&doc, &text);
    assert_eq!(got.len(), 5, "{text}");
    assert_eq!(
        got[&("dev-core-a".to_string(), "portal".to_string())],
        ("pending".to_string(), "other-ref".to_string())
    );
    assert_eq!(
        got[&("dev-edge-a".to_string(), "portal".to_string())],
        ("unknown".to_string(), "unreadable".to_string())
    );
    assert!(got.keys().all(|(_, c)| c == "portal"), "{text}");
}

#[test]
fn other_verbs_carry_no_machines_list() {
    let (_, doc, text) = run(&["diff", "-C", ".", "--mock", FIXTURE, "--env", "dev"]);
    assert_eq!(doc["verb"], "diff", "{text}");
    assert!(
        doc.get("machines").is_none(),
        "diff carries machines:\n{text}"
    );
}
