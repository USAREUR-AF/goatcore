//! The release-ladder gate (ADR 1120, GB-373) for `deploy`, applied as
//! ops/scripts/deploy-component.sh applies it: before any transport or host,
//! the digest each member group is about to receive is judged by
//! ops/release/ladder_gate.py in component mode. That module is the one
//! decision the script and ansible-preflight.sh already share, so it is called
//! here rather than ported, as `deploy` calls served-payloads.py and
//! fresh-source.sh: one decision, one set of words, on both avenues.
//!
//! The gate's three answers, as the script reads them:
//!   - exit 0: pass (not a live rung, the lowest rung, an open hotfix record,
//!     or --skip-rung with both a reason and an ack);
//!   - exit 3: pinned on the rung below; the caller confirms that rung
//!     converged on it (a `--verify-only` read of its ledgers);
//!   - anything else: refused, in the gate's own message. A gate that cannot
//!     run is a refusal too (fail-closed).

use std::path::Path;

use anyhow::Result;

use crate::runner::Cmd;

pub const CODE: &str = "GB-373";
/// The gate, relative to the repository root.
pub const GATE: &str = "ops/release/ladder_gate.py";
/// Set on every play once the gate has judged the run, so ansible-preflight.sh
/// does not judge it a second time (and without the skip).
pub const CHECKED_ENV: &str = "GOAT_LADDER_CHECKED";
/// The skip the rollback-ledger role writes into the deploy ledger row (`ladder_skip`).
pub const SKIP_ENV: &str = "GOAT_DEPLOY_LADDER_SKIP";

/// How a deploy meets the ladder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode<'a> {
    /// Judge every member group's digest (the default).
    #[default]
    Gate,
    /// `--skip-rung --reason <why> --ack <operator>`: the gate passes it only
    /// with both, and the run records them in the ledger row.
    SkipRung { reason: &'a str, ack: &'a str },
    /// Not judged: `release --dry-run` prints its deploy step without running
    /// deploy-component.sh (rc.sh's dry run), so no gate decides there.
    NotJudged,
}

/// One answer of the gate for one member group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub exit: i32,
    pub message: String,
    pub lower_goatnet: Option<String>,
    pub skip_rung: bool,
}

/// What the gate decided for a whole run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// `(group, message)` for every group that passed, in member order.
    pub passed: Vec<(String, String)>,
    /// The lower goatnets whose convergence the run must confirm, each once.
    pub lower: Vec<String>,
    /// The JSON the ledger row records when the run skipped a rung.
    pub skip: Option<String>,
}

/// The command that judges one ref: the script's own argv.
pub fn command(repo: &Path, component: &str, goatnet: &str, reference: &str, mode: Mode) -> Cmd {
    let mut c = Cmd::new(crate::endstate::python());
    c.arg(repo.join(GATE)).arg("component").arg("--repo").arg(repo).args(["--component", component, "--goatnet", goatnet, "--ref", reference]);
    if let Mode::SkipRung { reason, ack } = mode {
        // An empty value is passed as one space, as the script passes it, so
        // the gate sees a skip and refuses the half that is missing.
        let or_space = |v: &str| if v.is_empty() { " ".to_string() } else { v.to_string() };
        c.args(["--skip-reason".to_string(), or_space(reason), "--skip-ack".to_string(), or_space(ack)]);
    }
    c
}

/// Read the gate's answer from its exit code and stdout, as the script does:
/// the JSON `message` when it parses, else the output as it came.
pub fn read(exit: i32, stdout: &str, stderr: &str) -> Decision {
    let raw = stdout.trim_end_matches('\n');
    let doc = serde_json::from_str::<serde_json::Value>(raw).ok();
    let field = |k: &str| doc.as_ref().and_then(|d| d.get(k)).and_then(|v| v.as_str()).map(String::from);
    let message = match field("message") {
        Some(m) => m,
        None if raw.trim().is_empty() => {
            // Nothing on stdout: the gate did not run to a decision (no PyYAML,
            // a traceback). Say why rather than refuse with an empty reason.
            let why = stderr.trim();
            format!("{GATE} exited {exit} without a decision{}", if why.is_empty() { String::new() } else { format!(": {why}") })
        }
        None => raw.to_string(),
    };
    Decision { exit, message, lower_goatnet: field("lower_goatnet"), skip_rung: field("exception").as_deref() == Some("skip-rung") }
}

fn refuse(text: String) -> anyhow::Error {
    goat_errors::coded(CODE, text).into()
}

