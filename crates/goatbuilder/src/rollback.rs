//! `rollback`: a deployed component back to its N-th most recent ref, by
//! artifact type, reading the ON-HOST ledger as the source of truth — the verb
//! that replaces ops/scripts/rollback.sh (ADR 0859 → ADR 1060,
//! universal-deploy-rollback.md) behind the parity gate. Same modes, same
//! precedence, same refusals and exit codes, same ledger row written:
//!
//!   rollback <component> [N]      roll back N steps (1..7)         exit 0 / 3 refused unsafe / 4 guided
//!   rollback <component> --list   the deploy history, newest first (host ledger, else in-repo)
//!   rollback <component> --status the running ref per host and the artifact type
//!
//! The connection map (host, user, container, compose, env file) is the role's
//! in-repo `deploy-history.yml`; the artifact type, reversibility, restore
//! pointer and ref history come from `/var/lib/goat/<component>/deploy-history.jsonl`
//! on the first reachable host, falling back to the in-repo `history:` block.
//! The flip itself is the script's own remote shell, run over the same `ssh`.

use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

use crate::{Check, Report, EXIT_OK, EXIT_REFUSED};

/// data / cert / forward-only / reversible:false — a ref swap is not the mechanism.
pub const EXIT_REFUSE_UNSAFE: i32 = 3;
/// config / acl — the manual revert path is printed.
pub const EXIT_GUIDED: i32 = 4;

#[derive(Debug, Clone)]
struct HostMap {
    host: String,
    user: String,
    container: String,
    compose: String,
    env_file: String,
    env_var: String,
    compose_files: Vec<String>,
    /// `yq -r` prints a missing key as `null` and an empty string as nothing; the
    /// status line shows which, as the script does.
    host_shown: String,
    container_shown: String,
    site: String,
    /// The `--site` goatnet's door, for the one host of a `--site` map: tried
    /// when the host does not answer directly (see [`load_reached`], [`ssh_args`]).
    door: Option<DoorWay>,
}

/// How one host is reached when it does not answer directly. Both answers are
/// decided once, on first use, as rollback.sh decides them per run.
#[derive(Debug, Clone)]
struct DoorWay {
    repo: std::path::PathBuf,
    site: String,
    members: Vec<String>,
    direct: std::cell::OnceCell<bool>,
    /// `(door user, local port)`, or `None` when no door opens.
    jump: std::cell::OnceCell<Option<(String, u16)>>,
}

#[derive(Debug, Clone)]
struct History {
    image: String,
    sha: String,
    merged_at: String,
    pr: String,
    summary: String,
    do_not_roll_to: bool,
}

