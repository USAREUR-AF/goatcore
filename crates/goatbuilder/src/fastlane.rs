//! `fastlane`: one command from a merged commit to every goatnet (ADR 1127) —
//! the deliberate exception to the ladder, and the verb that replaces
//! ops/scripts/fastlane.sh behind the parity gate. The script's steps, in its
//! order, with its refusals:
//!
//!   1 component   exists, has a role and a deploy play, rollback is a pin revert
//!   2 set         not in an OPEN graduation set (ADR 1125): fastlane moves one component
//!   3 digest      the ref resolves to a digest under ghcr.io/usareur-af (ADR 1195)
//!   --dry-run     stops here and says what the mutation would do
//!   4 fleet:      move the value of record; clear VALUE deviations; keep `absent: true`
//!   5 regenerate  pins + intents; re-sign every goatnet's intent
//!   6 record      a hotfix-class record with the previous ref as the way back; abandon
//!                 every open candidate the move overtook
//!   7 gates       graduate.py check, generate-pins --check; land the pin (pr-land.sh)
//!   8 deploy      every goatnet the component belongs on, through this binary's
//!                 push converger; report
//!
//! The pin edit, the regeneration, the re-sign of CHANGED intents only and the
//! records are typed (`pins`, `promote`); a re-run whose pin is already on main
//! lands nothing (the tree differs from main in signatures alone, or not at all).

use crate::runner::Cmd;
use std::path::Path;
use std::process::Stdio;

use anyhow::{anyhow, Context, Result};

use crate::pins::read_yaml;
use crate::{pins, promote, push};
use crate::{Check, Report, EXIT_OK, EXIT_REFUSED};

pub struct Options<'a> {
    pub component: &'a str,
    pub reference: &'a str,
    pub reason: &'a str,
    pub ack: &'a str,
    pub dry_run: bool,
    pub deploy: bool,
}

/// The reason a component's rollback is not a pin revert, from its declaration
/// (ADR 1127). An undeclared `rollback:` is not a refusal here: `--ack` is
/// already mandatory, which is what the deploy gate asks of an undeclared one.
pub fn not_a_pin_revert(comp: &serde_json::Value) -> Option<String> {
    // An empty artifact_type reads as the default, as the script reads it.
    let at = comp.get("artifact_type").and_then(|x| x.as_str()).filter(|s| !s.is_empty()).unwrap_or("image");
    if !matches!(at, "image" | "config") {
        return Some(format!("artifact_type is {at}"));
    }
    if comp.get("reversible").and_then(|x| x.as_bool()) == Some(false) {
        return Some("reversible is false".into());
    }
    match comp.get("rollback") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => match v.as_str() {
            Some("pin-revert") => None,
            Some(other) => Some(format!("rollback is {other}")),
            // The script refuses a non-string here too; neither invents a reading.
            None => Some(format!("rollback is {v}")),
        },
    }
}