/// The script's refusal of `--reason` without `--skip-rung`. `--ack` alone is
/// not refused here: on goatbuilder it also names who declares the run (C4).
pub fn check_flags(skip_rung: bool, reason: &str) -> Result<()> {
    if !skip_rung && !reason.is_empty() {
        return Err(refuse("--reason/--ack are for --skip-rung; pass --skip-rung with both, or neither".into()));
    }
    Ok(())
}

/// Judge one ref. Err only when the gate cannot be started.
pub fn judge(repo: &Path, component: &str, goatnet: &str, reference: &str, mode: Mode) -> Result<Decision> {
    let out = command(repo, component, goatnet, reference, mode)
        .output()
        .map_err(|e| refuse(format!("could not start {} {GATE}: {e}; the release-ladder gate cannot run, refusing (fail-closed)", crate::endstate::python())))?;
    Ok(read(out.status.code().unwrap_or(-1), &String::from_utf8_lossy(&out.stdout), &String::from_utf8_lossy(&out.stderr)))
}

/// Judge every member group's declared ref, in member order. A group with no
/// declared ref is passed over, as the script passes it over. The first
/// refusal ends the run in the script's words: `<group>: <message>`.
pub fn check(repo: &Path, component: &str, goatnet: &str, wants: &[(String, String)], mode: Mode) -> Result<Outcome> {
    if mode == Mode::NotJudged {
        return Ok(Outcome::default());
    }
    let gate = repo.join(GATE);
    if !gate.is_file() {
        return Err(refuse(format!("{} is missing: the release-ladder gate cannot run, refusing (fail-closed)", gate.display())));
    }
    decide_all(wants, mode, |g_ref| judge(repo, component, goatnet, g_ref, mode))
}

/// [`check`]'s loop over a judge, apart from the process it starts.
fn decide_all(wants: &[(String, String)], mode: Mode, mut judge: impl FnMut(&str) -> Result<Decision>) -> Result<Outcome> {
    let mut out = Outcome::default();
    for (g, want) in wants {
        if want.is_empty() {
            continue;
        }
        let d = judge(want)?;
        match d.exit {
            0 => {
                if d.skip_rung {
                    if let Mode::SkipRung { reason, ack } = mode {
                        out.skip = Some(serde_json::json!({"reason": reason, "ack": ack}).to_string());
                    }
                }
                out.passed.push((g.clone(), d.message));
            }
            3 => {
                out.passed.push((g.clone(), d.message));
                if let Some(lg) = d.lower_goatnet {
                    if !out.lower.contains(&lg) {
                        out.lower.push(lg);
                    }
                }
            }
            _ => return Err(refuse(format!("{g}: {}", d.message))),
        }
    }
    Ok(out)
}

/// The dry run's line for a lower rung it would read, in the script's words.
pub fn dry_confirm_line(component: &str, lower: &str) -> String {
    format!("DRY: would confirm {component} converged on {lower} (deploy-component.sh {component} --goatnet {lower} --verify-only)")
}

/// The real run's line before it reads a lower rung's ledgers.
pub fn confirming_line(component: &str, lower: &str, goatnet: &str) -> String {
    format!("ladder: confirming {component} converged on {lower} before touching {goatnet}")
}

