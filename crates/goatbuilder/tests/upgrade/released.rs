//! The released goatbuilder binaries the upgrade tests run, read from
//! `$GOATBUILDER_UPGRADE_DIR/<version>/goatbuilder-<os>-<arch>` and checked
//! against the pins in releases.sha256 (filled by fetch.sh). With the variable
//! unset a test says it did not run and passes, unless
//! `GOATBUILDER_UPGRADE_REQUIRED=1`.

use std::path::PathBuf;

use sha2::{Digest, Sha256};

pub fn asset() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        o => o,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        a => a,
    };
    format!("goatbuilder-{os}-{arch}")
}

/// The pinned SHA-256 of `<version>/<asset>`.
pub fn pin(version: &str, asset: &str) -> Option<String> {
    let want = format!("{version}/{asset}");
    include_str!("releases.sha256").lines().find_map(|l| {
        let (sum, path) = l.split_once("  ")?;
        (path == want).then(|| sum.to_string())
    })
}

/// The released binary for `version`, checked against its pin; `None`, with
/// the reason printed, when the test cannot run here.
pub fn binary(version: &str) -> Option<PathBuf> {
    let required = std::env::var("GOATBUILDER_UPGRADE_REQUIRED").as_deref() == Ok("1");
    let skip = |why: String| -> Option<PathBuf> {
        assert!(!required, "GOATBUILDER_UPGRADE_REQUIRED=1 and {why}");
        eprintln!("upgrade test NOT RUN for {version}: {why}");
        None
    };
    let asset = asset();
    let Some(want) = pin(version, &asset) else {
        return skip(format!("releases.sha256 pins no {version}/{asset}"));
    };
    let Some(dir) = std::env::var_os("GOATBUILDER_UPGRADE_DIR") else {
        return skip(
            "GOATBUILDER_UPGRADE_DIR is unset (tests/upgrade/fetch.sh <dir> fills one)".into(),
        );
    };
    let path = PathBuf::from(dir).join(version).join(&asset);
    let Ok(bytes) = std::fs::read(&path) else {
        return skip(format!("no {}", path.display()));
    };
    let got: String = Sha256::digest(&bytes)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect();
    assert_eq!(
        got,
        want,
        "{} is not the released {version} binary (its SHA-256 differs from the pin)",
        path.display()
    );
    Some(path)
}