fn sh(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Cmd::new(cmd).args(args).output().with_context(|| cmd.to_string())?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn step(checks: &mut Vec<Check>, s: &str) {
    checks.push(Check { name: "step".into(), ok: true, detail: format!("==> {s}"), fix: None });
}
fn note(checks: &mut Vec<Check>, s: String) {
    checks.push(Check { name: "note".into(), ok: true, detail: s, fix: None });
}

/// A registry answer is a digest only in this exact shape; anything else (an
/// error body, an empty line, a tag) is refused rather than carried as one.
fn is_digest(d: &str) -> bool {
    d.len() == 71 && d.starts_with("sha256:") && d[7..].chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// `die` in the script's words: a refusal is "REFUSE — <why>".
fn refuse(why: String) -> anyhow::Error {
    anyhow::Error::from(goat_errors::coded("GB-021", format!("REFUSE — {why}")))
}

/// A refusal keeps the steps already printed, as the script's stderr `REFUSE`
/// follows its stdout steps: the report carries them and ends with the refusal.
pub fn fastlane(repo: &Path, o: &Options) -> Result<Report> {
    let mut checks = Vec::new();
    match run(repo, o, &mut checks) {
        Ok(r) => Ok(r),
        Err(e) => {
            checks.push(Check { name: "refused".into(), ok: false, detail: e.to_string(), fix: None });
            let mut r = Report::new("fastlane", checks, EXIT_REFUSED);
            r.ok = false;
            r.exit = EXIT_REFUSED;
            Ok(r)
        }
    }
}

fn run(repo: &Path, o: &Options, checks: &mut Vec<Check>) -> Result<Report> {
    if o.reference.is_empty() {
        return Err(refuse("--ref is required".into()));
    }
    if o.reason.is_empty() {
        return Err(refuse("--reason is required — the record has to say why the ladder was skipped".into()));
    }
    if o.ack.is_empty() {
        return Err(refuse("--ack '<name>' is required — a fastlane reaches production and is attributed".into()));
    }
    let repo_s = repo.to_str().unwrap_or(".").to_string();
    let pins_path = repo.join("ops/pins/fleet-pins.yaml");

    // 1. component
    step(checks, &format!("component {}", o.component));
    let table = read_yaml(&pins_path)?;
    let components = table.get("components").and_then(|c| c.as_object()).cloned().unwrap_or_default();
    let comp = match components.get(o.component).filter(|c| c.is_object()) {
        Some(c) => {
            let devs = c.get("deviations").and_then(|d| d.as_object()).cloned().unwrap_or_default();
            let mut names: Vec<&String> = devs.keys().collect();
            names.sort();
            let mut absent: Vec<&String> = devs.iter().filter(|(_, v)| v.get("absent").map(|a| a.as_bool() == Some(true)).unwrap_or(false)).map(|(k, _)| k).collect();
            absent.sort();
            let goatnets: serde_json::Map<String, serde_json::Value> = table.get("goatnets").and_then(|g| g.as_object()).map(|m| m.iter().map(|(g, v)| (g.clone(), v.get("members").cloned().unwrap_or(serde_json::json!([])))).collect()).unwrap_or_default();
            serde_json::json!({
                "var": c.get("var"), "image_workflow": c.get("image_workflow"),
                "ledger_component": c.get("ledger_component").and_then(|l| l.as_str()).filter(|l| !l.is_empty()).unwrap_or(o.component),
                "deploy_playbook": c.get("deploy_playbook"), "not_a_pin_revert": not_a_pin_revert(c),
                "fleet": c.get("fleet"), "deviations": names, "deviations_absent": absent, "goatnets": goatnets,
            })
        }
        None => {
            let mut known: Vec<&String> = components.keys().collect();
            known.sort();
            return Err(refuse(format!("unknown component; known: [{}]", known.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(", "))));
        }
    };
    let current_fleet = comp.get("fleet").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let ledger = comp.get("ledger_component").and_then(|x| x.as_str()).unwrap_or(o.component).to_string();
    note(checks, format!("current fleet ref: {}", if current_fleet.is_empty() { "<none>" } else { &current_fleet }));
    note(checks, format!("ledger component:  {ledger}"));
    if !repo.join("ops/ansible/roles").join(&ledger).is_dir() {
        return Err(refuse(format!("no ansible role at ops/ansible/roles/{ledger} — fastlane deploys through the component's role, and rollback.sh reads the same path")));
    }
    let declared_play = comp.get("deploy_playbook").and_then(|x| x.as_str()).unwrap_or("");
    let play = match push::find_play(&repo.join("ops/ansible"), declared_play, &ledger, o.component) {
        push::Play::Found(p) => p,
        push::Play::DeclaredMissing => return Err(refuse(format!("{} declares deploy_playbook: {declared_play}, which is not in ops/ansible/", o.component))),
        push::Play::NotNamed => return Err(refuse(format!("no deploy playbook (tried {ledger}-deploy.yml and {}-deploy.yml) — fastlane will not invent a deploy path", o.component))),
    };
    if let Some(why) = comp.get("not_a_pin_revert").and_then(|x| x.as_str()) {
        return Err(refuse(format!("{why}, so a pin revert cannot undo this (ADR 1127). Use the ladder.")));
    }

    // 2. graduation-set membership
    step(checks, "graduation-set membership");
    let set_name = promote::open_set_name(repo, o.component);
    if !set_name.is_empty() {
        return Err(refuse(format!("component is in OPEN graduation set '{set_name}' — its members must move together (ADR 1125).\n        Fastlane moves one component, so graduate the set normally.\n        Shipping one member alone is the failure the set exists to prevent.")));
    }
    note(checks, "not in an open set".into());

    // 3. digest under the one org (ADR 1195)
    step(checks, "resolving image digest");
    let image_name = current_fleet
        .rsplit('/')
        .next()
        .map(|last| last.split([':', '@']).next().unwrap_or("").to_string())
        .unwrap_or_default();
    if image_name.is_empty() || !current_fleet.contains('/') {
        return Err(refuse("cannot derive the image name from the current fleet ref".into()));
    }
    let (pinned, tag) = if let Some(i) = o.reference.find("@sha256:") {
        let d = o.reference[i + 1..].to_string();
        let t = o.reference[..i].rsplit(':').next().unwrap_or("").to_string();
        (d, t)
    } else {
        let mut t = o.reference.to_string();
        if !(t.starts_with("sha-") || t.starts_with('v') && t.chars().nth(1).map(|c| c.is_ascii_digit()).unwrap_or(false)) {
            t = format!("sha-{t}");
        }
        (String::new(), t)
    };
    // Declared in goatbuilder.yaml (`repo: source:`), so another organisation's
    // checkout reads its own packages. ADR 1195 fixes the namespace, not the owner.
    let Some(owner) = crate::config::owner(repo) else {
        return Err(refuse("ops/goatbuilder/goatbuilder.yaml declares no repo.source, so the registry owner is unknown. Nothing moved.".into()));
    };
    let org = owner.as_str();
    // The packages API takes the owner as GitHub spells it; a registry path is
    // lower case, and the page names the image the one way it exists (ADR 1195).
    let image_org = owner.to_lowercase();
    // Every page: a busy image has hundreds of versions, and the tag asked for
    // is often not among the newest.
    let api = format!("/orgs/{org}/packages/container/{image_name}/versions?per_page=100");
    // An unreadable registry is not a missing image: say which it was, or a
    // reader without access to this project's packages is sent to look at CI.
    if Cmd::new("gh").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        return Err(refuse("the GitHub CLI (gh) is not on PATH, and fastlane reads the image digest through it. Without it the digest cannot be resolved, so nothing moved.".into()));
    }
    // A tag reaches jq as part of the program text, so it has to be a tag: the
    // OCI shape, which is what every tag this repository builds already is.
    // The script applies the same test, in the same words.
    let tag_shaped = !tag.is_empty()
        && tag.len() <= 128
        && tag.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
        && tag.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !tag_shaped {
        return Err(refuse(format!("{tag} is not a container tag: a tag is letters, digits, dot, dash or underscore. Nothing moved.")));
    }
    let d = match sh("gh", &["api", "--paginate", &api, "--jq", &format!(".[]|select(.metadata.container.tags|index(\"{tag}\"))|.name")]) {
        Ok(text) => text.lines().next().unwrap_or("").to_string(),
        Err(e) => {
            let why = format!("{e:#}");
            let first = why.lines().find(|l| !l.trim().is_empty() && l.trim() != "gh").unwrap_or("").trim().to_string();
            return Err(refuse(format!("cannot read ghcr.io/{image_org}/{image_name}: {first}. Without it the digest cannot be resolved, so nothing moved.")));
        }
    };
    // Only a digest is a digest (the script applies the same shape test).
    if !is_digest(&d) {
        return Err(refuse(format!("tag {tag} not found under ghcr.io/{image_org}/{image_name} — has CI built it from a merged commit?")));
    }
    // A ref given with a digest must be the digest the registry holds for its tag.
    if !pinned.is_empty() && pinned != d {
        return Err(refuse(format!("{tag} is {d} under ghcr.io/{image_org}/{image_name}, not {pinned}.")));
    }
    let digest = d;
    note(checks, format!("{org}: {digest}"));
    let new_ref = format!("ghcr.io/{{{{ ghcr_org }}}}/{image_name}:{tag}@{digest}");
    note(checks, format!("new fleet ref: {new_ref}"));
    if new_ref == current_fleet {
        return Err(refuse("that ref is already the fleet value — nothing to do".into()));
    }

    // 4. dry run stops here
    let deviations: Vec<String> = comp.get("deviations").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    let keep: Vec<String> = comp.get("deviations_absent").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    if o.dry_run {
        step(checks, "dry run — stopping before any mutation");
        note(checks, format!("would set components.{}.fleet = {new_ref}", o.component));
        let clear: Vec<&String> = deviations.iter().filter(|k| !keep.contains(k)).collect();
        let mut line = format!("would clear deviations: {}", if clear.is_empty() { "<none>".to_string() } else { clear.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ") });
        if !keep.is_empty() {
            line.push_str(&format!("\n    would PRESERVE (absent: true — topology, not skew): {}", keep.join(", ")));
        }
        note(checks, line);
        note(checks, format!("would deploy via {} to every goatnet member group", play.file_name().unwrap_or_default().to_string_lossy()));
        let mut r = Report::new("fastlane", std::mem::take(checks), EXIT_REFUSED);
        r.exit = EXIT_OK;
        r.ok = true;
        return Ok(r);
    }

    // 5. move fleet:, regenerate, re-sign
    step(checks, "moving the value of record");
    for l in pins::edit_move_fleet(&pins_path, o.component, &new_ref, &current_fleet, o.reason, o.ack)? {
        note(checks, l.trim_start().to_string());
    }
    step(checks, "regenerating pins + intents");
    let regen = pins::regenerate(repo)?;
    for w in &regen.wrote {
        note(checks, format!("wrote {w}"));
    }
    step(checks, "re-signing every goatnet intent whose body changed");
    let goatnets: Vec<String> = comp.get("goatnets").and_then(|g| g.as_object()).map(|m| m.keys().cloned().collect()).unwrap_or_default();
    if regen.changed_intents.is_empty() {
        note(checks, "none changed — no re-sign (a fresh signature over an unchanged body would land a PR of .sig files)".into());
    }
    for g in &regen.changed_intents {
        let intent = repo.join("ops/consistency/intent").join(g).join("intent.json");
        let (key, override_var, ca) = pins::ca_key_for(&intent, g).map_err(|e| refuse(e.to_string()))?;
        if !Path::new(&key).is_file() {
            return Err(refuse(format!("CA key not found at {key} for goatnet {g} (ca_id '{ca}').\n    This goatnet's root key is not on this laptop, or its store does not follow\n    <deploy.state_dir>/<ca-name>/ca.key.age. Point {override_var} at the file.")));
        }
        for l in pins::sign_intent(repo, &intent, &key)? {
            note(checks, l);
        }
    }

    // 6. record the debt as data; abandon what it overtook
    step(checks, "recording the debt");
    let (rec_out, _) = promote::call(repo, &promote::Sub::Hotfix { component: o.component.into(), reference: new_ref.clone(), reason: format!("FASTLANE (ADR 1127): {} — shipped to EVERY goatnet at once with no soak; backfill owed", o.reason), ack: Some(o.ack.into()), previous_ref: Some(current_fleet.clone()) });
    note(checks, rec_out.iter().find(|l| l.contains("hotfix record written")).cloned().unwrap_or_else(|| "hotfix record written".into()));
    note(checks, "NOTE: fastlane moved fleet: and cleared deviations — there is no deviation edit to make.".into());
    step(checks, "abandoning records this fastlane superseded");
    let today = {
        let s = push::stamp_pub();
        format!("{}-{}-{}", &s[0..4], &s[4..6], &s[6..8])
    };
    let mut superseded = 0;
    for rec in promote::candidates_other_than(repo, o.component, &new_ref) {
        let reason = format!("Superseded {today} by a FASTLANE of {} to {tag} (ADR 1127, ack {}): {}. Every rung moved at once, so this candidate's ref is no longer running anywhere. Its evidence stands — it was overtaken, not invalidated.", o.component, o.ack, o.reason);
        let (lines, code) = promote::call(repo, &promote::Sub::Abandon { record: rec.clone(), reason, superseded_by: Some(new_ref.clone()) });
        if code != 0 {
            return Err(anyhow!("{}", lines.join("\n")));
        }
        for l in lines {
            note(checks, l);
        }
        superseded += 1;
    }
    if superseded == 0 {
        note(checks, "none were open".into());
    }

    // 7. gates; land the pin
    step(checks, "gates that still apply");
    let (chk, _) = promote::call(repo, &promote::Sub::Check);
    for l in chk.iter().rev().take(3).collect::<Vec<_>>().into_iter().rev() {
        note(checks, l.clone());
    }
    let gp = pins::run(repo, true)?;
    if let Some(l) = gp.checks.last() {
        note(checks, l.detail.clone());
    }
    step(checks, "landing the pin");
    let mut changed: Vec<String> = sh("git", &["-C", &repo_s, "diff", "--name-only", "origin/main"]).unwrap_or_default().lines().map(String::from).collect();
    changed.extend(sh("git", &["-C", &repo_s, "ls-files", "--others", "--exclude-standard"]).unwrap_or_default().lines().map(String::from));
    if pins::nothing_to_land(&changed) {
        return Err(refuse("pins already current on main; nothing to land — the fleet value is already this ref".into()));
    }
    let br = format!("chore/fastlane-{}-{tag}", o.component);
    let _ = sh("git", &["-C", &repo_s, "checkout", "-q", "-B", &br]);
    let body = std::env::temp_dir().join(format!("goatbuilder-fastlane-body-{}", std::process::id()));
    std::fs::write(&body, format!("chore(pins): fastlane {} to {tag} (ADR 1127)\n\nFASTLANE, ack {}. {}\nRollback: pin revert to {current_fleet}. Backfill owed until this ref graduates normally.\n", o.component, o.ack, o.reason))?;
    let landed = Cmd::new(repo.join("ops/scripts/pr-land.sh"))
        .args(["--worktree", &repo_s, "--branch", &br, "--title", &format!("chore(pins): fastlane {} to {tag} (ADR 1127)", o.component), "--body-file", body.to_str().unwrap_or(""), "--paths", "ops/pins/fleet-pins.yaml", "ops/ansible/group_vars", "ops/consistency/intent", "ops/release/records"])
        .stdout(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !landed {
        return Err(refuse(
            "the pin PR did not land — nothing was deployed".into(),
        ));
    }
    // Back to main at the merge (fastlane.sh does the same): the deploys run
    // Ansible from this checkout, and leaving it on the landed branch made the
    // next driver run refuse and the next fastlane branch from the wrong base.
    let _ = sh("git", &["-C", &repo_s, "fetch", "-q", "origin", "main"]);
    let back = sh("git", &["-C", &repo_s, "checkout", "-q", "main"]).is_ok()
        && Cmd::new("git")
            .args(["-C", &repo_s, "merge", "-q", "--ff-only", "origin/main"])
            .env("GOAT_ALLOW_MASTER_GIT_MUTATE", "1")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    if !back {
        return Err(refuse(format!("the pin landed, but {repo_s} could not return to main at origin/main — fast-forward it, then deploy with goatbuilder deploy --component {} --env <g>", o.component)));
    }

    // 8. deploy everywhere
    if o.deploy {
        step(checks, "deploying to every goatnet the component belongs on");
        let mut failed = Vec::new();
        for g in &goatnets {
            match push::deploy(repo, &push::Options { component: o.component, goatnet: g, only_group: None, dry_run: false, verify_only: false, ack: o.ack }) {
                Ok(r) => {
                    let ok = r.ok;
                    checks.extend(r.checks);
                    if !ok {
                        failed.push(g.clone());
                    }
                }
                Err(e) => {
                    note(checks, format!("{g}: {e:#}"));
                    failed.push(g.clone());
                }
            }
        }
        step(checks, "result");
        if !failed.is_empty() {
            note(checks, format!("NOT converged on: {} — the pin is in main; re-run goatbuilder deploy --env <g> --component {} once the host is reachable, or goatbuilder rollback {}", failed.join(" "), o.component, o.component));
            let mut r = Report::new("fastlane", std::mem::take(checks), EXIT_REFUSED);
            r.ok = false;
            r.exit = 1;
            return Ok(r);
        }
        note(checks, format!("{} is {tag} on every goatnet, ledger-verified. Debt: hotfix-backfill-owed until it graduates (graduate.py status).", o.component));
    } else {
        step(checks, "next");
        note(checks, format!("--no-deploy: the pin is in main; deploy with goatbuilder deploy --env <g> --component {} per goatnet", o.component));
    }
    let mut r = Report::new("fastlane", std::mem::take(checks), EXIT_REFUSED);
    r.exit = EXIT_OK;
    r.ok = true;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn opts<'a>(reference: &'a str, reason: &'a str, ack: &'a str) -> Options<'a> {
        Options { component: "microscope", reference, reason, ack, dry_run: true, deploy: true }
    }

    fn refusal(o: &Options) -> String {
        let r = fastlane(&repo(), o).unwrap();
        assert_eq!(r.exit, EXIT_REFUSED);
        let last = r.checks.last().unwrap();
        assert_eq!(last.name, "refused");
        last.detail.clone()
    }

    #[test]
    fn the_three_required_flags_refuse_in_the_scripts_words() {
        assert_eq!(refusal(&opts("", "r", "a")), "REFUSE — --ref is required");
        assert_eq!(refusal(&opts("sha-1", "", "a")), "REFUSE — --reason is required — the record has to say why the ladder was skipped");
        assert_eq!(refusal(&opts("sha-1", "r", "")), "REFUSE — --ack '<name>' is required — a fastlane reaches production and is attributed");
    }

    /// The steps taken before a refusal stay in the report, as the script's stdout precedes its REFUSE.
    #[test]
    fn an_unknown_component_is_refused_with_the_known_list() {
        let o = Options { component: "no-such", reference: "sha-1", reason: "r", ack: "a", dry_run: true, deploy: true };
        let r = fastlane(&repo(), &o).unwrap();
        assert_eq!(r.checks[0].detail, "==> component no-such");
        let e = r.checks.last().unwrap().detail.clone();
        assert!(e.starts_with("REFUSE — unknown component; known: ["), "{e}");
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;

    #[test]
    fn only_a_lowercase_sha256_of_64_hex_is_a_digest() {
        let good = format!("sha256:{}", "a1".repeat(32));
        assert!(is_digest(&good));
        assert!(!is_digest(&good.to_uppercase().replace("SHA256", "sha256")), "uppercase hex is not the registry's form");
        assert!(!is_digest(&format!("sha256:{}", "a".repeat(63))));
        assert!(!is_digest(&format!("sha512:{}", "a".repeat(64))));
        assert!(!is_digest(""));
        assert!(!is_digest(r#"{"message":"Not Found","status":"404"}"#), "an error body is never carried as a digest");
        assert!(!is_digest(&format!("sha256:{}g", "a".repeat(63))));
    }

    #[test]
    fn a_refusal_reads_in_the_scripts_words() {
        assert_eq!(refuse("x".into()).to_string(), "REFUSE — x");
    }

    #[test]
    fn steps_and_notes_are_ok_rows_a_reader_can_follow() {
        let mut c = Vec::new();
        step(&mut c, "resolve");
        note(&mut c, "detail".into());
        assert_eq!(c[0].detail, "==> resolve");
        assert!(c.iter().all(|x| x.ok && x.fix.is_none()));
    }
}

