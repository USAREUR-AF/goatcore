//! Confirmation 2, phase 10: what our core will admit, approved by a person,
//! then done by the command.
//!
//! Everything our core will load is staged first, in
//! `<work>/admission/<run>/`, from bytes read once: the core's org fragment
//! with this partner's block merged in (one block per partner, replaced on a
//! re-run, every other line kept), the core configuration assembled from it
//! with `roles/goat-zenoh/files/acl-assemble.py` (when the deployment
//! assembles one), their CA certificate, and our core certificate and key.
//! The summary shows the fragment as rendered, region by region, the
//! certificates, and where each staged file goes. Approval binds to a digest
//! over every staged file, the fragment, the agreement digest, the plan hash
//! (in `deploy`) and the targets and restart route. A changed input is a
//! different digest, so an old approval never carries over to it.
//!
//! On approval, in this order, stopping at the first failure:
//!   1. place the staged files where the deployment names, each written
//!      beside its target and renamed into place, keeping a backup per run in
//!      `<work>/admission/<run>/backup/`. The router reads its access control
//!      only when it restarts, so nothing is live yet;
//!   2. the core router's dry run (the deployment's play with `--check --diff`,
//!      or its own command);
//!   3. the committed slot list checked against the registry
//!      (`ops/scripts/sync-slot-anchors.py --check`), just before the restart;
//!   4. the restart, through the deployment's play or its own command.
//!
//! A failure in 1 to 3 puts the backups back and says whether every one went
//! back. "Review in a pull request instead" (`pr` at the terminal, or
//! `--review-in-pr`) writes the summary to `<work>/admission/review.md` and
//! stops; the next run asks again, or takes `--approve-admission <digest>`.
//!
//! This runs only when asked for: `--approve-admission`, `--review-in-pr`,
//! or `--confirm` at a terminal. Otherwise phase 10 stops for the operator
//! exactly as before. It needs the agreement digest confirmed with the
//! partner (confirmation 1) for the agreement in place.
//!
//! Every path and command comes from the deployment's environment (one build
//! for every deployment): FED_CORE_ACL_ORG (the core's org fragment,
//! required: it is what every other vendor and partner is kept from), and
//! FED_CORE_ACL_SEED with FED_CORE_ACL_OUT when the core configuration is
//! assembled; FED_CORE_CERT and FED_CORE_KEY; FED_CORE_PLAY (with
//! FED_CORE_GROUP) or FED_CORE_RESTART and FED_CORE_DRY_RUN, unless the core
//! is the goat-zenoh role's (federate/admit_role.rs: its own play, found from
//! its declarations); and
//! FED_SLOT_REGISTRY_DB or FED_SLOT_PG_SSH (or FED_SLOT_CHECK=skip). A
//! deployment that keeps no committed slot list (no
//! `group_vars/<env>_hosts/zenoh-lazy-storage.yml`, `FED_SLOT_ENV` naming
//! another environment) needs none of those: the check is NOT APPLICABLE, as
//! preflight judges it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::confirm::{pem_fingerprints, sha256, Step, Verdict};
use super::prompt::{self, groups};
use super::run::{Paths, Surface};
use super::state::{digest, State};
use crate::Check;

const REGIONS: [&str; 4] = ["storages", "write-rules", "subjects", "policies"];
const HDR: &str = "##ACL-ORG-REGION ";

fn ok(name: &str, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        ok: true,
        detail: detail.into(),
        fix: None,
    }
}

