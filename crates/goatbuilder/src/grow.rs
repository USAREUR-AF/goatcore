//! `goatbuilder grow`: the hand-over of a goatlite to the full preset (gate row G1,
//! docs/design/goatlite-growth.md §6).
//!
//! It reads `goatlite.yaml` at the checkout root, grows it with [`config::grow`] and
//! shows the difference. A real run then hands the goatlite over, in this order:
//!
//! 1. writes the grown configuration to `ops/goatbuilder/goatbuilder.yaml`;
//! 2. runs `goatlite-render.py --hand-over`, which takes the generated header off
//!    every whole file the render wrote, so each is the learner's;
//! 3. removes `goatlite.yaml`, last, so no later run takes the lite path.
//!
//! A run that stopped after step 1 is run again: a `goatbuilder.yaml` that already
//! holds the grown configuration is kept and the run carries on. One that holds
//! anything else is refused (GB-305) and nothing changes.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use crate::{config, Check, Report, EXIT_REFUSED};

/// The file a goatlite is declared in, at the checkout root.
pub const LITE_FILE: &str = "goatlite.yaml";
/// Where a full run reads its configuration.
pub const TARGET: &str = "ops/goatbuilder/goatbuilder.yaml";
/// The renderer that wrote the goatlite's file set, and takes its headers off.
pub const RENDERER: &str = "ops/scripts/goatlite-render.py";
/// `ops/goatbuilder/goatbuilder.yaml` already exists and holds another configuration.
pub const EXISTS_CODE: &str = "GB-305";
/// The renderer, a command grow runs, failed (or could not be started).
pub const RENDERER_FAILED_CODE: &str = "GB-258";

/// The first line of the written configuration.
const HEADER: &str = "# written by goatbuilder grow from goatlite.yaml (docs/design/goatlite-growth.md §6).\n# The goatlite is handed over: this file is yours, and `goatbuilder validate` checks it.\n";

pub fn grow(repo: &Path, dry_run: bool) -> Result<Report> {
    grow_with(repo, dry_run, None)
}

