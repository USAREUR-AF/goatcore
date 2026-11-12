//! The developer's verbs: `verify`, `diff`, `history` (read-only, contract C8),
//! `plan` / `apply` (a reviewed decision run exactly as reviewed), and `adopt`
//! (declare what an environment already runs). Design of record:
//! docs/design/goatbuilder.md §"Verbs"; status per verb in
//! goatbuilder-conformance.md §"What is built".
//!
//! On real hosts every read goes over the same transport `deploy` decides (mesh,
//! door, base inventory) and reads the same on-host ledger,
//! `/var/lib/goat/<ledger>/deploy-history.jsonl`. On a fixture (`--mock`) the
//! scripted converger's records stand in for the hosts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde_json::json;

use crate::converger::Converger;
use crate::push;
use crate::verbs::{self, Fixture};
use crate::{Check, Report, EXIT_NOT_CONVERGED, EXIT_REFUSED};

fn ok(name: &str, detail: String) -> Check {
    Check { name: name.to_string(), ok: true, detail, fix: None }
}
fn not_ok(name: &str, detail: String) -> Check {
    Check { name: name.to_string(), ok: false, detail, fix: None }
}
fn refused(name: &str, detail: String, fix: &str) -> Check {
    Check { name: name.to_string(), ok: false, detail, fix: Some(fix.to_string()) }
}

pub(crate) fn digest(r: &str) -> &str {
    r.rsplit('@').next().unwrap_or(r)
}

/// The image repository's last path segment, without tag or digest: `oryd/kratos:v1@sha…` → `kratos`.
pub(crate) fn image_name(r: &str) -> &str {
    let n = r.split('@').next().unwrap_or(r);
    let n = n.rsplit('/').next().unwrap_or(n);
    n.split(':').next().unwrap_or(n)
}

/// The image name and tag of a ref, without the registry org or the digest: what a reader compares.
pub(crate) fn short(r: &str) -> String {
    let name = r.split('@').next().unwrap_or(r);
    let name = name.rsplit('/').next().unwrap_or(name);
    let d = digest(r);
    if d.starts_with("sha256:") && d.len() > 19 { format!("{name}@{}", &d[..19]) } else { name.to_string() }
}

// ---------------------------------------------------------------------------
// verify
// ---------------------------------------------------------------------------

/// `verify`: every host's newest ledger row against the declared ref, reading only.
/// The same code path as `deploy --verify-only`, so the two never disagree.
pub fn verify(repo: &Path, component: &str, env: &str, group: Option<&str>, dry_run: bool) -> Result<Report> {
    let mut r = push::deploy(repo, &push::Options { component, goatnet: env, only_group: group, dry_run, verify_only: true, ack: "" })?;
    r.verb = "verify";
    // Reading the ledgers templates nothing: the secrets row belongs to a deploy.
    r.checks.retain(|c| !c.name.ends_with("/secrets"));
    Ok(r)
}

pub fn verify_mock(f: &Fixture, component: &str, env: &str) -> Result<Report> {
    let mut r = verbs::status(f, env, Some(component))?;
    r.verb = "verify";
    Ok(r)
}

// ---------------------------------------------------------------------------
// reading every ledger of an environment in one pass per member group
// ---------------------------------------------------------------------------

/// host → ledger component → newest row (raw JSON text).
type Tails = BTreeMap<String, BTreeMap<String, String>>;

/// One member group as read: its ledgers, its unreachable hosts, its hosts' own facts.
type GroupRead = (Tails, Vec<String>, BTreeMap<String, HostFacts>);

/// What a host says about itself beside its ledgers, read in the same call:
/// the goat CLI it runs and, where it serves goat-get, the release tags it serves
/// (its `version.json` files, the files `goat update` reads).
#[derive(Debug, Clone, Default, serde::Serialize, PartialEq)]
pub struct HostFacts {
    pub group: String,
    pub goat_cli: Option<String>,
    /// served path (e.g. `goat-cli`, `goatbuilder`) → release tag
    pub served: BTreeMap<String, String>,
    /// The host carries a goat-get ledger: it serves downloads, so an unread
    /// `version.json` is unknown rather than absent.
    pub goat_get: bool,
    /// Every container, running or stopped (a warm standby keeps its stack
    /// stopped): its name, state, the image ref it was created from and that
    /// image's registry digests. None when docker could not be read.
    pub containers: Option<Vec<Container>>,
}

#[derive(Debug, Clone, Default, serde::Serialize, PartialEq)]
pub struct Container {
    pub name: String,
    pub image: String,
    pub repo_digests: Vec<String>,
    /// docker's state: running, exited, created, …
    pub state: String,
}

/// The shell one ansible call runs per member group: every ledger's newest row,
/// the host's goat CLI version, and the tags its goat-get serves.
const READ_HOST: &str = concat!(
    "for f in /var/lib/goat/*/deploy-history.jsonl; do [ -f \"$f\" ] || continue; printf 'GBROW %s %s\\n' \"$(basename \"$(dirname \"$f\")\")\" \"$(tail -n 1 \"$f\")\"; done; ",
    "for b in goat /usr/local/bin/goat; do if command -v \"$b\" >/dev/null 2>&1; then printf 'GBGOAT %s\\n' \"$(\"$b\" --version 2>/dev/null | head -n 1)\"; break; fi; done; ",
    "for v in /opt/goat-get/www/version.json /opt/goat-get/www/goatbuilder/version.json; do c=$(cat \"$v\" 2>/dev/null || sudo -n cat \"$v\" 2>/dev/null); [ -n \"$c\" ] && printf 'GBSERVED %s %s\\n' \"$v\" \"$(printf '%s' \"$c\" | tr -d '\\n')\"; done; ",
    // Docker's own templates are `{{ }}`, which Ansible would render: raw.
    "{% raw %}if sudo -n docker info >/dev/null 2>&1; then echo GBDOCKER ok; ",
    "ids=$(sudo -n docker ps -aq); if [ -n \"$ids\" ]; then sudo -n docker inspect -f 'GBCTR {{.Name}} {{.Config.Image}} {{.Image}} {{.State.Status}}' $ids; ",
    "sudo -n docker image inspect -f 'GBIMG {{.Id}} {{join .RepoDigests \" \"}}' $(sudo -n docker inspect -f '{{.Image}}' $ids | sort -u); fi; ",
    "fi{% endraw %}; echo GBEND"
);

/// The release tag a served `version.json` names, keyed by what it serves.
fn served_entry(path: &str, json_text: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(json_text).ok()?;
    let tag = v.get("tag").and_then(|t| t.as_str())?.to_string();
    let what = if path.contains("/goatbuilder/") { "goatbuilder" } else { "goat-cli" };
    Some((what.to_string(), tag))
}