fn fail(code: &str, detail: impl Into<String>, fix: impl Into<String>) -> Step {
    let d = format!("{code}: {}", detail.into());
    Step {
        rows: vec![Check {
            name: "failed".into(),
            ok: false,
            detail: d.clone(),
            fix: Some(fix.into()),
        }],
        verdict: Verdict::Fail(d),
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Lines before the first header, and each region's lines.
type Regions = (Vec<String>, BTreeMap<&'static str, Vec<String>>);

/// A fragment split into its regions.
fn regions(text: &str) -> Result<Regions, String> {
    let mut pre = Vec::new();
    let mut map: BTreeMap<&'static str, Vec<String>> =
        REGIONS.iter().map(|r| (*r, Vec::new())).collect();
    let mut cur: Option<&'static str> = None;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix(HDR) {
            let name = name.trim();
            cur = Some(
                *REGIONS
                    .iter()
                    .find(|r| **r == name)
                    .ok_or_else(|| format!("unknown region '{name}'"))?,
            );
        } else if let Some(c) = cur {
            map.get_mut(c)
                .expect("every region is in the map")
                .push(line.to_string());
        } else {
            pre.push(line.to_string());
        }
    }
    Ok((pre, map))
}

fn begin(them: &str) -> String {
    format!("      // BEGIN peering {them} (goatbuilder federate)")
}
fn end(them: &str) -> String {
    format!("      // END peering {them}")
}

/// The core's org fragment with this partner's block in each region: any
/// earlier block for the same partner is replaced, every other line is kept.
/// A block that begins and never ends is refused, never guessed at.
pub fn merge_org(existing: &str, partner: &str, them: &str) -> Result<String, String> {
    let (pre, mut cur) = regions(existing)?;
    let (_, add) = regions(partner)?;
    let (b, e) = (begin(them), end(them));
    let mut out = String::new();
    for l in &pre {
        out.push_str(l);
        out.push('\n');
    }
    for r in REGIONS {
        let lines = cur.get_mut(r).expect("region");
        let mut kept = Vec::new();
        let mut inside = false;
        for l in lines.iter() {
            if l.trim() == b.trim() {
                if inside {
                    return Err(format!(
                        "region {r}: the block for {them} begins twice without ending"
                    ));
                }
                inside = true;
            } else if inside && l.trim() == e.trim() {
                inside = false;
            } else if !inside {
                kept.push(l.clone());
            }
        }
        if inside {
            return Err(format!(
                "region {r}: the block for {them} begins and never ends"
            ));
        }
        out.push_str(&format!("{HDR}{r}\n"));
        for l in kept {
            out.push_str(&l);
            out.push('\n');
        }
        if add[r].iter().any(|l| !l.trim().is_empty()) {
            out.push_str(&b);
            out.push('\n');
            for l in &add[r] {
                out.push_str(l);
                out.push('\n');
            }
            out.push_str(&e);
            out.push('\n');
        }
    }
    Ok(out)
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn cert_line(p: &Path) -> String {
    let out = Command::new("openssl")
        .args(["x509", "-noout", "-subject", "-enddate", "-in"])
        .arg(p)
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.replace("notAfter=", "valid until ").trim().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        _ => std::fs::read(p)
            .map(|b| sha256(&b))
            .unwrap_or_else(|_| "absent".into()),
    }
}

enum Slot {
    Db(String),
    Pg(String),
    Skip,
    /// The deployment declares no lazy slot storage, so there is no slot list
    /// to check (the reason, as preflight words it).
    NotApplicable(String),
}

/// The committed slot list a deployment with lazy slot storage keeps, as
/// preflight finds it (preflight_live.py `slot_anchor_file`).
fn slot_anchor_file(repo: &Path, env_name: &str) -> PathBuf {
    repo.join("ops/ansible/group_vars")
        .join(format!("{}_hosts", env_name.replace('-', "_")))
        .join("zenoh-lazy-storage.yml")
}

struct Core {
    org: PathBuf,
    seed: Option<PathBuf>,
    out: Option<PathBuf>,
    cert: Option<PathBuf>,
    key: Option<PathBuf>,
    play: Option<String>,
    group: String,
    restart: Option<String>,
    dry_run: Option<String>,
    slot: Slot,
    /// The goat-zenoh role core, when the deployment names no route of its own
    /// (federate/admit_role.rs).
    role: Option<super::admit_role::RoleCore>,
}

impl Core {
    /// The deployment's settings, or the names of the ones missing (GB-194).
    fn from_env(
        repo: &Path,
        with: &str,
        env_name: &str,
        role: Option<super::admit_role::RoleCore>,
    ) -> Result<Core, Vec<&'static str>> {
        let mut missing = Vec::new();
        let (org, seed, out) = (
            env("FED_CORE_ACL_ORG").map(PathBuf::from),
            env("FED_CORE_ACL_SEED").map(PathBuf::from),
            env("FED_CORE_ACL_OUT").map(PathBuf::from),
        );
        if org.is_none() {
            missing.push("FED_CORE_ACL_ORG (our core's org fragment: every other vendor and partner is kept from it)");
        }
        if seed.is_some() != out.is_some() {
            missing.push("FED_CORE_ACL_SEED and FED_CORE_ACL_OUT together");
        }
        let (play, restart, dry_run) = (
            env("FED_CORE_PLAY"),
            env("FED_CORE_RESTART"),
            env("FED_CORE_DRY_RUN"),
        );
        if play.is_none() && restart.is_none() && role.is_none() {
            missing
                .push("FED_CORE_PLAY (the play that deploys our core router) or FED_CORE_RESTART");
        }
        if play.is_none() && restart.is_some() && dry_run.is_none() {
            missing.push("FED_CORE_DRY_RUN (a command, or skip) beside FED_CORE_RESTART");
        }
        let slot_env = env("FED_SLOT_ENV").unwrap_or_else(|| env_name.to_string());
        let slot = match (
            env("FED_SLOT_CHECK").as_deref(),
            env("FED_SLOT_REGISTRY_DB"),
            env("FED_SLOT_PG_SSH"),
        ) {
            (Some("skip"), _, _) => Slot::Skip,
            (_, Some(db), _) => Slot::Db(db),
            (_, None, Some(h)) => Slot::Pg(h),
            // No registry named: when the deployment keeps no slot list there
            // is nothing to check, exactly as preflight judges it.
            _ if !slot_anchor_file(repo, &slot_env).is_file() => Slot::NotApplicable(format!(
                "no {}: this deployment declares no lazy slot storage",
                slot_anchor_file(Path::new(""), &slot_env).display()
            )),
            _ => {
                missing.push("FED_SLOT_REGISTRY_DB or FED_SLOT_PG_SSH (or FED_SLOT_CHECK=skip)");
                Slot::Skip
            }
        };
        let Some(org) = org.filter(|_| missing.is_empty()) else {
            return Err(missing);
        };
        Ok(Core {
            org,
            seed,
            out,
            cert: env("FED_CORE_CERT").map(PathBuf::from),
            key: env("FED_CORE_KEY").map(PathBuf::from),
            play,
            group: env("FED_CORE_GROUP").unwrap_or_else(|| format!("{with}_core")),
            restart,
            dry_run,
            slot,
            role,
        })
    }

    fn play_cmd(&self, repo: &Path, work: &Path, check: bool) -> Option<String> {
        let Some(play) = self.play.as_ref() else {
            return self.role.as_ref().map(|r| r.play_cmd(repo, check));
        };
        let aws = crate::settings::aws_profile(repo);
        // The per-peering inventory files too, as the driver's plays read
        // them: the core group may be in one of them (plan::per_peering_inventories).
        let more: String = super::plan::per_peering_inventories(repo)
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(|n| format!(" -i {}", sh_quote(&format!("inventory/{n}")))))
            .collect();
        Some(format!(
            "cd {} && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws {} -- ../scripts/ansible-preflight.sh -i inventory/peering-edges.ini{more} --limit {} -e fed_work={} {}{}",
            sh_quote(&repo.join("ops/ansible").display().to_string()),
            sh_quote(&aws),
            sh_quote(&self.group),
            sh_quote(&work.display().to_string()),
            sh_quote(play),
            if check { " --check --diff" } else { "" }
        ))
    }

    fn route(&self) -> String {
        match (&self.play, &self.restart) {
            (Some(p), _) => format!("the play {p} on {}", self.group),
            (None, Some(c)) => format!("FED_CORE_RESTART ({c})"),
            (None, None) => self.role.as_ref().map(|r| r.route()).unwrap_or_else(|| "nothing".into()),
        }
    }
}

