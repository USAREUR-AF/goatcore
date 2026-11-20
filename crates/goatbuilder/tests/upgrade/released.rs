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

/// `v<MAJOR>.<MINOR>.<PATCH>` as numbers.
fn parse(tag: &str) -> Option<(u32, u32, u32)> {
    let mut n = tag.strip_prefix('v')?.split('.').map(|x| x.parse::<u32>().ok());
    Some((n.next()??, n.next()??, n.next()??))
}

/// Every 0.x release pinned in releases.sha256 at or after `from`, oldest
/// first. A release pinned later is tested without a change to any test.
#[allow(dead_code)] // each test file includes this module and uses part of it
pub fn pinned_0x(from: (u32, u32, u32)) -> Vec<String> {
    let mut v: Vec<(u32, u32, u32)> = include_str!("releases.sha256")
        .lines()
        .filter_map(|l| l.split_once("  ")?.1.split('/').next())
        .filter_map(parse)
        .filter(|&t| t.0 == 0 && t >= from)
        .collect();
    v.sort();
    v.dedup();
    v.iter().map(|(a, b, c)| format!("v{a}.{b}.{c}")).collect()
}

/// The newest 0.x release pinned in releases.sha256.
#[allow(dead_code)] // each test file includes this module and uses part of it
pub fn newest_pinned_0x() -> String {
    pinned_0x((0, 0, 0))
        .pop()
        .expect("releases.sha256 pins no 0.x release")
}

/// The pinned SHA-256 of a published release file, `<version>/<file>`, in
/// signatures.sha256 (filled by hand when a release is pinned; fetch.sh reads
/// the same file).
#[allow(dead_code)] // each test file includes this module and uses part of it
pub fn signature_pin(version: &str, file: &str) -> Option<String> {
    let want = format!("{version}/{file}");
    include_str!("signatures.sha256").lines().find_map(|l| {
        let (sum, path) = l.split_once("  ")?;
        (path == want).then(|| sum.to_string())
    })
}

/// The release's published SHA256SUMS and SHA256SUMS.cosign-bundle, read from
/// `$GOATBUILDER_UPGRADE_DIR/<version>/` and checked against their pins; `None`,
/// with the reason printed, when the test cannot run here.
#[allow(dead_code)] // each test file includes this module and uses part of it
pub fn published_signature(version: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let required = std::env::var("GOATBUILDER_UPGRADE_REQUIRED").as_deref() == Ok("1");
    let skip = |why: String| -> Option<(Vec<u8>, Vec<u8>)> {
        assert!(!required, "GOATBUILDER_UPGRADE_REQUIRED=1 and {why}");
        eprintln!("upgrade test NOT RUN for {version}'s signature: {why}");
        None
    };
    let Some(dir) = std::env::var_os("GOATBUILDER_UPGRADE_DIR") else {
        return skip(
            "GOATBUILDER_UPGRADE_DIR is unset (tests/upgrade/fetch.sh <dir> fills one)".into(),
        );
    };
    let mut out = Vec::new();
    for file in ["SHA256SUMS", "SHA256SUMS.cosign-bundle"] {
        let Some(want) = signature_pin(version, file) else {
            return skip(format!("signatures.sha256 pins no {version}/{file}"));
        };
        let path = PathBuf::from(&dir).join(version).join(file);
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
            "{} is not the published {version} {file} (its SHA-256 differs from the pin)",
            path.display()
        );
        out.push(bytes);
    }
    let bundle = out.pop().unwrap();
    Some((out.pop().unwrap(), bundle))
}
