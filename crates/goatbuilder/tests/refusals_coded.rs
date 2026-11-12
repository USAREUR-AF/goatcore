//! Gate row C5: every refusal is in the error catalog with its fix, and says
//! which (docs/design/goatbuilder-v1-gate.md, row C5: "the drill logs contain
//! no error without a GB-nnn code").
//!
//! The corpus is the command-line golden cases (`tests/cmd/*.toml`): every case
//! that exits nonzero is a refusal someone met and recorded, except the
//! verdicts listed in [`VERDICTS`], which exit nonzero to report a state, not
//! to refuse. Each refusal is run through the built binary exactly as the
//! golden test runs it (its arguments, its environment, the case's directory),
//! with the explanation page turned off (`GOAT_EXPLAIN=0`), because a log is
//! not a terminal, and its output must carry a `GB-nnn` code. A new refusal
//! case without a code fails this test; a case cannot be excused except by
//! adding it to [`VERDICTS`] with the reason, where a reviewer sees it.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Cases that exit nonzero to report a verdict, each with why it is not a refusal.
const VERDICTS: &[(&str, &str)] = &[
    (
        "diff_mock_basic_dev_all",
        "exit 3 reports drift: the verdict of diff, not a refusal",
    ),
    (
        "diff_mock_basic_dev_all_json",
        "exit 3 reports drift: the verdict of diff, not a refusal",
    ),
    (
        "status_mock_basic_dev",
        "exit 3 reports pending and unknown hosts: the verdict of status",
    ),
    (
        "status_mock_basic_dev_json",
        "exit 3 reports pending and unknown hosts: the verdict of status",
    ),
    (
        "survey_mock_fixture",
        "exit 3 reports what the survey found: its verdict",
    ),
    (
        "survey_mock_fixture_inter",
        "exit 3 reports what the survey found: its verdict",
    ),
    (
        "preflight_mock_mystery_undeclared",
        "exit 3 grades a component not clean: the verdict of preflight (its rows carry GB-076)",
    ),
    (
        "verify_mock_basic_dev_portal",
        "exit 3 reports hosts not converged: the verdict of verify (C7)",
    ),
    (
        "explain_unrecognised_text",
        "exit 1 answers `explain` with \"not recognised\": an answer, not a refusal",
    ),
];

/// Cases that quote this checkout's own deployment, skipped where it declares
/// none (as tests/cli_golden.rs skips them).
const NEED_A_DEPLOYMENT: &[&str] = &[
    "deploy_unknown_component",
    "federate_undeclared_is_refused",
    "join_node_unknown_class",
    "join_node_unknown_env",
];

fn cases() -> Vec<(String, PathBuf, toml::Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cmd");
    let mut v: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    v.sort();
    v.into_iter()
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().to_string();
            let doc: toml::Value = toml::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            (name, p, doc)
        })
        .collect()
}

fn exit_of(doc: &toml::Value) -> Option<i64> {
    doc.get("status")?.get("code")?.as_integer()
}

fn code_in(text: &str) -> bool {
    let b = text.as_bytes();
    (0..b.len().saturating_sub(5))
        .any(|i| &b[i..i + 3] == b"GB-" && b[i + 3..i + 6].iter().all(u8::is_ascii_digit))
}

#[test]
fn every_refusal_case_names_its_catalog_code() {
    let declares = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ops/goatbuilder/goatbuilder.yaml")
        .is_file();
    let mut uncoded = Vec::new();
    let mut refusals = 0;
    for (name, path, doc) in cases() {
        let Some(code) = exit_of(&doc) else { continue };
        if code == 0 || VERDICTS.iter().any(|(v, _)| *v == name) {
            continue;
        }
        if !declares && NEED_A_DEPLOYMENT.contains(&name.as_str()) {
            continue;
        }
        refusals += 1;
        let args: Vec<String> = doc["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_string())
            .collect();
        let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
        // trycmd runs a case from the crate directory.
        let _ = &path;
        c.args(&args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env("GOAT_EXPLAIN", "0");
        if let Some(env) = doc.get("env") {
            for (k, v) in env.as_table().unwrap() {
                match (k.as_str(), v) {
                    ("add", toml::Value::Table(t)) => {
                        for (ak, av) in t {
                            c.env(ak, av.as_str().unwrap());
                        }
                    }
                    ("remove", toml::Value::Array(a)) => {
                        for r in a {
                            if r.as_str() != Some("GOAT_EXPLAIN") {
                                c.env_remove(r.as_str().unwrap());
                            }
                        }
                    }
                    (k, toml::Value::String(s)) => {
                        c.env(k, s);
                    }
                    _ => {}
                }
            }
        }
        let out = c.output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        if !code_in(&text) {
            let first = String::from_utf8_lossy(&out.stderr)
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .chars()
                .take(160)
                .collect::<String>();
            uncoded.push(format!(
                "{name} (exit {}): {first}",
                out.status.code().unwrap_or(-1)
            ));
        }
    }
    assert!(refusals >= 30, "the refusal cases were read: {refusals}");
    assert!(
        uncoded.is_empty(),
        "{} of {refusals} refusal cases print no GB-nnn code:\n  {}",
        uncoded.len(),
        uncoded.join("\n  ")
    );
}

/// The exclusions name real cases, so a renamed case cannot slip past.
#[test]
fn every_verdict_named_is_a_case() {
    let names: Vec<String> = cases().into_iter().map(|(n, _, _)| n).collect();
    for (v, why) in VERDICTS {
        assert!(
            names.iter().any(|n| n == v),
            "{v} ({why}) is not a case in tests/cmd"
        );
    }
}