fn bash(repo: &Path, cmd: &str) -> Result<(), i32> {
    // Their output goes to stderr: stdout carries the report (-o json).
    match Command::new("bash")
        .args(["-euo", "pipefail", "-c", cmd])
        .current_dir(repo)
        .stdout(std::process::Stdio::from(std::io::stderr()))
        .status()
    {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(s.code().unwrap_or(1)),
        Err(_) => Err(127),
    }
}

/// Write a file only its owner can read (a key, or a staged copy of one).
fn write_private(p: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let _ = std::fs::remove_file(p);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    o.mode(mode);
    o.open(p)?.write_all(bytes)
}

/// Write `bytes` beside `to` and rename it into place. The file keeps the
/// mode of what it replaces; a new key file is 0600.
fn write_in_place(to: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(d) = to.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = to.with_file_name(format!(
        ".{}.goatbuilder-new",
        to.file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default()
    ));
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(to) {
            Ok(m) => m.permissions().mode() & 0o7777,
            Err(_) if to.to_string_lossy().contains("key") => 0o600,
            Err(_) => 0o644,
        }
    };
    #[cfg(not(unix))]
    let mode = 0o644;
    write_private(&tmp, bytes, mode)?;
    std::fs::rename(&tmp, to)
}

/// Place `bytes` at `to`, keeping what was there in `backups` (in `dir`).
fn publish(
    bytes: &[u8],
    to: &Path,
    backups: &mut Vec<(PathBuf, Option<PathBuf>)>,
    dir: &Path,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let kept = if to.exists() {
        let b = dir.join(format!(
            "{}-{}",
            backups.len(),
            to.file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default()
        ));
        std::fs::copy(to, &b)?;
        Some(b)
    } else {
        None
    };
    backups.push((to.to_path_buf(), kept));
    write_in_place(to, bytes)
}

/// Put every backup back; the files that could not be, by name.
fn restore(backups: &[(PathBuf, Option<PathBuf>)]) -> Vec<String> {
    let mut failed = Vec::new();
    for (to, kept) in backups.iter().rev() {
        let r = match kept {
            Some(b) => std::fs::read(b).and_then(|bytes| write_in_place(to, &bytes)),
            None => std::fs::remove_file(to).or_else(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(e)
                }
            }),
        };
        if let Err(e) = r {
            failed.push(format!("{} ({e})", to.display()));
        }
    }
    failed
}