fn s(v: Option<&serde_yaml::Value>) -> String {
    match v {
        Some(serde_yaml::Value::String(x)) => x.clone(),
        Some(serde_yaml::Value::Number(n)) => n.to_string(),
        Some(serde_yaml::Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// A field as `yq -r` prints it: a missing key or YAML null is `null`; anything else its value.
fn shown(v: Option<&serde_yaml::Value>) -> String {
    match v {
        None | Some(serde_yaml::Value::Null) => "null".into(),
        other => s(other),
    }
}

fn load_ledger(repo: &Path, component: &str) -> Result<(Vec<HostMap>, Vec<History>)> {
    load_ledger_at(repo, component, None)
}

/// The ledger with its `hosts:` narrowed to one `site:` when one is named, so a
/// component that runs on several goatnets is listed, read and rolled back on one.
fn load_ledger_at(repo: &Path, component: &str, site: Option<&str>) -> Result<(Vec<HostMap>, Vec<History>)> {
    let role = repo.join("ops/ansible/roles").join(component);
    if !role.is_dir() {
        return Err(goat_errors::coded("GB-023", format!("no Ansible role at {} — is '{component}' a deploying component?", role.display())).into());
    }
    let p = role.join("deploy-history.yml");
    if !p.is_file() {
        return Err(anyhow!("no deploy-history.yml at {} — '{component}' not yet adopted (ADR 0859/1060)", p.display()));
    }
    let doc: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&p)?).with_context(|| p.display().to_string())?;
    let hosts: Vec<HostMap> = doc
        .get("hosts")
        .and_then(|h| h.as_sequence())
        .map(|seq| {
            seq.iter()
                .map(|h| HostMap {
                    host: s(h.get("host")),
                    user: s(h.get("ssh_user")),
                    container: s(h.get("container_name")),
                    compose: s(h.get("compose_path")),
                    env_file: s(h.get("env_file")),
                    env_var: s(h.get("env_var")),
                    compose_files: h.get("compose_files").and_then(|c| c.as_sequence()).map(|a| a.iter().map(|x| s(Some(x))).collect()).unwrap_or_default(),
                    host_shown: shown(h.get("host")),
                    container_shown: shown(h.get("container_name")),
                    site: s(h.get("site")),
                    door: None,
                })
                .collect()
        })
        .unwrap_or_default();
    if hosts.is_empty() {
        return Err(anyhow!("deploy-history.yml has no hosts entries"));
    }
    let hosts: Vec<HostMap> = match site {
        Some(want) => {
            let kept: Vec<HostMap> = hosts.into_iter().filter(|h| h.site == want).collect();
            if kept.is_empty() {
                return Err(anyhow!("deploy-history.yml has no hosts entry with site: {want}"));
            }
            kept
        }
        None => hosts,
    };
    let history: Vec<History> = doc
        .get("history")
        .and_then(|h| h.as_sequence())
        .map(|seq| {
            seq.iter()
                .map(|e| History {
                    image: s(e.get("image")),
                    sha: s(e.get("sha")),
                    merged_at: s(e.get("merged_at")),
                    pr: s(e.get("pr")),
                    summary: s(e.get("summary")),
                    do_not_roll_to: e.get("do_not_roll_to").and_then(|b| b.as_bool()).unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((hosts, history))
}

fn ledger_path(component: &str) -> String {
    format!("/var/lib/goat/{component}/deploy-history.jsonl")
}

/// The ledger narrowed to `site`. With a `site` whose map is one host, that host
/// is reached as rollback.sh reaches it (ADR 1201): direct ssh first; when that
/// does not answer, through the goatnet's env door, decided by `deploy`'s own
/// transport ([`crate::push::transport`]: the netbird profile, the mesh host on
/// tcp/22, then the door registry, opening the door). A door that does not open
/// falls through to direct ssh, which then reads as unreachable: `--list` takes
/// the in-repo history and a roll skips the host, in both avenues. A multi-host
/// map never goes through a door; with no `site` the hosts are reached directly.
fn load_reached(repo: &Path, component: &str, site: Option<&str>) -> Result<(Vec<HostMap>, Vec<History>)> {
    let (mut hosts, history) = load_ledger_at(repo, component, site)?;
    if let (Some(site), 1) = (site, hosts.len()) {
        // The member groups only feed the mesh probe; a component the pins do
        // not declare on this goatnet is decided on the profile and the door alone.
        let members = crate::push::declared(repo, component, &site.replace('-', "_"), None).map(|d| d.members).unwrap_or_default();
        hosts[0].door = Some(DoorWay { repo: repo.to_path_buf(), site: site.to_string(), members, direct: Default::default(), jump: Default::default() });
    }
    Ok((hosts, history))
}

/// The door `deploy` would use for `site`, `(door user, local port)`, or `None`
/// when deploy's transport is not the door or the door does not open.
fn resolve_jump(repo: &Path, site: &str, members: &[String]) -> Option<(String, u16)> {
    match crate::push::transport(repo, site, members, false) {
        Ok(t) if t.kind == "door" => door_jump(repo, site),
        _ => None,
    }
}

/// The door's user and local port, from `env-door.sh <site> --ssh-target`
/// (`user@127.0.0.1:<port>`), the one place the door's facts are decided.
fn door_jump(repo: &Path, site: &str) -> Option<(String, u16)> {
    let out = Command::new("bash").arg(repo.join("ops/scripts/env-door.sh")).args([site, "--ssh-target"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (u, p) = text.trim().rsplit_once("@127.0.0.1:")?;
    if u.is_empty() {
        return None;
    }
    Some((u.to_string(), p.parse().ok()?))
}

/// Where an ssh to this host goes, as the dry run prints it.
fn ssh_dest(h: &HostMap) -> String {
    match h.door.as_ref().and_then(|w| w.jump.get().cloned().flatten()) {
        Some((u, p)) => format!("{}@{} (through the door, jump {u}@127.0.0.1:{p})", h.user, h.host),
        None => format!("{}@{}", h.user, h.host),
    }
}

/// The ssh arguments that reach a host, as rollback.sh's `rssh` orders them:
/// direct when the host answers (or has no door to try), else through the door.
/// With `say`, a door in use is named on stderr in the script's words. The
/// script's reads send rssh's stderr to /dev/null, so only its flip shows the
/// note; the binary says it on the flip alone too, and the read modes print
/// exactly what the script prints.
fn ssh_args(h: &HostMap, say: bool) -> Vec<String> {
    let Some(w) = &h.door else { return direct_args(h) };
    if *w.direct.get_or_init(|| Command::new("ssh").args(direct_args(h)).arg("true").output().map(|o| o.status.success()).unwrap_or(false)) {
        return direct_args(h);
    }
    match w.jump.get_or_init(|| resolve_jump(&w.repo, &w.site, &w.members)) {
        Some((u, p)) => {
            if say {
                eprintln!("goatbuilder: {} is not reachable directly; jumping through the {} door ({u}@127.0.0.1:{p})", h.host, w.site);
            }
            jump_args(h, u, *p)
        }
        None => direct_args(h),
    }
}

fn direct_args(h: &HostMap) -> Vec<String> {
    ["-o", "StrictHostKeyChecking=no", "-o", "ConnectTimeout=4", "-o", "BatchMode=yes"].iter().map(|x| x.to_string()).chain([format!("{}@{}", h.user, h.host)]).collect()
}

/// Through a door the ssh still goes to `user@<the host's address>`; a
/// ProxyCommand carries it over the door's tunnel port (`-W %h:%p`), as
/// ops/validation/probes.py `_ssh_jump` does. Only that hop, whose key changes
/// with the tunnel, skips the host-key check; the host's own key is checked
/// (`accept-new`), so a door that lands elsewhere can never stand in for it.
fn jump_args(h: &HostMap, door_user: &str, port: u16) -> Vec<String> {
    vec![
        "-o".into(),
        "ConnectTimeout=4".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        format!("ProxyCommand=ssh -o BatchMode=yes -o ConnectTimeout=6 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -p {port} -W %h:%p {door_user}@127.0.0.1"),
        format!("{}@{}", h.user, h.host),
    ]
}

fn ssh(h: &HostMap, cmd: &str) -> Option<String> {
    let out = Command::new("ssh").args(ssh_args(h, false)).arg(cmd).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

fn host_rows_newest_first(h: &HostMap, component: &str) -> Vec<serde_json::Value> {
    ssh(h, &format!("sudo tac {} 2>/dev/null", ledger_path(component)))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .collect()
}

pub(crate) fn row_ref(r: &serde_json::Value) -> Option<String> {
    r.get("ref").or_else(|| r.get("image")).and_then(|x| x.as_str()).map(String::from)
}

fn host_refs_newest_first(h: &HostMap, component: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    host_rows_newest_first(h, component).iter().filter_map(row_ref).filter(|r| seen.insert(r.clone())).collect()
}

struct ArtifactMeta {
    kind: String,
    reversible: bool,
    pointer: String,
    source: String,
}

fn artifact_meta(hosts: &[HostMap], component: &str) -> ArtifactMeta {
    for h in hosts {
        let Some(row) = ssh(h, &format!("sudo tail -n 1 {} 2>/dev/null", ledger_path(component))) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(row.trim()) else { continue };
        return ArtifactMeta {
            kind: v.get("artifact_type").and_then(|x| x.as_str()).unwrap_or("image").to_string(),
            reversible: v.get("reversible").and_then(|x| x.as_bool()).unwrap_or(true),
            pointer: v.get("restore_pointer").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            source: format!("host-ledger({})", h.host),
        };
    }
    ArtifactMeta { kind: "image".into(), reversible: true, pointer: String::new(), source: "default(image)".into() }
}

fn line(detail: String) -> Check {
    Check { name: "rollback".into(), ok: true, detail, fix: None }
}

/// One ledger row as `--list` prints it; `history` prints the same line.
pub(crate) fn format_row(r: &serde_json::Value) -> String {
    let rev = if r.get("reversible").and_then(|x| x.as_bool()) == Some(false) { "  (reversible:false)" } else { "" };
    // `ts` is when the play started; `promoted_at` (rows promoted)
    // is when it finished on this host.
    let fin = r.get("promoted_at").and_then(|x| x.as_str()).map(|p| format!("  (finished {p})")).unwrap_or_default();
    format!(
        "  {}  [{}]  {}{rev}{fin}",
        r.get("ts").and_then(|x| x.as_str()).unwrap_or("?"),
        r.get("artifact_type").and_then(|x| x.as_str()).unwrap_or("image"),
        row_ref(r).unwrap_or_else(|| "—".into())
    )
}

/// The in-repo `history:` block as `--list` prints it when no host ledger is reachable.
pub(crate) fn in_repo_lines(repo: &Path, component: &str) -> Result<Vec<String>> {
    let (_, history) = load_ledger(repo, component)?;
    // The script renders the in-repo rows through `yq … | @tsv`, which escapes a
    // newline, tab or backslash inside a field as the two characters `\n`, `\t`, `\\`.
    // jq's @tsv renders a null or missing field as empty; only the PR column, built
    // as `"PR#" + (.pr | tostring)`, shows a missing number as `PR#null`.
    let tsv = |s: &str| s.replace('\\', "\\\\").replace('\n', "\\n").replace('\t', "\\t");
    let pr = |s: &str| if s.is_empty() { "null".to_string() } else { tsv(s) };
    Ok(history.iter().enumerate().map(|(i, e)| format!("  [{i}] {:<14} {}  PR#{}\n    {}", tsv(&e.sha), tsv(&e.merged_at), pr(&e.pr), tsv(&e.summary))).collect())
}

pub fn list(repo: &Path, component: &str, site: Option<&str>) -> Result<Report> {
    let (hosts, _) = load_reached(repo, component, site)?;
    let mut checks = vec![line(format!("deploy history for {component} (newest first):"))];
    for h in &hosts {
        let rows = host_rows_newest_first(h, component);
        if !rows.is_empty() {
            // Name the goatnet the host belongs to, and with no --site on a
            // multi-host map say that only the first reachable host is shown,
            // as rollback.sh --list does.
            let goatnet = if h.site.is_empty() { String::new() } else { format!(", goatnet {}", h.site) };
            checks.push(line(format!("  (on-host ledger @ {}{goatnet} — source of truth)", h.host)));
            if site.is_none() && hosts.len() > 1 {
                checks.push(line(format!(
                    "this is the first reachable of {} hosts; pass --site <goatnet> for one goatnet's history",
                    hosts.len()
                )));
            }
            for r in rows {
                checks.push(line(format_row(&r)));
            }
            return Ok(Report::new("rollback", checks, EXIT_REFUSED));
        }
    }
    checks.push(line("no reachable on-host ledger — falling back to in-repo deploy-history.yml".into()));
    for l in in_repo_lines(repo, component)? {
        checks.push(line(l));
    }
    Ok(Report::new("rollback", checks, EXIT_REFUSED))
}

pub fn status(repo: &Path, component: &str, site: Option<&str>) -> Result<Report> {
    let (hosts, _) = load_reached(repo, component, site)?;
    let m = artifact_meta(&hosts, component);
    let mut checks = vec![line(format!("{component} — artifact_type={} reversible={} (from {})", m.kind, m.reversible, m.source))];
    for h in &hosts {
        let current = ssh(h, &format!("sudo docker inspect {} --format '{{{{.Config.Image}}}}'", h.container)).map(|x| x.trim().to_string()).unwrap_or_else(|| "<unreachable>".into());
        // `yq -r` prints a missing container_name as `null`, as the script shows it.
        checks.push(line(format!("  {} @ {}: {current}", h.container_shown, h.host_shown)));
    }
    Ok(Report::new("rollback", checks, EXIT_REFUSED))
}

/// The remote shell the script runs, verbatim, for the three flip shapes.
fn flip_script(shape: &str, h: &HostMap, component: &str, prev: &str, target: &str, ts: &str, pr: &str) -> String {
    let ledger = ledger_path(component);
    let ledger_dir = ledger.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default();
    let operator = format!("{}@{}", std::env::var("USER").unwrap_or_else(|_| "unknown".into()), hostname());
    let row = |via: &str| format!("{{\"ts\":\"{ts}\",\"component\":\"{component}\",\"artifact_type\":\"image\",\"action\":\"rollback\",\"ref\":\"{target}\",\"prior_ref\":\"{prev}\",\"image\":\"{target}\",\"reversible\":true,\"operator\":\"{operator}\",\"via\":\"{via}\",\"target_pr\":\"{pr}\"}}");
    let esc = |x: &str| x.replace('/', "\\/");
    match shape {
        "compose_files" => {
            let cf = h.compose_files.join(" ");
            let mf_dir = h.compose_files.first().and_then(|f| f.rsplit_once('/')).map(|x| x.0.to_string()).unwrap_or_default();
            let ff: String = h.compose_files.iter().map(|f| format!(" -f {f}")).collect();
            format!(
                "set -euo pipefail\nhit=''\nfor f in {cf}; do\n  if sudo grep -qF '{prev}' \"$f\"; then hit=\"$f\"; break; fi\ndone\n[ -n \"$hit\" ] || {{ echo 'rollback.sh: prev image not found in any compose file' >&2; exit 1; }}\nsudo sed -i.rollback-bak \"s|^\\(\\s*image:\\s*\\\"\\?\\){}\\(\\\"\\?\\)|\\1{}\\2|\" \"$hit\"\nif sudo grep -qF '{prev}' \"$hit\"; then echo \"rollback.sh: sed did not rewrite $hit\" >&2; exit 1; fi\nsudo docker compose --project-directory '{mf_dir}'{ff} up -d --force-recreate {}\nsudo mkdir -p '{ledger_dir}'\necho '{}' | sudo tee -a '{ledger}' >/dev/null\n",
                esc(prev), esc(target), h.container, row("rollback.sh:compose_files")
            )
        }
        "env_file" => format!(
            "set -euo pipefail\nsudo sed -i.rollback-bak \"s|^{}=.*|{}={target}|\" '{}' \\\n  && sudo grep -q '^{}=' '{}' \\\n  || echo '{}={target}' | sudo tee -a '{}' >/dev/null\nsudo docker compose --project-directory '{}' -f '{}' up -d --force-recreate {}\nsudo mkdir -p '{ledger_dir}'\necho '{}' | sudo tee -a '{ledger}' >/dev/null\n",
            h.env_var, h.env_var, h.env_file, h.env_var, h.env_file, h.env_var, h.env_file, h.compose.rsplit_once('/').map(|x| x.0).unwrap_or("."), h.compose, h.container, row("rollback.sh:env_file")
        ),
        _ => format!(
            "set -euo pipefail\nsudo sed -i.rollback-bak \"s|^\\(\\s*image:\\s*\\\"\\?\\){}\\(\\\"\\?\\)|\\1{}\\2|\" '{}'\nif sudo grep -qF '{prev}' '{}'; then\n  echo 'rollback.sh: sed did not rewrite {} — prev-image still present (image: line pattern miss). Aborting before docker recreate.' >&2\n  exit 1\nfi\nsudo docker compose --project-directory '{}' -f '{}' up -d --force-recreate {}\nsudo mkdir -p '{ledger_dir}'\necho '{}' | sudo tee -a '{ledger}' >/dev/null\n",
            esc(prev), esc(target), h.compose, h.compose, h.compose, h.compose.rsplit_once('/').map(|x| x.0).unwrap_or("."), h.compose, h.container, row("rollback.sh")
        ),
    }
}

fn hostname() -> String {
    Command::new("hostname").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_else(|_| "unknown".into())
}

/// Which of the three flip shapes a host map calls for — the script's order of tests.
fn shape_for(h: &HostMap) -> &'static str {
    if !h.compose_files.is_empty() {
        "compose_files"
    } else if !h.env_file.is_empty() && !h.env_var.is_empty() {
        "env_file"
    } else {
        "compose"
    }
}

/// The ref `rollback <component> <n>` would move to, where it was read (the
/// first REACHABLE host's ledger, else the in-repo history), and the newest ref
/// in that same source.
fn pick_target(hosts: &[HostMap], history: &[History], component: &str, n: u32) -> Result<(String, String, String)> {
    for h in hosts {
        let refs = host_refs_newest_first(h, component);
        if refs.is_empty() {
            continue;
        }
        if refs.len() > n as usize {
            return Ok((refs[n as usize].clone(), format!("host-ledger({})", h.host), refs[0].clone()));
        }
        break;
    }
    if history.is_empty() {
        return Err(anyhow!("no history available (host ledger absent + in-repo history empty)"));
    }
    if (n as usize) >= history.len() {
        return Err(anyhow!("history has only {} entries; cannot roll back {n}", history.len()));
    }
    Ok((history[n as usize].image.clone(), "in-repo deploy-history.yml".into(), history[0].image.clone()))
}

/// An image ref's repository: the tag and digest dropped, the registry org
/// normalized as `check-deploy-history.sh` normalizes it. A rollback moves a
/// container between refs of ONE repository; a ledger whose rows name another
/// repository (one ledger fed by two image vars) gives a target that would
/// swap the container's image for a different program.
fn image_repo(r: &str) -> String {
    let r = r.split('@').next().unwrap_or("");
    let r = match r.rfind(':') {
        Some(i) if !r[i..].contains('/') => &r[..i],
        _ => r,
    };
    match r.strip_prefix("ghcr.io/").and_then(|rest| rest.split_once('/')) {
        Some((_, path)) => format!("ghcr.io/<ORG>/{path}"),
        None => r.to_string(),
    }
}

/// The distinct goatnets a component is deployed on, read from each host's
/// `site:` — the host→goatnet fact generate-pins.py resolves and records in the
/// ledger, and the same field `--site` narrows on. Hosts with no `site:` (the
/// group-based relay ledgers, rolled per-goatnet by their own inventory) are not
/// goatnets here; the set is sorted and deduplicated.
fn goatnets_of(hosts: &[HostMap]) -> Vec<String> {
    let mut v: Vec<String> = hosts.iter().map(|h| h.site.clone()).filter(|s| !s.is_empty()).collect();
    v.sort();
    v.dedup();
    v
}

/// Fail-closed (GB-055): a rollback with no `--site` that would flip more than
/// one goatnet is refused, so `rollback <c> <n>` can never flip prod as a side
/// effect of touching dev. A single-goatnet component still rolls back with no
/// `--site`. `hosts` is the un-narrowed set when `site` is `None`; a named site
/// has already scoped the roll, so the check does not apply.
fn require_site_when_multi_goatnet(component: &str, site: Option<&str>, hosts: &[HostMap]) -> Result<()> {
    if site.is_some() {
        return Ok(());
    }
    let nets = goatnets_of(hosts);
    if nets.len() > 1 {
        return Err(goat_errors::coded(
            "GB-055",
            format!(
                "'{component}' is deployed on {} goatnets ({}); a rollback with no --site would flip every one, prod included. Re-run with --site <{}>.",
                nets.len(),
                nets.join(", "),
                nets.join("|"),
            ),
        )
        .into());
    }
    Ok(())
}

/// What a rollback would do, decided without touching a host's state: the artifact
/// type (and whether it refuses), the target ref and where it came from. `plan` saves it.
/// With no `site`, a component that spans more than one goatnet is refused (GB-055)
/// before anything is saved, so a plan can never be written that flips every goatnet.
pub(crate) fn decide(repo: &Path, component: &str, n: u32, site: Option<&str>) -> Result<serde_json::Value> {
    if !(1..=7).contains(&n) {
        return Err(anyhow!("N must be 1..7 (got: {n})"));
    }
    let (hosts, history) = load_reached(repo, component, site)?;
    require_site_when_multi_goatnet(component, site, &hosts)?;
    let m = artifact_meta(&hosts, component);
    let (target, src, _) = pick_target(&hosts, &history, component, n)?;
    Ok(serde_json::json!({
        "artifact_type": m.kind, "reversible": m.reversible, "artifact_source": m.source,
        "target": target, "target_source": src,
        "hosts": hosts.iter().map(|h| h.host.clone()).collect::<Vec<_>>(),
    }))
}

pub fn roll(repo: &Path, component: &str, n: u32, dry_run: bool, site: Option<&str>) -> Result<Report> {
    if !(1..=7).contains(&n) {
        return Err(anyhow!("N must be 1..7 (got: {n})"));
    }
    // Fresh source (C3): the guards below read this checkout's deploy-history.yml
    // (the hosts map, do_not_roll_to), so a checkout behind the trunk is refused
    // before any host is read or touched, as rollback.sh orders it.
    let fresh = if dry_run { None } else { Some(crate::fresh::check(repo)?) };
    if let Some(f) = &fresh {
        if f.refused() {
            let mut r = Report::new("rollback", vec![f.check.clone()], EXIT_REFUSED);
            r.ok = false;
            r.exit = EXIT_REFUSED;
            return Ok(r);
        }
    }
    let (hosts, history) = load_reached(repo, component, site)?;
    require_site_when_multi_goatnet(component, site, &hosts)?;
    let m = artifact_meta(&hosts, component);
    let mut checks: Vec<Check> = fresh.map(|f| f.check).into_iter().collect();
    let refuse = |checks: Vec<Check>, exit: i32| {
        let mut r = Report::new("rollback", checks, exit);
        r.ok = false;
        r.exit = exit;
        Ok(r)
    };
    match m.kind.as_str() {
        "data" => {
            checks.push(line(format!("REFUSING: '{component}' is artifact_type=data ({}). A ref swap cannot undo committed data.", m.source)));
            checks.push(line(format!("  Restore via backup: {}", if m.pointer.is_empty() { "<no restore_pointer recorded; see pgbackrest.md / audit Object-Lock>" } else { &m.pointer })));
            return refuse(checks, EXIT_REFUSE_UNSAFE);
        }
        "cert" => {
            checks.push(line(format!("REFUSING auto-rollback: '{component}' is artifact_type=cert ({}). Certs are re-minted, not reverted (ADR 1044).", m.source)));
            checks.push(line(format!("  Rotate: {}", if m.pointer.is_empty() { "goat admin rotate <anchor|alias>" } else { &m.pointer })));
            return refuse(checks, EXIT_REFUSE_UNSAFE);
        }
        "forward-only" => {
            checks.push(line(format!("REFUSING: '{component}' is artifact_type=forward-only ({}) — not reversible.", m.source)));
            checks.push(line(format!("  {}", if m.pointer.is_empty() { "Forward-fix only; there is no safe rollback target." } else { &m.pointer })));
            return refuse(checks, EXIT_REFUSE_UNSAFE);
        }
        _ => {}
    }
    if !m.reversible {
        checks.push(line(format!("REFUSING: '{component}' latest deploy is flagged reversible:false ({}).", m.source)));
        checks.push(line(format!("  {}", if m.pointer.is_empty() { "Forward-fix only." } else { &m.pointer })));
        return refuse(checks, EXIT_REFUSE_UNSAFE);
    }

    let (target, target_src, current) = pick_target(&hosts, &history, component, n)?;

    if m.kind == "config" || m.kind == "acl" {
        checks.push(line(format!("'{component}' is artifact_type={} ({}) — guided rollback (no auto-flip).", m.kind, m.source)));
        checks.push(line(format!("  Target ref ({target_src}): {}", if target.is_empty() { "<none>" } else { &target })));
        checks.push(line("  Revert in git, then redeploy the role: cd ops/ansible && <env>-refresh.yml -e target=...".into()));
        if m.kind == "acl" {
            checks.push(line("  WARNING (acl): a full router re-render clobbers portal-appended rules — make a SURGICAL edit, not a blind re-render.".into()));
        }
        return refuse(checks, EXIT_GUIDED);
    }

    if !target.is_empty() && !current.is_empty() && image_repo(&target) != image_repo(&current) {
        checks.push(line(format!("REFUSING: the target ref {target} ({target_src}) is another image than the newest one, {current}.")));
        checks.push(line("  The ledger mixes two images; a flip would put the other program in this container. Roll back by hand from the in-repo history.".into()));
        return refuse(checks, EXIT_REFUSE_UNSAFE);
    }

    let known = history.iter().find(|e| e.image == target);
    let target_pr = known.map(|e| e.pr.clone()).filter(|p| !p.is_empty()).unwrap_or_else(|| "?".into());
    if known.map(|e| e.do_not_roll_to).unwrap_or(false) {
        return Err(anyhow!("target ref {target} flagged do_not_roll_to=true in the in-repo ledger; refusing. See PR #{target_pr}."));
    }
    checks.push(line(format!("rolling {component} (artifact_type=image) to [{n}] {target} (target source: {target_src}; PR #{target_pr})")));

    let ts = crate::push::stamp_pub();
    let ts = format!("{}-{}-{}T{}:{}:{}Z", &ts[0..4], &ts[4..6], &ts[6..8], &ts[9..11], &ts[11..13], &ts[13..15]);
    for h in &hosts {
        checks.push(line(format!("  host {}", h.host)));
        let prev = if dry_run {
            "<running image>".to_string()
        } else {
            ssh(h, &format!("sudo docker inspect {} --format '{{{{.Config.Image}}}}'", h.container)).map(|x| x.trim().to_string()).unwrap_or_default()
        };
        if prev.is_empty() {
            checks.push(line(format!("    unreachable or no '{}' container — skipping this host.", h.container)));
            continue;
        }
        checks.push(line(format!("    prev: {prev}")));
        if prev == target {
            checks.push(line("    already on target — no-op.".into()));
            continue;
        }
        if !dry_run {
            let known_on_host = host_refs_newest_first(h, component).iter().any(|r| r == &prev) || history.iter().any(|e| e.image == prev);
            if !known_on_host {
                return Err(anyhow!("host {} is running {prev} which is in NO known history (host ledger + in-repo); refusing — operator must investigate. Use --status.", h.host));
            }
        }
        let shape = shape_for(h);
        if shape == "compose_files" {
            let ff: String = h.compose_files.iter().map(|f| format!(" -f {f}")).collect();
            checks.push(line(format!("    flipping image across compose files [{}] (recreate with{ff})", h.compose_files.join(" "))));
        } else if shape == "env_file" {
            checks.push(line(format!("    flipping {} in {} (compose-time substitution)", h.env_var, h.env_file)));
        }
        let script = flip_script(shape, h, component, &prev, &target, &ts, &target_pr);
        if dry_run {
            checks.push(line(format!("    DRY: ssh {} <<'SH'\n{script}SH", ssh_dest(h))));
        } else {
            let st = Command::new("ssh")
                .args(ssh_args(h, true))
                .arg(&script)
                .status()
                .with_context(|| format!("ssh {}", h.host))?;
            if !st.success() {
                return Err(anyhow!("rollback on {} failed (see above)", h.host));
            }
        }
        checks.push(line(format!("    rolled back: {prev} -> {target}")));
    }
    checks.push(line(format!("done — verify with: goatbuilder rollback {component} --status")));
    let mut r = Report::new("rollback", checks, EXIT_REFUSED);
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

    #[test]
    fn the_in_repo_ledger_loads_for_a_real_component() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let (hosts, history) = load_ledger(&repo(), "goat-get").unwrap();
        assert!(!hosts.is_empty());
        assert!(hosts.iter().all(|h| !h.host.is_empty() && !h.container.is_empty()));
        let _ = history;
    }

    #[test]
    fn an_unadopted_component_is_refused_in_the_scripts_words() {
        let e = load_ledger(&repo(), "no-such-component").unwrap_err().to_string();
        assert!(e.starts_with("no Ansible role at "), "{e}");
    }

    #[test]
    fn n_outside_one_to_seven_is_refused() {
        assert_eq!(roll(&repo(), "goat-get", 0, true, None).unwrap_err().to_string(), "N must be 1..7 (got: 0)");
        assert_eq!(roll(&repo(), "goat-get", 8, true, None).unwrap_err().to_string(), "N must be 1..7 (got: 8)");
    }

    /// The three flip shapes pick as the script picks: compose_files, then env_file, then the compose file.
    #[test]
    fn flip_shape_follows_the_scripts_order() {
        let base = HostMap { host: "h".into(), user: "u".into(), container: "c".into(), compose: "/opt/c/docker-compose.yml".into(), env_file: String::new(), env_var: String::new(), compose_files: vec![], host_shown: "h".into(), container_shown: "c".into(), site: String::new(), door: None };
        assert_eq!(shape_for(&base), "compose");
        let env = HostMap { env_file: "/opt/c/.env".into(), env_var: "C_IMAGE".into(), ..base.clone() };
        assert_eq!(shape_for(&env), "env_file");
        let multi = HostMap { compose_files: vec!["/opt/c/a.yml".into(), "/opt/c/b.yml".into()], ..env.clone() };
        assert_eq!(shape_for(&multi), "compose_files");
        let sc = flip_script("env_file", &env, "c", "img:old", "img:new", "2026-10-28T00:00:00Z", "1");
        assert!(sc.contains("sudo sed -i.rollback-bak \"s|^C_IMAGE=.*|C_IMAGE=img:new|\" '/opt/c/.env'"));
        assert!(sc.contains("\"via\":\"rollback.sh:env_file\""));
        assert!(sc.contains("\"action\":\"rollback\""));
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;

    #[test]
    fn a_site_narrows_the_hosts_and_an_unknown_site_is_refused() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (all, _) = load_ledger(&repo, "goat-get").unwrap();
        let (dev, _) = load_ledger_at(&repo, "goat-get", Some("zagros-dev")).unwrap();
        assert!(all.len() > dev.len() && !dev.is_empty());
        assert!(dev.iter().all(|h| h.site == "zagros-dev"));
        let e = load_ledger_at(&repo, "goat-get", Some("nowhere")).unwrap_err().to_string();
        assert!(e.contains("no hosts entry with site: nowhere"), "{e}");
    }

    fn sited(site: &str) -> HostMap {
        HostMap { site: site.into(), ..host(&[], "", "") }
    }

    #[test]
    fn goatnets_of_are_distinct_sorted_and_drop_the_empty_ones() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let hs = [sited("zagros"), sited("nova-a"), sited("zagros"), host(&[], "", "")];
        assert_eq!(goatnets_of(&hs), vec!["nova-a".to_string(), "zagros".to_string()]);
        // A group-based ledger (every host site-less) spans zero goatnets here.
        assert!(goatnets_of(&[host(&[], "", ""), host(&[], "", "")]).is_empty());
    }

    #[test]
    fn a_single_goatnet_component_rolls_back_with_no_site() {
        // One goatnet, no --site: the guard does not fire.
        require_site_when_multi_goatnet("c", None, &[sited("zagros-dev")]).unwrap();
        // A site-less (group-based) ledger is single-goatnet for the guard too.
        require_site_when_multi_goatnet("c", None, &[host(&[], "", "")]).unwrap();
    }

    #[test]
    fn a_multi_goatnet_rollback_with_no_site_is_refused_naming_the_goatnets() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let e = require_site_when_multi_goatnet("goat-get", None, &[sited("zagros"), sited("zagros-prod"), sited("nova-a")])
            .unwrap_err();
        assert_eq!(e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.as_str()), Some("GB-055"));
        let m = e.to_string();
        assert!(m.contains("deployed on 3 goatnets"), "{m}");
        assert!(m.contains("nova-a, zagros, zagros-prod"), "{m}");
        assert!(m.contains("prod included") && m.contains("--site <nova-a|zagros|zagros-prod>"), "{m}");
    }

    #[test]
    fn a_named_site_bypasses_the_multi_goatnet_guard() {
        // With a site named the roll is already scoped; the guard is a no-op even
        // though the (pre-narrowing) host set spans several goatnets.
        require_site_when_multi_goatnet("goat-get", Some("zagros-prod"), &[sited("zagros"), sited("zagros-prod")]).unwrap();
    }

    #[test]
    fn roll_and_decide_refuse_a_multi_goatnet_component_with_no_site() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        // A goat-get ledger can span several goatnets.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for e in [
            roll(&repo, "goat-get", 1, true, None).unwrap_err(),
            decide(&repo, "goat-get", 1, None).unwrap_err(),
        ] {
            assert_eq!(e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.as_str()), Some("GB-055"), "{e}");
            assert!(e.to_string().contains("a rollback with no --site would flip every one"), "{e}");
        }
    }

    /// A checkout whose goatnet zagros-dev is reached through an ssm door that
    /// lands on 100.127.0.9: the door registry, the real env-door.sh and
    /// generate-door-inventory.py that read it, a door-open.sh that reports the
    /// door open, and a role whose ledger maps one host on that site at
    /// `ledger_host`. No fleet-pins.yaml, so no mesh probe.
    fn door_checkout(tag: &str, door: bool, ledger_host: &str) -> std::path::PathBuf {
        let t = std::env::temp_dir().join(format!("gb-rollback-door-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(t.join("ops/scripts")).unwrap();
        std::fs::create_dir_all(t.join("ops/ansible/inventory")).unwrap();
        std::fs::create_dir_all(t.join("ops/ansible/roles/e2e-canary")).unwrap();
        for f in ["env-door.sh", "generate-door-inventory.py"] {
            std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts").join(f), t.join("ops/scripts").join(f)).unwrap();
        }
        let opener = t.join("ops/scripts/door-open.sh");
        std::fs::write(&opener, "#!/bin/sh\nexit 0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&opener, std::fs::Permissions::from_mode(0o755)).unwrap();
        let reg = if door {
            "doors:\n  zagros-dev:\n    type: ssm\n    instance: i-abc\n    region: zz-test-1\n    port: 12203\n    user: doorkeeper\n    mgmt:\n      - {name: mgmt-zagros-dev-01, mesh_ip: 100.127.0.9, gv_group: zagros_dev_hosts, landed: true}\n"
        } else {
            "doors:\n"
        };
        std::fs::write(t.join("ops/ansible/inventory/doors.yaml"), reg).unwrap();
        std::fs::write(
            t.join("ops/ansible/roles/e2e-canary/deploy-history.yml"),
            format!("hosts:\n  - host: {ledger_host}\n    ssh_user: ops\n    container_name: e2e-canary\n    compose_path: /opt/e2e-canary/docker-compose.yml\n    site: zagros-dev\n  - host: 100.127.0.10\n    ssh_user: ops\n    container_name: e2e-canary\n    compose_path: /opt/e2e-canary/docker-compose.yml\n    site: zagros\nhistory: []\n"),
        )
        .unwrap();
        t
    }

    const JUMP: &str = "ProxyCommand=ssh -o BatchMode=yes -o ConnectTimeout=6 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -p 12203 -W %h:%p doorkeeper@127.0.0.1";

    #[test]
    fn a_site_behind_its_door_is_reached_through_the_door_deploy_uses() {
        let t = door_checkout("door", true, "100.127.0.9");
        // deploy's transport for this goatnet: the door, through the door inventory.
        let deploy = crate::push::transport(&t, "zagros-dev", &[], false).unwrap();
        assert_eq!(deploy.kind, "door", "{deploy:?}");
        assert!(deploy.inventories.iter().any(|i| i == "inventory/doors-groupvars.ini"), "{deploy:?}");
        // rollback --site gives the one host of the site its door; the door
        // rollback falls back to is the one deploy decides, at env-door.sh's port.
        let (hosts, _) = load_reached(&t, "e2e-canary", Some("zagros-dev")).unwrap();
        assert_eq!(hosts.len(), 1);
        let w = hosts[0].door.as_ref().unwrap();
        assert_eq!(resolve_jump(&t, &w.site, &w.members), Some(("doorkeeper".to_string(), 12203)));
        // Through the door: the host's own address, its key checked; only the
        // tunnel hop skips the check.
        let a = jump_args(&hosts[0], "doorkeeper", 12203);
        assert_eq!(a.last().unwrap(), "ops@100.127.0.9", "{a:?}");
        assert!(a.iter().any(|x| x == JUMP) && a.iter().any(|x| x == "StrictHostKeyChecking=accept-new"), "{a:?}");
        assert!(!a.iter().any(|x| x == "UserKnownHostsFile=/dev/null" || x == "StrictHostKeyChecking=no"), "{a:?}");
        // The dry run names the door only once it is in use.
        assert_eq!(ssh_dest(&hosts[0]), "ops@100.127.0.9");
        w.jump.set(Some(("doorkeeper".into(), 12203))).unwrap();
        assert_eq!(ssh_dest(&hosts[0]), "ops@100.127.0.9 (through the door, jump doorkeeper@127.0.0.1:12203)");
        // A host that answers directly is reached directly, door or not.
        w.direct.set(true).unwrap();
        assert_eq!(ssh_args(&hosts[0], false), direct_args(&hosts[0]));
        // With no --site there is no door to try.
        let (hosts, _) = load_reached(&t, "e2e-canary", None).unwrap();
        assert!(hosts.iter().all(|h| h.door.is_none()));
        assert_eq!(ssh_args(&hosts[0], false).last().unwrap(), "ops@100.127.0.9");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn a_host_the_door_does_not_land_on_is_still_the_destination() {
        // The door lands on 100.127.0.9; the ledger's host is 100.127.0.20 (a
        // joiner riding its founder's door, say). The ssh still goes to
        // 100.127.0.20 and its key; the door only carries the TCP connection.
        let t = door_checkout("joiner", true, "100.127.0.20");
        let (hosts, _) = load_reached(&t, "e2e-canary", Some("zagros-dev")).unwrap();
        let w = hosts[0].door.as_ref().unwrap();
        w.direct.set(false).unwrap();
        w.jump.set(Some(("doorkeeper".into(), 12203))).unwrap();
        let a = ssh_args(&hosts[0], false);
        assert_eq!(a.last().unwrap(), "ops@100.127.0.20", "{a:?}");
        assert!(a.iter().any(|x| x == JUMP), "{a:?}");
        assert!(!a.iter().any(|x| x != JUMP && (x.contains("127.0.0.1") || x == "-p")), "the door is never the destination: {a:?}");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn a_site_with_no_door_has_nothing_to_fall_back_to() {
        let t = door_checkout("direct", false, "100.127.0.9");
        assert_eq!(crate::push::transport(&t, "zagros-dev", &[], false).unwrap().kind, "direct");
        let (hosts, _) = load_reached(&t, "e2e-canary", Some("zagros-dev")).unwrap();
        let w = hosts[0].door.as_ref().unwrap();
        assert_eq!(resolve_jump(&t, &w.site, &w.members), None);
        w.direct.set(false).unwrap();
        assert_eq!(ssh_args(&hosts[0], false), direct_args(&hosts[0]));
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn a_door_that_does_not_open_falls_through_to_direct_as_the_script_does() {
        let t = door_checkout("shut", true, "100.127.0.9");
        std::fs::write(t.join("ops/scripts/door-open.sh"), "#!/bin/sh\nexit 1\n").unwrap();
        // No error for any mode: the door is simply not there, and the host is
        // tried directly (unreachable: --list reads the in-repo history and a
        // roll skips the host), as rollback.sh's rssh does.
        let (hosts, _) = load_reached(&t, "e2e-canary", Some("zagros-dev")).unwrap();
        let w = hosts[0].door.as_ref().unwrap();
        assert_eq!(resolve_jump(&t, &w.site, &w.members), None);
        w.direct.set(false).unwrap();
        w.jump.set(None).unwrap();
        assert_eq!(ssh_args(&hosts[0], false), direct_args(&hosts[0]));
        let _ = std::fs::remove_dir_all(&t);
    }

    fn host(compose_files: &[&str], env_file: &str, env_var: &str) -> HostMap {
        HostMap {
            host: "100.127.0.9".into(),
            user: "goat".into(),
            container: "netvis".into(),
            compose: "/opt/netvis/docker-compose.yml".into(),
            env_file: env_file.into(),
            env_var: env_var.into(),
            compose_files: compose_files.iter().map(|s| s.to_string()).collect(),
            host_shown: "100.127.0.9".into(),
            container_shown: "netvis".into(),
            site: String::new(),
            door: None,
        }
    }

    #[test]
    fn a_row_names_its_ref_and_falls_back_to_image() {
        assert_eq!(row_ref(&serde_json::json!({"ref": "r@sha256:1", "image": "i"})).as_deref(), Some("r@sha256:1"));
        assert_eq!(row_ref(&serde_json::json!({"image": "i@sha256:2"})).as_deref(), Some("i@sha256:2"));
        assert_eq!(row_ref(&serde_json::json!({"ts": "x"})), None);
    }

    #[test]
    fn the_flip_shape_follows_the_scripts_order_of_tests() {
        assert_eq!(shape_for(&host(&["/opt/a/compose.yml"], "/opt/a/.env", "IMG")), "compose_files");
        assert_eq!(shape_for(&host(&[], "/opt/a/.env", "IMG")), "env_file");
        assert_eq!(shape_for(&host(&[], "/opt/a/.env", "")), "compose");
        assert_eq!(shape_for(&host(&[], "", "")), "compose");
    }

    #[test]
    fn every_flip_appends_a_rollback_row_and_never_rewrites_history() {
        for h in [host(&["/opt/a/one.yml", "/opt/a/two.yml"], "", ""), host(&[], "/opt/a/.env", "IMG"), host(&[], "", "")] {
            let sh = flip_script(shape_for(&h), &h, "netvis", "ghcr.io/o/netvis:b", "ghcr.io/o/netvis:a", "2026-10-29T00:00:00Z", "42");
            assert!(sh.starts_with("set -euo pipefail"), "{sh}");
            assert!(sh.contains("\"action\":\"rollback\""), "{sh}");
            assert!(sh.contains("\"ref\":\"ghcr.io/o/netvis:a\"") && sh.contains("\"prior_ref\":\"ghcr.io/o/netvis:b\""), "{sh}");
            assert!(sh.contains("| sudo tee -a '/var/lib/goat/netvis/deploy-history.jsonl'"), "appends, never truncates: {sh}");
            assert!(!sh.contains("> '/var/lib/goat"), "a ledger must never be truncated: {sh}");
            assert!(sh.contains("--force-recreate netvis"), "{sh}");
        }
    }

    #[test]
    fn a_compose_flip_aborts_before_recreate_when_the_image_line_did_not_change() {
        let h = host(&[], "", "");
        let sh = flip_script("compose", &h, "netvis", "ghcr.io/o/netvis:b", "ghcr.io/o/netvis:a", "t", "1");
        let abort = sh.find("Aborting before docker recreate").expect("abort guard");
        let recreate = sh.find("--force-recreate").expect("recreate");
        assert!(abort < recreate, "the guard must run before the recreate: {sh}");
        assert!(sh.contains("ghcr.io\\/o\\/netvis:b"), "slashes in a ref are escaped for sed: {sh}");
    }

    #[test]
    fn an_env_file_flip_rewrites_or_appends_the_one_variable() {
        let h = host(&[], "/opt/a/.env", "NETVIS_IMAGE");
        let sh = flip_script("env_file", &h, "netvis", "p", "t", "ts", "1");
        assert!(sh.contains("s|^NETVIS_IMAGE=.*|NETVIS_IMAGE=t|"), "{sh}");
        assert!(sh.contains("echo 'NETVIS_IMAGE=t' | sudo tee -a '/opt/a/.env'"), "{sh}");
        // No `cd`: a compose directory is often root-only (0750), and a failed `cd X &&`
        // does not trip `set -e`, so the recreate was skipped and the row still written.
        assert!(sh.contains("sudo docker compose --project-directory '/opt/netvis' -f '/opt/netvis/docker-compose.yml' up -d --force-recreate netvis"), "{sh}");
        assert!(!sh.contains("cd '"), "{sh}");
        assert!(sh.contains("&& sudo grep -q '^NETVIS_IMAGE='"), "{sh}");
    }

    #[test]
    fn a_compose_files_flip_searches_every_file_for_the_previous_image() {
        let h = host(&["/opt/a/one.yml", "/opt/a/two.yml"], "", "");
        let sh = flip_script("compose_files", &h, "netvis", "p", "t", "ts", "1");
        assert!(sh.contains("for f in /opt/a/one.yml /opt/a/two.yml"), "{sh}");
        assert!(sh.contains(" -f /opt/a/one.yml -f /opt/a/two.yml"), "{sh}");
        assert!(sh.contains("prev image not found in any compose file"), "{sh}");
    }

    #[test]
    fn the_ledger_path_is_per_component() {
        assert_eq!(ledger_path("herd"), "/var/lib/goat/herd/deploy-history.jsonl");
    }

    #[test]
    fn yaml_scalars_read_as_strings() {
        let v: serde_yaml::Value = serde_yaml::from_str("a: x\nb: 7\nc: true\n").unwrap();
        assert_eq!(s(v.get("a")), "x");
        assert_eq!(s(v.get("b")), "7");
        assert_eq!(s(v.get("c")), "true");
        assert_eq!(s(v.get("missing")), "");
    }

    fn role(dir: &Path, name: &str, body: Option<&str>) {
        let r = dir.join("ops/ansible/roles").join(name);
        std::fs::create_dir_all(&r).unwrap();
        if let Some(b) = body {
            std::fs::write(r.join("deploy-history.yml"), b).unwrap();
        }
    }

    #[test]
    fn an_image_repo_drops_tag_digest_and_registry_org() {
        assert_eq!(image_repo("ghcr.io/usareur-af/kratos-selfservice-ui-node-patched:sha-c69025c@sha256:58f6"), "ghcr.io/<ORG>/kratos-selfservice-ui-node-patched");
        assert_eq!(image_repo("ghcr.io/{{ ghcr_org }}/kratos-selfservice-ui-node-patched:sha-1a8cbfc@sha256:953e"), "ghcr.io/<ORG>/kratos-selfservice-ui-node-patched");
        assert_eq!(image_repo("oryd/kratos:v1.3.1"), "oryd/kratos");
        assert_eq!(image_repo("localhost:5000/x:1"), "localhost:5000/x");
        assert_ne!(image_repo("oryd/kratos:v1.3.1"), image_repo("ghcr.io/o/kratos-selfservice-ui-node-patched:a"));
    }

    #[test]
    fn a_ledger_that_mixes_two_images_gives_a_target_of_another_repository() {
        let t = tempfile::tempdir().unwrap();
        role(t.path(), "mixed", Some("service: mixed\nhosts:\n  - host: 192.0.2.1\n    ssh_user: nobody\n    container_name: ui\n    compose_path: /opt/x/docker-compose.yml\nhistory:\n  - image: ghcr.io/o/ui:b\n  - image: oryd/kratos:v1.3.1\n"));
        let (_, history) = load_ledger(t.path(), "mixed").unwrap();
        let (target, src, current) = pick_target(&[], &history, "mixed", 1).unwrap();
        assert_eq!(src, "in-repo deploy-history.yml");
        assert_eq!(current, "ghcr.io/o/ui:b");
        assert_ne!(image_repo(&target), image_repo(&current), "roll() refuses exactly this");
        let (target, _, current) = pick_target(&[], &history, "mixed", 0).unwrap();
        assert_eq!(image_repo(&target), image_repo(&current));
    }

    #[test]
    fn the_in_repo_ledger_loads_hosts_and_history() {
        let t = tempfile::tempdir().unwrap();
        role(t.path(), "netvis", Some("service: netvis\nhosts:\n  - host: 100.127.0.9\n    ssh_user: goat\n    container_name: netvis\n    compose_path: /opt/netvis/docker-compose.yml\nhistory:\n  - image: ghcr.io/o/netvis:a\n    sha: abc\n    pr: 7\n    do_not_roll_to: true\n"));
        let (hosts, history) = load_ledger(t.path(), "netvis").unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].user, "goat");
        assert_eq!(history.len(), 1);
        assert!(history[0].do_not_roll_to);
        assert_eq!(history[0].pr, "7");
    }

    #[test]
    fn a_role_without_a_ledger_or_without_hosts_is_refused_by_name() {
        let t = tempfile::tempdir().unwrap();
        role(t.path(), "bare", None);
        let e = load_ledger(t.path(), "bare").unwrap_err().to_string();
        assert!(e.contains("not yet adopted"), "{e}");
        role(t.path(), "nohosts", Some("service: x\nhosts: []\n"));
        let e = load_ledger(t.path(), "nohosts").unwrap_err().to_string();
        assert!(e.contains("no hosts"), "{e}");
        let e = load_ledger(t.path(), "absent").unwrap_err().to_string();
        assert!(e.contains("no Ansible role"), "{e}");
    }
}