/// Parse one `ansible -m shell` answer of [`READ_HOST`].
fn parse_hosts(text: &str, group: &str) -> GroupRead {
    let head = regex::Regex::new(r"^(\S+) \| (CHANGED|SUCCESS|FAILED!?|UNREACHABLE!)").unwrap();
    let mut tails: Tails = BTreeMap::new();
    let mut facts: BTreeMap<String, HostFacts> = BTreeMap::new();
    let mut unreachable = Vec::new();
    let mut host: Option<String> = None;
    // host → (image id, container); host → image id → registry digests
    let mut images: BTreeMap<String, Vec<(String, Container)>> = BTreeMap::new();
    let mut digests: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for line in text.lines() {
        if let Some(c) = head.captures(line) {
            let h = c[1].to_string();
            if c[2].starts_with("UNREACHABLE") || c[2].starts_with("FAILED") {
                unreachable.push(h);
                host = None;
            } else {
                tails.entry(h.clone()).or_default();
                facts.entry(h.clone()).or_insert_with(|| HostFacts { group: group.to_string(), ..Default::default() });
                host = Some(h);
            }
            continue;
        }
        let Some(h) = &host else { continue };
        if let Some(rest) = line.strip_prefix("GBROW ") {
            if let Some((comp, row)) = rest.split_once(' ') {
                tails.entry(h.clone()).or_default().insert(comp.to_string(), row.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("GBGOAT ") {
            let v = rest.trim().trim_start_matches("goat ").trim().to_string();
            if !v.is_empty() {
                facts.entry(h.clone()).or_default().goat_cli = Some(v);
            }
        } else if line == "GBDOCKER ok" {
            facts.entry(h.clone()).or_default().containers.get_or_insert_with(Vec::new);
        } else if let Some(rest) = line.strip_prefix("GBCTR ") {
            let mut it = rest.split_whitespace();
            if let (Some(n), Some(img), Some(id)) = (it.next(), it.next(), it.next()) {
                let state = it.next().unwrap_or("").to_string();
                let c = Container { name: n.trim_start_matches('/').to_string(), image: img.to_string(), repo_digests: vec![], state };
                images.entry(h.clone()).or_default().push((id.to_string(), c));
            }
        } else if let Some(rest) = line.strip_prefix("GBIMG ") {
            let mut it = rest.split_whitespace();
            if let Some(id) = it.next() {
                digests.entry(h.clone()).or_default().insert(id.to_string(), it.map(String::from).collect());
            }
        } else if let Some(rest) = line.strip_prefix("GBSERVED ") {
            if let Some((path, body)) = rest.split_once(' ') {
                if let Some((what, tag)) = served_entry(path, body) {
                    facts.entry(h.clone()).or_default().served.insert(what, tag);
                }
            }
        }
    }
    for (h, ledgers) in &tails {
        if ledgers.contains_key("goat-get") {
            facts.entry(h.clone()).or_default().goat_get = true;
        }
    }
    for (h, list) in images {
        let ds = digests.remove(&h).unwrap_or_default();
        let cs = facts.entry(h).or_default().containers.get_or_insert_with(Vec::new);
        for (id, mut c) in list {
            c.repo_digests = ds.get(&id).cloned().unwrap_or_default();
            cs.push(c);
        }
    }
    (tails, unreachable, facts)
}

/// One `ansible` call per member group returns every ledger's newest row on every
/// host of the group, and the host's own facts: a whole environment is read in
/// seconds, not one call per component.
fn read_all_tails(repo: &Path, inv: &[String], group: &str) -> Result<GroupRead> {
    let mut args = inv.to_vec();
    args.push(group.to_string());
    args.extend(["-m".into(), "shell".into(), "-a".into(), READ_HOST.into()]);
    let out = push::ansible_cmd(repo, "ansible", &args).output().with_context(|| "ansible")?;
    Ok(parse_hosts(&String::from_utf8_lossy(&out.stdout), group))
}

/// Every fleet-pins component on this goatnet with a ledger to read.
fn components_of(repo: &Path, env: &str) -> Result<Vec<String>> {
    let pins: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(repo.join("ops/pins/fleet-pins.yaml"))?)?;
    let comps = pins.get("components").and_then(|c| c.as_mapping()).ok_or_else(|| anyhow!("fleet-pins.yaml has no components"))?;
    // A component is read wherever the pins put it, whether or not it has a
    // deploy play of its own: the comparison needs the ledger, not the play.
    Ok(comps.keys().filter_map(|k| k.as_str().map(String::from)).filter(|c| push::declared(repo, c, env, None).map(|d| !d.members.is_empty()).unwrap_or(false)).collect())
}

fn ledger_null(repo: &Path, component: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(repo.join("ops/pins/fleet-pins.yaml")) else { return false };
    let Ok(pins) = serde_yaml::from_str::<serde_yaml::Value>(&text) else { return false };
    pins.get("components").and_then(|c| c.get(component)).and_then(|c| c.get("ledger_component")).map(|v| v.is_null()).unwrap_or(false)
}

/// The router_role a single-homed component runs on (`single_homed_role` in
/// fleet-pins.yaml, ADR 1084 §D3); empty when it runs on every host of a group.
fn single_homed_role(repo: &Path, component: &str) -> String {
    let Ok(text) = std::fs::read_to_string(repo.join("ops/pins/fleet-pins.yaml")) else { return String::new() };
    let Ok(pins) = serde_yaml::from_str::<serde_yaml::Value>(&text) else { return String::new() };
    pins.get("components").and_then(|c| c.get(component)).and_then(|c| c.get("single_homed_role")).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// One row of `diff`: what main declares against what one host runs.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Drift {
    pub component: String,
    pub group: String,
    pub host: String,
    pub declared: String,
    pub running: String,
    pub state: String, // same | drift | served-drift | served-unknown | no-ledger | unreachable | other-image | no-coverage
    pub row_ts: String,
    pub action: String,
}

/// Declared against running for one component or every component of an environment.
pub fn collect(repo: &Path, env: &str, only: Option<&str>) -> Result<(Vec<Drift>, String)> {
    collect_full(repo, env, only).map(|(rows, t, _)| (rows, t))
}

/// [`collect`], plus each host's own facts from the same read (`survey`).
pub fn collect_full(repo: &Path, env: &str, only: Option<&str>) -> Result<(Vec<Drift>, String, BTreeMap<String, HostFacts>)> {
    read_rows(repo, env, only).map(|(rows, t, facts)| (rows, t.detail, facts))
}

/// The one pass over an environment's hosts that [`collect_full`], [`diff`] and
/// [`status`] share: the rows, and the transport they were read over.
fn read_rows(repo: &Path, env: &str, only: Option<&str>) -> Result<(Vec<Drift>, push::Transport, BTreeMap<String, HostFacts>)> {
    let comps = match only {
        Some(c) => {
            push::declared(repo, c, env, None)?;
            vec![c.to_string()]
        }
        None => components_of(repo, env)?,
    };
    if comps.is_empty() {
        return Err(goat_errors::coded("GB-050", format!("no component is declared on {env}")).into());
    }
    // Every component of a goatnet shares its member groups; read each group once.
    let first = push::declared(repo, &comps[0], env, None)?;
    let mut all_members: Vec<String> = first.members.clone();
    all_members.extend(first.skipped.clone());
    let name = env.replace('_', "-");
    let t = push::transport(repo, &name, &all_members, false)?;
    let inv = push::inventory_args(&t);
    let mut by_group: BTreeMap<String, GroupRead> = BTreeMap::new();
    let mut served_wanted: BTreeMap<(String, String), bool> = BTreeMap::new();
    // A single-homed component (ADR 1084 §D3, F-547) runs on its writer only; the
    // group's other hosts are not expected to carry it, so they are not read for it.
    let mut not_writer: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let mut rows = Vec::new();
    for c in &comps {
        let r = push::declared(repo, c, env, None)?;
        for g in &r.members {
            let Some(want) = push::want(repo, g, &r.var) else { continue };
            let declared = push::render_ref(&want);
            if ledger_null(repo, c) {
                rows.push(Drift { component: c.clone(), group: g.clone(), host: "*".into(), declared, running: String::new(), state: "no-coverage".into(), row_ts: String::new(), action: String::new() });
                continue;
            }
            if !by_group.contains_key(g) {
                by_group.insert(g.clone(), read_all_tails(repo, &inv, g)?);
            }
            let role = single_homed_role(repo, c);
            let skip = not_writer.entry((g.clone(), role.clone())).or_insert_with(|| push::skip_hosts(repo, &t, g, &role)).clone();
            let (tails, unreachable, _) = &by_group[g];
            if tails.is_empty() && unreachable.is_empty() {
                // Ansible answered for no host at all. A group whose inventory still
                // holds TBD_ placeholders was never provisioned: say so, by name, rather
                // than calling it unreachable. Otherwise (no transport, no aws-vault, the
                // group empty in the loaded inventories) nothing was read, so nothing is "same".
                let (state, running) = match placeholder_group(repo, g) {
                    Some(p) => ("not-provisioned", p),
                    None => ("unreachable", String::new()),
                };
                rows.push(Drift { component: c.clone(), group: g.clone(), host: format!("<{g}>"), declared: declared.clone(), running, state: state.into(), row_ts: String::new(), action: String::new() });
            }
            for h in unreachable.iter().filter(|h| !skip.contains(h)) {
                rows.push(Drift { component: c.clone(), group: g.clone(), host: h.clone(), declared: declared.clone(), running: String::new(), state: "unreachable".into(), row_ts: String::new(), action: String::new() });
            }
            for (h, ledgers) in tails.iter().filter(|(h, _)| !skip.contains(h)) {
                let (running, ts, action, state) = match ledgers.get(&r.ledger).and_then(|row| serde_json::from_str::<serde_json::Value>(row).ok()) {
                    None => (String::new(), String::new(), String::new(), "no-ledger".to_string()),
                    Some(v) => {
                        let running = v.get("ref").or_else(|| v.get("image")).and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let mut ts = v.get("ts").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        if let Some(p) = v.get("promoted_at").and_then(|x| x.as_str()) {
                            ts = format!("{ts}, finished {p}");
                        }
                        let action = v.get("action").and_then(|x| x.as_str()).unwrap_or("deploy").to_string();
                        // A ledger shared by several images (ory: kratos, hydra, the UI) records
                        // whichever image its play last wrote; another image's row says nothing
                        // about this component's ref.
                        let mut state = if digest(&running) == digest(&declared) {
                            "same"
                        } else if image_name(&running) != image_name(&declared) {
                            "other-image"
                        } else {
                            "drift"
                        }
                        .to_string();
                        let mut running = running;
                        // A serving surface (goat-get, goat-docs) is "same" only when what it
                        // serves is too: the releases are role vars, not its image.
                        if state == "same" && served_wanted.entry((c.clone(), g.clone())).or_insert_with(|| push::served_declared(repo, &r.ledger, g).map(|w| !w.is_empty()).unwrap_or(true)).to_owned() {
                            let row = ledgers.get(&r.ledger).cloned().unwrap_or_default();
                            let (as_declared, note) = push::served_check(repo, &r.ledger, g, &row);
                            if !as_declared {
                                state = if note.starts_with("served payload DRIFT") { "served-drift".into() } else { "served-unknown".into() };
                                running = note;
                            }
                        }
                        (running, ts, action, state)
                    }
                };
                rows.push(Drift { component: c.clone(), group: g.clone(), host: h.clone(), declared: declared.clone(), running, state, row_ts: ts, action });
            }
        }
    }
    let facts = by_group.into_values().flat_map(|(_, _, f)| f).collect();
    Ok((rows, t, facts))
}

/// A row read off a host is at the declared ref only when its newest ledger row
/// names the declared digest AND records a finished deploy: `verify`'s rule (C7,
/// F-530). A row the play left at `action: pending`, or a rollback row, names the
/// ref without proving the deploy finished.
fn finished(d: &Drift) -> bool {
    d.state == "same" && d.action == "deploy"
}

/// [`read_rows`] completed by `verify`'s second rule: every host the inventory
/// expects to carry a component must answer, and silence is not proof (#1545).
/// A host the inventory names that gave no answer is a `no-answer` row. Returns
/// the rows, the transport's description, and a row per group whose inventory
/// could not be read (nothing about its hosts is then known).
fn read_expected(repo: &Path, env: &str, only: Option<&str>) -> Result<(Vec<Drift>, String, Vec<Check>)> {
    let (mut rows, t, _) = read_rows(repo, env, only)?;
    let n = env.replace('_', "-");
    let mut problems = Vec::new();
    // (component, group) pairs whose hosts were read; a group never provisioned,
    // or a component without ledger coverage, names no host to expect.
    let mut pairs: Vec<(String, String, String)> = Vec::new();
    for d in rows.iter().filter(|d| d.state != "no-coverage" && d.state != "not-provisioned") {
        let key = (d.component.clone(), d.group.clone(), d.declared.clone());
        if !pairs.contains(&key) {
            pairs.push(key);
        }
    }
    let mut expected: BTreeMap<(String, String), std::result::Result<Vec<String>, String>> = BTreeMap::new();
    for (c, g, declared) in pairs {
        let role = single_homed_role(repo, &c);
        let want = expected.entry((g.clone(), role.clone())).or_insert_with(|| push::expected_hosts(repo, &t, &g, &role).map(|(e, _)| e));
        let hosts = match want {
            Ok(h) => h.clone(),
            Err(e) => {
                let name = format!("{n}/{g}/inventory");
                if !problems.iter().any(|p: &Check| p.name == name) {
                    problems.push(not_ok(&name, format!("inventory read FAILED — {e}; which hosts must answer is unknown")));
                }
                continue;
            }
        };
        if hosts.is_empty() {
            continue;
        }
        // Ansible answered for no host at all: name each expected host instead of the group.
        let placeholder = format!("<{g}>");
        rows.retain(|d| !(d.component == c && d.group == g && d.host == placeholder));
        let answered: std::collections::BTreeSet<String> = rows.iter().filter(|d| d.component == c && d.group == g).map(|d| d.host.clone()).collect();
        for h in hosts.iter().filter(|h| !answered.contains(*h)) {
            rows.push(Drift { component: c.clone(), group: g.clone(), host: h.clone(), declared: declared.clone(), running: String::new(), state: "no-answer".into(), row_ts: String::new(), action: String::new() });
        }
    }
    // Keep the read's order: component, then group, then host as read.
    let order: Vec<(String, String)> = rows.iter().map(|d| (d.component.clone(), d.group.clone())).fold(Vec::new(), |mut v, k| {
        if !v.contains(&k) {
            v.push(k);
        }
        v
    });
    rows.sort_by_key(|d| order.iter().position(|k| k.0 == d.component && k.1 == d.group));
    Ok((rows, t.detail, problems))
}

/// A member group whose every host line in the base inventories still carries a
/// `TBD_` placeholder (an instance id or address never filled in) was never
/// provisioned. Returns the placeholder as evidence, e.g.
/// `ops/ansible/inventory/<site>.ini: mgmt-<site>-core-a ansible_aws_ssm_instance_id=TBD_CORE_A`.
pub fn placeholder_group(repo: &Path, group: &str) -> Option<String> {
    let dir = repo.join("ops/ansible/inventory");
    let mut files: Vec<_> = std::fs::read_dir(&dir).ok()?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "ini")).collect();
    files.sort();
    let header = format!("[{group}]");
    let mut hosts: Vec<(String, String)> = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let mut inside = false;
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with('[') {
                inside = t == header;
                continue;
            }
            if inside && !t.is_empty() && !t.starts_with('#') && !t.starts_with(';') {
                let rel = f.strip_prefix(repo).unwrap_or(&f).display().to_string();
                hosts.push((rel, t.split('#').next().unwrap_or(t).trim().to_string()));
            }
        }
    }
    if hosts.is_empty() || !hosts.iter().all(|(_, h)| h.contains("TBD_")) {
        return None;
    }
    let (file, line) = &hosts[0];
    let host = line.split_whitespace().next().unwrap_or("");
    let token = line.split_whitespace().find(|w| w.contains("TBD_")).unwrap_or("");
    Some(format!("inventory placeholder: {file}: {host} {token}"))
}

