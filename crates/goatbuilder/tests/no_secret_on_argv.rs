//! Standup passes secrets through the environment, not the command line.
//!
//! Every plan goatbuilder prints for a site (`standup --dry-run` for every
//! fixture site on both profiles, `join-node --dry-run` on the third-site
//! fixture, and the recorded plans in tests/fixtures/full-plans.golden.json) is
//! read line by line as the shell would split it. No line may expand the value
//! of a secret-class variable (a name ops/scripts/newsite-secrets.sh generates,
//! or one that says it is a token, secret or password) into a command's
//! arguments. A play reads such a value itself, with
//! `{{ lookup("env", "NAME") }}` inside single quotes.

#[path = "common/test_home.rs"]
mod test_home;

#[path = "../src/argv_secrets.rs"]
mod argv_secrets;

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

fn spec() -> Vec<String> {
    let s = argv_secrets::spec_suffixes(&std::fs::read_to_string(repo().join("ops/scripts/newsite-secrets.sh")).unwrap());
    assert!(s.iter().any(|n| n == "PG_PASS"), "newsite-secrets' SPEC was read: {s:?}");
    s
}

/// Every command row (`cmd`: a line the run hands to a shell) of `rows` that
/// puts a secret value on a command line, as `<where>: NAME in <line>`.
fn offences(place: &str, rows: &[serde_json::Value], spec: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for r in rows.iter().filter(|r| r["name"] == "cmd") {
        let Some(line) = r["detail"].as_str() else { continue };
        for n in argv_secrets::secret_values_on_argv(line, spec) {
            out.push(format!("{place}: {n} in {line}"));
        }
    }
    out
}

fn report(all: &[String]) -> String {
    let mut s = format!("{} line(s) put a secret value on a command line:\n", all.len());
    for o in all.iter().take(12) {
        s.push_str(o);
        s.push('\n');
    }
    s
}

/// Every fixture config, as (fixture dir, environment, site), goatlites included.
fn fixture_sites() -> Vec<(PathBuf, String, String)> {
    let root = repo().join("ops/goatbuilder/fixtures");
    let mut stack = vec![root];
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
    let mut out = Vec::new();
    for c in configs {
        let v: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&c).unwrap()).unwrap();
        if v.get("lite").is_some() {
            continue; // planned through its own file below
        }
        for env in v.get("environments").and_then(|e| e.as_sequence()).into_iter().flatten() {
            let name = env["name"].as_str().unwrap().to_string();
            let sites: Vec<String> = env.get("sites").and_then(|s| s.as_sequence()).into_iter().flatten().map(|s| s["name"].as_str().unwrap().to_string()).collect();
            for site in if sites.is_empty() { vec![name.clone()] } else { sites } {
                out.push((c.clone(), name.clone(), site));
            }
        }
    }
    out
}

fn goatbuilder(args: &[&str]) -> serde_json::Value {
    let o = test_home::isolate(&mut Command::new(env!("CARGO_BIN_EXE_goatbuilder")))
        .args(args)
        .env("GOAT_MESH_PROBE", "0")
        .env("GOAT_NO_FUN", "1")
        .env_remove("GOAT_AWS_PROFILE")
        .env_remove("OWNER_EMAIL")
        .output()
        .expect("run goatbuilder");
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(o.status.success(), "goatbuilder {args:?}: {out}{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("goatbuilder {args:?}: {e}: {out}"))
}

fn rows(v: &serde_json::Value) -> Vec<serde_json::Value> {
    v["checks"].as_array().cloned().unwrap_or_default()
}

#[test]
fn no_recorded_plan_puts_a_secret_value_on_a_command_line() {
    let spec = spec();
    let was: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/full-plans.golden.json")).unwrap()).unwrap();
    let mut all = Vec::new();
    let mut cmds = 0;
    for (key, plan) in was.as_object().unwrap() {
        let r = plan.as_array().unwrap();
        cmds += r.iter().filter(|x| x["name"] == "cmd").count();
        all.extend(offences(key, r, &spec));
    }
    assert!(cmds > 500, "the recorded plans were read: {cmds} commands");
    assert!(all.is_empty(), "{}", report(&all));
}

#[test]
fn no_standup_plan_of_any_fixture_site_puts_a_secret_value_on_a_command_line() {
    let spec = spec();
    let mut all = Vec::new();
    let mut plans = 0;
    for (config, env, site) in fixture_sites() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ops/goatbuilder")).unwrap();
        std::fs::copy(&config, dir.path().join("ops/goatbuilder/goatbuilder.yaml")).unwrap();
        for profile in ["lan", "internet"] {
            let v = goatbuilder(&["-o", "json", "standup", "-C", dir.path().to_str().unwrap(), &env, "--site", &site, "--profile", profile, "--dry-run"]);
            plans += 1;
            all.extend(offences(&format!("{} {env}/{site} {profile}", config.display()), &rows(&v), &spec));
        }
    }
    assert!(plans >= 20, "the fixtures were planned: {plans} plans");
    assert!(all.is_empty(), "{}", report(&all));
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

#[test]
fn the_join_node_plan_puts_no_secret_value_on_a_command_line() {
    let spec = spec();
    let t = tempfile::tempdir().unwrap();
    copy_tree(&repo().join("ops/goatbuilder/fixtures/join-node-third-site"), t.path());
    let v = goatbuilder(&["-C", t.path().to_str().unwrap(), "-o", "json", "join-node", "zagros", "--site", "gamma", "--node", "gamma-core-1", "--class", "core", "--address", "192.0.2.30", "--dry-run"]);
    let r = rows(&v);
    assert!(r.iter().filter(|x| x["name"] == "cmd").count() >= 10, "the join-node plan was printed: {v}");
    assert!(r.iter().any(|x| x["detail"].as_str().is_some_and(|d| d.contains("lookup(\"env\", \"GAMMA_PG_PASS\")"))), "a play reads the Postgres password itself: {v}");
    let all = offences("join-node gamma-core-1", &r, &spec);
    assert!(all.is_empty(), "{}", report(&all));
}