/// [`grow`], asking `ask` before it changes anything: at a terminal without
/// `--yes`, the run shows every write first (the difference, the config, each
/// file handed over, the removal) and goes on only on a typed yes. `None` is
/// `--yes`, or no terminal (where C10 already required `--yes`).
pub fn grow_with(
    repo: &Path,
    dry_run: bool,
    ask: Option<&mut dyn crate::federate::prompt::Ask>,
) -> Result<Report> {
    let lite_path = repo.join(LITE_FILE);
    let target = repo.join(TARGET);
    // A finished hand-over: goatlite.yaml is gone and the config is grow's own.
    // Nothing is left to do, so it says so (exit 0) instead of failing to read
    // the lite file.
    if !lite_path.exists()
        && std::fs::read_to_string(&target)
            .map(|t| t.starts_with(HEADER))
            .unwrap_or(false)
    {
        return Ok(handed_over_already(&target));
    }
    let lite = config::load_value(&lite_path)?;
    let checkout = config::checkout_for(&lite_path);
    let grown = config::grow(&lite, checkout.as_ref())?;
    // A goatbuilder.yaml that already holds the grown configuration is a run that
    // stopped after writing it; anything else is the learner's, and is never replaced.
    let already = if target.is_file() {
        if config::load_value(&target).ok().as_ref() != Some(&grown) {
            // grow's own header: a file an earlier run wrote, since edited, or
            // grown from a goatlite.yaml that has changed since.
            let ours = std::fs::read_to_string(&target)
                .map(|t| t.starts_with(HEADER))
                .unwrap_or(false);
            let msg = if ours {
                format!(
                    "{TARGET} is grow's own file from an earlier run, and it no longer holds what {LITE_FILE} grows into: it was edited, or {LITE_FILE} changed since. Nothing was changed. \
                     Keep the edits by merging them into {LITE_FILE} and deleting {TARGET}, or delete it to grow afresh, then grow again."
                )
            } else {
                format!(
                    "{TARGET} already exists and holds another configuration, so grow would replace it. Nothing was changed. \
                     Move it aside (or merge what it declares into goatlite.yaml), then grow again."
                )
            };
            return Err(goat_errors::coded(EXISTS_CODE, msg).into());
        }
        true
    } else {
        false
    };
    let name = lite
        .pointer("/lite/name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let env = grown
        .pointer("/environments/0/name")
        .and_then(Value::as_str)
        .unwrap_or(&name)
        .to_string();

    let mut checks = vec![row(
        "config",
        format!(
            "{LITE_FILE} grows into a full configuration that validates against {}",
            config::schema_version()
        ),
    )];
    checks.extend(
        difference(&lite, &grown)
            .into_iter()
            .map(|d| row("difference", d)),
    );
    checks.push(row(
        "kept",
        "lite.host has no field here: the inventory and zone the render wrote carry it, and become yours".to_string(),
    ));
    // The renderer command the real run executes, shown verbatim in the plan.
    let renderer = repo.join(RENDERER);
    let render_args: Vec<std::ffi::OsString> = vec![
        "--site".into(),
        name.clone().into(),
        "--config".into(),
        lite_path.clone().into_os_string(),
        "--hand-over".into(),
    ];
    let shown = format!(
        "python3 {} {}",
        renderer.display(),
        render_args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    );
    let interactive = ask.as_ref().is_some_and(|a| a.interactive());
    if dry_run || interactive {
        let mut plan = vec![
            if already {
                format!("keep {TARGET}: it already holds the grown configuration")
            } else {
                format!("would write {TARGET}")
            },
            format!("would hand the rendered files over: {shown}"),
        ];
        // The same command with --check lists each file and changes nothing.
        let out = crate::runner::Cmd::new("python3")
            .arg(&renderer)
            .args(&render_args)
            .arg("--check")
            .current_dir(repo)
            .output()
            .map_err(|e| {
                goat_errors::coded(
                    RENDERER_FAILED_CODE,
                    format!("cannot run python3 {RENDERER} --check: {e}"),
                )
            })?;
        // --check exits 1 for "would change", and the renderer also exits 1 when
        // it refuses to read the goatlite at all. Only its report's own heading
        // says it answered; without it, the run says what the renderer said.
        let answered = String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l.starts_with("goatlite-render: ") && l.ends_with("would hand over:"));
        if !matches!(out.status.code(), Some(0 | 1)) || !answered {
            return Err(goat_errors::coded(
                RENDERER_FAILED_CODE,
                format!(
                    "{RENDERER} --hand-over --check could not say which files the hand-over takes ({}):\n{}{}",
                    out.status,
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                ),
            )
            .into());
        }
        for line in String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.starts_with("  "))
        {
            let l = line.trim();
            if let Some(i) = l.find("  would hand over") {
                plan.push(format!("would hand over {}", l[..i].trim_end()));
            }
        }
        plan.push(format!("would remove {LITE_FILE}"));
        if dry_run {
            checks.extend(plan.into_iter().map(|p| row("plan", p)));
            checks.push(row(
                "next",
                format!(
                    "goatbuilder grow --yes, then goatbuilder standup {env} --site {name} --dry-run"
                ),
            ));
            return Ok(Report::new("grow", checks, EXIT_REFUSED));
        }
        let ask = ask.expect("interactive");
        let mut text: Vec<String> = checks
            .iter()
            .map(|c| format!("  {}  {}", c.name, c.detail))
            .collect();
        text.extend(plan.iter().map(|p| format!("  plan  {p}")));
        ask.say(&format!(
            "goatbuilder grow hands {LITE_FILE} over to the full preset. It changes files in this checkout only, never a host; before you commit, git undoes it.\n{}\n",
            text.join("\n")
        ));
        let answer = ask.ask(&format!(
            "Hand the goatlite over? Type yes to write {TARGET}, take the headers off and remove {LITE_FILE}, or press Enter to stop:"
        ));
        if !crate::federate::prompt::is_yes(&answer) {
            checks.push(Check {
                name: "stopped".into(),
                ok: false,
                detail: "not confirmed: nothing was changed".into(),
                fix: Some("goatbuilder grow, and type yes; or goatbuilder grow --yes".into()),
            });
            return Ok(Report::new("grow", checks, EXIT_REFUSED));
        }
    }

    if already {
        checks.push(row(
            "wrote",
            format!("{TARGET} already holds the grown configuration; kept"),
        ));
    } else {
        write(
            &target,
            &format!("{HEADER}{}", serde_yaml::to_string(&grown)?),
        )?;
        checks.push(row("wrote", format!("wrote {TARGET}")));
    }
    let out = crate::runner::Cmd::new("python3")
        .arg(&renderer)
        .args(&render_args)
        .current_dir(repo)
        .output()
        .map_err(|e| {
            goat_errors::coded(
                RENDERER_FAILED_CODE,
                format!("cannot run python3 {RENDERER}: {e}"),
            )
        })?;
    if !out.status.success() {
        return Err(goat_errors::coded(
            RENDERER_FAILED_CODE,
            format!(
                "{RENDERER} --hand-over did not take the generated headers off ({}); {LITE_FILE} is kept, so grow can run again:\n{}{}",
                out.status,
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
        .into());
    }
    for line in String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("  "))
    {
        checks.push(row("handed over", line.trim().to_string()));
    }
    std::fs::remove_file(&lite_path).map_err(|e| {
        goat_errors::coded(
            "GB-254",
            format!("cannot remove {}: {e}", lite_path.display()),
        )
    })?;
    checks.push(row(
        "removed",
        format!("removed {LITE_FILE}: no later run takes the lite path"),
    ));
    checks.push(row("next", format!("goatbuilder standup {env} --site {name} --dry-run   (after the hand-over, --site is always given)")));
    Ok(Report::new("grow", checks, EXIT_REFUSED))
}

