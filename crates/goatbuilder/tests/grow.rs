//! Gate row G1 through the binary (docs/design/goatlite-growth.md §8): on the lite
//! fixture `ops/goatbuilder/fixtures/goatnet-from-scratch/00-goatlite`,
//! `goatbuilder grow --dry-run` produces a full configuration that passes
//! validation, shows its difference from goatlite.yaml, and writes nothing.
//!
//! The rest of the row is proven next to the code: the standup plan from the grown
//! configuration changes nothing that exists
//! (`config::tests::a_standup_plan_from_the_grown_config_changes_nothing_that_exists`),
//! and a real run's hand-over and refusals (`grow::tests`).

#[path = "common/test_home.rs"]
mod test_home;
use std::path::Path;
use std::process::Command;

fn lite_fixture() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../ops/goatbuilder/fixtures/goatnet-from-scratch/00-goatlite/goatbuilder.yaml",
        ),
    )
    .unwrap()
}

#[test]
fn grow_dry_run_on_the_lite_fixture_shows_a_valid_full_config_and_writes_nothing() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("goatlite.yaml"), lite_fixture()).unwrap();
    // The dry run asks the checkout's own renderer which files the hand-over
    // takes (`--hand-over --check`, which writes nothing); nothing is rendered
    // here, so it names none.
    let renderer = t.path().join("ops/scripts/goatlite-render.py");
    std::fs::create_dir_all(renderer.parent().unwrap()).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts/goatlite-render.py"),
        &renderer,
    )
    .unwrap();
    let out = test_home::isolate(&mut Command::new(env!("CARGO_BIN_EXE_goatbuilder")))
        .args(["-C"])
        .arg(t.path())
        .args(["grow", "--dry-run", "-o", "json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "exit {:?}\nstdout:\n{stdout}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["verb"], "grow");
    assert_eq!(report["ok"], true);
    let details: Vec<String> = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            format!(
                "{} {}",
                c["name"].as_str().unwrap(),
                c["detail"].as_str().unwrap()
            )
        })
        .collect();
    let all = details.join("\n");
    assert!(all.contains("config goatlite.yaml grows into a full configuration that validates against goatbuilder/v1"), "{all}");
    assert!(
        all.contains("difference ~ preset: \"lite\" -> \"full\""),
        "{all}"
    );
    assert!(
        all.contains("plan would write ops/goatbuilder/goatbuilder.yaml"),
        "{all}"
    );
    assert!(
        all.contains(&format!(
            "plan would hand the rendered files over: python3 {} --site pasture --config {} --hand-over",
            renderer.display(),
            t.path().join("goatlite.yaml").display()
        )),
        "the dry run shows the command the real run executes: {all}"
    );
    let mut left: Vec<String> = std::fs::read_dir(t.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        vec!["goatlite.yaml".to_string(), "ops".to_string()],
        "a dry run writes nothing into the checkout"
    );
    assert!(
        !t.path().join("ops/goatbuilder").exists(),
        "a dry run writes no config"
    );
}
