//! `goatbuilder federate lint|compose` prints exactly what the library prints
//! for `goat federate lint|compose`: the built binary against
//! `goat_federation::verbs::run` (which `goat federate` calls with stdout), over
//! every conformance vector. ops/goatbuilder/parity-federate-verbs.sh compares
//! the two built binaries; this holds goatbuilder to the library in
//! `cargo test`.

#[path = "common/test_home.rs"]
mod test_home;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Parser;
use goat_federation::verbs::{self, FederateCmd, Site};

#[derive(Parser)]
#[command(no_binary_name = true)]
struct Line {
    #[command(subcommand)]
    cmd: FederateCmd,
}

fn vectors() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/federation/vectors")
}

/// The library's answer, as `goat federate` gives it.
fn library(words: &[String]) -> (i32, Vec<u8>) {
    let line = Line::try_parse_from(words).unwrap();
    let mut out = Vec::new();
    let no_site = || -> anyhow::Result<Site> { anyhow::bail!("lint and compose read no site") };
    let code = verbs::run(line.cmd, &no_site, &mut out).unwrap();
    (code, out)
}

/// The binary's answer.
fn binary(words: &[String]) -> (i32, Vec<u8>) {
    let o = test_home::isolate(&mut Command::new(env!("CARGO_BIN_EXE_goatbuilder")))
        .arg("federate")
        .args(words)
        .env("GOAT_LANG", "en")
        .output()
        .unwrap();
    (o.status.code().unwrap_or(-1), o.stdout)
}

fn same(words: Vec<String>) {
    let (lc, lo) = library(&words);
    let (bc, bo) = binary(&words);
    assert_eq!(bc, lc, "exit code of federate {}", words.join(" "));
    assert_eq!(
        String::from_utf8_lossy(&bo),
        String::from_utf8_lossy(&lo),
        "stdout of federate {}",
        words.join(" ")
    );
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

#[test]
fn lint_matches_goat_federate_lint_on_every_vector() {
    let v = vectors();
    let mut n = 0;
    for e in std::fs::read_dir(&v).unwrap() {
        let dir = e.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(&dir).unwrap() {
            let f = f.unwrap().path();
            let name = f.file_name().unwrap().to_string_lossy().into_owned();
            let package = name.starts_with("package-")
                && name.ends_with(".canonical.json")
                && !name.contains("signing-input");
            let malformed = dir.ends_with("malformed") && name.ends_with(".json");
            if !(package || malformed) {
                continue;
            }
            same(vec!["lint".into(), s(&f)]);
            same(vec!["lint".into(), "--json".into(), s(&f)]);
            n += 1;
        }
    }
    assert!(n > 40, "found only {n} packages");
}

#[test]
fn compose_matches_goat_federate_compose_on_every_vector() {
    let v = vectors();
    let now = std::fs::read_to_string(v.join("now.txt"))
        .unwrap()
        .trim()
        .to_string();
    let key = |p: &str| {
        let k = std::fs::read_to_string(v.join(format!("testkeys/{p}.public.b64"))).unwrap();
        format!("{p}={}", k.trim())
    };
    let mut n = 0;
    for e in std::fs::read_dir(&v).unwrap() {
        let dir = e.unwrap().path();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        if !(name.starts_with("compose-") || name.starts_with("refuse-")) {
            continue;
        }
        for extra in [&[][..], &["--json"][..], &["--print"][..]] {
            let mut packages: Vec<String> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| {
                    let n = p.file_name().unwrap().to_string_lossy();
                    n.starts_with("package-")
                        && n.ends_with(".canonical.json")
                        && !n.contains("signing-input")
                })
                .map(|p| s(&p))
                .collect();
            packages.sort();
            assert_eq!(packages.len(), 2, "{name}: {packages:?}");
            let mut w = vec![
                "compose".to_string(),
                packages[0].clone(),
                packages[1].clone(),
                "--now".into(),
                now.clone(),
                "--key".into(),
                key("xaa"),
                "--key".into(),
                key("xab"),
            ];
            w.extend(extra.iter().map(|x| x.to_string()));
            same(w);
        }
        n += 1;
    }
    assert!(n > 20, "found only {n} pairs");
}
