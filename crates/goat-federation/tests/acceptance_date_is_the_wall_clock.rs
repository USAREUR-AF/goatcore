//! verify and apply decide acceptance, so the date they judge an agreement's
//! term on is today's, in UTC, and nothing a caller supplies: no flag, no
//! environment variable, no configuration key. The library takes the date as a
//! parameter (`run_verify_at`, `run_apply_at`) so its own tests can pass the
//! vectors' evaluation date; this test holds the line between that and every
//! binary:
//!
//! - both `_at` functions are `pub(crate)`, so no other crate can name them;
//! - outside the test modules, each is called exactly once, from its public
//!   verb, with `chrono::Utc::now().date_naive()` as the date;
//! - the term check they run (`render::verify_locally`, which is public) is
//!   called from nowhere else in any Rust source in the repository.

use std::fs;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The part of a source file before its test module.
fn production(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    match text.find("\n#[cfg(test)]\nmod tests") {
        Some(i) => text[..i].to_string(),
        None => text,
    }
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if name == "target" || name.starts_with('.') || name.ends_with("-target") {
                continue;
            }
            rust_sources(&p, out);
        } else if name.ends_with(".rs") {
            out.push(p);
        }
    }
}

const WALL_CLOCK: &str = "chrono::Utc::now().date_naive()";

fn the_one_call(verb: &str, public: &str, at: &str) {
    let file = repo().join(format!("crates/goat-federation/src/verbs/{verb}.rs"));
    let prod = production(&file);
    assert!(
        prod.contains(&format!("pub(crate) fn {at}(")),
        "{at} must be pub(crate) in {}, so no binary can call it",
        file.display()
    );
    let calls: Vec<&str> = prod
        .match_indices(&format!("{at}("))
        .map(|(i, _)| &prod[i..])
        .filter(|rest| !prod[..prod.len() - rest.len()].ends_with("fn "))
        .collect();
    assert_eq!(
        calls.len(),
        1,
        "{at} is called once outside the tests, from {public}"
    );
    let call = &calls[0][..calls[0].find(';').expect("the call's statement ends")];
    assert!(
        call.trim_end().ends_with(&format!("{WALL_CLOCK})")),
        "{public} must pass {WALL_CLOCK} to {at} as the date, and nothing else: {call}"
    );
    let public_body = &prod[prod.find(&format!("pub fn {public}(")).expect(public)..];
    let public_body = &public_body[..public_body.find("\n}\n").expect("end of fn")];
    assert!(
        public_body.contains(&format!("{at}(")) && public_body.contains(WALL_CLOCK),
        "{public} is the one caller of {at}"
    );
}

#[test]
fn verify_is_judged_on_the_wall_clock_only() {
    the_one_call("verify", "run_verify", "run_verify_at");
}

#[test]
fn apply_is_judged_on_the_wall_clock_only() {
    the_one_call("apply", "run_apply", "run_apply_at");
}

#[test]
fn the_term_check_is_called_only_by_the_two_dated_verbs() {
    let mut files = Vec::new();
    for root in ["crates", "ops"] {
        rust_sources(&repo().join(root), &mut files);
    }
    assert!(
        files.len() > 50,
        "the sources were not found: {}",
        files.len()
    );
    let mut callers = Vec::new();
    for f in &files {
        let prod = production(f);
        let n =
            prod.matches("verify_locally(").count() - prod.matches("fn verify_locally(").count();
        if n > 0 {
            let rel = f.strip_prefix(repo()).unwrap_or(f).display().to_string();
            callers.push((rel, n));
        }
    }
    callers.sort();
    let want: Vec<(String, usize)> = vec![
        ("crates/goat-federation/src/verbs/apply.rs".into(), 1),
        ("crates/goat-federation/src/verbs/verify.rs".into(), 1),
    ];
    assert_eq!(
        callers, want,
        "verify_locally is called outside run_verify_at and run_apply_at"
    );
}