fn put_back(backups: &[(PathBuf, Option<PathBuf>)]) -> String {
    match restore(backups).as_slice() {
        [] => "every file placed is put back".into(),
        f => format!(
            "NOT every file could be put back: {}; the backups are in place to copy by hand",
            f.join(", ")
        ),
    }
}

/// What the person gives for confirmation 2: by flag, or at a terminal.
#[derive(Default, Clone)]
pub struct Admission {
    /// `--approve-admission <digest>`.
    pub approve: Option<String>,
    /// `--review-in-pr`.
    pub review_in_pr: bool,
}

/// Whether confirmation 2 was asked for in this run. Only a deploy, which
/// executes an approved and rechecked plan, admits the edge by the command.
pub fn active(s: &Surface) -> bool {
    s.plan.is_some() && (s.admission.approve.is_some() || s.admission.review_in_pr || s.values.ask)
}

fn stop(code: &str, detail: impl Into<String>, fix: impl Into<String>) -> Step {
    let d = format!("{code}: {}", detail.into());
    Step {
        rows: vec![
            Check {
                name: "stopped".into(),
                ok: true,
                detail: d.clone(),
                fix: Some(fix.into()),
            },
            ok(
                "note",
                "^ or admit by hand as before, then re-run with --from 11",
            ),
        ],
        verdict: Verdict::Stop(d),
    }
}

/// The fragment as rendered, region by region, for a person to read.
fn fragment_text(fragment: &str) -> String {
    let mut s = String::new();
    if let Ok((_, map)) = regions(fragment) {
        for r in REGIONS {
            let body: Vec<&String> = map[r].iter().filter(|l| !l.trim().is_empty()).collect();
            if body.is_empty() {
                continue;
            }
            s.push_str(&format!("    [{r}]\n"));
            for l in body {
                s.push_str(&format!("      {}\n", l.trim_end()));
            }
        }
    }
    s
}

