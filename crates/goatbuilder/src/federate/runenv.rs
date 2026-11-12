//! The environment of one federate run.
//!
//! A run derives values the operator did not set (phase 1's inputs, the
//! profile directory, the edge group) and every phase, helper and child reads
//! them. They used to be written into the process environment (`set_var`),
//! which every thread of the process shares: tests run in parallel, and a run
//! on one thread changed what a plan on another read.
//!
//! They are kept here instead, on the run's own thread: [`set`] records one,
//! [`var`] reads a name from here first and then from the process
//! environment (so a value the operator set still wins where the caller asks
//! for it first), and [`command`] starts a child with every recorded value in
//! its environment. Nothing here touches the process environment.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::process::Command;

thread_local! {
    static RUN: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
}

/// Record `name` for this run: its own reads and its children see it.
pub fn set(name: &str, value: impl Into<String>) {
    RUN.with(|r| r.borrow_mut().insert(name.to_string(), value.into()));
}

/// Forget `name` for this run (a test that recorded one).
#[cfg(test)]
pub fn unset(name: &str) {
    RUN.with(|r| r.borrow_mut().remove(name));
}

/// `name` as this run sees it: the value it recorded, else the process
/// environment's. Same shape as `std::env::var`.
pub fn var(name: &str) -> Result<String, std::env::VarError> {
    match RUN.with(|r| r.borrow().get(name).cloned()) {
        Some(v) => Ok(v),
        None => std::env::var(name),
    }
}

/// [`var`], as an `OsString`.
pub fn var_os(name: &str) -> Option<std::ffi::OsString> {
    match RUN.with(|r| r.borrow().get(name).cloned()) {
        Some(v) => Some(v.into()),
        None => std::env::var_os(name),
    }
}

/// Every value this run recorded, for a child's environment.
pub fn pairs() -> Vec<(String, String)> {
    RUN.with(|r| r.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// A child of this run: `Command::new(program)` with every recorded value in
/// its environment.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut c = Command::new(program);
    c.envs(pairs());
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_this_threads_and_its_childrens_only() {
        set("FED_GB_RUNENV_TEST", "here");
        assert_eq!(var("FED_GB_RUNENV_TEST").as_deref(), Ok("here"));
        assert!(std::env::var_os("FED_GB_RUNENV_TEST").is_none());
        let out = command("sh").args(["-c", "printf %s \"$FED_GB_RUNENV_TEST\""]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "here");
        let other = std::thread::spawn(|| var("FED_GB_RUNENV_TEST").is_err()).join().unwrap();
        assert!(other, "another thread does not see it");
    }
}