fn diff_checks(env: &str, rows: &[Drift]) -> Vec<Check> {
    rows.iter()
        .map(|d| {
            let name = format!("{}/{}/{}", env.replace('_', "-"), d.host, d.component);
            match d.state.as_str() {
                "same" if finished(d) => ok(&name, format!("same: {} (row {} {})", short(&d.declared), d.action, d.row_ts)),
                "same" => Check {
                    fix: Some(unfinished_fix(d)),
                    ..not_ok(&name, format!("pending: record names {}, declared {}", short(&d.running), short(&d.declared)))
                },
                "no-answer" => not_ok(&name, format!("unknown: no answer from this host (the inventory names it); main declares {}", short(&d.declared))),
                "drift" => not_ok(&name, format!("DRIFT: running {} — main declares {} (row {} {})", short(&d.running), short(&d.declared), d.action, d.row_ts)),
                "no-ledger" => not_ok(&name, format!("unknown: no ledger on the host; main declares {}", short(&d.declared))),
                "unreachable" => not_ok(&name, format!("unknown: host unreachable; main declares {}", short(&d.declared))),
                "not-provisioned" => ok(&name, format!("not provisioned ({}); main declares {}", d.running, short(&d.declared))),
                "other-image" => not_ok(&name, format!("unknown: the shared ledger's newest row records another image ({}); main declares {}", short(&d.running), short(&d.declared))),
                "served-drift" => not_ok(&name, format!("DRIFT: image same ({}), but {} (row {} {})", short(&d.declared), d.running, d.action, d.row_ts)),
                "served-unknown" => not_ok(&name, format!("unknown: image same ({}), but {}", short(&d.declared), d.running)),
                _ => ok(&name, format!("no ledger coverage (ledger_component: null); main declares {}", short(&d.declared))),
            }
        })
        .collect()
}

/// Why a row naming the declared ref is not convergence, in `verify`'s words.
fn unfinished_fix(d: &Drift) -> String {
    format!("the newest ledger row is action={} (row ts {}), not a finished deploy: deploy it to record convergence", if d.action.is_empty() { "<none>" } else { &d.action }, d.row_ts)
}

/// `diff`: what is different from main right now. Exit 3 on any drift or unknown host.
pub fn diff(repo: &Path, env: &str, only: Option<&str>) -> Result<Report> {
    let (rows, transport, problems) = read_expected(repo, env, only)?;
    let mut checks = vec![ok(&format!("{}/transport", env.replace('_', "-")), format!("read via {transport}"))];
    checks.extend(problems);
    checks.extend(diff_checks(env, &rows));
    let drift = rows.iter().filter(|d| matches!(d.state.as_str(), "drift" | "served-drift") || (d.state == "same" && !finished(d))).count();
    let unknown = rows.iter().filter(|d| matches!(d.state.as_str(), "no-ledger" | "unreachable" | "no-answer" | "other-image" | "served-unknown")).count();
    let same = rows.iter().filter(|d| finished(d)).count();
    let summary = format!("{same} same, {drift} drift, {unknown} unknown across {} component(s)", rows.iter().map(|d| &d.component).collect::<std::collections::BTreeSet<_>>().len());
    if rows.is_empty() {
        // An empty comparison is not agreement.
        checks.push(not_ok(&format!("{}/diff", env.replace('_', "-")), format!("{summary}: nothing was read")));
        return Ok(Report::new("diff", checks, EXIT_NOT_CONVERGED));
    }
    let n = env.replace('_', "-");
    checks.push(if drift + unknown == 0 { ok(&format!("{n}/diff"), summary) } else { not_ok(&format!("{n}/diff"), summary) });
    Ok(Report::new("diff", checks, EXIT_NOT_CONVERGED))
}

// ---------------------------------------------------------------------------
// status on real hosts (gate row C3)
// ---------------------------------------------------------------------------

/// One machine's state for one component, in `status`'s three words (C7):
/// converged (its record names the declared ref, deploy finished), pending (it
/// was read and is not there: another ref, no record, an unfinished row) or
/// unknown (it was not read: unreachable, or silent where the inventory names it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum Verdict {
    Converged,
    Pending,
    Unknown,
}

/// `status`'s grade of one row of [`read_expected`]: the rules `verify` applies
/// to the same ledger rows. `None` for a row that names no machine (a group never
/// provisioned, a component without ledger coverage).
pub fn grade(d: &Drift) -> Option<Verdict> {
    match d.state.as_str() {
        "same" if finished(d) => Some(Verdict::Converged),
        "same" | "drift" | "served-drift" | "served-unknown" | "no-ledger" | "other-image" => Some(Verdict::Pending),
        "unreachable" | "no-answer" => Some(Verdict::Unknown),
        _ => None,
    }
}

