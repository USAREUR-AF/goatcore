//! The push converger: `deploy` on real hosts, the verb that replaces
//! ops/scripts/deploy-component.sh behind the parity gate
//! (goatbuilder-conformance.md §The parity gate). Every decision the script
//! makes is made here in the same order, from the same files, with the same
//! words, so `deploy --dry-run` and the script's `--dry-run` agree line for line
//! and the CI parity check can say so:
//!
//!   1. the component and the goatnet's member groups from the pins (C1: the
//!      declared ref, never the tree's);
//!   2. the play, through any `import_playbook` alias, and its target mode;
//!   3. every secret the play templates, named before any host is touched;
//!   4. the transport — the mesh when the laptop is on the profile or reaches
//!      the goatnet's mesh host on tcp/22, the door when the registry has one,
//!      the base inventory otherwise;
//!   5. per group: run the play through ansible-preflight.sh, then read the
//!      deploy ledger back from every host and call it done only when the
//!      newest row names the declared digest AND says the deploy finished (C7,
//!      F-530).
//!
//! The Ansible run itself stays `ansible-preflight.sh` under `aws-vault`: that
//! wrapper is the stale-tree guard and the secrets resolver, not the driver.

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use crate::runner::Cmd;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

use crate::doctor::{resolve_target_mode, secrets_needed};
use crate::{Check, Report, EXIT_NOT_CONVERGED, EXIT_REFUSED};

/// What the pins say about one component on one goatnet.
#[derive(Debug, Clone, Serialize)]
pub struct Resolved {
    pub var: String,
    pub ledger: String,
    pub play: PathBuf,
    pub hosts_play: PathBuf,
    pub target_flag: String,
    pub write_role: String,
    pub members: Vec<String>,
    pub skipped: Vec<String>,
}

/// The transport the run will use, decided as the script decides it.
#[derive(Debug, Clone, Serialize)]
pub struct Transport {
    pub kind: String,
    pub detail: String,
    pub inventories: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub component: String,
    pub goatnet: String,
    pub name: String,
    pub resolved: Resolved,
    pub secrets: BTreeMap<String, String>,
    pub secrets_missing: Vec<String>,
    pub transport: Transport,
    pub wants: BTreeMap<String, String>,
}

pub struct Options<'a> {
    pub component: &'a str,
    pub goatnet: &'a str,
    pub only_group: Option<&'a str>,
    pub dry_run: bool,
    pub verify_only: bool,
    /// Who is accepting the rollout (C4, and ADR 1211's ack gate: a `none` or
    /// `restore-required` rollout proceeds only with this set, and it is recorded).
    pub ack: &'a str,
}

fn yaml(path: &Path) -> Result<serde_yaml::Value> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    serde_yaml::from_str(&text).with_context(|| path.display().to_string())
}

fn s(v: Option<&serde_yaml::Value>) -> String {
    v.and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// What the pins alone say about one component on one goatnet: its var, its
/// ledger and the member groups that run it. Reading a ledger needs only this.
#[derive(Debug, Clone)]
pub struct Declared {
    pub var: String,
    pub ledger: String,
    pub members: Vec<String>,
    pub skipped: Vec<String>,
}

/// What looking for a component's deploy playbook found.
pub enum Play {
    Found(std::path::PathBuf),
    /// `deploy_playbook:` names a file that is not there.
    DeclaredMissing,
    /// Nothing declared, and no `<ledger>-deploy.yml` or `<component>-deploy.yml`.
    NotNamed,
}

/// The one lookup: the declared `deploy_playbook:` wins, then the two names.
pub fn find_play(ansible: &Path, declared: &str, ledger: &str, component: &str) -> Play {
    if !declared.is_empty() {
        let p = ansible.join(declared);
        return if p.is_file() { Play::Found(p) } else { Play::DeclaredMissing };
    }
    [format!("{ledger}-deploy.yml"), format!("{component}-deploy.yml")]
        .iter()
        .map(|n| ansible.join(n))
        .find(|p| p.is_file())
        .map_or(Play::NotNamed, Play::Found)
}

/// Step 1: the pins. Same refusals, same words, as the script's inline Python.
pub fn resolve(repo: &Path, component: &str, goatnet: &str, only_group: Option<&str>) -> Result<Resolved> {
    let d = declared(repo, component, goatnet, only_group)?;
    let pins = yaml(&repo.join("ops/pins/fleet-pins.yaml"))?;
    let comp = pins.get("components").and_then(|c| c.get(component)).cloned().unwrap_or_default();
    let ledger = d.ledger.clone();
    let ansible = repo.join("ops/ansible");
    let declared_play = s(comp.get("deploy_playbook"));
    let play = match find_play(&ansible, &declared_play, &ledger, component) {
        Play::Found(p) => p,
        Play::DeclaredMissing => {
            return Err(goat_errors::coded("GB-005", format!("{component} declares deploy_playbook: {declared_play}, which is not in ops/ansible/")).into());
        }
        Play::NotNamed => {
            return Err(anyhow!(
                "no deploy playbook for {component} (looked for {ledger}-deploy.yml, {component}-deploy.yml; declare deploy_playbook in fleet-pins.yaml if it is named otherwise)"
            ));
        }
    };
    let mode = resolve_target_mode(&ansible, &play)?;
    let target_flag = mode.mode.trim_end_matches("<group>").to_string();
    Ok(Resolved {
        var: d.var,
        ledger,
        play,
        hosts_play: mode.play,
        target_flag,
        write_role: s(comp.get("single_homed_role")),
        members: d.members,
        skipped: d.skipped,
    })
}

/// The pins half of [`resolve`]: var, ledger and members, without the play.
pub fn declared(repo: &Path, component: &str, goatnet: &str, only_group: Option<&str>) -> Result<Declared> {
    // The merged table: a lite goatnet is declared in the sidecar, never in fleet-pins.yaml.
    let table = crate::pins::load_table(&repo.join("ops/pins/fleet-pins.yaml"))?;
    let pins = serde_yaml::to_value(serde_json::json!({"goatnets": table.goatnets, "components": table.components}))?;
    let comp = pins
        .get("components")
        .and_then(|c| c.get(component))
        .ok_or_else(|| goat_errors::coded("GB-001", format!("unknown component {component:?}")))?;
    let g = pins.get("goatnets").and_then(|g| g.get(goatnet)).ok_or_else(|| {
        let mut known: Vec<String> = pins
            .get("goatnets")
            .and_then(|g| g.as_mapping())
            .map(|m| m.keys().filter_map(|k| k.as_str().map(String::from)).collect())
            .unwrap_or_default();
        known.sort();
        goat_errors::coded("GB-002", format!("unknown goatnet {goatnet:?}; known: {known:?}"))
    })?;
    let deviations = comp.get("deviations").and_then(|d| d.as_mapping());
    let is_absent = |scope: &str| -> bool {
        deviations
            .and_then(|d| d.get(serde_yaml::Value::String(scope.to_string())))
            .and_then(|v| v.get("absent"))
            .and_then(|a| a.as_bool())
            .unwrap_or(false)
    };
    let declared: Vec<String> = g
        .get("members")
        .and_then(|m| m.as_sequence())
        .map(|seq| seq.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let (members, skipped) = match only_group {
        Some(req) => {
            if !declared.iter().any(|m| m == req) {
                return Err(goat_errors::coded("GB-003", format!("--group {req:?} is not a member of goatnet {goatnet:?}")).into());
            }
            // The script takes the FIRST deviation scope that exists, member before goatnet.
            let scope = [req, goatnet].into_iter().find(|sc| {
                deviations.map(|d| d.contains_key(serde_yaml::Value::String(sc.to_string()))).unwrap_or(false)
            });
            if scope.map(is_absent).unwrap_or(false) {
                return Err(goat_errors::coded("GB-003", format!("--group {req:?} does not run component {component:?} (absent: true)")).into());
            }
            (vec![req.to_string()], vec![])
        }
        None => {
            let members = declared.iter().filter(|m| !is_absent(m)).cloned().collect();
            let skipped = declared.iter().filter(|m| is_absent(m)).cloned().collect();
            (members, skipped)
        }
    };
    let var = s(comp.get("var"));
    if var.is_empty() {
        return Err(goat_errors::coded("GB-004", format!("component {component:?} declares no var in fleet-pins.yaml")).into());
    }
    let ledger = {
        let l = s(comp.get("ledger_component"));
        if l.is_empty() { component.to_string() } else { l }
    };
    Ok(Declared { var, ledger, members, skipped })
}

/// The mesh probe a deploy starts: what a dry run prints, and what runs.
fn netbird_status_cmd() -> Cmd {
    let mut c = Cmd::new("netbird");
    c.arg("status");
    c
}

fn netbird_status() -> (String, String) {
    let out = netbird_status_cmd().output();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let field = |key: &str| {
        text.lines()
            .find(|l| l.starts_with(key))
            .map(|l| l[key.len()..].trim().to_string())
            .unwrap_or_default()
    };
    (field("Profile:"), field("Management:"))
}

/// The first `ansible_host=` of the first member group in the mesh inventory.
fn mesh_host(repo: &Path, members: &[String]) -> Option<String> {
    let text = std::fs::read_to_string(repo.join("ops/ansible/inventory/mgmt-hosts-mesh.ini")).ok()?;
    for g in members {
        let header = format!("[{g}]");
        let mut inside = false;
        for line in text.lines() {
            let t = line.trim();
            if t == header {
                inside = true;
                continue;
            }
            if inside && t.starts_with('[') {
                break;
            }
            if inside {
                if let Some(i) = t.find("ansible_host=") {
                    let v: String = t[i + "ansible_host=".len()..].chars().take_while(|c| !c.is_whitespace()).collect();
                    if !v.is_empty() {
                        return Some(v);
                    }
                }
            }
        }
    }
    None
}

fn reachable(host: &str, port: u16) -> bool {
    let addr: Option<SocketAddr> = format!("{host}:{port}").to_socket_addrs().ok().and_then(|mut a| a.next());
    addr.map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(3)).is_ok()).unwrap_or(false)
}

