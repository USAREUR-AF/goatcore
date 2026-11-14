//! Keep the operator's Mac awake for the life of a long-running verb (#2122).
//!
//! A deploy, release or standup drives hosts for minutes to an hour. A Mac that
//! idles into sleep mid-play drops the mesh and stalls the run: a 145 s herd
//! deploy took 1095 s for exactly that reason. The scripts avenue
//! does the same in `ops/scripts/lib/keep-awake.sh` (ADR 1201).
//!
//! `caffeinate -i -w <pid>` prevents idle sleep until this process exits, so the
//! assertion can never outlive the run. `GOAT_KEEP_AWAKE` carries the holder's
//! pid to child processes, so a script that runs goatbuilder (or the reverse)
//! holds one assertion, not two. `GOAT_NO_CAFFEINATE=1` opts out. Off macOS, or
//! without `caffeinate`, nothing happens.

use crate::runner::Cmd;
use std::process::Stdio;

pub const MARKER: &str = "GOAT_KEEP_AWAKE";
pub const OPT_OUT: &str = "GOAT_NO_CAFFEINATE";

/// What the environment says, gathered so the decision is a pure function.
pub struct Facts {
    pub macos: bool,
    pub opted_out: bool,
    /// A holder further up is alive: its assertion already covers us.
    pub held_above: bool,
    pub caffeinate: bool,
    pub long_running: bool,
}

pub fn should_hold(f: &Facts) -> bool {
    f.long_running && f.macos && !f.opted_out && !f.held_above && f.caffeinate
}

fn pid_alive(pid: &str) -> bool {
    pid.parse::<u32>().is_ok()
        && Cmd::new("kill")
            .args(["-0", pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
}

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(tool).is_file()))
        .unwrap_or(false)
}

/// Start the assertion when the verb is long-running and nothing above holds one.
/// Called once from `main`, before any thread starts.
pub fn hold(long_running: bool) {
    let facts = Facts {
        macos: cfg!(target_os = "macos"),
        opted_out: std::env::var(OPT_OUT).as_deref() == Ok("1"),
        held_above: std::env::var(MARKER).map(|p| pid_alive(&p)).unwrap_or(false),
        caffeinate: on_path("caffeinate"),
        long_running,
    };
    if !should_hold(&facts) {
        return;
    }
    let me = std::process::id().to_string();
    let started = Cmd::new("caffeinate")
        .args(["-i", "-w", &me])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if started.is_ok() {
        // Single-threaded here (main, before any verb runs), so set_var is sound.
        std::env::set_var(MARKER, &me);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts { macos: true, opted_out: false, held_above: false, caffeinate: true, long_running: true }
    }

    #[test]
    fn a_long_verb_on_a_mac_holds_the_assertion() {
        assert!(should_hold(&facts()));
    }

    #[test]
    fn each_condition_alone_turns_it_off() {
        assert!(!should_hold(&Facts { macos: false, ..facts() }));
        assert!(!should_hold(&Facts { opted_out: true, ..facts() }));
        assert!(!should_hold(&Facts { held_above: true, ..facts() }));
        assert!(!should_hold(&Facts { caffeinate: false, ..facts() }));
        assert!(!should_hold(&Facts { long_running: false, ..facts() }));
    }

    #[test]
    fn a_dead_or_malformed_holder_pid_is_not_a_holder() {
        assert!(!pid_alive("not-a-pid"));
        assert!(!pid_alive("999999"));
        assert!(pid_alive(&std::process::id().to_string()));
    }
}
