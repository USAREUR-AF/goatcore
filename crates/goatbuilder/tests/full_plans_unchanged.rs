//! Growing a goatlite (gate row G1, docs/design/goatlite-growth.md §6) adds
//! declared fields that change a standup plan only where a grown config declares
//! them. Every site that exists today keeps its plan byte for byte: this test runs
//! `standup --dry-run -o json` for every site of every fixture that is not a
//! goatlite, on both profiles, and compares it with the plans recorded in
//! `tests/fixtures/full-plans.golden.json`.
//!
//! A change that is meant to alter a full site's plan regenerates the record and
//! says why in the same change:
//! `GOATBUILDER_GOLDEN=overwrite cargo test -p goatbuilder --test full_plans_unchanged`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/full-plans.golden.json")
}

/// Every fixture config with no `lite:` block, as (fixture dir, environment, site).
/// An environment that declares no sites is planned for a site of its own name.
fn full_sites() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let root = repo().join("ops/goatbuilder/fixtures");
    let mut stack = vec![root.clone()];
    let mut configs = Vec::new();
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| n == "goatbuilder.yaml") {
                configs.push(p);
            }
        }
    }
    configs.sort();
    for c in configs {
        let v: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&c).unwrap()).unwrap();
        if v.get("lite").is_some() {
            continue;
        }
        let rel = c.parent().unwrap().strip_prefix(&root).unwrap().display().to_string();
        for env in v.get("environments").and_then(|e| e.as_sequence()).into_iter().flatten() {
            let name = env["name"].as_str().unwrap().to_string();
            let sites: Vec<String> = env.get("sites").and_then(|s| s.as_sequence()).into_iter().flatten().map(|s| s["name"].as_str().unwrap().to_string()).collect();
            for site in if sites.is_empty() { vec![name.clone()] } else { sites } {
                out.push((rel.clone(), name.clone(), site));
            }
        }
    }
    out
}

/// The plan's rows, with the throwaway checkout path and the home directory taken
/// out so the record is the same on every machine.
fn plan(fixture: &str, env: &str, site: &str, profile: &str) -> Vec<serde_json::Value> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("ops/goatbuilder")).unwrap();
    std::fs::copy(repo().join("ops/goatbuilder/fixtures").join(fixture).join("goatbuilder.yaml"), dir.path().join("ops/goatbuilder/goatbuilder.yaml")).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(["-o", "json", "standup", "-C", dir.path().to_str().unwrap(), env, "--site", site, "--profile", profile, "--dry-run"])
        .env("GOAT_MESH_PROBE", "0")
        .env_remove("GOAT_AWS_PROFILE")
        .env_remove("OWNER_EMAIL")
        .output()
        .expect("run goatbuilder");
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(o.status.success(), "{fixture} {env}/{site} ({profile}): {out}{}", String::from_utf8_lossy(&o.stderr));
    let home = std::env::var("HOME").unwrap_or_default();
    let mut text = out.replace(dir.path().to_str().unwrap(), "<repo>");
    if !home.is_empty() {
        text = text.replace(&home, "<home>");
    }
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    v["checks"].as_array().unwrap().clone()
}

#[test]
fn every_full_fixture_site_keeps_its_standup_plan() {
    let mut now = serde_json::Map::new();
    for (fixture, env, site) in full_sites() {
        for profile in ["lan", "internet"] {
            now.insert(format!("{fixture} {env}/{site} {profile}"), serde_json::Value::Array(plan(&fixture, &env, &site, profile)));
        }
    }
    assert!(now.len() >= 20, "the fixtures were read: {} plans", now.len());
    let now = serde_json::Value::Object(now);
    if std::env::var("GOATBUILDER_GOLDEN").as_deref() == Ok("overwrite") {
        std::fs::write(golden_path(), serde_json::to_string_pretty(&now).unwrap() + "\n").unwrap();
        return;
    }
    let was: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(golden_path()).expect("the recorded plans")).unwrap();
    for (key, plan) in now.as_object().unwrap() {
        let before = was.get(key).unwrap_or_else(|| panic!("{key}: no recorded plan (a new fixture site: regenerate the record)"));
        if before != plan {
            let (b, a) = (before.as_array().unwrap(), plan.as_array().unwrap());
            let first = b.iter().zip(a.iter()).position(|(x, y)| x != y).unwrap_or(b.len().min(a.len()));
            panic!("{key}: the plan changed at row {first}\nrecorded: {}\nnow:      {}", b.get(first).map(|r| r.to_string()).unwrap_or_default(), a.get(first).map(|r| r.to_string()).unwrap_or_default());
        }
    }
    assert_eq!(was.as_object().unwrap().len(), now.as_object().unwrap().len(), "a recorded fixture site is gone");
}