fn has_door(repo: &Path, name: &str) -> bool {
    yaml(&repo.join("ops/ansible/inventory/doors.yaml"))
        .ok()
        .and_then(|d| d.get("doors").and_then(|x| x.get(name)).and_then(|x| x.get("type")).and_then(|t| t.as_str()).map(|t| t == "ssm"))
        .unwrap_or(false)
}

/// Step 4: the transport, in the script's order and words.
pub fn transport(repo: &Path, name: &str, members: &[String], dry_run: bool) -> Result<Transport> {
    let (profile, mgmt) = netbird_status();
    let mesh = mesh_host(repo, members);
    let base = "inventory/mgmt-hosts.ini".to_string();
    if profile == name && mgmt == "Connected" {
        return Ok(Transport {
            kind: "mesh".into(),
            detail: format!("mesh (this laptop is on the {name} profile)"),
            inventories: vec![base, "inventory/mgmt-hosts-mesh.ini".into()],
        });
    }
    // GOAT_MESH_PROBE=0 skips the three-second reach test (CI has no mesh; the
    // parity gate runs both sides without it). The script honours the same variable.
    let probe = std::env::var("GOAT_MESH_PROBE").map(|v| v != "0").unwrap_or(true);
    if let (Some(h), true) = (&mesh, probe) {
        if reachable(h, 22) {
            return Ok(Transport {
                kind: "mesh".into(),
                detail: format!("mesh (this laptop reaches {h}:22; netbird profile reads '{profile}')"),
                inventories: vec![base, "inventory/mgmt-hosts-mesh.ini".into()],
            });
        }
    }
    if has_door(repo, name) {
        let opener = repo.join("ops/scripts/door-open.sh");
        // GOAT_DOOR_OPEN=0 decides the door without opening it: CI's parity gates
        // pin the path where nothing is reachable, and an SSM door there only times out.
        let open = std::env::var("GOAT_DOOR_OPEN").map(|v| v != "0").unwrap_or(true);
        if !dry_run && open {
            let st = Cmd::new(&opener).arg(name).output().map(|o| o.status.success()).unwrap_or(false);
            if !st {
                return Err(goat_errors::coded("GB-006", format!("door to {name} did not open")).into());
            }
        }
        return Ok(Transport {
            kind: "door".into(),
            detail: format!("door ({} {name})", opener.display()),
            inventories: vec![base, "inventory/doors-groupvars.ini".into()],
        });
    }
    Ok(Transport { kind: "direct".into(), detail: "direct (mgmt-hosts.ini)".into(), inventories: vec![base] })
}

pub(crate) fn want(repo: &Path, group: &str, var: &str) -> Option<String> {
    let text = std::fs::read_to_string(repo.join("ops/ansible/group_vars").join(group).join("pins.yml")).ok()?;
    let line = text.lines().find(|l| l.starts_with(&format!("{var}:")))?;
    let v = line[var.len() + 1..].trim().trim_matches('"').trim().to_string();
    if v.is_empty() { None } else { Some(v) }
}

/// Steps 1–4 without touching a host: what a run would do.
pub fn plan(repo: &Path, o: &Options) -> Result<Plan> {
    let resolved = resolve(repo, o.component, o.goatnet, o.only_group)?;
    let name = o.goatnet.replace('_', "-");
    let secrets = secrets_needed(repo, &resolved.members, o.component, &resolved.ledger, &resolved.hosts_play);
    // Presence is the declared posture's (ADR 1204): under a value store the
    // values are in its file, not yet in this shell.
    let have = crate::secrets::available(repo);
    let secrets_missing: Vec<String> = secrets.values().filter(|e| !crate::doctor::satisfied(&have, e)).cloned().collect();
    let transport = transport(repo, &name, &resolved.members, o.dry_run)?;
    let mut wants = BTreeMap::new();
    for g in &resolved.members {
        if let Some(w) = want(repo, g, &resolved.var) {
            wants.insert(g.clone(), w);
        }
    }
    Ok(Plan { component: o.component.into(), goatnet: o.goatnet.into(), name, resolved, secrets, secrets_missing, transport, wants })
}

