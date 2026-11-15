//! Gate row C5: `goatbuilder pins` on a fleet-pins.yaml that breaks the pins
//! schema prints GB-337 and exits 2, as the uncoded error did.

use std::process::Command;

#[test]
fn pins_on_a_table_that_breaks_the_schema_prints_gb_337_and_exits_2() {
    let dir = std::env::temp_dir().join(format!("gb-pins-schema-bin-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("ops/pins")).unwrap();
    std::fs::write(dir.join("ops/pins/fleet-pins.yaml"), "goatnets: []\ncomponents: {}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["pins", "-C"])
        .arg(&dir)
        .arg("--check")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.contains("`goatnets:` must be a mapping"), "{err}");
    assert!(err.contains("GB-337"), "the refusal carries its code: {err}");
}
