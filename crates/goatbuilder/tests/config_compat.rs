//! Gate row A2 (docs/design/goatbuilder-v1-gate.md): configuration compatibility
//! is a test.
//!
//! `tests/compat/configs/` keeps, for each published goatbuilder release since
//! goatbuilder/v0.4.1, every configuration file that release shipped: the
//! goatbuilder.yaml of each fixture (the full and the lite preset, with their
//! peering declarations), every refusal fixture, and every whole example in its
//! documents. Each was validated by that release's own published binary, and
//! the outcome is kept beside it in `manifest.yaml`. This build's `validate` must
//! give each file the same outcome: valid, or refused the same way, with the
//! catalog code that release printed or the one its documents name for the
//! refusal (`named:`).
//!
//! The files are recorded with `tests/compat/record.py` from a release's tag and
//! its published binary, checked against that release's SHA256SUMS
//! (`tests/compat/README.md`).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

fn here() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat/configs")
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

#[derive(Deserialize)]
struct Manifest {
    releases: Vec<Release>,
    configs: Vec<Kept>,
}

#[derive(Deserialize)]
struct Release {
    tag: String,
    /// The published linux-amd64 binary's SHA-256 the outcomes were recorded
    /// with, or empty when the tag was never published (`note` says so).
    #[serde(default)]
    binary_sha256: String,
    #[serde(default)]
    note: String,
}

#[derive(Deserialize, Clone)]
struct Kept {
    release: String,
    source: String,
    file: String,
    expect: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    named: Option<String>,
}

fn manifest() -> Manifest {
    let text = std::fs::read_to_string(here().join("manifest.yaml"))
        .expect("tests/compat/configs/manifest.yaml");
    serde_yaml::from_str(&text).expect("manifest.yaml parses")
}

/// valid (exit 0), refused (exit 1 with a checks document) or error (anything
/// else, with the catalog code its error document names, if any), and the code a
/// refused document's first failing row names (`GB-nnn: …`).
fn outcome(file: &Path, home: &Path) -> (String, Option<String>) {
    let o = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["-o", "json", "validate", "-C"])
        .arg(repo())
        .arg(file)
        .env("HOME", home)
        .env("GOAT_LANG", "en")
        .env_remove("GOAT_EXPLAIN")
        .output()
        .expect("run goatbuilder validate");
    let doc: serde_json::Value =
        serde_json::from_slice(&o.stdout).unwrap_or(serde_json::Value::Null);
    match o.status.code() {
        Some(0) => ("valid".into(), None),
        Some(1) if doc["schema"] == "goatbuilder.output/v1" => {
            let code = doc["checks"]
                .as_array()
                .and_then(|c| c.iter().find(|r| r["ok"] == false))
                .and_then(|r| r["detail"].as_str())
                .and_then(|d| d.split_once(": "))
                .map(|(c, _)| c)
                .filter(|c| c.len() == 6 && c.starts_with("GB-"))
                .map(String::from);
            ("refused".into(), code)
        }
        _ => ("error".into(), doc["code"].as_str().map(String::from)),
    }
}

/// Every published release since 0.4.1 has its set; a tag that was never
/// published is listed with the reason, and has none.
#[test]
fn every_release_since_0_4_1_has_its_set() {
    let m = manifest();
    let published: Vec<&Release> = m
        .releases
        .iter()
        .filter(|r| !r.binary_sha256.is_empty())
        .collect();
    for tag in [
        "goatbuilder/v0.4.1",
        "goatbuilder/v0.4.2",
        "goatbuilder/v0.4.3",
        "goatbuilder/v0.5.1",
        "goatbuilder/v0.5.3",
    ] {
        assert!(
            published.iter().any(|r| r.tag == tag),
            "{tag} is recorded with its published binary"
        );
        assert!(
            m.configs.iter().any(|k| k.release == tag),
            "{tag} has its set of configuration files"
        );
    }
    for r in &m.releases {
        if r.binary_sha256.is_empty() {
            assert!(
                !r.note.trim().is_empty(),
                "{} has no binary and no reason",
                r.tag
            );
            assert!(
                !m.configs.iter().any(|k| k.release == r.tag),
                "{} was never published, so it has no set",
                r.tag
            );
        } else {
            assert_eq!(r.binary_sha256.len(), 64, "{}", r.tag);
        }
    }
    for k in &m.configs {
        assert!(
            m.releases.iter().any(|r| r.tag == k.release),
            "{}: unknown release {}",
            k.file,
            k.release
        );
        assert!(here().join(&k.file).is_file(), "{} is kept", k.file);
        assert!(
            ["valid", "refused", "error"].contains(&k.expect.as_str()),
            "{}: expect {}",
            k.file,
            k.expect
        );
    }
}

/// The promise: each kept file validates on this build, or is refused the way
/// its release refused it.
#[test]
fn every_kept_file_gets_the_outcome_its_release_gave_it() {
    let home = tempfile::tempdir().unwrap();
    let mut broken = Vec::new();
    for k in manifest().configs {
        let (got, code) = outcome(&here().join(&k.file), home.path());
        // A file its release accepted is accepted. One it did not accept (refused,
        // or an error) is still not accepted, with the code that release printed
        // or the one its documents name: a refusal may move from an error
        // document to a checks row, never to an acceptance or another code.
        let same = if k.expect == "valid" {
            got == "valid"
        } else {
            got != "valid" && (code == k.code || (code.is_some() && code == k.named))
        };
        if !same {
            broken.push(format!(
                "{} ({}, {}): {} with code {:?} in its release, {} with code {:?} now",
                k.file, k.release, k.source, k.expect, k.code, got, code
            ));
        }
    }
    assert!(
        broken.is_empty(),
        "configuration files a release accepted or refused are treated differently now:\n{}",
        broken.join("\n")
    );
}

/// The kept files hold synthetic values only: documentation addresses (RFC 5737,
/// and 100.127.0.0/16 for the mesh), example domains, and the RFC 1918 LAN
/// address a release's own goatlite example uses (`192.168.1.50`, kept as the
/// release published it, since a reader copies it as written), never a real
/// deployment's.
#[test]
fn the_kept_files_hold_only_synthetic_values() {
    let ip = regex::Regex::new(r"\b(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})\b").unwrap();
    let mail = regex::Regex::new(r"[A-Za-z0-9._+-]+@([A-Za-z0-9.-]+\.[A-Za-z]+)").unwrap();
    for k in manifest().configs {
        let text = std::fs::read_to_string(here().join(&k.file)).unwrap();
        for c in ip.captures_iter(&text) {
            let o: Vec<u32> = (1..=4).map(|i| c[i].parse().unwrap_or(999)).collect();
            let synthetic = matches!(
                (o[0], o[1], o[2]),
                (192, 0, 2) | (198, 51, 100) | (203, 0, 113)
            ) || (o[0] == 100 && o[1] == 127)
                || (o[0] == 192 && o[1] == 168)
                || o[0] == 127
                || o == [0, 0, 0, 0];
            assert!(
                synthetic,
                "{}: {} is not a documentation address",
                k.file, &c[0]
            );
        }
        for c in mail.captures_iter(&text) {
            let d = &c[1];
            assert!(
                d.ends_with("example.org") || d.ends_with("example.com") || d.ends_with(".lan"),
                "{}: mail domain {d}",
                k.file
            );
        }
    }
}