/// One `status` row, in the words `status` uses on a fixture, so the same page
/// renders both (`verbs::status_page`).
fn status_check(n: &str, d: &Drift) -> Check {
    let name = format!("{n}/{}/{}", d.host, d.component);
    let (decl, run) = (short(&d.declared), short(&d.running));
    let pending = |fix: Option<String>| Check { fix, ..not_ok(&name, format!("pending: record names {run}, declared {decl}")) };
    match (grade(d), d.state.as_str()) {
        (Some(Verdict::Converged), _) => ok(&name, format!("converged on {decl} (row {} {})", d.action, d.row_ts)),
        (Some(Verdict::Pending), "same") => pending(Some(unfinished_fix(d))),
        (Some(Verdict::Pending), "drift") => pending(None),
        (Some(Verdict::Pending), "no-ledger") => not_ok(&name, format!("pending: no record yet; declared {decl}")),
        (Some(Verdict::Pending), "other-image") => pending(Some("the shared ledger's newest row records another image: this component's own ref is not on record".into())),
        // A serving surface: the image is the declared one, what it serves is not (or is unread).
        (Some(Verdict::Pending), _) => Check { fix: Some(d.running.clone()), ..not_ok(&name, format!("pending: record names {decl}, declared {decl}")) },
        (Some(Verdict::Unknown), "no-answer") => not_ok(&name, format!("unknown: {}: no answer from this host (the inventory names it)", d.host)),
        (Some(Verdict::Unknown), _) => not_ok(&name, format!("unknown: {}: host unreachable", d.host)),
        (None, "not-provisioned") => ok(&name, format!("not provisioned ({}); main declares {decl}", d.running)),
        (None, _) => ok(&name, format!("no ledger coverage (ledger_component: null); main declares {decl}")),
    }
}

/// `status` on real hosts: declared against recorded, per machine, read the way
/// `diff` reads (one pass per member group) and graded the way `verify` grades,
/// so the three never disagree about a machine (gate row C3). Reads only: no
/// play runs and no file is written. Exit 3 while any machine is pending or unknown.
// main.rs's `status` arm calls this once it is wired (the arm still refuses with GB-026).
#[allow(dead_code)]
pub fn status(repo: &Path, env: &str, only: Option<&str>) -> Result<Report> {
    let (rows, _transport, mut checks) = read_expected(repo, env, only)?;
    let n = env.replace('_', "-");
    if rows.is_empty() {
        // An empty read is not convergence.
        checks.push(not_ok(&format!("{n}/status"), "nothing was read".into()));
    }
    checks.extend(rows.iter().map(|d| status_check(&n, d)));
    Ok(Report::new("status", checks, EXIT_NOT_CONVERGED))
}

pub fn diff_mock(f: &Fixture, env: &str, only: Option<&str>) -> Result<Report> {
    let s = verbs::status(f, env, only)?;
    let checks = s
        .checks
        .into_iter()
        .map(|c| {
            let detail = if c.ok {
                c.detail.replacen("converged on", "same:", 1)
            } else if c.detail.starts_with("pending: record names") {
                c.detail.replacen("pending: record names", "DRIFT: running", 1).replacen(", declared", " — main declares", 1)
            } else {
                c.detail
            };
            Check { detail, ..c }
        })
        .collect();
    Ok(Report::new("diff", checks, EXIT_NOT_CONVERGED))
}

// ---------------------------------------------------------------------------
// history
// ---------------------------------------------------------------------------

fn ledger_of(repo: &Path, component: &str) -> String {
    let Ok(text) = std::fs::read_to_string(repo.join("ops/pins/fleet-pins.yaml")) else { return component.into() };
    let Ok(pins) = serde_yaml::from_str::<serde_yaml::Value>(&text) else { return component.into() };
    pins.get("components").and_then(|c| c.get(component)).and_then(|c| c.get("ledger_component")).and_then(|v| v.as_str()).unwrap_or(component).to_string()
}

fn line(detail: String) -> Check {
    Check { name: "history".into(), ok: true, detail, fix: None }
}

/// `history`: the ledger as a timeline, newest first, on every host of the
/// environment — each row as `rollback --list` prints it, then its action, prior
/// ref and operator. With no host reachable it prints the in-repo history exactly
/// as `rollback --list` does, so the two agree in CI too.
pub fn history(repo: &Path, component: &str, env: &str) -> Result<Report> {
    let ledger = ledger_of(repo, component);
    let mut checks = vec![line(format!("deploy history for {component} (newest first):"))];
    let read = (|| -> Result<Vec<(String, Vec<serde_json::Value>)>> {
        let r = push::resolve(repo, component, env, None)?;
        let t = push::transport(repo, &env.replace('_', "-"), &r.members, false)?;
        let mut out = Vec::new();
        let head = regex::Regex::new(r"^(\S+) \| (CHANGED|SUCCESS)").unwrap();
        for g in &r.members {
            let mut args = push::inventory_args(&t);
            args.extend([g.clone(), "-m".into(), "shell".into(), "-a".into(), format!("tac /var/lib/goat/{ledger}/deploy-history.jsonl 2>/dev/null; echo GBEND")]);
            let o = push::ansible_cmd(repo, "ansible", &args).output().with_context(|| "ansible")?;
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            let mut host: Option<String> = None;
            for l in text.lines() {
                if let Some(c) = head.captures(l) {
                    host = Some(c[1].to_string());
                    out.push((c[1].to_string(), Vec::new()));
                    continue;
                }
                if host.is_some() {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(l) {
                        if let Some(last) = out.last_mut() {
                            last.1.push(v);
                        }
                    }
                }
            }
        }
        Ok(out)
    })()
    .unwrap_or_default();
    let read: Vec<_> = read.into_iter().filter(|(_, rows)| !rows.is_empty()).collect();
    if read.is_empty() {
        checks.push(line("no reachable on-host ledger — falling back to in-repo deploy-history.yml".into()));
        for l in crate::rollback::in_repo_lines(repo, &ledger)? {
            checks.push(line(l));
        }
        return Ok(Report::new("history", checks, EXIT_REFUSED));
    }
    for (host, rows) in read {
        checks.push(line(format!("  (on-host ledger @ {host} — source of truth)")));
        for r in rows {
            checks.push(line(crate::rollback::format_row(&r)));
            let g = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let prior = g("prior_ref");
            checks.push(line(format!(
                "      action={} prior={} operator={}",
                if g("action").is_empty() { "deploy".into() } else { g("action") },
                if prior.is_empty() { "—".into() } else { short(&prior) },
                if g("operator").is_empty() { "—".into() } else { g("operator") }
            )));
        }
    }
    Ok(Report::new("history", checks, EXIT_REFUSED))
}

pub fn history_mock(f: &Fixture, component: &str, env: &str) -> Result<Report> {
    f.declared_ref(env, component)?;
    let mut checks = vec![line(format!("deploy history for {component} (newest first):"))];
    for machine in f.converger.machines(env) {
        let p = f.root.join("records").join(env).join(format!("{machine}.jsonl"));
        let rows: Vec<serde_json::Value> = std::fs::read_to_string(&p)
            .unwrap_or_default()
            .lines()
            .rev()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v.get("component").and_then(|c| c.as_str()) == Some(component))
            .collect();
        checks.push(line(format!("  (ledger @ {machine})")));
        if rows.is_empty() {
            checks.push(line("  (no rows)".into()));
        }
        for r in rows {
            checks.push(line(crate::rollback::format_row(&r)));
            checks.push(line(format!("      action={}", r.get("action").and_then(|x| x.as_str()).unwrap_or("deploy"))));
        }
    }
    Ok(Report::new("history", checks, EXIT_REFUSED))
}

// ---------------------------------------------------------------------------
// plan / apply
// ---------------------------------------------------------------------------

fn git(repo: &Path, args: &[&str]) -> String {
    Command::new("git").arg("-C").arg(repo).args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

/// The facts a plan pins: if any of these moves before `apply`, the plan is stale.
fn deploy_facts(repo: &Path, component: &str, env: &str, group: Option<&str>) -> Result<serde_json::Value> {
    let p = push::plan(repo, &push::Options { component, goatnet: env, only_group: group, dry_run: true, verify_only: false, ack: "" })?;
    Ok(json!({
        "members": p.resolved.members, "play": p.resolved.play.file_name().map(|x| x.to_string_lossy().to_string()),
        "target_flag": p.resolved.target_flag, "ledger": p.resolved.ledger,
        "declared": p.wants.iter().map(|(g, w)| (g.clone(), push::render_ref(w))).collect::<BTreeMap<_, _>>(),
        "pins_commit": git(repo, &["log", "-1", "--format=%H", "--", "ops/pins/fleet-pins.yaml"]),
        "transport": p.transport.detail,
    }))
}

pub struct PlanOpts<'a> {
    pub verb: &'a str,
    pub component: &'a str,
    pub env: Option<&'a str>,
    pub group: Option<&'a str>,
    pub n: u32,
    /// `rollback` only: the one goatnet to roll back (its `site:` in the ledger).
    /// A plan of a multi-goatnet component with no `--site` is refused (GB-055);
    /// the site is carried in the plan so `apply` stays narrowed to it.
    pub site: Option<&'a str>,
    pub out: &'a Path,
}