fn ok(name: &str, detail: String) -> Check {
    Check { name: name.to_string(), ok: true, detail, fix: None }
}
fn not_done(name: &str, detail: String) -> Check {
    Check { name: name.to_string(), ok: false, detail, fix: None }
}
fn refused(name: &str, detail: String, fix: &str) -> Check {
    Check { name: name.to_string(), ok: false, detail, fix: Some(fix.to_string()) }
}

pub(crate) fn render_ref(r: &str) -> String {
    match crate::settings::ghcr_org_global() {
        Some(org) => r.replace("{{ ghcr_org }}", &org),
        None => r.to_string(),
    }
}

/// The deploy play's arguments for group `g`: the inventories, the play, the target as two words, and the accepted risk (ADR 1211).
fn deploy_args(inv: &[String], play: &Path, target_flag: &str, g: &str, accept: Option<&str>) -> Vec<String> {
    let mut args = inv.to_vec();
    args.push(play.file_name().unwrap_or_default().to_string_lossy().to_string());
    // The flag and its value, two words: `-e`, `<var>=<group>` or `-l`, `<group>`.
    match target_flag.split_once(' ') {
        Some((flag, value)) => {
            args.push(flag.to_string());
            args.push(format!("{value}{g}"));
        }
        None => args.push(format!("{target_flag}{g}")),
    }
    if let Some(accept) = accept {
        args.push("-e".into());
        args.push(format!("{{\"rollback_preflight_accept\": {accept}}}"));
    }
    args
}

/// Run `ansible-preflight.sh` (or `ansible`/`ansible-inventory`) from ops/ansible under ops/scripts/with-aws, as the script does.
pub(crate) fn ansible_cmd(repo: &Path, tool: &str, args: &[String]) -> Cmd {
    // AWS credentials through ops/scripts/with-aws (ADR 1204): aws-vault where the
    // team uses it, a plain AWS profile or none where it does not.
    let mut c = Cmd::new("../scripts/with-aws");
    c.current_dir(repo.join("ops/ansible"));
    c.env("OBJC_DISABLE_INITIALIZE_FORK_SAFETY", "YES");
    c.arg(crate::settings::aws_profile(repo)).arg("--");
    if tool == "preflight" {
        c.arg("../scripts/ansible-preflight.sh");
    } else {
        c.arg(tool);
    }
    c.args(args);
    // `-v` logs the line when it runs: the runner traces every command.
    c
}

pub(crate) fn inventory_args(t: &Transport) -> Vec<String> {
    t.inventories.iter().flat_map(|i| ["-i".to_string(), i.clone()]).collect()
}

/// The hosts of a group that are NOT the single-homed writer (ADR 1084 §D3, F-547).
pub(crate) fn skip_hosts(repo: &Path, t: &Transport, group: &str, write_role: &str) -> Vec<String> {
    if write_role.is_empty() {
        return vec![];
    }
    let mut args = inventory_args(t);
    args.push("--list".into());
    let out = ansible_cmd(repo, "ansible-inventory", &args).output();
    let Ok(out) = out else { return vec![] };
    let Ok(inv) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return vec![] };
    let hosts: Vec<String> = inv
        .get(group)
        .and_then(|g| g.get("hosts"))
        .and_then(|h| h.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let meta = inv.get("_meta").and_then(|m| m.get("hostvars")).cloned().unwrap_or_default();
    hosts
        .into_iter()
        .filter(|h| {
            meta.get(h).and_then(|v| v.get("router_role")).and_then(|r| r.as_str()).unwrap_or("A").to_uppercase() != write_role.to_uppercase()
        })
        .collect()
}

/// The hosts of a group (and its child groups, as an ansible pattern targets them) that
/// must answer a ledger read, and the single-homed non-writers that need not
/// (ADR 1084 §D3). Unlike [`skip_hosts`], a failed or unreadable inventory is an error:
/// verification must not read "nothing to check" as "converged" (#1545).
pub(crate) fn expected_hosts(repo: &Path, t: &Transport, group: &str, write_role: &str) -> std::result::Result<(Vec<String>, Vec<String>), String> {
    let mut args = inventory_args(t);
    args.push("--list".into());
    let out = ansible_cmd(repo, "ansible-inventory", &args).output().map_err(|e| format!("cannot run ansible-inventory: {e}"))?;
    if !out.status.success() {
        return Err(format!("ansible-inventory exited {}", out.status.code().map_or("by signal".into(), |c| c.to_string())));
    }
    let inv: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|_| "ansible-inventory output is not JSON".to_string())?;
    Ok(split_expected(&inv, group, write_role))
}

