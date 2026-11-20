//! A pin table in the checkout that is not YAML stops `goatbuilder pins --check`
//! with GB-329 and exit 1 (refused: a precondition failed), naming the file. It
//! was GB-254, "cannot be read or written", which sent the reader to check the
//! file's permissions although the file had been read.

#[path = "common/test_home.rs"]
mod test_home;
use std::process::Command;

#[test]
fn a_pin_table_that_is_not_yaml_is_refused_with_gb_329_and_exit_1() {
    let tree = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let pins = tree.path().join("ops/pins");
    std::fs::create_dir_all(&pins).unwrap();
    std::fs::write(pins.join("fleet-pins.yaml"), "goatnets:\n  zagros: {}\ncomponents:\n  portal: [unclosed\n").unwrap();
    let out = test_home::isolate(&mut Command::new(env!("CARGO_BIN_EXE_goatbuilder")))
        .env("GOAT_EXPLAIN", "0")
        .arg("-C")
        .arg(tree.path())
        .args(["pins", "--check"])
        .output()
        .expect("run goatbuilder pins --check");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("GB-329"), "{err}");
    assert!(err.contains("fleet-pins.yaml"), "{err}");
    assert!(!err.contains("GB-254"), "{err}");
}