/// `plan`: write what a declarative verb would do to a file, decided now, touching no host's state.
pub fn plan(repo: &Path, mock: Option<&Path>, o: &PlanOpts) -> Result<Report> {
    let facts = match (o.verb, mock) {
        ("deploy", Some(dir)) => {
            let env = o.env.ok_or_else(|| goat_errors::coded("GB-051", "plan deploy needs --env".to_string()))?;
            let f = Fixture::open(dir)?;
            json!({ "declared": f.declared_ref(env, o.component)?, "serial": f.converger.last_serial(), "fixture": dir })
        }
        ("deploy", None) => deploy_facts(repo, o.component, o.env.ok_or_else(|| goat_errors::coded("GB-051", "plan deploy needs --env".to_string()))?, o.group)?,
        ("rollback", None) => crate::rollback::decide(repo, o.component, o.n, o.site)?,
        ("rollback", Some(_)) => return Err(goat_errors::coded("GB-051", "plan rollback reads a host ledger; the fixture has no rollback converger".to_string()).into()),
        (v, _) => return Err(goat_errors::coded("GB-051", format!("plan covers deploy and rollback; '{v}' is not planned yet")).into()),
    };
    let doc = json!({
        "schema": "goatbuilder.plan/v1", "verb": o.verb, "component": o.component, "env": o.env, "group": o.group, "n": o.n, "site": o.site,
        "main_commit": git(repo, &["rev-parse", "HEAD"]), "written_by": std::env::var("USER").unwrap_or_default(), "facts": facts,
    });
    std::fs::write(o.out, serde_json::to_string_pretty(&doc)? + "\n").with_context(|| o.out.display().to_string())?;
    let mut checks = vec![ok("plan/written", format!("{} — review it, then: goatbuilder apply {}", o.out.display(), o.out.display()))];
    if let Some(m) = doc["facts"].as_object() {
        for (k, v) in m {
            checks.push(ok(&format!("plan/{k}"), v.to_string()));
        }
    }
    Ok(Report::new("plan", checks, EXIT_REFUSED))
}

/// `apply`: run exactly the reviewed plan, refusing (exit 1, naming what moved) when
/// the declaration or the target moved since it was written.
pub fn apply(repo: &Path, mock: Option<&Path>, file: &Path) -> Result<Report> {
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file).with_context(|| file.display().to_string())?)?;
    if doc["schema"] != "goatbuilder.plan/v1" {
        return Err(goat_errors::coded("GB-052", format!("{} is not a goatbuilder plan (schema goatbuilder.plan/v1)", file.display())).into());
    }
    let verb = doc["verb"].as_str().unwrap_or("");
    let component = doc["component"].as_str().unwrap_or("").to_string();
    let env = doc["env"].as_str().map(String::from);
    let group = doc["group"].as_str().map(String::from);
    let n = doc["n"].as_u64().unwrap_or(1) as u32;
    let site = doc["site"].as_str().map(String::from);
    let saved = &doc["facts"];
    let now = match (verb, mock) {
        ("deploy", Some(dir)) => {
            let f = Fixture::open(dir)?;
            let e = env.as_deref().unwrap_or("");
            json!({ "declared": f.declared_ref(e, &component)?, "serial": f.converger.last_serial(), "fixture": dir })
        }
        ("deploy", None) => deploy_facts(repo, &component, env.as_deref().unwrap_or(""), group.as_deref())?,
        ("rollback", None) => crate::rollback::decide(repo, &component, n, site.as_deref())?,
        _ => return Err(goat_errors::coded("GB-051", format!("plan verb '{verb}' cannot be applied here")).into()),
    };
    // The facts that decide WHAT runs must not have moved; the transport may (the laptop moves).
    // `hosts` is pinned too: a plan narrowed to one goatnet's hosts must not apply
    // against a ledger whose host set has since changed under it (GB-055 guards the write).
    let moved: Vec<String> = ["declared", "pins_commit", "members", "play", "target_flag", "ledger", "serial", "target", "artifact_type", "reversible", "hosts"]
        .iter()
        .filter(|k| saved.get(**k) != now.get(**k))
        .map(|k| format!("{k}: planned {} now {}", saved.get(*k).map(|v| v.to_string()).unwrap_or_else(|| "—".into()), now.get(*k).map(|v| v.to_string()).unwrap_or_else(|| "—".into())))
        .collect();
    if !moved.is_empty() {
        return Ok(Report::new(
            "apply",
            vec![refused("apply/stale", format!("GB-053: the plan in {} no longer matches: {}", file.display(), moved.join("; ")), "write a new plan with the same `goatbuilder plan …` and review it")],
            EXIT_REFUSED,
        ));
    }
    let mut r = match (verb, mock) {
        ("deploy", Some(dir)) => {
            let mut f = Fixture::open(dir)?;
            verbs::deploy(&mut f, env.as_deref().unwrap_or(""), Some(&component), doc["written_by"].as_str().unwrap_or(""), false)?
        }
        ("deploy", None) => push::deploy(repo, &push::Options { component: &component, goatnet: env.as_deref().unwrap_or(""), only_group: group.as_deref(), dry_run: false, verify_only: false, ack: doc["written_by"].as_str().unwrap_or("") })?,
        _ => crate::rollback::roll(repo, &component, n, false, site.as_deref())?,
    };
    r.checks.insert(0, ok("apply/plan", format!("{} matches the tree; running {verb} {component}", file.display())));
    r.verb = "apply";
    Ok(r)
}

// ---------------------------------------------------------------------------
// adopt
// ---------------------------------------------------------------------------

/// A running reference in the form the pins write it, for a project on ghcr.io.
fn templated(r: &str, org: Option<&str>) -> String {
    match org {
        Some(org) => r.replacen(&format!("ghcr.io/{org}/"), "ghcr.io/{{ ghcr_org }}/", 1),
        None => r.to_string(),
    }
}

/// One proposed declaration: on `scope`, `component` runs `running`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Proposal {
    pub component: String,
    pub scope: String,
    pub running: String,
    pub declared: String,
}

/// From the diff rows, the declarations that would make main say what runs: the goatnet
/// key when every host of the goatnet agrees, else each member group whose hosts agree.
/// Hosts that disagree inside a group are reported, never guessed between.
fn proposals(env: &str, rows: &[Drift], org: Option<&str>) -> (Vec<Proposal>, Vec<String>) {
    let mut out = Vec::new();
    let mut conflicts = Vec::new();
    let mut by_comp: BTreeMap<&str, Vec<&Drift>> = BTreeMap::new();
    for d in rows.iter().filter(|d| d.state == "same" || d.state == "drift") {
        by_comp.entry(&d.component).or_default().push(d);
    }
    for (c, ds) in by_comp {
        if ds.iter().all(|d| d.state == "same") {
            continue;
        }
        let refs: std::collections::BTreeSet<&str> = ds.iter().map(|d| digest(&d.running)).collect();
        if refs.len() == 1 {
            out.push(Proposal { component: c.into(), scope: env.into(), running: templated(&ds[0].running, org), declared: ds[0].declared.clone() });
            continue;
        }
        let mut by_group: BTreeMap<&str, Vec<&Drift>> = BTreeMap::new();
        for d in &ds {
            by_group.entry(&d.group).or_default().push(d);
        }
        for (g, gd) in by_group {
            let r: std::collections::BTreeSet<&str> = gd.iter().map(|d| digest(&d.running)).collect();
            if r.len() == 1 {
                if gd[0].state == "drift" {
                    out.push(Proposal { component: c.into(), scope: g.into(), running: templated(&gd[0].running, org), declared: gd[0].declared.clone() });
                }
            } else {
                conflicts.push(format!("{c} on {g}: hosts run different refs ({}) — adopt refuses to choose", gd.iter().map(|d| format!("{}={}", d.host, short(&d.running))).collect::<Vec<_>>().join(", ")));
            }
        }
    }
    (out, conflicts)
}