/// The report of a grow run after the hand-over finished.
fn handed_over_already(target: &Path) -> Report {
    let cfg = config::load_value(target).ok();
    let env = cfg
        .as_ref()
        .and_then(|c| c.pointer("/environments/0/name"))
        .and_then(Value::as_str)
        .map(String::from);
    let site = cfg
        .as_ref()
        .and_then(|c| c.pointer("/environments/0/sites/0/name"))
        .and_then(Value::as_str)
        .map(String::from);
    let mut checks = vec![row(
        "handed over",
        format!("already handed over: {LITE_FILE} is gone and {TARGET} is grow's own file, so there is nothing to grow. Nothing was changed."),
    )];
    checks.push(row(
        "next",
        match (env, site) {
            (Some(env), Some(site)) => format!(
                "goatbuilder validate, then goatbuilder standup {env} --site {site} --dry-run"
            ),
            _ => "goatbuilder validate".to_string(),
        },
    ));
    Report::new("grow", checks, EXIT_REFUSED)
}

fn row(name: &str, detail: String) -> Check {
    Check {
        name: name.into(),
        ok: true,
        detail,
        fix: None,
    }
}

/// Write through a file beside the target and a rename, so a failed write leaves
/// no half-written configuration.
fn write(target: &Path, text: &str) -> Result<()> {
    let io = |e: std::io::Error| {
        goat_errors::coded("GB-254", format!("cannot write {}: {e}", target.display()))
    };
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let tmp = target.with_extension("yaml.grow-tmp");
    std::fs::write(&tmp, text).map_err(io)?;
    std::fs::rename(&tmp, target).map_err(io)?;
    Ok(())
}

/// Every leaf that differs between the lite file and the grown configuration, by
/// its dotted path: `+` added, `-` gone, `~` changed (old -> new), in the grown
/// configuration's order, then the lite file's leaves it has no place for.
fn difference(lite: &Value, grown: &Value) -> Vec<String> {
    let (mut a, mut b) = (Vec::new(), Vec::new());
    leaves("", lite, &mut a);
    leaves("", grown, &mut b);
    let old: std::collections::HashMap<&str, &str> =
        a.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let new: std::collections::HashSet<&str> = b.iter().map(|(k, _)| k.as_str()).collect();
    let mut out = Vec::new();
    for (k, v) in &b {
        match old.get(k.as_str()) {
            None => out.push(format!("+ {k}: {v}")),
            Some(o) if o != v => out.push(format!("~ {k}: {o} -> {v}")),
            Some(_) => {}
        }
    }
    for (k, v) in &a {
        if !new.contains(k.as_str()) {
            out.push(format!("- {k}: {v}"));
        }
    }
    out
}