/// [`expected_hosts`] on an inventory already read: (expected, skipped).
fn split_expected(inv: &serde_json::Value, group: &str, write_role: &str) -> (Vec<String>, Vec<String>) {
    let strs = |v: Option<&serde_json::Value>| -> Vec<String> {
        v.and_then(|h| h.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
    };
    let (mut hosts, mut seen, mut todo) = (Vec::<String>::new(), std::collections::HashSet::new(), vec![group.to_string()]);
    while let Some(g) = todo.pop() {
        if !seen.insert(g.clone()) {
            continue;
        }
        let entry = inv.get(&g);
        for h in strs(entry.and_then(|e| e.get("hosts"))) {
            if !hosts.contains(&h) {
                hosts.push(h);
            }
        }
        todo.extend(strs(entry.and_then(|e| e.get("children"))));
    }
    let meta = inv.get("_meta").and_then(|m| m.get("hostvars"));
    let writer = write_role.to_uppercase();
    hosts.into_iter().partition(|h| {
        writer.is_empty()
            || meta.and_then(|m| m.get(h)).and_then(|v| v.get("router_role")).and_then(|r| r.as_str()).unwrap_or("A").to_uppercase() == writer
    })
}

/// `deploy` on real hosts. Exit 0 when every host's ledger names the declared
/// digest with `action: deploy`; 3 otherwise; 1 when refused before any host.
pub fn deploy(repo: &Path, o: &Options) -> Result<Report> {
    let mut checks = Vec::new();
    let name = o.goatnet.replace('_', "-");
    // The runner frames and records each step's commands under it
    // (crates/goatbuilder/src/runner.rs); the names are the five steps, the
    // last two once per group.
    crate::runner::phase("prep", "the rollout facts and the ack gate", "");

    // Prep phase (ADR 1211): the reversibility readout, first, before anything
    // is cut — the three facts, then the ack gate. A `none` or
    // `restore-required` rollout is not cleanly reversible, so it proceeds only
    // with an explicit --ack, whose acceptance is recorded in the ledger row. It
    // does NOT hard-block: a stateful cutover stays possible, knowingly.
    let accept_var: Option<String> = {
        let facts = crate::preflight::declaration(repo, o.component).ok().map(|d| crate::preflight::grade_facts(&d));
        match &facts {
            Some(f) => {
                // The readout is informational here (ok rows): the prep phase prints
                // the three facts, it does not flip the deploy's convergence verdict.
                // The ack gate below is what enforces; `goatbuilder preflight` is where
                // an undeclared fact grades not-clean.
                checks.push(ok(&format!("{name}/preflight/state"), format!("State: {}", f.state)));
                checks.push(ok(&format!("{name}/preflight/rollback"), format!("Rollback: {}", f.rollback)));
                checks.push(ok(&format!("{name}/preflight/scope"), format!("Scope: {}", f.scope)));
                checks.push(ok(&format!("{name}/preflight"), format!("prep: {}", crate::preflight::readout_line(f))));
            }
            None => checks.push(ok(&format!("{name}/preflight"), "prep: rollout facts unavailable from the pins (the resolve below refuses an unknown component)".into())),
        }
        let mut accept = None;
        if !o.dry_run && !o.verify_only {
            if let Some(f) = &facts {
                if let Some(kind) = crate::preflight::needs_ack(f) {
                    if o.ack.trim().is_empty() {
                        checks.push(refused(
                            &format!("{name}/preflight/ack"),
                            format!("rollback: {kind} — this rollout is not cleanly reversible (undo is not a pin flip)"),
                            "re-run with --ack \"<your name>\": the readout and the accepted risk are written into the deploy ledger row",
                        ));
                        return Ok(Report::new("deploy", checks, EXIT_REFUSED));
                    }
                    accept = Some(crate::preflight::accepted_risk_json(f, o.ack));
                }
            }
        }
        accept
    };

    // Fresh source (C3): once, before the secrets gate and the transport (a door
    // opens there), as the script orders it; every play of this run is then
    // judged against the same trunk commit (ops/scripts/fresh-source.sh).
    let mut trunk = String::new();
    if !o.dry_run && !o.verify_only {
        resolve(repo, o.component, o.goatnet, o.only_group)?;
        crate::runner::phase("fresh-source", "the checkout is current with the trunk", "");
        let f = crate::fresh::check(repo)?;
        let refused = f.refused();
        trunk = f.trunk.clone();
        checks.push(f.check);
        if refused {
            return Ok(Report::new("deploy", checks, EXIT_REFUSED));
        }
    }
    // A dry run names the probes a real run starts before its plays, built by
    // the code that starts them; their verdicts are known only when they run.
    if o.dry_run {
        checks.push(ok("probe", crate::fresh::command(repo).line()));
        checks.push(ok("probe", crate::secrets::posture_cmd(repo, &["available"]).line()));
        checks.push(ok("probe", netbird_status_cmd().line()));
    }
    crate::runner::phase("transport", "the secrets, and how the hosts are reached", "");
    let p = plan(repo, o)?;
    let r = &p.resolved;
    if !r.skipped.is_empty() {
        checks.push(ok(&format!("{}/skipped", p.name), format!("absent: true, topology not skew: {}", r.skipped.join(" "))));
    }
    if !p.secrets_missing.is_empty() {
        let detail = format!("{} on {} templates {} variable(s) from your environment; NOT SET: {}", p.component, p.name, p.secrets.len(), p.secrets_missing.join(" "));
        let fix_text = crate::secrets::hint(repo, &p.secrets_missing);
        let fix = fix_text.as_str();
        if o.dry_run {
            // A dry run notes it and continues, as the script does; a real run refuses before any host.
            checks.push(ok(&format!("{}/{}/secrets", p.name, p.component), format!("{detail}; {fix}")));
        } else if o.verify_only {
            // Reading the ledgers back templates nothing: the secrets gate belongs to the play.
            checks.push(ok(&format!("{}/{}/secrets", p.name, p.component), format!("{detail}; not needed for --verify-only (no play runs)")));
        } else {
            checks.push(refused(&format!("{}/{}/secrets", p.name, p.component), detail, fix));
            return Ok(Report::new("deploy", checks, EXIT_REFUSED));
        }
    }
    checks.push(ok(&format!("{}/transport", p.name), format!("groups [{}] via {}", r.members.join(" "), p.transport.detail)));
    checks.push(ok(
        &format!("{}/playbook", p.name),
        format!("{} {}<group>; ledger component {}", r.play.file_name().unwrap_or_default().to_string_lossy(), r.target_flag, r.ledger),
    ));
    let logdir = std::env::var("GOAT_DEPLOY_LOG_DIR").or_else(|_| std::env::var("TMPDIR")).unwrap_or_else(|_| "/tmp".into());
    let ts = stamp();
    let inv = inventory_args(&p.transport);
    let mut rc_not_converged = false;
    let mut verified = 0usize;
    for g in &r.members {
        let Some(w) = p.wants.get(g) else {
            checks.push(ok(&format!("{}/{g}", p.name), format!("no {} in group_vars/{g}/pins.yml — nothing declared, skipping", r.var)));
            continue;
        };
        let digest = w.rsplit('@').next().unwrap_or(w).to_string();
        let log = format!("{logdir}/goat-deploy-{}-{g}-{ts}.log", p.component);
        // The ledger rows as they stand before the play: the only reliable way to say
        // afterwards whether this run wrote a row or found one (a row's own ts is the
        // play's fact-gathering time, which a fact cache can make stale).
        crate::runner::phase(&format!("deploy:{g}"), &format!("the play on {g}"), "");
        let before = if o.dry_run || o.verify_only { String::new() } else { read_tails(repo, &inv, g, &r.ledger)? };
        if !o.verify_only {
            let args = deploy_args(&inv, &r.play, &r.target_flag, g, accept_var.as_deref());
            if o.dry_run {
                checks.push(ok("probe", ledger_read_cmd(repo, &inv, g, &r.ledger).line()));
                // The play's line, built from the argv a real run starts.
                checks.push(ok(&format!("{}/{g}/deploy", p.name), format!("DRY: deploying {}: (cd ops/ansible && {}) > {log}", render_ref(w), ansible_cmd(repo, "preflight", &args).line())));
            } else {
                checks.push(ok(&format!("{}/{g}/deploy", p.name), format!("deploying {}", render_ref(w))));
                let mut cmd = ansible_cmd(repo, "preflight", &args);
                if !trunk.is_empty() {
                    cmd.env(crate::fresh::TRUNK_ENV, &trunk);
                }
                let out = cmd.output().with_context(|| "ansible-preflight.sh")?;
                let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
                let _ = std::fs::write(&log, &text);
                let recap: Vec<&str> = text.lines().skip_while(|l| !l.contains("PLAY RECAP")).skip(1).take(8).filter(|l| l.contains("ok=")).collect();
                if recap.is_empty() && text.contains("skipping: no hosts matched") {
                    checks.push(ok(&format!("{}/{g}/hosts", p.name), "no host of this group in the loaded inventories (declared, not built?) — nothing deployed, skipping".into()));
                    continue;
                }
                // A nonzero exit is a failed deploy even when a clean recap printed (#1545).
                let bad = !out.status.success() || recap.is_empty() || recap.iter().any(|l| recap_count(l, "unreachable") > 0 || recap_count(l, "failed") > 0);
                if bad {
                    // fatal: lines from the play, and the wrapper's own refusal (a stale
                    // checkout is refused by ansible-preflight before any play runs).
                    let fatal: Vec<String> = text.lines().filter(|l| l.contains("fatal:") || l.contains("REFUSE")).take(3).map(|l| l.chars().take(200).collect()).collect();
                    checks.push(not_done(&format!("{}/{g}/deploy", p.name), format!("deploy FAILED — log {log}; {}", fatal.join(" | "))));
                    rc_not_converged = true;
                    continue;
                }
                for l in recap {
                    checks.push(ok(&format!("{}/{g}/recap", p.name), l.trim().to_string()));
                }
            }
        }
        crate::runner::phase(&format!("verify:{g}"), &format!("every host of {g} names the declared ref"), "");
        // Served payloads (goat-get, goat-docs): what the surface serves is versioned by
        // role vars, not by its image; the same module as the script judges them.
        let served_note = match served_declared(repo, &r.ledger, g) {
            Ok(w) if w.is_empty() => String::new(),
            Ok(w) => format!(" and served payloads {w}"),
            Err(e) => format!(" and served payloads UNRESOLVED ({e})"),
        };
        if o.dry_run {
            checks.push(ok(&format!("{}/{g}/verify", p.name), format!("DRY: would read /var/lib/goat/{}/deploy-history.jsonl on {g} and require ref digest {digest}{served_note}", r.ledger)));
            continue;
        }
        // The inventory says which hosts MUST answer: a failed read, or a host that
        // never returns a row, is not convergence (#1545).
        let (expected, skip) = match expected_hosts(repo, &p.transport, g, &r.write_role) {
            Ok(x) => x,
            Err(e) => {
                checks.push(not_done(&format!("{}/{g}/inventory", p.name), format!("inventory read FAILED — {e}")));
                rc_not_converged = true;
                continue;
            }
        };
        if !skip.is_empty() {
            checks.push(ok(&format!("{}/{g}/single-homed", p.name), format!("single-homed on router_role={} — not expected on: {}", r.write_role, skip.join(" "))));
        }
        if expected.is_empty() {
            checks.push(ok(&format!("{}/{g}/hosts", p.name), format!("no host of this group is expected to carry {} in the loaded inventories — nothing to verify", r.ledger)));
            continue;
        }
        let group_failed_before = rc_not_converged;
        rc_not_converged = false;
        let (rows, ledger_status) = read_tails_status(repo, &inv, g, &r.ledger)?;
        let mut answered = std::collections::HashSet::new();
        for line in rows.lines().filter(|l| !l.trim().is_empty()) {
            let host = line.split_whitespace().next().unwrap_or("").to_string();
            if skip.contains(&host) {
                checks.push(ok(&format!("{}/{host}", p.name), format!("skipped — single-homed on router_role={} (ADR 1084 §D3)", r.write_role)));
                continue;
            }
            answered.insert(host.clone());
            if line.contains("| UNREACHABLE!") || line.contains("| FAILED!") {
                // Never "not converged": a host that cannot be read is unknown (C7). A read
                // that FAILED (become refused, no python) read no ledger either.
                let why = if line.contains("| FAILED!") { "the ledger read FAILED on the host" } else { "host unreachable" };
                checks.push(not_done(&format!("{}/{host}", p.name), format!("unknown — {why}; its ledger was not read")));
                rc_not_converged = true;
                continue;
            }
            let tail: String = line.split_once(">>").map(|x| x.1).unwrap_or(line).trim().chars().take(140).collect();
            let names_digest = line.contains(&digest);
            let finished = line.contains("\"action\": \"deploy\"") || line.contains("\"action\":\"deploy\"");
            if names_digest && finished {
                // Say whether this run wrote the row or found it: a no-change converge is a
                // true verdict (C7), but the reader must see that nothing moved.
                let how = row_provenance(&before, &host, line, o.verify_only);
                match served_check(repo, &r.ledger, g, line) {
                    (true, note) => {
                        checks.push(ok(&format!("{}/{host}", p.name), format!("converged on {digest} — {how}")));
                        if !note.is_empty() {
                            checks.push(ok(&format!("{}/{host}/served", p.name), note));
                        }
                    }
                    (false, note) => {
                        checks.push(not_done(&format!("{}/{host}", p.name), format!("NOT converged — image on {digest}, but {note}")));
                        rc_not_converged = true;
                    }
                }
            } else if names_digest {
                checks.push(not_done(&format!("{}/{host}", p.name), not_deploy_reason(line, &digest, &tail)));
                rc_not_converged = true;
            } else {
                checks.push(not_done(&format!("{}/{host}", p.name), format!("NOT converged — ledger tail: {tail}")));
                rc_not_converged = true;
            }
        }
        if let Some(code) = ledger_status {
            checks.push(not_done(&format!("{}/{g}/ledger", p.name), format!("ledger read FAILED (exit {code})")));
            rc_not_converged = true;
        }
        // Every expected host must answer; silence is not proof.
        for h in expected.iter().filter(|h| !answered.contains(*h)) {
            checks.push(not_done(&format!("{}/{h}", p.name), "NOT converged — no ledger row came back from this host".into()));
            rc_not_converged = true;
        }
        if !rc_not_converged {
            verified += 1;
        }
        rc_not_converged |= group_failed_before;
    }
    if !o.dry_run && !rc_not_converged && verified == 0 {
        checks.push(not_done(&format!("{}/{}/verified", p.name, p.component), format!("nothing was verified — no group had a host expected to carry {}", r.ledger)));
        rc_not_converged = true;
    }
    // A dry run reads no ledger, so it must not say every ledger agrees
    // (docs/design/goatbuilder-output.md, rule 2).
    let verdict = if o.dry_run {
        format!("{} on {}: DRY: nothing was read; a real run reads every host's ledger", p.component, p.goatnet)
    } else if rc_not_converged {
        format!("{} on {}: NOT converged everywhere (see above)", p.component, p.goatnet)
    } else {
        format!("{} on {}: every host's ledger names the declared ref", p.component, p.goatnet)
    };
    if rc_not_converged {
        checks.push(not_done(&format!("{}/{}", p.name, p.component), verdict));
    } else {
        checks.push(ok(&format!("{}/{}", p.name, p.component), verdict));
    }
    Ok(Report::new("deploy", checks, EXIT_NOT_CONVERGED))
}

/// The payloads a serving surface (goat-get, goat-docs) should serve to one member group,
/// as main declares them: `ops/scripts/served-payloads.py declared`, the module the script
/// calls too. Empty when the component serves nothing but its image.
pub(crate) fn served_declared(repo: &Path, ledger: &str, group: &str) -> std::result::Result<String, String> {
    let out = Cmd::new(crate::endstate::python())
        .arg(repo.join("ops/scripts/served-payloads.py"))
        .args(["declared", "--repo"])
        .arg(repo)
        .args(["--ledger", ledger, "--group", group])
        .output()
        .map_err(|e| format!("cannot run python3: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if text.is_empty() { err } else { text });
    }
    Ok(if text == "{}" { String::new() } else { text })
}

/// One host's newest row against the declared served payloads: (as declared, the line to print).
/// `served-payloads.py check` decides, so the verdict and its words are the script's.
pub(crate) fn served_check(repo: &Path, ledger: &str, group: &str, line: &str) -> (bool, String) {
    use std::io::Write;
    let child = Cmd::new(crate::endstate::python())
        .arg(repo.join("ops/scripts/served-payloads.py"))
        .args(["check", "--repo"])
        .arg(repo)
        .args(["--ledger", ledger, "--group", group])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let Ok(mut child) = child else { return (false, "served payloads: cannot run python3".into()) };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(line.as_bytes());
    }
    match child.wait_with_output() {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if text.is_empty() {
                text = String::from_utf8_lossy(&o.stderr).trim().to_string();
            }
            (o.status.success(), text)
        }
        Err(e) => (false, format!("served payloads: {e}")),
    }
}