/// `adopt`: declare what an environment already runs. A report by default; with
/// `--yes`, the edits written into a fresh worktree of main for review — never a deploy.
pub fn adopt(repo: &Path, env: &str, yes: bool) -> Result<Report> {
    let (rows, transport) = collect(repo, env, None)?;
    let org = crate::settings::ghcr_org(repo);
    let (props, conflicts) = proposals(env, &rows, org.as_deref());
    let mut checks = vec![ok(&format!("{}/transport", env.replace('_', "-")), format!("read via {transport}"))];
    for c in &conflicts {
        checks.push(not_ok(&format!("{}/conflict", env.replace('_', "-")), c.clone()));
    }
    let unread: Vec<String> = rows.iter().filter(|d| matches!(d.state.as_str(), "no-ledger" | "unreachable" | "other-image")).map(|d| format!("{}@{}", d.component, d.host)).collect();
    if !unread.is_empty() {
        // Adopt declares only what it read; say what it could not, never imply it is in step.
        checks.push(not_ok(&format!("{}/unread", env.replace('_', "-")), format!("{} component-host pair(s) not read (no ledger, host unreachable, or a shared ledger naming another image): {}", unread.len(), unread.join(" "))));
    }
    if props.is_empty() {
        checks.push(ok(&format!("{}/adopt", env.replace('_', "-")), "main already declares what every host it read runs; nothing to adopt".into()));
        return Ok(Report::new("adopt", checks, EXIT_REFUSED));
    }
    for p in &props {
        checks.push(ok(&format!("{}/{}", env.replace('_', "-"), p.component), format!("on {}: would declare {} (main declares {})", p.scope, short(&p.running), short(&p.declared))));
    }
    if !yes {
        checks.push(ok(&format!("{}/adopt", env.replace('_', "-")), format!("{} declaration(s) proposed; nothing written. Re-run with --yes to write them into a worktree of main for review", props.len())));
        return Ok(Report::new("adopt", checks, EXIT_REFUSED));
    }
    let stamp = push::stamp_pub();
    let base = repo.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| "goat".into());
    let wt: PathBuf = repo.join("..").join(format!("{base}-wt-adopt-{stamp}"));
    let branch = format!("chore/adopt-{}-{}", env.replace('_', "-"), stamp.to_lowercase());
    let st = Command::new("git").arg("-C").arg(repo).args(["worktree", "add", "-q", "-b", &branch]).arg(&wt).arg("origin/main").status()?;
    if !st.success() {
        return Err(goat_errors::coded("GB-054", format!("git worktree add {} failed", wt.display())).into());
    }
    let pins_path = wt.join("ops/pins/fleet-pins.yaml");
    for p in &props {
        let reason = format!("ADOPTED {} by goatbuilder adopt: the running ref on {}, read from every host's deploy ledger, declared so main says what runs. Review before landing.", stamp, p.scope);
        crate::pins::edit_set_deviation(&pins_path, &p.component, &p.scope, &p.running, &reason)?;
    }
    checks.push(ok(
        &format!("{}/adopt", env.replace('_', "-")),
        format!(
            "{} declaration(s) written to {} on branch {branch}. Review the diff, then regenerate and land: goatbuilder -C {} pins && (sign the intents) && ops/scripts/pr-land.sh --worktree {} --branch {branch} …",
            props.len(),
            pins_path.display(),
            wt.display(),
            wt.display()
        ),
    ));
    Ok(Report::new("adopt", checks, EXIT_REFUSED))
}

/// `adopt` on a fixture: the running ref is each machine's newest record; `--yes`
/// rewrites the fixture's declared/pins.yaml (the fixture is its own source of record).
pub fn adopt_mock(f: &Fixture, env: &str, yes: bool) -> Result<Report> {
    let mut rows = Vec::new();
    let comps: Vec<String> = f.config.components.keys().cloned().collect();
    for c in &comps {
        let Ok(declared) = f.declared_ref(env, c) else { continue };
        for m in f.converger.machines(env) {
            match f.converger.read_record(env, &m, c) {
                Ok(Some(r)) => {
                    let state = if r.reference == declared { "same" } else { "drift" };
                    rows.push(Drift { component: c.clone(), group: env.into(), host: m, declared: declared.clone(), running: r.reference, state: state.into(), row_ts: r.ts, action: r.action });
                }
                _ => rows.push(Drift { component: c.clone(), group: env.into(), host: m, declared: declared.clone(), running: String::new(), state: "no-ledger".into(), row_ts: String::new(), action: String::new() }),
            }
        }
    }
    let (props, conflicts) = proposals(env, &rows, f.config.repo.as_ref().and_then(|r| r.image_namespace.strip_prefix("ghcr.io/")));
    let mut checks: Vec<Check> = conflicts.iter().map(|c| not_ok(&format!("{env}/conflict"), c.clone())).collect();
    for p in &props {
        checks.push(ok(&format!("{env}/{}/{}", p.scope, p.component), format!("would declare {} (declared {})", short(&p.running), short(&p.declared))));
    }
    if props.is_empty() {
        checks.push(ok(&format!("{env}/adopt"), "the declaration already names what every machine with a record runs".into()));
    } else if yes {
        let path = f.root.join("declared/pins.yaml");
        let mut doc: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&path)?)?;
        for p in &props {
            doc["components"][p.component.as_str()][env]["ref"] = serde_yaml::Value::String(p.running.clone());
        }
        std::fs::write(&path, serde_yaml::to_string(&doc)?)?;
        checks.push(ok(&format!("{env}/adopt"), format!("{} declaration(s) written to {}", props.len(), path.display())));
    }
    Ok(Report::new("adopt", checks, EXIT_REFUSED))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EXIT_OK;

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let dst = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy_dir(&e.path(), &dst);
            } else {
                std::fs::copy(e.path(), dst).unwrap();
            }
        }
    }

    fn fixture(tag: &str) -> crate::testdir::Held<PathBuf> {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/fixtures/basic");
        let dst = crate::testdir::path(&format!("goatbuilder-inspect-{tag}-"));
        copy_dir(&src, &dst);
        dst
    }

    #[test]
    fn verify_on_the_fixture_is_status_for_one_component_and_reads_only() {
        let dir = fixture("verify");
        let f = Fixture::open(&dir).unwrap();
        let r = verify_mock(&f, "portal", "preprod").unwrap();
        assert_eq!(r.verb, "verify");
        assert_eq!(r.exit, EXIT_OK, "preprod-core-a's record already names the declared v1.3.2: {:?}", r.checks);
        let r = verify_mock(&f, "portal", "dev").unwrap();
        assert_eq!(r.exit, crate::EXIT_NOT_CONVERGED, "dev has never been deployed to v1.4.0");
    }

    #[test]
    fn diff_names_drift_against_the_declaration() {
        let dir = fixture("diff");
        let mut f = Fixture::open(&dir).unwrap();
        verbs::deploy(&mut f, "preprod", Some("portal"), "A Person", false).unwrap();
        let r = diff_mock(&f, "preprod", Some("portal")).unwrap();
        assert_eq!(r.exit, EXIT_OK, "{:?}", r.checks);
        assert!(r.checks[0].detail.starts_with("same:"), "{}", r.checks[0].detail);
        // Move the declaration: the same machine now reads as drift.
        let p = dir.join("declared/pins.yaml");
        let t = std::fs::read_to_string(&p).unwrap().replace("portal:v1.3.2@sha256:9f8e", "portal:v1.3.3@sha256:1f8e");
        std::fs::write(&p, t).unwrap();
        let f = Fixture::open(&dir).unwrap();
        let r = diff_mock(&f, "preprod", Some("portal")).unwrap();
        assert_eq!(r.exit, crate::EXIT_NOT_CONVERGED);
        assert!(r.checks[0].detail.starts_with("DRIFT: running"), "{}", r.checks[0].detail);
    }

    #[test]
    fn history_lists_every_row_newest_first() {
        let dir = fixture("history");
        let mut f = Fixture::open(&dir).unwrap();
        verbs::deploy(&mut f, "dev", Some("portal"), "A Person", false).unwrap();
        let r = history_mock(&f, "portal", "dev").unwrap();
        let rows: Vec<&str> = r.checks.iter().map(|c| c.detail.as_str()).collect();
        let core_a = rows.iter().position(|d| *d == "  (ledger @ dev-core-a)").unwrap();
        assert!(rows[core_a + 1].contains("portal:v1.4.0"), "newest first: {rows:?}");
        assert!(rows[core_a + 3].contains("portal:v1.3.2"), "then the older row: {rows:?}");
    }

    #[test]
    fn apply_runs_a_fresh_plan_and_refuses_a_stale_one() {
        let dir = fixture("plan");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let file = dir.join("deploy.plan.json");
        plan(&repo, Some(&dir), &PlanOpts { verb: "deploy", component: "portal", env: Some("preprod"), group: None, n: 1, site: None, out: &file }).unwrap();
        let r = apply(&repo, Some(&dir), &file).unwrap();
        assert_eq!(r.exit, EXIT_OK, "{:?}", r.checks);
        // A newer publication since the plan: the plan is stale.
        let r = apply(&repo, Some(&dir), &file).unwrap();
        assert_eq!(r.exit, EXIT_REFUSED, "the serial moved since the plan was written: {:?}", r.checks);
        assert!(r.checks[0].detail.contains("serial: planned"), "{}", r.checks[0].detail);
    }

    #[test]
    fn adopt_proposes_what_runs_and_writes_only_with_yes() {
        let dir = fixture("adopt");
        let f = Fixture::open(&dir).unwrap();
        // preprod-core-a's record names v1.3.2, the declaration names v1.3.2: nothing to adopt.
        let r = adopt_mock(&f, "preprod", false).unwrap();
        assert!(r.checks.last().unwrap().detail.contains("already names"), "{:?}", r.checks);
        let p = dir.join("declared/pins.yaml");
        let t = std::fs::read_to_string(&p).unwrap().replace("portal:v1.3.2@sha256:9f8e", "portal:v1.3.3@sha256:1f8e");
        std::fs::write(&p, &t).unwrap();
        let f = Fixture::open(&dir).unwrap();
        let before = std::fs::read_to_string(&p).unwrap();
        let r = adopt_mock(&f, "preprod", false).unwrap();
        assert!(r.checks.iter().any(|c| c.detail.starts_with("would declare portal:v1.3.2")), "{:?}", r.checks);
        assert_eq!(before, std::fs::read_to_string(&p).unwrap(), "no --yes: nothing written");
        adopt_mock(&f, "preprod", true).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("portal:v1.3.2@sha256:9f8e"), "--yes declares what runs");
    }

    #[test]
    fn a_shared_ledgers_other_image_is_not_drift() {
        assert_eq!(image_name("oryd/kratos:v1.3.1@sha256:fe24"), "kratos");
        assert_eq!(image_name("ghcr.io/usareur-af/kratos-selfservice-ui-node-patched:sha-1a8cbfc@sha256:953e"), "kratos-selfservice-ui-node-patched");
    }

    #[test]
    fn proposals_refuse_to_choose_between_disagreeing_hosts() {
        let d = |h: &str, r: &str| Drift { component: "c".into(), group: "g".into(), host: h.into(), declared: "x@sha256:aaaa".into(), running: r.into(), state: "drift".into(), row_ts: String::new(), action: "deploy".into() };
        let (p, c) = proposals("env", &[d("a", "x@sha256:bbbb"), d("b", "x@sha256:cccc")], None);
        assert!(p.is_empty());
        assert_eq!(c.len(), 1);
        let (p, _) = proposals("env", &[d("a", "ghcr.io/acme/x:1@sha256:bbbb"), d("b", "ghcr.io/acme/x:1@sha256:bbbb")], Some("acme"));
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].scope, "env");
        assert_eq!(p[0].running, "ghcr.io/{{ ghcr_org }}/x:1@sha256:bbbb");
    }
}