fn leaves(at: &str, v: &Value, out: &mut Vec<(String, String)>) {
    let join = |k: &str| {
        if at.is_empty() {
            k.to_string()
        } else {
            format!("{at}.{k}")
        }
    };
    match v {
        Value::Object(m) if !m.is_empty() => m.iter().for_each(|(k, v)| leaves(&join(k), v, out)),
        Value::Array(a) if !a.is_empty() => a
            .iter()
            .enumerate()
            .for_each(|(i, v)| leaves(&join(&i.to_string()), v, out)),
        _ => out.push((at.to_string(), v.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn lite_fixture_text() -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../ops/goatbuilder/fixtures/goatnet-from-scratch/00-goatlite/goatbuilder.yaml",
        ))
        .unwrap()
    }

    /// A checkout holding the lite fixture as its goatlite.yaml, and a stand-in
    /// renderer that records its arguments in `renderer-args` and takes the header
    /// off the one rendered file the checkout has.
    fn checkout() -> crate::testdir::Held<PathBuf> {
        let dir = crate::testdir::path("gb-grow-verb-");
        std::fs::write(dir.join(LITE_FILE), lite_fixture_text()).unwrap();
        std::fs::create_dir_all(dir.join("ops/scripts")).unwrap();
        std::fs::write(
            dir.join(RENDERER),
            "import sys, pathlib\n\
             root = pathlib.Path(__file__).resolve().parents[2]\n\
             v = root / 'ops/ansible/group_vars/pasture_hosts/vars.yml'\n\
             if '--check' in sys.argv: (root / 'renderer-check-args').write_text(' '.join(sys.argv[1:])); print('goatlite-render: pasture -- would hand over:'); print('  ' + str(v) + '  would hand over (take the generated header off)'); sys.exit(1)\n\
             (root / 'renderer-args').write_text(' '.join(sys.argv[1:]))\n\
             assert (root / 'goatlite.yaml').exists(), 'the hand-over runs before goatlite.yaml goes'\n\
             assert (root / 'ops/goatbuilder/goatbuilder.yaml').exists(), 'the config is written first'\n\
             v.write_text(v.read_text().split('\\n', 1)[1])\n\
             print('goatlite-render: pasture -- handed over:')\n\
             print('  ' + str(v) + '  handed over')\n",
        )
        .unwrap();
        let gv = dir.join("ops/ansible/group_vars/pasture_hosts");
        std::fs::create_dir_all(&gv).unwrap();
        std::fs::write(gv.join("vars.yml"), "# generated by ops/scripts/goatlite-render.py from goatlite.yaml\nsite_name: pasture\n").unwrap();
        dir
    }

    fn details(r: &Report) -> String {
        r.checks
            .iter()
            .map(|c| {
                format!(
                    "{} {} {}",
                    c.name,
                    if c.ok { "ok" } else { "FAIL" },
                    c.detail
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn code_of(e: &anyhow::Error) -> Option<String> {
        e.downcast_ref::<goat_errors::Coded>()
            .map(|c| c.code.clone())
    }

    /// G1: on the lite fixture, the dry run shows a full configuration that
    /// validates and the difference from goatlite.yaml, and writes nothing.
    #[test]
    fn the_dry_run_shows_a_full_config_and_its_difference_and_writes_nothing() {
        let dir = checkout();
        let before = std::fs::read_to_string(dir.join(LITE_FILE)).unwrap();
        let r = grow(&dir, true).unwrap();
        let all = details(&r);
        assert!(r.ok, "{all}");
        assert!(all.contains("validates against goatbuilder/v1"), "{all}");
        for want in [
            "~ preset: \"lite\" -> \"full\"",
            "+ deploy.owner: \"admin@example.org\"",
            "+ deploy.cloud: \"none\"",
            "+ deploy.cross_goatnet: false",
            "+ deploy.ssh_user: \"ubuntu\"",
            "- lite.host: \"pasture-box.lan\"",
        ] {
            assert!(all.contains(want), "the difference shows {want}:\n{all}");
        }
        assert!(all.contains(&format!("would write {TARGET}")), "{all}");
        assert!(all.contains("would remove goatlite.yaml"), "{all}");
        assert!(!dir.join(TARGET).exists(), "a dry run writes no config");
        assert!(
            !dir.join("renderer-args").exists(),
            "a dry run runs the renderer only to check"
        );
        let vars = dir.join("ops/ansible/group_vars/pasture_hosts/vars.yml");
        assert!(
            all.contains(&format!("would hand over {}", vars.display())),
            "the dry run lists each file the hand-over takes: {all}"
        );
        assert!(
            std::fs::read_to_string(&vars)
                .unwrap()
                .starts_with("# generated by"),
            "the dry run takes no header off"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(LITE_FILE)).unwrap(),
            before,
            "a dry run keeps goatlite.yaml as it was"
        );
    }

    /// A real run writes the grown config (validating, and equal to config::grow's
    /// value), hands the files over through the renderer, and removes goatlite.yaml.
    #[test]
    fn a_real_run_writes_the_config_hands_the_files_over_and_removes_the_lite_file() {
        let dir = checkout();
        let r = grow(&dir, false).unwrap();
        let all = details(&r);
        assert!(r.ok, "{all}");
        let written = config::load_value(&dir.join(TARGET)).unwrap();
        let want = config::grow(
            &serde_json::to_value(
                serde_yaml::from_str::<serde_yaml::Value>(&lite_fixture_text()).unwrap(),
            )
            .unwrap(),
            None,
        )
        .unwrap();
        assert_eq!(written, want, "the file holds the grown configuration");
        assert!(std::fs::read_to_string(dir.join(TARGET))
            .unwrap()
            .starts_with("# written by goatbuilder grow"));
        let report = config::validate_report(&dir.join(TARGET)).unwrap();
        assert!(
            report.ok,
            "the written config validates: {}",
            details(&report)
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("renderer-args")).unwrap(),
            format!(
                "--site pasture --config {} --hand-over",
                dir.join(LITE_FILE).display()
            )
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("ops/ansible/group_vars/pasture_hosts/vars.yml"))
                .unwrap(),
            "site_name: pasture\n"
        );
        assert!(!dir.join(LITE_FILE).exists(), "goatlite.yaml is removed");
        assert!(
            all.contains("handed over"),
            "the renderer's lines are shown: {all}"
        );
        assert!(
            all.contains("goatbuilder standup pasture --site pasture --dry-run"),
            "the next step names --site: {all}"
        );
    }

    /// F3: the dry run shows the renderer command the real run executes, verbatim,
    /// and checks the hand-over with the same command and --check.
    #[test]
    fn the_dry_run_shows_the_renderer_command_the_real_run_executes() {
        let dir = checkout();
        let plan = details(&grow(&dir, true).unwrap());
        let check_args = std::fs::read_to_string(dir.join("renderer-check-args")).unwrap();
        grow(&dir, false).unwrap();
        let args = std::fs::read_to_string(dir.join("renderer-args")).unwrap();
        let shown = format!("python3 {} {args}", dir.join(RENDERER).display());
        assert!(
            plan.contains(&format!("would hand the rendered files over: {shown}")),
            "{shown}\n{plan}"
        );
        assert_eq!(check_args, format!("{args} --check"));
    }

    /// At a terminal, without --yes, the run shows every write first (the
    /// difference, the config, each file handed over, the removal) and asks;
    /// anything but a typed yes changes nothing.
    #[test]
    fn at_a_terminal_every_write_is_shown_before_it_asks() {
        let dir = checkout();
        let mut no = crate::federate::prompt::Scripted::new(&[""]);
        let r = grow_with(&dir, false, Some(&mut no)).unwrap();
        assert!(
            !r.ok,
            "a run that was not confirmed is not a success: {}",
            details(&r)
        );
        assert!(
            details(&r).contains("nothing was changed"),
            "{}",
            details(&r)
        );
        let shown = no.transcript.clone();
        let vars = dir.join("ops/ansible/group_vars/pasture_hosts/vars.yml");
        for want in [
            "~ preset: \"lite\" -> \"full\"".to_string(),
            format!("would write {TARGET}"),
            format!("would hand over {}", vars.display()),
            "would remove goatlite.yaml".to_string(),
        ] {
            assert!(
                shown.contains(&want),
                "shown before asking: {want}\n{shown}"
            );
        }
        let asked = shown.rfind("Type yes").unwrap();
        assert!(
            shown.find("would remove goatlite.yaml").unwrap() < asked,
            "{shown}"
        );
        assert!(
            !dir.join(TARGET).exists()
                && dir.join(LITE_FILE).exists()
                && !dir.join("renderer-args").exists()
        );
        let mut yes = crate::federate::prompt::Scripted::new(&["yes"]);
        let r = grow_with(&dir, false, Some(&mut yes)).unwrap();
        assert!(r.ok, "{}", details(&r));
        assert!(dir.join(TARGET).exists() && !dir.join(LITE_FILE).exists());
    }

    /// GB-305 on a file grow wrote in an earlier run and someone edited says so,
    /// rather than only asking for it to be moved aside.
    #[test]
    fn an_edited_file_from_an_earlier_grow_is_named_as_grows() {
        let dir = checkout();
        let want = config::grow(&config::load_value(&dir.join(LITE_FILE)).unwrap(), None).unwrap();
        std::fs::create_dir_all(dir.join("ops/goatbuilder")).unwrap();
        let edited = format!("{HEADER}{}", serde_yaml::to_string(&want).unwrap())
            .replace("ssh_user: ubuntu", "ssh_user: someone");
        assert!(edited.contains("ssh_user: someone"));
        std::fs::write(dir.join(TARGET), &edited).unwrap();
        let e = grow(&dir, true).unwrap_err();
        assert_eq!(code_of(&e).as_deref(), Some(EXISTS_CODE), "{e:#}");
        assert!(
            format!("{e:#}").contains("grow's own file from an earlier run"),
            "{e:#}"
        );
        assert_eq!(std::fs::read_to_string(dir.join(TARGET)).unwrap(), edited);
    }

    /// A run that stopped after writing the config carries on: the same config is kept.
    /// This also pins that a hand-written goatbuilder.yaml equal to the grown
    /// configuration (no grow header) is adopted as if grow had written it: the
    /// file written here carries no header.
    #[test]
    fn a_rerun_after_the_config_was_written_carries_on() {
        let dir = checkout();
        let want = config::grow(&config::load_value(&dir.join(LITE_FILE)).unwrap(), None).unwrap();
        std::fs::create_dir_all(dir.join("ops/goatbuilder")).unwrap();
        std::fs::write(dir.join(TARGET), serde_yaml::to_string(&want).unwrap()).unwrap();
        let r = grow(&dir, false).unwrap();
        assert!(r.ok, "{}", details(&r));
        assert!(
            details(&r).contains("already holds the grown configuration"),
            "{}",
            details(&r)
        );
        assert!(!dir.join(LITE_FILE).exists());
    }

    /// GB-305: another configuration at the target is refused, and nothing changes.
    /// A refused precondition exits 1 (EXIT_REFUSED), as the contract says.
    #[test]
    fn another_config_at_the_target_is_refused_and_nothing_changes() {
        assert_eq!(
            goat_errors::explain(EXISTS_CODE).and_then(|e| e.exit),
            Some(EXIT_REFUSED)
        );
        let dir = checkout();
        std::fs::create_dir_all(dir.join("ops/goatbuilder")).unwrap();
        let other = "schema: goatbuilder/v1\npreset: full\nrepo: {source: example-org/other}\n";
        std::fs::write(dir.join(TARGET), other).unwrap();
        for dry_run in [true, false] {
            let e = grow(&dir, dry_run).unwrap_err();
            assert_eq!(code_of(&e).as_deref(), Some(EXISTS_CODE), "{e:#}");
            assert!(format!("{e:#}").contains(TARGET), "{e:#}");
        }
        assert_eq!(std::fs::read_to_string(dir.join(TARGET)).unwrap(), other);
        assert!(dir.join(LITE_FILE).exists());
        assert!(!dir.join("renderer-args").exists());
    }

    /// GB-304: a file that is not a goatlite (another preset, or no lite: block);
    /// GB-306: a four-fact file with no repo or components; GB-244: no
    /// goatlite.yaml at all.
    #[test]
    fn the_refusals_carry_their_codes() {
        let dir = checkout();
        std::fs::write(
            dir.join(LITE_FILE),
            lite_fixture_text().replace("preset: lite", "preset: full"),
        )
        .unwrap();
        assert_eq!(
            code_of(&grow(&dir, true).unwrap_err()).as_deref(),
            Some("GB-304")
        );
        let four = "schema: goatbuilder/v1\npreset: lite\nlite:\n  name: pasture\n  host: 192.0.2.50\n  ssh_user: ubuntu\n  admin: admin@example.org\n";
        std::fs::write(dir.join(LITE_FILE), four).unwrap();
        let e = grow(&dir, true).unwrap_err();
        assert_eq!(code_of(&e).as_deref(), Some("GB-306"), "{e:#}");
        assert!(
            !format!("{e:#}").contains("does not validate"),
            "GB-025's words are not GB-306's: {e:#}"
        );
        // GB-304's second site: preset: lite with no lite: block.
        let no_block =
            "schema: goatbuilder/v1\npreset: lite\nrepo: {source: example-org/pasture}\n";
        std::fs::write(dir.join(LITE_FILE), no_block).unwrap();
        let e = grow(&dir, true).unwrap_err();
        assert_eq!(code_of(&e).as_deref(), Some("GB-304"), "{e:#}");
        assert!(format!("{e:#}").contains("no lite: block"), "{e:#}");
        std::fs::remove_file(dir.join(LITE_FILE)).unwrap();
        assert_eq!(
            code_of(&grow(&dir, true).unwrap_err()).as_deref(),
            Some("GB-244")
        );
        assert!(!dir.join(TARGET).exists());
    }

    /// A renderer that fails stops the run before goatlite.yaml is removed, so the
    /// run can be repeated. It is a command the verb runs that failed (GB-258,
    /// exit 1), not a file that cannot be read or written (GB-254).
    #[test]
    fn a_failed_hand_over_keeps_the_lite_file() {
        let dir = checkout();
        std::fs::write(
            dir.join(RENDERER),
            "import sys\nprint('goatlite-render: no', file=sys.stderr)\nsys.exit(1)\n",
        )
        .unwrap();
        let e = grow(&dir, false).unwrap_err();
        assert_eq!(code_of(&e).as_deref(), Some("GB-258"), "{e:#}");
        assert!(format!("{e:#}").contains("goatlite-render: no"), "{e:#}");
        // The dry run's --check is the same command; exit 1 there means "would
        // change", so a renderer that cannot answer exits otherwise.
        std::fs::write(
            dir.join(RENDERER),
            "import sys\nprint('goatlite-render: no', file=sys.stderr)\nsys.exit(2)\n",
        )
        .unwrap();
        let e = grow(&dir, true).unwrap_err();
        assert_eq!(code_of(&e).as_deref(), Some("GB-258"), "{e:#}");
        assert!(
            dir.join(LITE_FILE).exists(),
            "goatlite.yaml stays for the next run"
        );
    }

    /// Grow run again after a finished hand-over says the goatlite was already
    /// handed over and exits 0, a dry run too: goatlite.yaml is gone and the
    /// config is grow's own. Without grow's header it is not grow's hand-over, and
    /// the missing goatlite.yaml is still GB-244.
    #[test]
    fn grow_again_after_a_finished_hand_over_says_so() {
        let dir = checkout();
        grow(&dir, false).unwrap();
        for dry_run in [true, false] {
            let r = grow(&dir, dry_run).unwrap();
            assert!(r.ok, "{}", details(&r));
            assert_eq!(r.exit, crate::EXIT_OK, "{}", details(&r));
            assert!(
                details(&r).contains("already handed over"),
                "{}",
                details(&r)
            );
        }
        let written = std::fs::read_to_string(dir.join(TARGET)).unwrap();
        std::fs::write(dir.join(TARGET), written.strip_prefix(HEADER).unwrap()).unwrap();
        assert_eq!(
            code_of(&grow(&dir, true).unwrap_err()).as_deref(),
            Some("GB-244")
        );
    }
}
