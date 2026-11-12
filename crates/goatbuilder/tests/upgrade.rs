//! Gate row E1 (docs/design/goatbuilder-v1-gate.md, "Upgrading is tested"):
//! a deployment made with a released 0.x binary continues with the build
//! under test, with no change to its files.
//!
//! For each release pinned in `tests/upgrade/releases.sha256`:
//!   1. the released binary stands the tutorial-1 fixture up
//!      (`goatnet-from-scratch/01-founder`), then deploys one changed pin, so
//!      the deployment has a history to roll back to;
//!   2. the build under test runs `validate`, `status`, `deploy` and
//!      `rollback` on the same directory, which nothing edits in between.
//!
//! The released binaries are not built here and are never downloaded by the
//! test. `tests/upgrade/fetch.sh <dir>` puts them in `<dir>/<version>/`,
//! checked against the pinned checksums; the test reads them from
//! `$GOATBUILDER_UPGRADE_DIR` and checks each against its pin again. With the
//! variable unset the test says so and passes without running, unless
//! `GOATBUILDER_UPGRADE_REQUIRED=1`, which makes a missing binary a failure.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "upgrade/released.rs"]
mod released;

use released::pin;

fn new_bin() -> &'static str {
    env!("CARGO_BIN_EXE_goatbuilder")
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn run(bin: &Path, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        .env("GOAT_MESH_PROBE", "0")
        .env("GOAT_NO_FUN", "1")
        .output()
        .expect("run goatbuilder")
}

fn says(o: &Output) -> String {
    format!(
        "exit {:?}\n{}{}",
        o.status.code(),
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), &dest).unwrap();
        }
    }
}