#[cfg(test)]
mod host_read_tests {
    use super::*;

    #[test]
    fn one_host_answer_carries_ledgers_goat_cli_served_tags_and_containers() {
        let text = "mgmt-a | CHANGED | rc=0 >>\n\
GBROW goat-get {\"ref\":\"nginx@sha256:11\",\"action\":\"deploy\"}\n\
GBROW herd {\"ref\":\"ghcr.io/o/herd:v0.2.1@sha256:22\",\"action\":\"deploy\"}\n\
GBGOAT goat 0.5.6\n\
GBSERVED /opt/goat-get/www/version.json {  \"version\": \"0.5.6\",  \"tag\": \"goat-cli-v0.5.6\"}\n\
GBDOCKER ok\n\
GBCTR /herd ghcr.io/o/herd:v0.2.1 sha256:img1 running\n\
GBCTR /herd-b ghcr.io/o/herd:v0.2.0 sha256:img2 exited\n\
GBIMG sha256:img1 ghcr.io/o/herd@sha256:22\n\
GBIMG sha256:img2\n\
GBEND\n\
mgmt-b | UNREACHABLE! => {}\n";
        let (tails, unreachable, facts) = parse_hosts(text, "g_hosts");
        assert_eq!(unreachable, vec!["mgmt-b".to_string()]);
        assert_eq!(tails["mgmt-a"].len(), 2);
        let h = &facts["mgmt-a"];
        assert_eq!(h.group, "g_hosts");
        assert_eq!(h.goat_cli.as_deref(), Some("0.5.6"));
        assert_eq!(h.served["goat-cli"], "goat-cli-v0.5.6");
        assert!(h.goat_get);
        let cs = h.containers.as_ref().unwrap();
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].repo_digests, vec!["ghcr.io/o/herd@sha256:22".to_string()]);
        assert_eq!(cs[1].state, "exited");
        assert!(cs[1].repo_digests.is_empty());
    }

    #[test]
    fn a_host_where_docker_cannot_be_read_has_no_container_list() {
        let (_, _, facts) = parse_hosts("mgmt-a | CHANGED | rc=0 >>\nGBEND\n", "g");
        assert_eq!(facts["mgmt-a"].containers, None);
    }

    #[test]
    fn a_group_of_placeholders_is_not_provisioned() {
        let d = tempfile::tempdir().unwrap();
        let inv = d.path().join("ops/ansible/inventory");
        std::fs::create_dir_all(&inv).unwrap();
        std::fs::write(inv.join("a.ini"), "[cloud_hosts]\ncore-a ansible_aws_ssm_instance_id=TBD_CORE_A site_name=x # note\n\n[real_hosts]\nthor ansible_host=100.127.0.46\n[mixed_hosts]\nm1 ansible_host=TBD_M1\nm2 ansible_host=10.0.0.2\n").unwrap();
        let p = placeholder_group(d.path(), "cloud_hosts").unwrap();
        assert!(p.contains("core-a") && p.contains("TBD_CORE_A") && p.contains("a.ini"), "{p}");
        assert_eq!(placeholder_group(d.path(), "real_hosts"), None);
        assert_eq!(placeholder_group(d.path(), "mixed_hosts"), None, "one real host means the group is provisioned");
        assert_eq!(placeholder_group(d.path(), "absent_hosts"), None);
    }
}

/// Gate row C3: `status` on real hosts agrees with `verify` and `diff` for every
/// machine. The hosts are a checkout whose `ops/scripts/with-aws` answers
/// `ansible` and `ansible-inventory` from one table of host states, in the
/// formats each read asks for, so all three verbs read the same machines
/// through their real code paths.
#[cfg(test)]
mod status_agreement_tests {
    use super::*;
    use std::collections::BTreeSet;

    const PORTAL: &str = "ghcr.io/zagros/portal:v1.3.2@sha256:9f8e00000000000000000000000000000000000000000000000000000000aa01";
    const PORTAL_OLD: &str = "ghcr.io/zagros/portal:v1.3.1@sha256:1d2c00000000000000000000000000000000000000000000000000000000bb02";
    const HERD: &str = "ghcr.io/zagros/herd:v0.2.1@sha256:4b5a00000000000000000000000000000000000000000000000000000000cc03";

    /// Answers the two reads the verbs make: a READ_HOST shell per group (diff,
    /// status) and a one-line `tail -n 1` of one ledger per group (verify), plus
    /// `ansible-inventory --list`. Every call is logged; nothing else runs.
    const STUB: &str = r#"#!/usr/bin/env python3
import json, os, sys
root = os.path.abspath(os.path.join(os.getcwd(), "..", ".."))
argv = sys.argv[1:]
with open(os.path.join(root, "calls.log"), "a") as f:
    f.write(json.dumps(argv) + "\n")
tool, args = argv[2], argv[3:]
world = json.load(open(os.path.join(root, "hosts.json")))
if tool == "ansible-inventory":
    print(json.dumps(world["inventory"]))
    sys.exit(0)
if tool != "ansible":
    sys.exit(9)
i, group, cmd = 0, None, ""
while i < len(args):
    a = args[i]
    if a in ("-i", "-m", "-a", "-e", "-l"):
        if a == "-a":
            cmd = args[i + 1]
        i += 2
        continue
    if a.startswith("-"):
        i += 1
        continue
    if group is None:
        group = a
    i += 1
rc = 0
for h in world["inventory"].get(group, {}).get("hosts", []):
    st = world["hosts"].get(h, {})
    if st.get("state") == "silent":
        continue
    if st.get("state") == "unreachable":
        print(h + ' | UNREACHABLE! => {"changed": false, "unreachable": true}')
        rc = 4
        continue
    if st.get("state") == "failed":
        print(h + ' | FAILED! => {"changed": false, "msg": "Missing sudo password"}')
        rc = 2
        continue
    ledgers = st.get("ledgers", {})
    if cmd.startswith("tail -n 1"):
        ledger = cmd.split("/var/lib/goat/")[1].split("/")[0]
        row = ledgers.get(ledger)
        out = json.dumps(row, separators=(",", ":")) if row is not None else "NO_LEDGER"
        print(h + " | CHANGED | rc=0 | (stdout) " + out)
    else:
        print(h + " | CHANGED | rc=0 >>")
        for l, row in ledgers.items():
            print("GBROW " + l + " " + json.dumps(row))
        print("GBEND")
sys.exit(rc)
"#;

    fn row(r: &str, action: &str) -> serde_json::Value {
        json!({"ref": r, "action": action, "ts": "2026-11-01T00:00:00Z", "prior_ref": ""})
    }