/// Phase 10 by the command: stage what our core will load, show it, take the
/// approval bound to its digest, then place, dry-run, check slots, restart.
#[allow(clippy::too_many_arguments)]
pub fn admit(
    repo: &Path,
    paths: &Paths,
    with: &str,
    env_name: &str,
    edge_group: &str,
    s: &mut Surface,
    st: &mut State,
    run: &str,
) -> Step {
    let again = s
        .again
        .clone()
        .unwrap_or_else(|| format!("goatbuilder federate {with} --from 10"));
    let by_hand = format!("run it again without the admission flags so phase 10 stops, admit by hand, then: {again} --from 11");
    let read_json = |p: &Path| {
        std::fs::read(p)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    };
    let Some(us) = read_json(&paths.ours).and_then(|v| {
        v.get("sovereign_id")
            .and_then(|x| x.as_str())
            .map(String::from)
    }) else {
        return fail(
            "GB-194",
            format!("our half is not at {}", paths.ours.display()),
            "run phase 3 again: goatbuilder federate <partner> --from 3",
        );
    };
    let Ok(ag_bytes) = std::fs::read(&paths.agreement) else {
        return fail(
            "GB-194",
            format!("no agreement at {}", paths.agreement.display()),
            "run phase 6 again",
        );
    };
    let ag: serde_json::Value = serde_json::from_slice(&ag_bytes).unwrap_or_default();
    let agreement_digest = sha256(&ag_bytes);
    let party = ag.get("parties").and_then(|p| p.as_array()).and_then(|ps| {
        ps.iter()
            .find(|p| p.get("sovereign_id").and_then(|x| x.as_str()) != Some(us.as_str()))
            .cloned()
    });
    let Some(party) = party else {
        return fail(
            "GB-194",
            format!("{} names no party besides {us}", paths.agreement.display()),
            "run phase 6 again",
        );
    };
    let them = party
        .get("sovereign_id")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    if them.is_empty()
        || !them
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    {
        return fail(
            "GB-194",
            format!(
                "{} names the other party with an id that is not plain",
                paths.agreement.display()
            ),
            "run phase 6 again",
        );
    }
    if env_name.is_empty() {
        return fail(
            "GB-194",
            format!("no environment is declared for peering '{with}'"),
            "pass --env, or declare the peering's environment in goatbuilder.yaml peerings:",
        );
    }
    let agreed_fp = party
        .get("ca_fingerprint")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    let frag_path = paths
        .applied
        .join(format!("ops/federation/acl/{them}-fragment.json5"));
    let Ok(fragment) = std::fs::read_to_string(&frag_path) else {
        return fail(
            "GB-194",
            format!("no access-control fragment at {}", frag_path.display()),
            "run phase 9 again",
        );
    };
    // Their CA: exactly the one certificate the agreement names.
    let their_ca = std::fs::read(&paths.their_ca).unwrap_or_default();
    if pem_fingerprints(&their_ca) != [agreed_fp.clone()] {
        return fail(
            "GB-194",
            format!("{} must hold exactly one certificate, the CA the agreement names for {them} ({agreed_fp})", paths.their_ca.display()),
            "do not admit the edge; ask the partner which CA is current",
        );
    }
    // A core the goat-zenoh role deploys is restarted by the role's own play,
    // found from its declarations, unless the deployment names a route.
    let role = if env("FED_CORE_PLAY").is_none() && env("FED_CORE_RESTART").is_none() {
        let core_group = env("FED_CORE_GROUP").unwrap_or_else(|| format!("{with}_core"));
        match super::admit_role::find(repo, &core_group) {
            Ok(r) => r,
            Err(e) => return fail("GB-236", e, "declare what it names, or set FED_CORE_PLAY or FED_CORE_RESTART, make a new plan, and deploy again"),
        }
    } else {
        None
    };
    let core = match Core::from_env(repo, with, env_name, role) {
        Ok(c) => c,
        Err(missing) => {
            return fail(
                "GB-194",
                format!(
                    "the deployment does not say how to admit the edge on our core: set {}",
                    missing.join("; ")
                ),
                by_hand,
            )
        }
    };

    // Stage everything the core will load, from bytes read once.
    let dir = paths.work.join("admission").join(run);
    let staged = dir.join("staged");
    let existing = match std::fs::read_to_string(&core.org) {
        Ok(t) => t,
        Err(e) => {
            return fail(
                "GB-194",
                format!(
                    "cannot read our core's org fragment {} ({e})",
                    core.org.display()
                ),
                "check FED_CORE_ACL_ORG names the org fragment our core is assembled from",
            )
        }
    };
    let merged = match merge_org(&existing, &fragment, &them) {
        Ok(m) => m,
        Err(e) => return fail("GB-194", format!("the fragment does not merge into the core's org fragment: {e}"), "check both files carry only the four ##ACL-ORG-REGION headers and whole BEGIN/END blocks"),
    };
    let core_pem = paths.pki.join("core.pem");
    let core_cert = std::fs::read(&core_pem).unwrap_or_default();
    let core_key = core
        .key
        .as_ref()
        .map(|_| std::fs::read(paths.pki.join("core-key.pem")).unwrap_or_default());
    // A certificate or key that is to be placed must be there and look like
    // one: an empty file placed at the core's key path would take it down.
    let pem_has =
        |b: &[u8], what: &str| String::from_utf8_lossy(b).contains(&format!("-----BEGIN {what}"));
    if core.cert.is_some() && !pem_has(&core_cert, "CERTIFICATE") {
        return fail(
            "GB-194",
            format!(
                "{} is missing or is not a PEM certificate, and FED_CORE_CERT says to place it",
                core_pem.display()
            ),
            "run phase 8 again (it mints our core certificate when FED_CORE_SAN is set)",
        );
    }
    if let Some(k) = &core_key {
        if !pem_has(k, "") || !String::from_utf8_lossy(k).contains("PRIVATE KEY") {
            return fail(
                "GB-194",
                format!(
                    "{} is missing or is not a PEM private key, and FED_CORE_KEY says to place it",
                    paths.pki.join("core-key.pem").display()
                ),
                "run phase 8 again",
            );
        }
    }
    // Where their CA goes for the edge router: the controller-side file the
    // edge role copies (goat_zenoh_edge_partner_ca_path), unless the role
    // installs it from <work>/theirs-ca.pem itself (goat_zenoh_edge_install_certs).
    let edge_ca = super::preflight::group_var(repo, edge_group, "goat_zenoh_edge_partner_ca_path")
        .filter(|v| !v.trim().is_empty() && !v.contains("{{"));
    let role_installs =
        super::preflight::group_var(repo, edge_group, "goat_zenoh_edge_install_certs")
            .is_some_and(|v| matches!(v.trim(), "true" | "True" | "yes" | "1"));
    if edge_ca.is_none() && !role_installs {
        return fail(
            "GB-194",
            format!("nothing says where their CA goes for the edge router: set goat_zenoh_edge_partner_ca_path (or goat_zenoh_edge_install_certs) in group_vars/{edge_group}/"),
            "declare it, make a new plan, and deploy again",
        );
    }
    let mut plan: Vec<(String, Vec<u8>, PathBuf)> = vec![(
        "acl-org-fragment.json5".into(),
        merged.clone().into_bytes(),
        core.org.clone(),
    )];
    // The staging folder and its files are the owner's only: a key is staged.
    let write_staged = |name: &str, bytes: &[u8]| {
        std::fs::create_dir_all(&staged)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        write_private(&staged.join(name), bytes, 0o600)
    };
    if let Err(e) = write_staged("acl-org-fragment.json5", merged.as_bytes()) {
        return fail(
            "GB-194",
            format!("cannot stage in {}: {e}", staged.display()),
            "check the working folder is writable",
        );
    }
    if let (Some(seed), Some(out)) = (&core.seed, &core.out) {
        let cfg = staged.join("core-config.json5");
        let assembled = Command::new("python3")
            .arg(repo.join("ops/ansible/roles/goat-zenoh/files/acl-assemble.py"))
            .args(["assemble", "--seed"])
            .arg(seed)
            .arg("--org")
            .arg(staged.join("acl-org-fragment.json5"))
            .arg("--out")
            .arg(&cfg)
            .stdout(std::process::Stdio::from(std::io::stderr()))
            .status();
        let bytes = std::fs::read(&cfg).unwrap_or_default();
        if !assembled.map(|x| x.success()).unwrap_or(false) || bytes.is_empty() {
            return fail(
                "GB-194",
                format!(
                    "acl-assemble.py could not assemble the core configuration from {}",
                    seed.display()
                ),
                "run acl-assemble.py assemble by hand with the same files to see why",
            );
        }
        plan.push(("core-config.json5".into(), bytes, out.clone()));
    }
    if let Some(c) = &core.cert {
        plan.push(("core.pem".into(), core_cert.clone(), c.clone()));
    }
    if let (Some(k), Some(bytes)) = (&core.key, &core_key) {
        plan.push(("core-key.pem".into(), bytes.clone(), k.clone()));
    }
    if let Some(ca) = &edge_ca {
        plan.push(("partner-ca.pem".into(), their_ca.clone(), PathBuf::from(ca)));
    }
    // The key is placed from memory and never written to the staging folder.
    for (name, bytes, _) in plan.iter().filter(|(n, _, _)| n != "core-key.pem") {
        if let Err(e) = write_staged(name, bytes) {
            return fail(
                "GB-194",
                format!("cannot stage {name}: {e}"),
                "check the working folder is writable",
            );
        }
    }
    let plan_hash = s.plan.as_ref().map(|p| p.hash.clone()).unwrap_or_default();
    let targets: String = plan
        .iter()
        .map(|(n, _, to)| format!("{n} -> {}\n", to.display()))
        .collect::<String>()
        + &core.route();
    let mut parts: Vec<(String, Vec<u8>)> = plan
        .iter()
        .map(|(n, b, _)| (format!("staged:{n}"), b.clone()))
        .collect();
    parts.push(("fragment".into(), fragment.clone().into_bytes()));
    parts.push(("partner-ca".into(), their_ca.clone()));
    parts.push(("agreement".into(), agreement_digest.clone().into_bytes()));
    parts.push(("plan".into(), plan_hash.clone().into_bytes()));
    parts.push(("targets".into(), targets.into_bytes()));
    let refs: Vec<(&str, &[u8])> = parts
        .iter()
        .map(|(l, b)| (l.as_str(), b.as_slice()))
        .collect();
    let digest = digest(&refs);

    // The summary a person approves.
    let mut sum = format!("What our core will admit for {them}\n\n  The access-control fragment, as rendered from the agreement:\n{}", fragment_text(&fragment));
    sum.push_str(&format!(
        "\n  Certificates:\n    their CA         {} (the one the agreement names)\n    our core         {}\n\n  Staged in {} (compare each with what is in place before approving):\n",
        groups(&agreed_fp),
        cert_line(&core_pem),
        staged.display()
    ));
    for (n, _, to) in &plan {
        sum.push_str(&format!("    {n}  ->  {}\n", to.display()));
    }
    sum.push_str(&format!("  Then: dry run, slot-list check, restart through {}.\n\n  Agreement digest:  {agreement_digest}\n", core.route()));
    if !plan_hash.is_empty() {
        sum.push_str(&format!("  Plan:              {plan_hash}\n"));
    }
    sum.push_str(&format!("  Approval digest:   {digest}\n"));

    // The decision.
    let interactive = s.values.ask && s.asker().interactive();
    let review = |sum: &str| -> Step {
        let file = paths.work.join("admission/review.md");
        let _ = std::fs::write(
            &file,
            format!(
                "{sum}\nTo approve after review:\n\n    {again} --approve-admission {digest}\n"
            ),
        );
        stop(
            "GB-197",
            format!("admission written for review to {}; nothing on the core changed", file.display()),
            format!("open a pull request with {} and {}, and after it is approved run: {again} --approve-admission {digest}", file.display(), staged.display()),
        )
    };
    let how = match s.admission.approve.clone() {
        Some(v) if prompt::same_value(&v, &digest) => "flag",
        Some(v) => {
            return fail(
                "GB-194",
                format!("--approve-admission {v} is not the digest of what would be admitted now ({digest}); something changed since it was approved"),
                "read the summary again (run without --approve-admission), then approve the digest it shows",
            )
        }
        None if s.admission.review_in_pr => return review(&sum),
        None if interactive => {
            s.asker().say(&format!("\n{sum}"));
            let a = s.asker().ask("Type yes to approve and restart our core router, pr to review in a pull request instead, or press Enter to stop:");
            match a.as_deref().map(str::to_ascii_lowercase).as_deref() {
                Some("yes") => "terminal",
                Some("pr") => return review(&sum),
                _ => return stop("GB-197", "the admission was not approved; nothing on the core changed", format!("when you are ready, run: {again}")),
            }
        }
        None => {
            let mut step = stop(
                "GB-197",
                format!("waiting for the admission to be approved: read the summary, then pass --approve-admission {digest} (or --review-in-pr)"),
                format!("{again} --approve-admission {digest}"),
            );
            step.rows.insert(0, ok("admission", sum));
            return step;
        }
    };

    // Confirmation 1 has to be done for the agreement in place.
    if st.confirmed("agreement", 6, &agreement_digest).is_none() {
        return fail(
            "GB-194",
            format!("the agreement digest {agreement_digest} has not been confirmed with the partner, so the edge is not admitted"),
            format!("compare the digest with the partner: goatbuilder federate {with} --from 6 --to 6 --agreement-digest <digest>"),
        );
    }
    st.confirm(
        run,
        10,
        "admission",
        how,
        &digest,
        BTreeMap::from([
            ("partner".to_string(), them.clone()),
            ("plan".to_string(), plan_hash.clone()),
        ]),
    );
    let mut rows = vec![ok(
        "confirm",
        format!("admission approved ({how}): {digest}"),
    )];

    // The plan is read again and its inputs compared right before anything
    // is placed: approval at a terminal can wait.
    if let Some(p) = &s.plan {
        if let Err(e) = super::deploy::inputs_hold(p) {
            return fail(
                "GB-188",
                format!("{e:#}"),
                "make a new plan from what is there now, approve its hash, and deploy again",
            );
        }
    }
    // 1. place the staged bytes, with backups for this run
    let mut backups = Vec::new();
    let bdir = dir.join("backup");
    for (n, bytes, to) in &plan {
        if let Err(e) = publish(bytes, to, &mut backups, &bdir) {
            let back = put_back(&backups);
            return fail(
                "GB-194",
                format!("cannot place {n} at {}: {e}; {back}", to.display()),
                "check the path is writable by this user",
            );
        }
    }
    rows.push(ok(
        "publish",
        format!("{} files placed; backups in {}", plan.len(), bdir.display()),
    ));
    // 2. dry run
    let dry = match (
        core.play_cmd(repo, &paths.work, true),
        core.dry_run.as_deref(),
    ) {
        (Some(cmd), _) => Some(cmd),
        (None, Some("skip")) => None,
        (None, Some(cmd)) => Some(cmd.to_string()),
        (None, None) => None,
    };
    match dry {
        Some(cmd) => {
            if let Err(rc) = bash(repo, &cmd) {
                let back = put_back(&backups);
                return fail(
                    "GB-195",
                    format!("the core router's dry run failed (exit {rc}); {back}"),
                    format!("run it by hand to read why: {cmd}"),
                );
            }
            rows.push(ok("dry-run", "the core router's dry run passed"));
        }
        None => rows.push(ok(
            "dry-run",
            "SKIP: FED_CORE_DRY_RUN=skip, so no dry run ran",
        )),
    }
    // 3. slot list against the registry, just before the restart
    let script = repo.join("ops/scripts/sync-slot-anchors.py");
    let slot = match &core.slot {
        Slot::Skip | Slot::NotApplicable(_) => None,
        Slot::Db(db) => Some(vec![
            "--env".to_string(),
            env_name.to_string(),
            "--check".into(),
            "--db".into(),
            db.clone(),
        ]),
        Slot::Pg(h) => Some(vec![
            "--env".to_string(),
            env_name.to_string(),
            "--check".into(),
            "--registry-store".into(),
            "postgres".into(),
            "--pg-ssh".into(),
            h.clone(),
        ]),
    };
    match slot {
        Some(args) => {
            let rc = Command::new("python3")
                .arg(&script)
                .args(&args)
                .current_dir(repo)
                .stdout(std::process::Stdio::from(std::io::stderr()))
                .status()
                .map(|s| s.code().unwrap_or(2))
                .unwrap_or(2);
            if rc != 0 {
                let back = put_back(&backups);
                let fix = format!("python3 {} {}   (writes the list; commit it, deploy the core, then run this again)", script.display(), args.iter().filter(|a| *a != "--check").cloned().collect::<Vec<_>>().join(" "));
                return if rc == 1 {
                    fail("GB-195", format!("the committed slot list differs from the registry; restarting now would drop or add stores; {back}"), fix)
                } else {
                    fail("GB-195", format!("the slot registry could not be read (exit {rc}); {back}"), "check FED_SLOT_REGISTRY_DB or FED_SLOT_PG_SSH names a registry this machine can read")
                };
            }
            rows.push(ok("slots", "the committed slot list matches the registry"));
        }
        None => rows.push(ok(
            "slots",
            match &core.slot {
                Slot::NotApplicable(why) => format!("NOT APPLICABLE: {why}"),
                _ => "SKIP: FED_SLOT_CHECK=skip, so the slot list was not checked".into(),
            },
        )),
    }
    // 4. restart
    let restart = core
        .play_cmd(repo, &paths.work, false)
        .or_else(|| core.restart.clone())
        .unwrap_or_default();
    if let Err(rc) = bash(repo, &restart) {
        return fail(
            "GB-196",
            format!("the core restart failed (exit {rc}); the new access control is in place but may not be loaded; what was there before is in {}", bdir.display()),
            format!("read the play or command output, fix the cause, then run: {again} --approve-admission <the digest it shows>"),
        );
    }
    rows.push(ok(
        "restart",
        format!("our core router restarted through {}", core.route()),
    ));
    Step {
        rows,
        verdict: Verdict::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAG: &str = "// GENERATED\n##ACL-ORG-REGION storages\n##ACL-ORG-REGION write-rules\n      {\n        id: \"fed-w-xab\",\n        messages: [\"put\", \"delete\"],\n        flows: [\"ingress\"],\n        permission: \"allow\",\n        key_exprs: [\n          \"from/xab/**\",\n        ],\n      },\n##ACL-ORG-REGION subjects\n      {\n        id: \"fed-xab\",\n        cert_common_names: [\"fed.xab.xaa.01\"],\n      },\n##ACL-ORG-REGION policies\n      { rules: [\"fed-w-xab\"], subjects: [\"fed-xab\"] },\n";

    #[test]
    fn a_merge_keeps_other_vendors_and_replaces_this_partner() {
        let existing = "##ACL-ORG-REGION storages\n##ACL-ORG-REGION write-rules\n      { id: \"vendor-a\" },\n##ACL-ORG-REGION subjects\n##ACL-ORG-REGION policies\n";
        let once = merge_org(existing, FRAG, "xab").unwrap();
        assert!(
            once.contains("vendor-a")
                && once.contains("fed-w-xab")
                && once.contains("BEGIN peering xab"),
            "{once}"
        );
        assert_eq!(
            merge_org(&once, FRAG, "xab").unwrap(),
            once,
            "a re-run changes nothing"
        );
        let changed = merge_org(
            &once,
            &FRAG.replace("from/xab/**", "from/xab/all/**"),
            "xab",
        )
        .unwrap();
        assert_eq!(
            changed.matches("id: \"fed-w-xab\"").count(),
            1,
            "the old block is replaced: {changed}"
        );
        let other = merge_org(&once, &FRAG.replace("xab", "xac"), "xac").unwrap();
        assert!(
            other.contains("fed-w-xab") && other.contains("fed-w-xac"),
            "a second partner leaves the first: {other}"
        );
        assert!(merge_org("##ACL-ORG-REGION nonsense\n", FRAG, "xab").is_err());
        // A block that begins and never ends is refused, not swallowed.
        let broken = once.replacen("      // END peering xab\n", "", 1);
        assert!(merge_org(&broken, FRAG, "xab")
            .unwrap_err()
            .contains("never ends"));
        // Indentation changed by hand still matches the markers.
        let reindented = once.replace("      // BEGIN peering xab", "  // BEGIN peering xab");
        assert_eq!(
            merge_org(&reindented, FRAG, "xab")
                .unwrap()
                .matches("BEGIN peering xab")
                .count(),
            3
        );
    }

    #[test]
    fn the_summary_shows_every_region_as_rendered() {
        let t = fragment_text(FRAG);
        assert!(
            t.contains("[write-rules]")
                && t.contains("[subjects]")
                && t.contains("[policies]")
                && !t.contains("[storages]"),
            "{t}"
        );
        assert!(
            t.contains("from/xab/**")
                && t.contains("fed.xab.xaa.01")
                && t.contains("rules: [\"fed-w-xab\"]"),
            "{t}"
        );
    }

    #[test]
    fn a_restore_reports_what_it_could_not_put_back() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("cfg.json5");
        std::fs::write(&target, "old").unwrap();
        let mut backups = Vec::new();
        publish(b"new", &target, &mut backups, &d.path().join("backup")).unwrap();
        let fresh = d.path().join("fresh.json5");
        publish(b"x", &fresh, &mut backups, &d.path().join("backup")).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert!(restore(&backups).is_empty());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
        assert!(
            !fresh.exists(),
            "a file that was not there is removed again"
        );
        let gone = vec![(target.clone(), Some(d.path().join("no-such-backup")))];
        assert_eq!(restore(&gone).len(), 1);
    }
}