/// Every file under `dir` with its bytes, in a stable order.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(d: &Path, root: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries: Vec<_> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, root, out);
            } else {
                out.push((
                    p.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(&p).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

/// The files a person writes: the build under test must leave each of them
/// byte for byte as the released binary left it.
const DECLARED: &[&str] = &[
    "goatbuilder.yaml",
    "converger.yaml",
    "assay-values.yaml",
    "declared/pins.yaml",
];

const PORTAL_OLD: &str =
    "ghcr.io/example-org/portal:v1.4.0@sha256:3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c";
const PORTAL_NEW: &str =
    "ghcr.io/example-org/portal:v1.4.1@sha256:3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d3d";

/// `status -o json` rows for the portal: (machine row name, ok, detail).
fn portal_rows(bin: &Path, r: &str, f: &str) -> Vec<(String, bool, String)> {
    let o = run(
        bin,
        &[
            "-o", "json", "status", "-C", r, "--mock", f, "--env", "zagros",
        ],
    );
    assert_eq!(o.status.code(), Some(0), "status: {}", says(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("status -o json");
    v["checks"]
        .as_array()
        .expect("status rows")
        .iter()
        .filter(|c| c["name"].as_str().unwrap_or("").ends_with("/portal"))
        .map(|c| {
            (
                c["name"].as_str().unwrap_or("").to_string(),
                c["ok"] == true,
                c["detail"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

fn continues_on_the_build_under_test(version: &str) {
    let Some(old) = released::binary(version) else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let fx = tmp.path().join("zagros");
    copy_tree(
        &repo().join("ops/goatbuilder/fixtures/goatnet-from-scratch/01-founder"),
        &fx,
    );
    let r = repo();
    let (r, f) = (r.to_str().unwrap(), fx.to_str().unwrap());
    let ack = ["--ack", "Upgrade Test", "--yes"];

    // 1. The deployment, made with the released binary: a standup, then one
    //    changed pin deployed, so there is an earlier ref to roll back to.
    let o = run(
        &old,
        &[
            &["standup", "-C", r, "zagros", "--site", "alpha", "--mock", f][..],
            &ack,
        ]
        .concat(),
    );
    assert_eq!(o.status.code(), Some(0), "{version} standup: {}", says(&o));
    let pins = fx.join("declared/pins.yaml");
    let text = std::fs::read_to_string(&pins).unwrap();
    assert!(
        text.contains(PORTAL_OLD),
        "the fixture's portal pin moved: {text}"
    );
    std::fs::write(&pins, text.replace(PORTAL_OLD, PORTAL_NEW)).unwrap();
    let deploy = [
        &[
            "deploy",
            "-C",
            r,
            "--mock",
            f,
            "--env",
            "zagros",
            "--component",
            "portal",
        ][..],
        &ack,
    ]
    .concat();
    let o = run(&old, &deploy);
    assert_eq!(o.status.code(), Some(0), "{version} deploy: {}", says(&o));
    let made = snapshot(&fx);
    let declared = |s: &[(PathBuf, Vec<u8>)]| -> Vec<(PathBuf, Vec<u8>)> {
        s.iter()
            .filter(|(p, _)| DECLARED.iter().any(|d| p == Path::new(d)))
            .cloned()
            .collect()
    };
    assert_eq!(
        declared(&made).len(),
        DECLARED.len(),
        "a declared file is missing"
    );

    // 2. The build under test, on the same directory, nothing edited.
    let new = Path::new(new_bin());
    let o = run(
        new,
        &[
            "validate",
            "-C",
            r,
            fx.join("goatbuilder.yaml").to_str().unwrap(),
        ],
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "validate after {version}: {}",
        says(&o)
    );

    let rows = portal_rows(new, r, f);
    assert!(!rows.is_empty(), "status after {version} shows no portal");
    for (name, ok, detail) in &rows {
        assert!(
            *ok && detail.contains("v1.4.1"),
            "status after {version}: {name}: {detail}"
        );
    }
    assert_eq!(
        snapshot(&fx),
        made,
        "status after {version} changed the deployment"
    );

    let o = run(new, &deploy);
    assert_eq!(
        o.status.code(),
        Some(0),
        "deploy after {version}: {}",
        says(&o)
    );
    assert_eq!(
        declared(&snapshot(&fx)),
        declared(&made),
        "deploy after {version} rewrote a declared file"
    );

    // A rollback is a new declaration at the earlier ref (C9): it writes
    // declared/pins.yaml. It must write what the released binary's own
    // rollback writes from the same state, run here on a copy.
    let twin = tmp.path().join("twin");
    copy_tree(&fx, &twin);
    let rollback = |bin: &Path, dir: &Path| {
        let d = dir.to_str().unwrap();
        let o = run(
            bin,
            &[
                &[
                    "rollback", "-C", r, "--mock", d, "portal", "1", "--env", "zagros",
                ][..],
                &ack,
            ]
            .concat(),
        );
        assert_eq!(
            o.status.code(),
            Some(0),
            "rollback by {} after {version}: {}",
            bin.display(),
            says(&o)
        );
    };
    rollback(&old, &twin);
    rollback(new, &fx);
    assert_eq!(
        std::fs::read_to_string(fx.join("declared/pins.yaml")).unwrap(),
        std::fs::read_to_string(twin.join("declared/pins.yaml")).unwrap(),
        "the build under test's rollback declares something else than {version}'s"
    );
    for (name, ok, detail) in portal_rows(new, r, f) {
        assert!(
            ok && detail.contains("v1.4.0"),
            "after rollback on {version}'s deployment: {name}: {detail}"
        );
    }

    // Every other declared file is as the released binary left it; the
    // ledgers the tools keep (records, publications, transports: the .jsonl
    // files) were only appended to. state/ticks.json is the fixture's clock,
    // rewritten by every run, so it is not compared.
    let after = snapshot(&fx);
    let others = |s: &[(PathBuf, Vec<u8>)]| -> Vec<(PathBuf, Vec<u8>)> {
        declared(s)
            .into_iter()
            .filter(|(p, _)| p != Path::new("declared/pins.yaml"))
            .collect()
    };
    assert_eq!(
        others(&after),
        others(&made),
        "the build under test rewrote a declared file"
    );
    for (path, bytes) in &made {
        if DECLARED.iter().any(|d| path == Path::new(d))
            || path.extension().and_then(|e| e.to_str()) != Some("jsonl")
        {
            continue;
        }
        let now = after
            .iter()
            .find(|(p, _)| p == path)
            .unwrap_or_else(|| panic!("{} is gone after the upgrade", path.display()));
        assert!(
            now.1.starts_with(bytes),
            "{} was rewritten, not appended to, after the upgrade from {version}",
            path.display()
        );
    }
}

#[test]
fn e1_a_deployment_made_with_0_4_3_continues() {
    continues_on_the_build_under_test("v0.4.3");
}

#[test]
fn e1_a_deployment_made_with_0_5_1_continues() {
    continues_on_the_build_under_test("v0.5.1");
}

/// Every release the test names has a pin for every platform it ships, and
/// fetch.sh reads the same file.
#[test]
fn every_release_is_pinned_for_each_platform() {
    for v in ["v0.4.3", "v0.5.1"] {
        for a in [
            "goatbuilder-linux-amd64",
            "goatbuilder-linux-arm64",
            "goatbuilder-darwin-amd64",
            "goatbuilder-darwin-arm64",
        ] {
            let p = pin(v, a).unwrap_or_else(|| panic!("no pin for {v}/{a}"));
            assert_eq!(p.len(), 64, "{v}/{a}: {p}");
        }
    }
    assert!(include_str!("upgrade/fetch.sh").contains("releases.sha256"));
}