    /// One goatnet, one member group, two components, six machines: one in each
    /// state a reader has to tell apart.
    fn world() -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        for dir in ["ops/pins", "ops/scripts", "ops/ansible/inventory", "ops/ansible/group_vars/zagros_hosts"] {
            std::fs::create_dir_all(p.join(dir)).unwrap();
        }
        std::fs::write(
            p.join("ops/pins/fleet-pins.yaml"),
            "goatnets:\n  zagros:\n    members: [zagros_hosts]\ncomponents:\n  portal:\n    var: portal_image\n  herd:\n    var: herd_image\n",
        )
        .unwrap();
        std::fs::write(p.join("ops/ansible/group_vars/zagros_hosts/pins.yml"), format!("portal_image: \"{PORTAL}\"\nherd_image: \"{HERD}\"\n")).unwrap();
        for c in ["portal", "herd"] {
            std::fs::write(p.join(format!("ops/ansible/{c}-deploy.yml")), "- name: deploy\n  hosts: \"{{ target }}\"\n").unwrap();
        }
        std::fs::write(p.join("ops/ansible/inventory/mgmt-hosts.ini"), "[zagros_hosts]\n").unwrap();
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts/served-payloads.py"), p.join("ops/scripts/served-payloads.py")).unwrap();
        let stub = p.join("ops/scripts/with-aws");
        std::fs::write(&stub, STUB).unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let hosts = json!({
            // converged on both components
            "core-a": {"ledgers": {"portal": row(PORTAL, "deploy"), "herd": row(HERD, "deploy")}},
            // portal one release behind
            "core-b": {"ledgers": {"portal": row(PORTAL_OLD, "deploy"), "herd": row(HERD, "deploy")}},
            // cannot be reached
            "core-c": {"state": "unreachable"},
            // portal's play started the declared ref and never finished (action: pending)
            "core-d": {"ledgers": {"portal": row(PORTAL, "pending"), "herd": row(HERD, "deploy")}},
            // portal never deployed here
            "core-e": {"ledgers": {"herd": row(HERD, "deploy")}},
            // the inventory names it, ansible says nothing for it
            "core-f": {"state": "silent"},
            // reached, but the read itself failed (become refused): nothing was read
            "core-g": {"state": "failed"},
        });
        let all: Vec<&str> = vec!["core-a", "core-b", "core-c", "core-d", "core-e", "core-f", "core-g"];
        let w = json!({"inventory": {"zagros_hosts": {"hosts": all}, "_meta": {"hostvars": {}}}, "hosts": hosts});
        std::fs::write(p.join("hosts.json"), w.to_string()).unwrap();
        d
    }

    /// What a machine is for one component, in the three verdicts C3 compares:
    /// at the declared ref, read and not at it, or not read at all.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum V {
        Healthy,
        Drifted,
        Unreachable,
    }

    type Verdicts = BTreeMap<(String, String), V>;

    /// `verify`'s per-host rows (`zagros/<host>`), for one component.
    fn from_verify(r: &Report, component: &str, hosts: &BTreeSet<String>, into: &mut Verdicts) {
        for c in &r.checks {
            let Some(host) = c.name.strip_prefix("zagros/").filter(|h| hosts.contains(*h)) else { continue };
            let v = if c.detail.starts_with("converged on") {
                V::Healthy
            } else if c.detail.starts_with("unknown — ") || c.detail.contains("no ledger row came back") {
                V::Unreachable
            } else if c.detail.starts_with("NOT converged") {
                V::Drifted
            } else {
                panic!("verify row with no verdict: {c:?}")
            };
            assert!(into.insert((host.to_string(), component.to_string()), v).is_none(), "two verify rows for {host}/{component}");
        }
    }

    /// `diff`'s or `status`'s rows (`zagros/<host>/<component>`). A host diff or
    /// status could not read is Unreachable; one read and not at the declared
    /// ref, whatever the reason, is Drifted.
    fn from_rows(r: &Report) -> Verdicts {
        let mut out = Verdicts::new();
        for c in &r.checks {
            let parts: Vec<&str> = c.name.split('/').collect();
            if parts.len() != 3 || parts[2] == "inventory" {
                continue;
            }
            let d = c.detail.as_str();
            let v = if d.starts_with("same: ") || d.starts_with("converged on ") {
                V::Healthy
            } else if d.starts_with("unknown: host unreachable") || d.starts_with("unknown: no answer") || d.ends_with(": host unreachable") || d.contains(": no answer from this host") {
                V::Unreachable
            } else if d.starts_with("DRIFT") || d.starts_with("pending: ") || d.starts_with("unknown: ") {
                V::Drifted
            } else {
                panic!("{} row with no verdict: {c:?}", r.verb)
            };
            assert!(out.insert((parts[1].to_string(), parts[2].to_string()), v).is_none(), "two {} rows for {}", r.verb, c.name);
        }
        out
    }

    fn snapshot(p: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut todo = vec![p.to_path_buf()];
        while let Some(d) = todo.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let e = e.unwrap().path();
                if e.is_dir() {
                    todo.push(e);
                } else if e.file_name().is_some_and(|n| n != "calls.log") {
                    out.insert(e.clone(), std::fs::read(&e).unwrap());
                }
            }
        }
        out
    }

    #[test]
    fn status_agrees_with_verify_and_diff_for_every_machine() {
        let w = world();
        let repo = w.path();
        let before = snapshot(repo);
        let hosts: BTreeSet<String> = ["core-a", "core-b", "core-c", "core-d", "core-e", "core-f", "core-g"].map(String::from).into();

        // The operator's comparison (ops/goatbuilder/c3-agreement.py) reads the -o json reports.
        let out = tempfile::tempdir().unwrap();
        let save = |name: &str, r: &Report| {
            let f = out.path().join(name);
            std::fs::write(&f, serde_json::to_string(r).unwrap()).unwrap();
            f.display().to_string()
        };
        let mut by_verify = Verdicts::new();
        let mut verify_args = Vec::new();
        for c in ["portal", "herd"] {
            let r = verify(repo, c, "zagros", None, false).unwrap();
            from_verify(&r, c, &hosts, &mut by_verify);
            verify_args.extend(["--verify".to_string(), format!("{c}={}", save(&format!("verify-{c}.json"), &r))]);
        }
        let d = diff(repo, "zagros", None).unwrap();
        let diff_json = save("diff.json", &d);
        let by_diff = from_rows(&d);
        // The expected table, written out: what each machine is.
        let want: Verdicts = [
            ("core-a", "portal", V::Healthy),
            ("core-a", "herd", V::Healthy),
            ("core-b", "portal", V::Drifted),
            ("core-b", "herd", V::Healthy),
            ("core-c", "portal", V::Unreachable),
            ("core-c", "herd", V::Unreachable),
            ("core-d", "portal", V::Drifted),
            ("core-d", "herd", V::Healthy),
            ("core-e", "portal", V::Drifted),
            ("core-e", "herd", V::Healthy),
            ("core-f", "portal", V::Unreachable),
            ("core-f", "herd", V::Unreachable),
            ("core-g", "portal", V::Unreachable),
            ("core-g", "herd", V::Unreachable),
        ]
        .into_iter()
        .map(|(h, c, v)| ((h.to_string(), c.to_string()), v))
        .collect();
        assert_eq!(by_verify, want, "verify, the deploy contract's own read (C7)");
        assert_eq!(by_diff, want, "diff agrees with verify for every machine and component");
        let s = status(repo, "zagros", None).unwrap();
        assert_eq!(s.verb, "status");
        let by_status = from_rows(&s);
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/c3-agreement.py");
        let c3 = Command::new(crate::endstate::python()).arg(&script).args(["--status", &save("status.json", &s), "--diff", &diff_json]).args(&verify_args).output().unwrap();
        let said = String::from_utf8_lossy(&c3.stdout).to_string();
        assert!(c3.status.success(), "c3-agreement.py: {said}{}", String::from_utf8_lossy(&c3.stderr));
        assert!(said.contains("c3: transport: read via direct (mgmt-hosts.ini)"), "{said}");
        assert!(said.contains("agree on 7 machine(s), 2 component(s): 5 healthy, 3 drifted, 6 unreachable; reached 4 of 7 machine(s)"), "{said}");
        assert_eq!(by_status, want, "status agrees with verify and diff for every machine and component");

        // Per machine: the same set, and the same worst verdict.
        let worst = |v: &Verdicts| -> BTreeMap<String, V> {
            let mut m = BTreeMap::new();
            for ((h, _), x) in v {
                let e = m.entry(h.clone()).or_insert(*x);
                *e = (*e).max(*x);
            }
            m
        };
        assert_eq!(worst(&by_status), worst(&by_verify));
        assert_eq!(worst(&by_status), worst(&by_diff));
        assert_eq!(worst(&by_status).keys().cloned().collect::<BTreeSet<_>>(), hosts, "every machine the inventory names, none other");

        // The page main.rs prints for a person: every row in one of the three words.
        let page = verbs::status_page(&s, "zagros", false);
        assert!(page.contains("converged") && page.contains("pending") && page.contains("unknown"), "{page}");
        assert!(!page.contains("in neither"), "every row is converged, pending or unknown: {page}");
        // Not every machine is at the declared ref: status says so in its exit code.
        assert_eq!(s.exit, EXIT_NOT_CONVERGED);

        // Read-only: no file changed, and no play ran (no ansible-preflight, no playbook).
        assert_eq!(before, snapshot(repo), "status, verify and diff changed no file");
        let calls = std::fs::read_to_string(repo.join("calls.log")).unwrap();
        for l in calls.lines() {
            let argv: Vec<String> = serde_json::from_str(l).unwrap();
            assert!(argv[2] == "ansible" || argv[2] == "ansible-inventory", "only reads: {argv:?}");
            assert!(!argv.iter().any(|a| a.ends_with(".yml")), "no playbook: {argv:?}");
        }
    }

    #[test]
    fn status_on_a_goatnet_where_every_machine_converged_exits_zero() {
        let w = world();
        let p = w.path().join("hosts.json");
        let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let ok = json!({"ledgers": {"portal": row(PORTAL, "deploy"), "herd": row(HERD, "deploy")}});
        for h in ["core-a", "core-b", "core-c", "core-d", "core-e", "core-f", "core-g"] {
            v["hosts"][h] = ok.clone();
        }
        std::fs::write(&p, v.to_string()).unwrap();
        let s = status(w.path(), "zagros", None).unwrap();
        assert_eq!(s.exit, crate::EXIT_OK, "{:?}", s.checks);
        assert_eq!(s.checks.iter().filter(|c| c.detail.starts_with("converged on ")).count(), 14);
    }
}
