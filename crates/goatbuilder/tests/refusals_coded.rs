//! Gate row C5: every refusal is in the error catalog with its fix, and says
//! which (docs/design/goatbuilder-v1-gate.md, row C5: "the drill logs contain
//! no error without a GB-nnn code").
//!
//! The corpus is the command-line golden cases (`tests/cmd/*.toml`): every case
//! that exits nonzero is a refusal someone met and recorded, except the
//! verdicts listed in [`VERDICTS`], which exit nonzero to report a state, not
//! to refuse. Each refusal is run through the built binary exactly as the
//! golden test runs it (its arguments, its environment, the crate directory),
//! with its own empty HOME, and with the explanation page turned off
//! (`GOAT_EXPLAIN=0`), because a log is not a terminal. Its output must carry
//! the one code [`EXPECTED`] names for it: any code is not enough, because a
//! refusal matched to the wrong entry sends the reader to the wrong fix.
//!
//! [`PENDING`] lists the refusals whose code is printed by a change to a file
//! another stream owns, each with the change that brings it. A pending case
//! must still lack its code: when that change lands, the case passes, this
//! test fails, and the change removes its line. The list only shrinks.

use std::path::Path;
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
        "verify_mock_basic_dev_portal",
        "exit 3 reports hosts not converged: the verdict of verify (C7)",
    ),
    (
        "preflight_mock_mystery_undeclared",
        "exit 3 grades a component not clean: the verdict of preflight (its rows carry GB-076)",
    ),
    (
        "explain_unrecognised_text",
        "exit 1 answers `explain` with \"not recognised\": an answer, not a refusal",
    ),
    (
        "explain_federate_error_text",
        "exit 1 answers `explain federate <error text>` with the nearest entries: an answer, not a refusal",
    ),
];

/// The code each refusal case must print.
const EXPECTED: &[(&str, &str)] = &[
    ("apply_missing_plan", "GB-008"),
    ("assay_unknown_site", "GB-049"),
    ("c10_deploy_mock_needs_yes", "GB-008"),
    ("c10_deploy_needs_yes", "GB-008"),
    ("c10_fastlane_needs_yes", "GB-008"),
    ("c10_federate_needs_yes", "GB-008"),
    ("c10_join_node_needs_yes", "GB-008"),
    ("c10_release_needs_yes", "GB-008"),
    ("c10_rollback_needs_yes", "GB-008"),
    ("c10_standup_needs_yes", "GB-008"),
    ("c4_deploy_mock_needs_ack", "GB-047"),
    ("c4_rollback_mock_needs_ack", "GB-047"),
    ("deploy_needs_one_component", "GB-018"),
    ("deploy_unknown_component", "GB-001"),
    ("endstate_unknown_site", "GB-066"),
    ("explain_federate_no_such_phase", "GB-262"),
    ("explain_federate_phase_report", "GB-242"),
    ("federate_bilateral_only_for_test", "GB-189"),
    ("federate_site_flag_before_verb", "GB-238"),
    ("federate_undeclared_is_refused", "GB-149"),
    ("federate_unknown_fed_verbs", "GB-240"),
    ("federate_verb_refuses_mock", "GB-239"),
    ("federate_verb_refuses_output", "GB-239"),
    ("join_node_unknown_class", "GB-247"),
    ("join_node_unknown_env", "GB-241"),
    ("plan_needs_out", "GB-242"),
    ("promote_abandon_needs_reason", "GB-242"),
    ("release_from_preprod_rows_needs_tag", "GB-018"),
    ("release_from_prod_needs_tag", "GB-018"),
    ("release_needs_ack", "GB-018"),
    ("release_needs_version_or_tag", "GB-018"),
    ("rollback_n_out_of_range", "GB-246"),
    ("rollback_unknown_component", "GB-023"),
    ("secrets_needs_a_subcommand", "GB-242"),
    ("standup_from_lettered_phase_is_refused", "GB-242"),
    ("standup_from_old_phase_zero_is_refused", "GB-242"),
    ("status_with_no_component_declared_refuses", "GB-050"),
    ("update_no_surface", "GB-072"),
    ("update_unknown_site", "GB-072"),
    ("update_unknown_site_json", "GB-072"),
    ("update_verify_release_nothing_to_verify", "GB-261"),
    ("usage_bad_output_format", "GB-242"),
    ("usage_join_node_needs_class", "GB-242"),
    ("usage_mock_on_a_verb_that_reads_the_checkout", "GB-276"),
    ("usage_no_verb", "GB-242"),
    ("usage_release_bad_from", "GB-018"),
    ("usage_standup_needs_site", "GB-243"),
    ("usage_unknown_verb", "GB-242"),
    ("usage_update_check_and_force", "GB-242"),
    ("validate_missing_file", "GB-244"),
    ("validate_refuses_json", "GB-025"),
    ("validate_refuses_not_yaml", "GB-245"),
    ("validate_refuses_rung_undeclared_environment", "GB-025"),
    ("validate_refuses_soak_without_min_days", "GB-025"),
    ("validate_refuses_unknown_converger", "GB-025"),
    ("validate_refuses_unknown_field", "GB-025"),
    ("validate_refuses_wrong_schema_version", "GB-025"),
];

/// Refusals whose code a change in another stream's file brings, each with
/// that change, one list per owner so each owner's change empties its own
/// list without touching another's. A pending case must still lack its
/// expected code.
const PENDING: &[&[(&str, &str)]] = &[PENDING_MAIN, PENDING_FEDERATE, PENDING_CONFIG];

