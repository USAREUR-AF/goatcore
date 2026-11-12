//! A home directory of the tests' own, so a verb a test runs never writes into
//! the real one (its state directory, `~/.goat/state`, holds run records and
//! logs). ops/ci/check-goatbuilder-tests-leave-home.sh holds the suite to it.
//!
//! The directory is under cargo's per-target scratch space, so nothing is left
//! in the system temp directory. Python keeps finding the packages installed for
//! the real user (PYTHONUSERBASE), and git keeps the real user's configuration
//! (GIT_CONFIG_GLOBAL), so a test sees the same tools it saw before.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The tests' home directory, made once.
pub fn path() -> PathBuf {
    let p = Path::new(env!("CARGO_TARGET_TMPDIR")).join("goatbuilder-test-home");
    std::fs::create_dir_all(&p).expect("the tests' home directory");
    p
}

/// The variables that point a run at the tests' home.
pub fn vars() -> Vec<(String, String)> {
    let mut v = vec![("HOME".to_string(), path().display().to_string())];
    let real = std::env::var("HOME").unwrap_or_default();
    if !real.is_empty() {
        let base = std::env::var("PYTHONUSERBASE").unwrap_or_else(|_| format!("{real}/.local"));
        v.push(("PYTHONUSERBASE".into(), base));
        let gitconfig =
            std::env::var("GIT_CONFIG_GLOBAL").unwrap_or_else(|_| format!("{real}/.gitconfig"));
        v.push(("GIT_CONFIG_GLOBAL".into(), gitconfig));
    }
    v
}

/// `c`, run with the tests' home.
pub fn isolate(c: &mut Command) -> &mut Command {
    for (k, val) in vars() {
        c.env(k, val);
    }
    c
}
