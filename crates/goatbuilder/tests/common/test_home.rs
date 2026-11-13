//! A home directory of the tests' own, so a verb a test runs never writes into
//! the real one (its state directory, `~/.goat/state`, holds run records and
//! logs). ops/ci/check-goatbuilder-tests-leave-home.sh holds the suite to it.
//!
//! The directory is one per test, under cargo's per-target scratch space, so nothing is left
//! in the system temp directory. Python keeps finding the packages installed for
//! the real user (PYTHONUSERBASE), and git keeps the real user's configuration
//! (GIT_CONFIG_GLOBAL), so a test sees the same tools it saw before.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// This test's home directory: one per test (libtest names each test's thread
/// after the test), under the test binary's own name, emptied the first time
/// this process uses it, so no test reads what another, or an earlier run, left.
///
/// The test is named by its thread, so call this (or [`vars`], [`isolate`]) on the
/// test's own thread and pass the path down to any thread the test starts: called
/// from a thread of its own, it names that thread, not the test.
pub fn path() -> PathBuf {
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    static MADE: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
    let exe = std::env::current_exe().ok();
    let binary = exe
        .as_deref()
        .and_then(|e| e.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tests".into());
    let test = std::thread::current().name().unwrap_or("main").replace(
        |c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "_",
    );
    let p = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("goatbuilder-test-home")
        .join(binary)
        .join(test);
    let mut made = MADE.lock().unwrap_or_else(|e| e.into_inner());
    if made.insert(p.clone()) {
        let _ = std::fs::remove_dir_all(&p);
    }
    std::fs::create_dir_all(&p).expect("the test's home directory");
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