/// main.rs (W-L1): clap's errors and main's own refusals.
/// Emptied by the change to main.rs.
const PENDING_MAIN: &[(&str, &str)] = &[];

/// federate/ (W-B2): the federate verbs' own refusals.
/// Emptied by the change to federate/.
const PENDING_FEDERATE: &[(&str, &str)] = &[];

/// config.rs (W-G): a config that cannot be read or is not YAML.
/// Emptied by the change to config.rs.
const PENDING_CONFIG: &[(&str, &str)] = &[];

/// Every pending case, whoever owns it.
fn pending() -> impl Iterator<Item = &'static (&'static str, &'static str)> {
    PENDING.iter().flat_map(|l| l.iter())
}

/// Cases that quote this checkout's own deployment, skipped where it declares
/// none (as tests/cli_golden.rs skips them).
const NEED_A_DEPLOYMENT: &[&str] = &[
    "deploy_unknown_component",
    "federate_undeclared_is_refused",
    "join_node_unknown_class",
    "join_node_unknown_env",
];

fn cases() -> Vec<(String, toml::Value)> {
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
            (name, doc)
        })
        .collect()
}

fn exit_of(doc: &toml::Value) -> Option<i64> {
    doc.get("status")?.get("code")?.as_integer()
}

/// Every GB-nnn code in a text.
fn codes_in(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    (0..b.len().saturating_sub(5))
        .filter(|&i| {
            &b[i..i + 3] == b"GB-"
                && b[i + 3..i + 6].iter().all(u8::is_ascii_digit)
                && b.get(i + 6).is_none_or(|c| !c.is_ascii_digit())
        })
        .map(|i| text[i..i + 6].to_string())
        .collect()
}

/// The case run as the golden test runs it, with its own HOME and no
/// explanation page: everything it printed.
fn run(doc: &toml::Value, home: &Path) -> (i32, String) {
    let args: Vec<String> = doc["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect();
    let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
    // trycmd runs a case from the crate directory.
    c.args(&args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("GOAT_EXPLAIN", "0")
        .env("HOME", home);
    if let Some(env) = doc.get("env") {
        for (k, v) in env.as_table().unwrap() {
            match (k.as_str(), v) {
                ("add", toml::Value::Table(t)) => {
                    for (ak, av) in t {
                        c.env(ak, av.as_str().unwrap());
                    }
                }
                ("remove", toml::Value::Array(a)) => {
                    for r in a.iter().filter_map(|r| r.as_str()) {
                        if r != "GOAT_EXPLAIN" && r != "HOME" {
                            c.env_remove(r);
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
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        ),
    )
}

#[test]
fn every_refusal_case_names_its_catalog_code() {
    let declares = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ops/goatbuilder/goatbuilder.yaml")
        .is_file();
    let home = tempfile::tempdir().unwrap();
    let mut wrong = Vec::new();
    let mut now_coded = Vec::new();
    let mut refusals = 0;
    for (name, doc) in cases() {
        let Some(code) = exit_of(&doc) else { continue };
        if code == 0 || VERDICTS.iter().any(|(v, _)| *v == name) {
            continue;
        }
        let Some((_, want)) = EXPECTED.iter().find(|(c, _)| *c == name) else {
            wrong.push(format!("{name}: a refusal case with no expected code; add it to EXPECTED with the code it prints"));
            continue;
        };
        if !declares && NEED_A_DEPLOYMENT.contains(&name.as_str()) {
            continue;
        }
        refusals += 1;
        let (exit, text) = run(&doc, home.path());
        let codes = codes_in(&text);
        let has = codes.iter().any(|c| c == want);
        let pending = pending().find(|(c, _)| *c == name);
        match (has, pending) {
            (true, None) | (false, Some(_)) => {}
            (true, Some((_, by))) => now_coded.push(format!(
                "{name}: now prints {want}; remove its PENDING line ({by})"
            )),
            (false, None) => {
                let first = text
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .chars()
                    .take(160)
                    .collect::<String>();
                wrong.push(format!(
                    "{name} (exit {exit}) prints {codes:?}, not {want}: {first}"
                ));
            }
        }
    }
    assert!(refusals >= 30, "the refusal cases were read: {refusals}");
    assert!(
        wrong.is_empty() && now_coded.is_empty(),
        "{}\n  {}",
        "refusals not as expected:",
        [wrong, now_coded].concat().join("\n  ")
    );
}

/// The lists name real cases and every expected code is in the catalog with
/// a fix, so a renamed case or a code with nothing behind it cannot slip past.
#[test]
fn the_lists_name_cases_and_catalog_codes() {
    let names: Vec<String> = cases().into_iter().map(|(n, _)| n).collect();
    let catalog = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/errors/catalog.yaml"),
    )
    .unwrap();
    for (v, _) in VERDICTS.iter().chain(pending()) {
        assert!(
            names.iter().any(|n| n == v),
            "{v} is not a case in tests/cmd"
        );
    }
    for (c, code) in EXPECTED {
        assert!(
            names.iter().any(|n| n == c),
            "{c} is not a case in tests/cmd"
        );
        let entry = catalog
            .split("\n  - code: ")
            .find(|e| e.starts_with(&format!("{code}\n")))
            .unwrap_or_else(|| panic!("{code} ({c}) is not in ops/errors/catalog.yaml"));
        assert!(
            entry.contains("\n    fixes:\n      - "),
            "{code} ({c}) has no fix in the catalog"
        );
    }
    for (p, _) in pending() {
        assert!(
            EXPECTED.iter().any(|(c, _)| c == p),
            "{p} is pending with no expected code"
        );
    }
}