/// The refusal when the lower rung has not converged, in the script's words.
pub fn not_converged(component: &str, lower: &str, goatnet: &str) -> anyhow::Error {
    refuse(format!(
        "{CODE} ladder: {component} has not converged on {lower} (the rung below {goatnet}): its hosts' ledgers do not name the pinned ref. \
         Run ops/scripts/deploy-component.sh {component} --goatnet {lower} first, or pass --skip-rung --reason \"<why>\" --ack \"<operator>\""
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const OLD: &str = "ghcr.io/example/app@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const NEW: &str = "ghcr.io/example/app@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn gate_source() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(GATE)
    }

    /// A synthetic tree: three live rungs (zagros_dev, zagros, zagros_prod),
    /// each with one member group pinning `app`, and the real gate copied in.
    fn tree(dev: Option<&str>, pre: &str, records: &[(&str, &str)]) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        fs::create_dir_all(r.join("ops/release/records/app")).unwrap();
        fs::create_dir_all(r.join("ops/pins")).unwrap();
        fs::copy(gate_source(), r.join(GATE)).unwrap();
        for (g, v) in [("zagros_dev_hosts", dev), ("zagros_hosts", Some(pre)), ("zagros_prod_hosts", Some(NEW))] {
            let d = r.join("ops/ansible/group_vars").join(g);
            fs::create_dir_all(&d).unwrap();
            if let Some(v) = v {
                fs::write(d.join("pins.yml"), format!("app_image: \"{v}\"\n")).unwrap();
            }
        }
        fs::write(
            r.join("ops/pins/fleet-pins.yaml"),
            "goatnets:\n  zagros_dev: {members: [zagros_dev_hosts]}\n  zagros: {members: [zagros_hosts]}\n  zagros_prod: {members: [zagros_prod_hosts]}\ncomponents:\n  app: {var: app_image, deploy_playbook: app-deploy.yml}\n",
        )
        .unwrap();
        fs::write(
            r.join("ops/release/ladder.yaml"),
            "rungs:\n  - {name: dev, goatnet: zagros_dev, status: live}\n  - {name: preprod, goatnet: zagros, status: live}\n  - {name: prod, goatnet: zagros_prod, status: live}\n",
        )
        .unwrap();
        for (name, body) in records {
            fs::write(r.join("ops/release/records/app").join(name), body).unwrap();
        }
        t
    }

    /// The gate needs PyYAML; CI installs it, so its absence fails, never skips.
    fn python_ok() {
        let ok = Cmd::new(crate::endstate::python()).args(["-c", "import yaml"]).output().map(|o| o.status.success()).unwrap_or(false);
        assert!(ok, "these tests run ops/release/ladder_gate.py, which needs python3 with PyYAML: pip install pyyaml");
    }

    fn wants(g: &str, v: &str) -> Vec<(String, String)> {
        vec![(g.to_string(), v.to_string())]
    }

    fn refusal(e: anyhow::Error) -> String {
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.as_str()), Some(CODE), "{e:#}");
        format!("{e:#}")
    }

    #[test]
    fn the_lowest_rung_and_a_goatnet_off_the_ladder_pass() {
        python_ok();
        let t = tree(Some(NEW), NEW, &[]);
        let o = check(t.path(), "app", "zagros_dev", &wants("zagros_dev_hosts", NEW), Mode::Gate).unwrap();
        assert_eq!(o.passed, vec![("zagros_dev_hosts".into(), "ladder: zagros_dev is the lowest rung (dev); nothing below it".into())]);
        assert!(o.lower.is_empty() && o.skip.is_none());
        let o = check(t.path(), "app", "elsewhere", &wants("elsewhere_hosts", NEW), Mode::Gate).unwrap();
        assert_eq!(o.passed[0].1, "ladder: goatnet elsewhere is not a live rung of the ladder; nothing to enforce");
    }

    #[test]
    fn pinned_on_the_rung_below_asks_for_its_convergence_once() {
        python_ok();
        let t = tree(Some(NEW), NEW, &[]);
        let two = vec![("zagros_hosts".to_string(), NEW.to_string()), ("zagros_other_hosts".to_string(), NEW.to_string())];
        let o = check(t.path(), "app", "zagros", &two, Mode::Gate).unwrap();
        assert_eq!(o.lower, vec!["zagros_dev".to_string()]);
        assert_eq!(o.passed[0].1, "ladder: app bbbbbbbbbbbb is pinned on rung dev (zagros_dev); confirming it converged there");
    }

    #[test]
    fn a_digest_the_rung_below_does_not_pin_is_refused_in_the_scripts_words() {
        python_ok();
        let t = tree(Some(OLD), NEW, &[]);
        let e = check(t.path(), "app", "zagros", &wants("zagros_hosts", NEW), Mode::Gate).unwrap_err();
        let text = refusal(e);
        assert!(
            text.starts_with("zagros_hosts: GB-373 ladder: app bbbbbbbbbbbb is not on rung dev (goatnet zagros_dev), the rung below preprod (zagros); it pins aaaaaaaaaaaa there. Put it there first:"),
            "{text}"
        );
    }

    #[test]
    fn no_lower_rung_pinning_it_is_refused_fail_closed() {
        python_ok();
        let t = tree(None, NEW, &[]);
        let text = refusal(check(t.path(), "app", "zagros", &wants("zagros_hosts", NEW), Mode::Gate).unwrap_err());
        assert!(text.starts_with("zagros_hosts: GB-373 ladder: no rung below preprod (zagros) pins app (passed over: zagros_dev), so bbbbbbbbbbbb has not run anywhere first."), "{text}");
    }

    #[test]
    fn an_open_hotfix_record_passes_and_an_abandoned_one_does_not() {
        python_ok();
        let rec = format!("class: hotfix\nstate: open\ncandidate_ref: \"{NEW}\"\n");
        let t = tree(Some(OLD), NEW, &[("hotfix-1.yaml", &rec)]);
        let o = check(t.path(), "app", "zagros", &wants("zagros_hosts", NEW), Mode::Gate).unwrap();
        assert!(o.passed[0].1.starts_with("ladder: app bbbbbbbbbbbb is an open hotfix (ops/release/records/app/hotfix-1.yaml)"), "{:?}", o.passed);
        assert!(o.lower.is_empty());
        let t = tree(Some(OLD), NEW, &[("hotfix-1.yaml", &rec.replace("state: open", "state: abandoned"))]);
        assert!(check(t.path(), "app", "zagros", &wants("zagros_hosts", NEW), Mode::Gate).is_err());
    }

    #[test]
    fn skip_rung_passes_with_both_and_is_recorded_and_refuses_either_half_alone() {
        python_ok();
        let t = tree(Some(OLD), NEW, &[]);
        let both = Mode::SkipRung { reason: "the rung below is down", ack: "an operator" };
        let o = check(t.path(), "app", "zagros", &wants("zagros_hosts", NEW), both).unwrap();
        assert_eq!(o.passed[0].1, "ladder: rung below preprod SKIPPED for app bbbbbbbbbbbb — reason: the rung below is down; ack: an operator (recorded in the ledger row)");
        let skip: serde_json::Value = serde_json::from_str(o.skip.as_deref().unwrap()).unwrap();
        assert_eq!(skip, serde_json::json!({"reason": "the rung below is down", "ack": "an operator"}));
        let text = refusal(check(t.path(), "app", "zagros", &wants("zagros_hosts", NEW), Mode::SkipRung { reason: "", ack: "an operator" }).unwrap_err());
        assert_eq!(text, "zagros_hosts: GB-373 ladder: --skip-rung needs --reason \"<why the rung below is skipped>\"");
        let text = refusal(check(t.path(), "app", "zagros", &wants("zagros_hosts", NEW), Mode::SkipRung { reason: "why", ack: "" }).unwrap_err());
        assert_eq!(text, "zagros_hosts: GB-373 ladder: --skip-rung needs --ack \"<operator>\": skipping a rung is attributed");
    }

    #[test]
    fn a_missing_gate_refuses_fail_closed_and_not_judged_reads_nothing() {
        let t = tempfile::tempdir().unwrap();
        let text = refusal(check(t.path(), "app", "zagros", &wants("zagros_hosts", NEW), Mode::Gate).unwrap_err());
        assert!(text.ends_with("ops/release/ladder_gate.py is missing: the release-ladder gate cannot run, refusing (fail-closed)"), "{text}");
        assert_eq!(check(t.path(), "app", "zagros", &wants("zagros_hosts", NEW), Mode::NotJudged).unwrap(), Outcome::default());
    }

    #[test]
    fn a_gate_that_answers_nothing_is_a_refusal_with_its_reason() {
        let d = read(1, "", "ladder_gate.py needs PyYAML\n");
        assert_eq!(d.message, "ops/release/ladder_gate.py exited 1 without a decision: ladder_gate.py needs PyYAML");
        let e = decide_all(&wants("g", NEW), Mode::Gate, |_| Ok(read(2, "", ""))).unwrap_err();
        assert_eq!(refusal(e), "g: ops/release/ladder_gate.py exited 2 without a decision");
        // Output that is not JSON is carried as it came, as the script carries it.
        assert_eq!(read(1, "Traceback\n", "").message, "Traceback");
    }

    #[test]
    fn a_group_with_no_declared_ref_is_passed_over() {
        let mut asked = 0;
        let o = decide_all(&wants("g", ""), Mode::Gate, |_| {
            asked += 1;
            Ok(read(1, "", ""))
        })
        .unwrap();
        assert_eq!((asked, o), (0, Outcome::default()));
    }

    #[test]
    fn reason_without_skip_rung_is_refused_in_the_scripts_words() {
        assert!(check_flags(false, "").is_ok() && check_flags(true, "").is_ok() && check_flags(true, "why").is_ok());
        assert_eq!(refusal(check_flags(false, "why").unwrap_err()), "--reason/--ack are for --skip-rung; pass --skip-rung with both, or neither");
    }

    #[test]
    fn the_command_is_the_scripts_argv() {
        let repo = Path::new("/r");
        let argv = command(repo, "app", "zagros", NEW, Mode::SkipRung { reason: "", ack: "op" }).argv();
        let tail: Vec<&str> = argv.iter().skip(1).map(String::as_str).collect();
        assert_eq!(tail, ["/r/ops/release/ladder_gate.py", "component", "--repo", "/r", "--component", "app", "--goatnet", "zagros", "--ref", NEW, "--skip-reason", " ", "--skip-ack", "op"]);
        assert!(!command(repo, "app", "zagros", NEW, Mode::Gate).argv().iter().any(|a| a.starts_with("--skip")));
    }
}
