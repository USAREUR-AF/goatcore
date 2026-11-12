//! The declared secrets posture (ADR 1204), read through `ops/secrets/posture.py`
//! — the same module the scripts avenue reads (ADR 1201), so the binary and
//! `deploy-component.sh` cannot disagree about which store is in force, which
//! variables are available, or what to tell the operator when one is missing.
//!
//! goat's own posture is references only, resolved by 1Password per command
//! (ADR 1186). A team adopting goat may declare `secrets: {store: env-file}`
//! (or sops, git-crypt, openbao, env) and owns that trade.

use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::{Check, Report, EXIT_REFUSED};

fn posture(repo: &Path) -> std::path::PathBuf {
    repo.join("ops/secrets/posture.py")
}

fn ask(repo: &Path, args: &[&str]) -> Option<(i32, String)> {
    ask_with(repo, args, true)
}

// A test reads goat's own declared store by pointing the posture at a goatlite.yaml
// that does not exist (GOAT_LITE_CONFIG). It is set for the test's own thread and
// passed to the posture it spawns, never written to the process environment, which
// other tests read in parallel.
#[cfg(test)]
thread_local! {
    static LITE_CONFIG: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// `GOAT_LITE_CONFIG` for every posture this test thread spawns.
#[cfg(test)]
pub(crate) fn set_test_lite_config(path: &str) {
    LITE_CONFIG.with(|c| *c.borrow_mut() = Some(path.to_string()));
}

/// Apply this test thread's `GOAT_LITE_CONFIG`, if it set one, to a command.
pub(crate) fn with_test_env(cmd: &mut Command) -> &mut Command {
    #[cfg(test)]
    LITE_CONFIG.with(|c| {
        if let Some(p) = c.borrow().as_ref() {
            cmd.env("GOAT_LITE_CONFIG", p);
        }
    });
    cmd
}

fn ask_with(repo: &Path, args: &[&str], probe: bool) -> Option<(i32, String)> {
    let mut cmd = Command::new("python3");
    cmd.arg(posture(repo)).args(args).current_dir(repo);
    with_test_env(&mut cmd);
    if !probe {
        cmd.env("GOAT_STORE_PROBE", "0");
    }
    let out = cmd.output().ok()?;
    Some((out.status.code().unwrap_or(2), String::from_utf8_lossy(&out.stdout).to_string()))
}

/// Every variable name with a value or a reference under the declared posture.
/// Falls back to the plain environment when the posture module cannot run.
pub fn available(repo: &Path) -> HashSet<String> {
    match ask(repo, &["available"]) {
        Some((0, text)) => text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
        _ => std::env::vars().filter(|(_, v)| !v.is_empty()).map(|(k, _)| k).collect(),
    }
}

/// The store in force: `1password`, `openbao`, `sops`, `env-file`, `env`, or
/// whatever else `secrets: store:` declares (ADR 1204).
pub fn store(repo: &Path) -> String {
    match ask(repo, &["store"]) {
        Some((0, text)) if !text.trim().is_empty() => text.trim().to_string(),
        // The posture could not answer; name what it would have defaulted to.
        _ => "env-file".to_string(),
    }
}

/// Whether the declared store keeps references (pointers resolved per command)
/// or the values themselves. The posture module owns the list, so a store added
/// there needs no change here.
pub fn keeps_references(repo: &Path) -> bool {
    matches!(ask(repo, &["kind"]), Some((0, text)) if text.trim() == "reference")
}

/// How to supply `names`, one step per line, in the declared store's own words.
pub fn hint_lines(repo: &Path, names: &[String]) -> Vec<String> {
    let mut args = vec!["hint"];
    args.extend(names.iter().map(|s| s.as_str()));
    match ask(repo, &args) {
        Some((0, text)) => text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
        _ => Vec::new(),
    }
}

/// How to supply `names` under the declared posture, as one line.
pub fn hint(repo: &Path, names: &[String]) -> String {
    let mut args = vec!["hint"];
    args.extend(names.iter().map(|s| s.as_str()));
    match ask(repo, &args) {
        Some((0, text)) if !text.trim().is_empty() => text.lines().map(str::trim).collect::<Vec<_>>().join(" "),
        _ => "supply those variables in the environment (ops/secrets/posture.py could not run: is python3 installed?)".to_string(),
    }
}

/// The posture's own doctor rows: the store in force, and whether it can serve.
/// `probe` false asks the posture for what it can see without touching a
/// credential store, so nothing prompts the operator (doctor's default).
pub fn doctor_rows(repo: &Path, probe: bool) -> Vec<Check> {
    let Some((_, text)) = ask_with(repo, &["doctor"], probe) else {
        return vec![Check { name: "secrets:posture".into(), ok: false, detail: "ops/secrets/posture.py did not run".into(), fix: Some("install python3".into()) }];
    };
    text.lines()
        .filter_map(|l| {
            let (ok, rest) = match l.strip_prefix("ok  ") {
                Some(r) => (true, r),
                None => (false, l.strip_prefix("FAIL")?),
            };
            let rest = rest.trim_start();
            let (name, after) = rest.split_once("  ").unwrap_or((rest, ""));
            let (detail, fix) = match after.split_once("  ||  fix: ") {
                Some((d, f)) => (d.to_string(), Some(f.to_string())),
                None => (after.to_string(), None),
            };
            Some(Check { name: name.to_string(), ok, detail, fix })
        })
        .collect()
}

/// `goatbuilder secrets <store|available|missing|hint|doctor|init|exec …>`: the posture
/// module, run with this terminal. `init --env <goatnet>` writes the zero-SaaS
/// quickstart's env file (strong random values, 0600, gitignored).
pub fn run(repo: &Path, args: &[String]) -> Result<Report> {
    // `secrets exec -- <cmd>` is the CLI avenue's name for ops/scripts/with-secrets:
    // the one run verb both avenues share (ADR 1204).
    let mut cmd = if args.first().map(String::as_str) == Some("exec") {
        let rest: Vec<&String> = args[1..].iter().skip_while(|a| a.as_str() == "--").collect();
        let mut c = Command::new(repo.join("ops/scripts/with-secrets"));
        c.args(rest);
        c
    } else {
        let mut c = Command::new("python3");
        c.arg(posture(repo)).args(args);
        c
    };
    let status = cmd
        .current_dir(repo)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| "ops/secrets/posture.py or ops/scripts/with-secrets")?;
    let code = status.code().unwrap_or(2);
    let mut r = Report::new("secrets", Vec::new(), EXIT_REFUSED);
    r.exit = code;
    r.ok = code == 0;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn the_hint_follows_the_declared_store() {
        // GOAT_SECRET_STORE is read by the child; set it for this process only.
        let out = Command::new("python3")
            .arg(posture(&repo()))
            .args(["hint", "EXAMPLE_SECRET"])
            .env("GOAT_SECRET_STORE", "env-file")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("EXAMPLE_SECRET") && text.contains("secrets init"), "{text}");
    }

    /// goat's declared store, not a learner's: a goatlite.yaml at the checkout root
    /// outranks goatbuilder.yaml for the posture (ADR 1213 D4), so the tests that ask
    /// for goat's own answer point the posture at a file that does not exist.
    fn own_checkout() {
        set_test_lite_config("/nonexistent/goatlite.yaml");
    }

    #[test]
    fn the_store_is_the_one_this_checkout_declares() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        own_checkout();
        // goat's own: ops/goatbuilder/posture.yml says 1password (ADR 1204).
        assert_eq!(store(&repo()), "1password");
        // A checkout with no posture at all cannot be asked, and the answer
        // names what the generator would have defaulted to rather than
        // guessing at a service nobody has an account for.
        assert_eq!(store(Path::new("/no/such/repo")), "env-file");
    }

    #[test]
    fn the_hint_lines_are_one_step_each() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        own_checkout();
        let lines = hint_lines(&repo(), &["EXAMPLE_SECRET".to_string()]);
        // Every line is a step, and no line is blank: the summary prints them
        // one under the other as the fix.
        assert!(!lines.is_empty(), "the declared store says how to supply a name");
        assert!(lines.iter().all(|l| !l.trim().is_empty()), "{lines:?}");
        // goat's declared store keeps references, so its steps are the
        // reference file and the sign-in, not the name itself.
        assert!(lines.iter().any(|l| l.contains("op signin")), "{lines:?}");
        // A checkout that cannot answer says nothing rather than something wrong.
        assert!(hint_lines(Path::new("/no/such/repo"), &["A".to_string()]).is_empty());
    }

    #[test]
    fn doctor_rows_parse_ok_and_fail_lines() {
        let rows = doctor_rows(&repo(), true);
        assert!(rows.iter().any(|c| c.name == "secrets:posture"), "{rows:?}");
    }
}