/// The count a PLAY RECAP line gives for `field` (`failed`, `unreachable`, …); 0 when the
/// field is absent or unparseable.
fn recap_count(line: &str, field: &str) -> u32 {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(field)?.strip_prefix('=')?.parse().ok())
        .unwrap_or(0)
}

/// The ad-hoc `ansible` words that read the newest ledger row of every host of a group.
pub(crate) fn ledger_read_args(inv: &[String], group: &str, ledger: &str) -> Vec<String> {
    let mut args = inv.to_vec();
    args.push(group.to_string());
    args.extend(["-m".into(), "shell".into(), "-a".into(), format!("tail -n 1 /var/lib/goat/{ledger}/deploy-history.jsonl 2>/dev/null || echo NO_LEDGER"), "-o".into()]);
    args
}

/// The ledger read a deploy starts on `group`: what a dry run prints, and what runs.
fn ledger_read_cmd(repo: &Path, inv: &[String], group: &str, ledger: &str) -> Cmd {
    ansible_cmd(repo, "ansible", &ledger_read_args(inv, group, ledger))
}

/// The newest ledger row per host of a group, one `host | … | (stdout) <row>` line each
/// (ansible `-o`); `NO_LEDGER` where the file is absent.
fn read_tails(repo: &Path, inv: &[String], group: &str, ledger: &str) -> Result<String> {
    let out = ledger_read_cmd(repo, inv, group, ledger).output().with_context(|| "ansible")?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// [`read_tails`], plus the ledger read's failure: `Some(exit code)` when ansible did
/// not exit 0 (an unreachable or failed host), `None` when it did.
fn read_tails_status(repo: &Path, inv: &[String], group: &str, ledger: &str) -> Result<(String, Option<String>)> {
    let out = ledger_read_cmd(repo, inv, group, ledger).output().with_context(|| "ansible")?;
    let failed = (!out.status.success()).then(|| out.status.code().map_or("by signal".into(), |c| c.to_string()));
    Ok((String::from_utf8_lossy(&out.stdout).to_string(), failed))
}

/// Whether a converged row was written by this run or found already there, decided by
/// comparing the host's row before the play with the row after — never by the row's own
/// `ts`, which is the play's fact-gathering time and can be the fact cache's.
fn row_provenance(before: &str, host: &str, line: &str, verify_only: bool) -> String {
    let mut row_ts = ledger_row_ts(line).unwrap_or_else(|| "no ts".into());
    if let Some(p) = ledger_row_field(line, "promoted_at") {
        row_ts = format!("{row_ts}, finished {p}");
    }
    if verify_only {
        return format!("ledger row ts {row_ts}");
    }
    let prior = before.lines().find(|l| l.split_whitespace().next() == Some(host));
    if prior == Some(line) {
        format!("already at the declared ref (row ts {row_ts}); this run wrote no new ledger row")
    } else {
        format!("ledger row written by this run (row ts {row_ts})")
    }
}

#[cfg(test)]
mod expected_hosts_tests {
    use super::*;

    #[test]
    fn a_group_and_its_children_must_all_answer() {
        let inv = serde_json::json!({"g": {"hosts": ["a"], "children": ["c"]}, "c": {"hosts": ["b", "a"]}});
        let (mut want, skip) = split_expected(&inv, "g", "");
        want.sort();
        assert_eq!((want, skip), (vec!["a".to_string(), "b".to_string()], vec![]));
    }

    #[test]
    fn a_single_homed_component_expects_only_its_writer_and_unset_roles_read_as_a() {
        let inv = serde_json::json!({"g": {"hosts": ["a", "b", "c"]}, "_meta": {"hostvars": {"b": {"router_role": "B"}, "c": {"router_role": "a"}}}});
        assert_eq!(split_expected(&inv, "g", "A"), (vec!["a".to_string(), "c".to_string()], vec!["b".to_string()]));
    }

    #[test]
    fn an_absent_group_expects_nobody() {
        assert_eq!(split_expected(&serde_json::json!({}), "g", ""), (vec![], vec![]));
    }

    #[test]
    fn a_child_cycle_terminates() {
        let inv = serde_json::json!({"g": {"children": ["c"]}, "c": {"hosts": ["a"], "children": ["g"]}});
        assert_eq!(split_expected(&inv, "g", "").0, vec!["a".to_string()]);
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::*;

    const OLD: &str = "thor | CHANGED | rc=0 | (stdout) {\"ts\": \"2026-09-15T18:21:22Z\", \"component\": \"netvis\", \"ref\": \"x@sha256:aa\", \"action\": \"deploy\"}";
    const NEW: &str = "thor | CHANGED | rc=0 | (stdout) {\"ts\": \"2026-10-28T07:36:28Z\", \"component\": \"netvis\", \"ref\": \"x@sha256:bb\", \"action\": \"deploy\"}";

    #[test]
    fn any_nonzero_failed_or_unreachable_count_is_read() {
        let l = "thor : ok=40   changed=2    unreachable=0    failed=3    skipped=1    rescued=0    ignored=0";
        assert_eq!(recap_count(l, "failed"), 3);
        assert_eq!(recap_count(l, "unreachable"), 0);
        assert_eq!(recap_count("thor : ok=1 failed=12", "failed"), 12);
        assert_eq!(recap_count("thor : ok=1", "failed"), 0);
    }

    #[test]
    fn a_row_identical_to_the_one_found_before_the_play_means_nothing_changed() {
        let how = row_provenance(OLD, "thor", OLD, false);
        assert!(how.starts_with("already at the declared ref (row ts 2026-09-15T18:21:22Z)"), "{how}");
    }

    #[test]
    fn a_row_different_from_the_one_before_the_play_was_written_by_this_run() {
        let how = row_provenance(OLD, "thor", NEW, false);
        assert_eq!(how, "ledger row written by this run (row ts 2026-10-28T07:36:28Z)");
    }

    #[test]
    fn verify_only_reports_the_ts_and_claims_nothing_about_this_run() {
        let how = row_provenance("", "thor", NEW, true);
        assert_eq!(how, "ledger row ts 2026-10-28T07:36:28Z");
    }

    #[test]
    fn a_stale_cached_ts_does_not_decide_the_verdict() {
        // The new row carries an OLDER ts than the run (the fact cache's clock); it is
        // still "written by this run" because the row itself differs from the one found.
        let stale = "thor | CHANGED | rc=0 | (stdout) {\"ts\": \"2026-09-01T00:00:00Z\", \"ref\": \"x@sha256:cc\", \"action\": \"deploy\"}";
        let how = row_provenance(OLD, "thor", stale, false);
        assert!(how.starts_with("ledger row written by this run"), "{how}");
    }

    #[test]
    fn a_promoted_row_says_when_it_finished() {
        let row = "thor | CHANGED | rc=0 | (stdout) {\"ts\": \"2026-10-29T17:24:46Z\", \"ref\": \"x@sha256:dd\", \"action\": \"deploy\", \"promoted_at\": \"2026-10-29T17:32:10Z\"}";
        assert_eq!(row_provenance("", "thor", row, true), "ledger row ts 2026-10-29T17:24:46Z, finished 2026-10-29T17:32:10Z");
    }

    #[test]
    fn a_rollback_row_is_not_called_an_unfinished_play() {
        let d = "sha256:0123456789abaaaa";
        let row = format!("thor | CHANGED | rc=0 | (stdout) {{\"ts\": \"2026-10-29T17:14:19Z\", \"action\": \"rollback\", \"ref\": \"ghcr.io/x/demo:v0.2.0@sha256:3a4954cd0000000000\", \"prior_ref\": \"ghcr.io/x/demo:v0.2.1@{d}\"}}");
        assert_eq!(
            not_deploy_reason(&row, d, "t"),
            "NOT converged — rolled back from ghcr.io/x/demo:v0.2.1@sha256:0123456789ab to ghcr.io/x/demo:v0.2.0@sha256:3a4954cd0000 at 2026-10-29T17:14:19Z; the host is not at the declared ref"
        );
        let pending = format!("thor | CHANGED | rc=0 | (stdout) {{\"action\": \"pending\", \"ref\": \"h@{d}\"}}");
        assert!(not_deploy_reason(&pending, d, "t").contains("the play did not finish on this host"));
        let back = format!("thor | CHANGED | rc=0 | (stdout) {{\"ts\": \"T\", \"action\": \"rollback\", \"ref\": \"h@{d}\", \"prior_ref\": \"h@sha256:99\"}}");
        assert!(not_deploy_reason(&back, d, "t").starts_with("NOT converged — rolled back TO the declared ref at T"));
    }

    #[test]
    fn a_row_without_ts_still_gets_a_verdict() {
        let bare = "thor | CHANGED | rc=0 | (stdout) {\"ref\": \"x@sha256:dd\", \"action\": \"deploy\"}";
        assert_eq!(row_provenance(OLD, "thor", bare, false), "ledger row written by this run (row ts no ts)");
    }
}

/// The `ts` a ledger row carries (ISO, as the rollback-ledger role writes it), read
/// from the raw line so a row with extra fields or spacing still yields it.
pub(crate) fn ledger_row_ts(line: &str) -> Option<String> {
    ledger_row_field(line, "ts")
}

/// A string field of a ledger row, read from the raw line. `promoted_at` is the
/// host's clock when the promote handler marked the row a deploy: the end of the
/// play, where `ts` is its start.
pub(crate) fn ledger_row_field(line: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":");
    let i = line.find(&pat)?;
    let rest = &line[i + pat.len()..];
    let start = rest.find('"')? + 1;
    let end = rest[start..].find('"')? + start;
    Some(rest[start..end].to_string())
}

/// `image@sha256:<12>` — enough of a ref to tell two apart in one line.
fn short_ref(r: &str) -> String {
    match r.split_once("@sha256:") {
        Some((name, d)) => format!("{name}@sha256:{}", &d[..d.len().min(12)]),
        None => r.to_string(),
    }
}

/// Why a host whose newest row names the declared digest is still not converged.
/// A rollback row names the declared digest as its `prior_ref`: the host was moved
/// AWAY from it, which is the opposite of an unfinished play. deploy-component.sh
/// prints the same words.
pub(crate) fn not_deploy_reason(line: &str, digest: &str, tail: &str) -> String {
    let row = line.find('{').zip(line.rfind('}')).and_then(|(i, j)| serde_json::from_str::<serde_json::Value>(&line[i..=j]).ok());
    let field = |k: &str| row.as_ref().and_then(|r| r.get(k)).and_then(|x| x.as_str()).unwrap_or("").to_string();
    if field("action") == "rollback" {
        let (to, from, ts) = (field("ref"), field("prior_ref"), field("ts"));
        if to.contains(digest) {
            return format!("NOT converged — rolled back TO the declared ref at {ts} (from {}); a rollback row is not a deploy: deploy it to record convergence", short_ref(&from));
        }
        return format!("NOT converged — rolled back from {} to {} at {ts}; the host is not at the declared ref", short_ref(&from), short_ref(&to));
    }
    format!("NOT converged — ledger names the ref but the row is not action=deploy (the play did not finish on this host): {tail}")
}

/// The same UTC stamp for callers outside this module (release's worktree and branch names).
pub fn stamp_pub() -> String {
    stamp()
}

fn stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // UTC yyyymmddThhmmssZ without a date crate.
    let days = secs / 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    let rem = secs % 86_400;
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// The target reaches ansible-playbook as two argv words, the flag and its
    /// value, in both modes: `-e <var>=<group>` and `-l <group>`. As one word
    /// (`"-e target=<group>"`) it parsed only because argparse took the rest of
    /// the word as the value and ansible stripped its leading space.
    #[test]
    fn the_target_is_two_argv_words() {
        let inv = vec!["-i".to_string(), "inventory/mgmt-hosts.ini".to_string()];
        let play = Path::new("/r/ops/ansible/microscope-deploy.yml");
        let argv = |flag: &str, accept: Option<&str>| -> Vec<String> {
            let c = ansible_cmd(Path::new("/r"), "preflight", &deploy_args(&inv, play, flag, "zagros_dev_hosts", accept));
            c.get_args().map(|a| a.to_string_lossy().to_string()).collect()
        };
        let head = |v: &[&str]| -> Vec<String> {
            let mut out: Vec<String> = vec![crate::settings::aws_profile(Path::new("/r")), "--".into(), "../scripts/ansible-preflight.sh".into()];
            out.extend(v.iter().map(|s| s.to_string()));
            out
        };
        assert_eq!(
            argv("-e target=", None),
            head(&["-i", "inventory/mgmt-hosts.ini", "microscope-deploy.yml", "-e", "target=zagros_dev_hosts"])
        );
        assert_eq!(
            argv("-l ", None),
            head(&["-i", "inventory/mgmt-hosts.ini", "microscope-deploy.yml", "-l", "zagros_dev_hosts"])
        );
        assert_eq!(
            argv("-e target=", Some("true")),
            head(&["-i", "inventory/mgmt-hosts.ini", "microscope-deploy.yml", "-e", "target=zagros_dev_hosts", "-e", "{\"rollback_preflight_accept\": true}"])
        );
    }

    /// The pins resolution agrees with the script on this repository's real components.
    #[test]
    fn resolves_microscope_on_zagros_dev_like_the_script() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let r = resolve(&repo(), "microscope", "zagros_dev", None).unwrap();
        assert_eq!(r.members, vec!["zagros_dev_hosts"]);
        assert_eq!(r.target_flag, "-e target=");
        assert!(r.hosts_play.ends_with("microscope.yml"));
        assert_eq!(r.ledger, "microscope");
    }

    #[test]
    fn unknown_component_and_goatnet_are_refused_in_the_scripts_words() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let e = resolve(&repo(), "no-such", "zagros_dev", None).unwrap_err().to_string();
        assert_eq!(e, "unknown component \"no-such\"");
        let e = resolve(&repo(), "microscope", "nowhere", None).unwrap_err().to_string();
        assert!(e.starts_with("unknown goatnet \"nowhere\"; known: ["), "{e}");
    }

    #[test]
    fn a_group_outside_the_goatnet_is_refused_before_anything() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let e = resolve(&repo(), "microscope", "zagros_dev", Some("zagros_hosts")).unwrap_err().to_string();
        assert_eq!(e, "--group \"zagros_hosts\" is not a member of goatnet \"zagros_dev\"");
    }

    /// A lite goatnet lives in the sidecar (ADR 1213), and the rollout verbs see it:
    /// `preflight <c> --env <lite goatnet>` said "unknown goatnet; known: []".
    #[test]
    fn a_goatnet_declared_only_in_the_lite_sidecar_is_known() {
        let d = crate::testdir::path("goatbuilder-push-lite-");
        std::fs::create_dir_all(d.join("ops/pins")).unwrap();
        std::fs::write(d.join("ops/pins/fleet-pins.yaml"), "goatnets: {}\ncomponents:\n  alpha:\n    var: alpha_image\n    fleet: \"a@sha256:1\"\n").unwrap();
        std::fs::write(d.join("ops/pins/lite-pins.yaml"), "goatnets:\n  pasture:\n    members: [pasture_hosts]\ncomponents: {}\n").unwrap();
        let got = declared(&d, "alpha", "pasture", None).unwrap();
        assert_eq!(got.members, vec!["pasture_hosts"]);
        let e = declared(&d, "alpha", "nowhere", None).unwrap_err().to_string();
        assert!(e.contains("known: [\"pasture\"]"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn stamp_is_utc_compact() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_754), (2026, 10, 28));
        assert!(stamp().ends_with('Z'));
        assert_eq!(stamp().len(), 16);
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
    }

    #[test]
    fn civil_dates_from_days_cover_the_epoch_leap_days_and_year_ends() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(19_783), (2024, 3, 1));
        assert_eq!(civil_from_days(20_818), (2026, 12, 31));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn the_stamp_is_utc_basic_iso() {
        let s = stamp();
        assert_eq!(s.len(), 16, "{s}");
        assert_eq!(&s[8..9], "T");
        assert!(s.ends_with('Z'));
        assert!(s[0..8].chars().all(|c| c.is_ascii_digit()) && s[9..15].chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn a_ledger_row_ts_is_read_from_the_raw_line() {
        let line = r#"thor | CHANGED | rc=0 | (stdout) {"ts": "2026-10-28T07:36:28Z", "ref": "x"}"#;
        assert_eq!(ledger_row_ts(line).as_deref(), Some("2026-10-28T07:36:28Z"));
        assert_eq!(ledger_row_ts(r#"{"ts":"2026-01-01T00:00:00Z"}"#).as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(ledger_row_ts("NO_LEDGER"), None);
    }

    #[test]
    fn inventory_args_pair_each_inventory_with_its_flag() {
        let t = Transport { kind: "mesh".into(), detail: String::new(), inventories: vec!["a.ini".into(), "b.ini".into()] };
        assert_eq!(inventory_args(&t), vec!["-i", "a.ini", "-i", "b.ini"]);
        let none = Transport { kind: "x".into(), detail: String::new(), inventories: vec![] };
        assert!(inventory_args(&none).is_empty());
    }

    #[test]
    fn render_ref_resolves_the_org_template() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let r = render_ref("ghcr.io/{{ ghcr_org }}/netvis:sha-1@sha256:aa");
        assert!(!r.contains("{{"), "{r}");
    }

    #[test]
    fn resolve_finds_the_var_play_and_members_for_a_declared_component() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let r = resolve(&repo(), "netvis", "zagros_dev", None).unwrap();
        assert_eq!(r.var, "netvis_image");
        assert_eq!(r.ledger, "netvis");
        assert!(r.members.contains(&"zagros_dev_hosts".to_string()), "{:?}", r.members);
        assert!(r.play.ends_with("netvis-deploy.yml"), "{}", r.play.display());
        assert!(r.target_flag.starts_with("-e target=") || r.target_flag.starts_with("-l "), "{}", r.target_flag);
    }

    #[test]
    fn resolve_follows_an_alias_play_to_the_one_that_declares_hosts() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let r = resolve(&repo(), "microscope", "zagros_dev", None).unwrap();
        assert!(r.play.ends_with("microscope-deploy.yml"), "{}", r.play.display());
        assert!(r.hosts_play.ends_with("microscope.yml"), "the alias's imported play declares hosts: {}", r.hosts_play.display());
        assert_eq!(r.target_flag, "-e target=");
    }

    #[test]
    fn resolve_skips_absent_deviations_as_topology() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let r = resolve(&repo(), "microscope", "zagros", None).unwrap();
        assert!(!r.skipped.is_empty(), "single-homed microscope has absent joiner groups");
        for g in &r.skipped {
            assert!(!r.members.contains(g), "{g} is both skipped and a member");
        }
    }

    #[test]
    fn resolve_refuses_an_unknown_component_goatnet_or_foreign_group() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let e = resolve(&repo(), "no-such", "zagros_dev", None).unwrap_err().to_string();
        assert!(e.contains("unknown component"), "{e}");
        let e = resolve(&repo(), "netvis", "nowhere", None).unwrap_err().to_string();
        assert!(e.contains("unknown goatnet") && e.contains("zagros_dev"), "{e}");
        assert!(resolve(&repo(), "netvis", "zagros_dev", Some("zagros_hosts")).is_err(), "a group from another goatnet is refused");
    }
}

