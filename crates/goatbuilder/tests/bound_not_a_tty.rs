//! The hidden `goatbuilder bound` (the goat endless runner, crates/goat-fun)
//! refuses in one line when it is not given a terminal, so a pipe or CI never
//! hangs, and it is hidden from `--help`.

#[path = "common/test_home.rs"]
mod test_home;
use std::process::{Command, Stdio};

#[test]
fn bound_refuses_a_pipe_in_one_line_and_exits_zero() {
    let out = test_home::isolate(&mut Command::new(env!("CARGO_BIN_EXE_goatbuilder")))
        .arg("bound")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run goatbuilder bound");
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "goatbuilder bound needs an interactive terminal of at least 50x12 (this one is piped or redirected).\n"
    );
}

#[test]
fn bound_is_hidden_from_help() {
    let out = test_home::isolate(&mut Command::new(env!("CARGO_BIN_EXE_goatbuilder"))).arg("--help").output().expect("run --help");
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(!help.lines().any(|l| l.trim_start().starts_with("bound")), "{help}");
}
