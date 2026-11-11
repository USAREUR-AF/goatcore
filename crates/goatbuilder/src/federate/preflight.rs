//! Phase 1 (preflight) inputs, derived from the deployment instead of exported
//! by hand (docs/design/federation-preflight.md).
//!
//! The 9 Nov run exported nineteen environment variables before phase 1. Each
//! one is a fact the deployment already holds: the peering's declaration in
//! goatbuilder.yaml `peerings:`, the peering inventory, the edge group's
//! group_vars, the project settings, and the allocated per-peering values.
//! [`derive_inputs`] reads them; [`export`] puts every derived value the
//! operator has not set into this process's environment before the plan is
//! built, so phase 1 (`preflight.py`), the commands of phases 3-14 and the
//! helpers they call all read one value. An explicitly set variable always wins,
//! which keeps the script avenue (`peering-standup.sh`, values exported by
//! hand) working unchanged.
//!
//! Nothing environment-specific is defaulted: a value no source declares stays
//! unset, and `preflight.py` names it, with where to declare it (GB-130).
//!
//! The checks themselves live in one place, `ops/federation/standup/preflight.py`
//! and `preflight_live.py`, which both avenues run as phase 1. [`CHECKS`] lists
//! them for a dry run; [`recheck_slot_anchors`] runs the slot-list check alone,
//! for the phase-10 step to call immediately before the core router restarts.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::config;
use crate::Check;

/// The checks phase 1 runs that `--only` can select, in order: (id, what it
/// checks, the code it reports when it does not pass). The same list as
/// `CHECKS` in ops/federation/standup/preflight_live.py; a test there and one
/// here compare the two.
pub const CHECKS: &[(&str, &str, &str)] = &[
    ("inputs", "every required input is present", "GB-130"),
    (
        "edge-ports",
        "the tunnel and edge router ports are free on the edge",
        "GB-131",
    ),
    (
        "core-ports",
        "the republisher metrics port is free on the core",
        "GB-132",
    ),
    (
        "slot-anchors",
        "the committed slot list equals the registry",
        "GB-134",
    ),
    (
        "core-router",
        "the core router container is found",
        "GB-138",
    ),
    (
        "ingress-stores",
        "the core's receive stores match what the edge grants",
        "GB-136",
    ),
    (
        "signing-ca",
        "our CA certificate and its signing key resolve",
        "GB-140",
    ),
];

/// The allocated per-peering values phase 1 checks: the ports and the interface
/// the plays will bind. These are fields of `alloc::PeeringValues`; until
/// allocation is wired in, the caller passes them (or `None`, and preflight.py
/// reads the group's own vars and then the roles' port defaults).
#[derive(Debug, Clone, Default)]
pub struct Allocated {
    pub wg_interface: Option<String>,
    pub wg_listen_port: Option<u16>,
    pub edge_listen_port: Option<u16>,
    pub republisher_metrics_port: Option<u16>,
    pub core_group: Option<String>,
}

/// One input a phase reads: the environment variable, its derived value, and
/// where that value came from (or, when `value` is `None`, where to declare it).
#[derive(Debug, Clone, PartialEq)]
pub struct Input {
    pub name: &'static str,
    pub value: Option<String>,
    pub source: String,
}

/// A derived value reaches phase commands that run through `bash -c` inside
/// single quotes, so a value holding a quote, `$`, a backtick, a backslash or a
/// control character is refused rather than exported: it stays unset, and
/// preflight names it.
fn shell_plain(v: &str) -> bool {
    !v.chars().any(|c| c.is_control() || "'\"`$\\".contains(c))
}

pub(crate) fn input(name: &'static str, value: Option<String>, source: impl Into<String>) -> Input {
    let source = source.into();
    match value.filter(|v| !v.is_empty()) {
        Some(v) if !shell_plain(&v) => Input {
            name,
            value: None,
            source: format!("{source} (its declared value holds a quote, $, backtick, backslash or control character, so it was not used)"),
        },
        value => Input { name, value, source },
    }
}

/// A group's own plain value: group_vars/<group>.yml, then each file in
/// group_vars/<group>/ in name order (the last that sets it wins, as Ansible
/// merges them). A templated value resolves only inside a play, so it is not a
/// value here.
pub fn group_var(repo: &Path, group: &str, key: &str) -> Option<String> {
    let files = group_files(repo, group);
    let mut found = None;
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
            continue;
        };
        let v = match doc.get(key) {
            Some(serde_yaml::Value::String(s)) if !s.contains("{{") => Some(s.clone()),
            Some(serde_yaml::Value::Bool(b)) => Some(b.to_string()),
            Some(serde_yaml::Value::Number(n)) => Some(n.to_string()),
            _ => None,
        };
        if v.is_some() {
            found = v;
        }
    }
    found
}

/// The files Ansible reads for a group's vars, in the order it merges them:
/// group_vars/<group>.yml, then each file in group_vars/<group>/ by name.
fn group_files(repo: &Path, group: &str) -> Vec<std::path::PathBuf> {
    let gv = repo.join("ops/ansible/group_vars");
    let mut files = vec![
        gv.join(format!("{group}.yml")),
        gv.join(format!("{group}.yaml")),
    ];
    if let Ok(rd) = std::fs::read_dir(gv.join(group)) {
        let mut more: Vec<_> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml"))
            .collect();
        more.sort();
        files.extend(more);
    }
    files
}

/// The host of each `tls/<host>:<port>` or `tcp/<host>:<port>` endpoint that
/// `key` holds (a string or a list of strings) in any of a group's vars files.
fn endpoint_hosts(repo: &Path, group: &str, key: &str, out: &mut Vec<String>) {
    let re = regex::Regex::new(r"^(?:tls|tcp)/([^:/#\s]+):").expect("regex");
    for f in group_files(repo, group) {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
            continue;
        };
        let items: Vec<&str> = match doc.get(key) {
            Some(serde_yaml::Value::String(e)) => vec![e.as_str()],
            Some(serde_yaml::Value::Sequence(es)) => es.iter().filter_map(|e| e.as_str()).collect(),
            _ => Vec::new(),
        };
        for e in items {
            if let Some(c) = re.captures(e) {
                push_new(out, &c[1]);
            }
        }
    }
}

fn push_new(out: &mut Vec<String>, v: &str) {
    if !v.is_empty() && !v.contains("{{") && !out.iter().any(|o| o == v) {
        out.push(v.to_string());
    }
}

/// The names the core certificate carries (FED_CORE_SAN; phase 8 mints the core
/// certificate only when it has them), each read where the deployment already
/// declares it, in this order, without duplicates:
/// 1. the addresses the edge dials: `goat_zenoh_edge_core_endpoints` in any
///    file of group_vars/<edge>/ (the hand-written deployment.yml or the
///    rendered peering-deployment.yml);
/// 2. the address the core's republisher dials, `goat_fed_republisher_core_endpoint`,
///    in group_vars/<core>/ or group_vars/all/;
/// 3. the peering declaration's `hosts: core`;
/// 4. the `ansible_host` of the one host of the core's inventory group.
///
/// preflight.py checks that every address of 1 is covered.
pub fn core_names(repo: &Path, peering: &config::Peering, edge_group: &str, core_group: &str) -> Vec<String> {
    let mut out = Vec::new();
    endpoint_hosts(repo, edge_group, "goat_zenoh_edge_core_endpoints", &mut out);
    for g in [core_group, "all"] {
        endpoint_hosts(repo, g, "goat_fed_republisher_core_endpoint", &mut out);
    }
    if let Some(h) = &peering.hosts {
        push_new(&mut out, &h.core);
    }
    if let Some((host, _)) = group_host(repo, core_group) {
        push_new(&mut out, &host);
    }
    let plain = |s: &String| s.chars().all(|c| c.is_ascii_alphanumeric() || ".-:".contains(c));
    out.retain(plain);
    out
}

/// Where [`core_names`] looks, for the message when it finds nothing.
pub fn core_names_sources(edge_group: &str, core_group: &str) -> String {
    format!(
        "goat_zenoh_edge_core_endpoints in group_vars/{edge_group}/, goat_fed_republisher_core_endpoint in group_vars/{core_group}/ or group_vars/all/, peerings[].hosts.core in goatbuilder.yaml, or the ansible_host of the one host of [{core_group}] in ops/ansible/inventory/peering-edges.ini"
    )
}

/// The one host of an inventory group in ops/ansible/inventory/peering-edges.ini:
/// its `ansible_host` (else its name) and `ansible_user`. `None` when the group
/// has no host or several: then nothing is guessed.
fn group_host(repo: &Path, group: &str) -> Option<(String, Option<String>)> {
    let text =
        std::fs::read_to_string(repo.join("ops/ansible/inventory/peering-edges.ini")).ok()?;
    let mut inside = false;
    let mut hosts = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            inside = line == format!("[{group}]");
            continue;
        }
        if inside {
            hosts.push(line.to_string());
        }
    }
    let [line] = hosts.as_slice() else {
        return None;
    };
    let mut words = line.split_whitespace();
    let mut host = words.next()?.to_string();
    let mut user = None;
    for w in words {
        if let Some(v) = w.strip_prefix("ansible_host=") {
            host = v.to_string();
        } else if let Some(v) = w.strip_prefix("ansible_user=") {
            user = Some(v.to_string());
        }
    }
    Some((host, user))
}

/// `ssh://<user>@<host>` for the one host of an inventory group, from its
/// `ansible_host` and `ansible_user` (else the project's `deploy.ssh_user`).
/// `None` when the group has no host or several: then nothing is guessed.
fn docker_host_of(repo: &Path, group: &str) -> Option<String> {
    let (host, user) = group_host(repo, group)?;
    let user = user.or_else(|| crate::settings::ssh_user(repo))?;
    let plain = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-:".contains(c))
    };
    (plain(&host) && plain(&user)).then(|| format!("ssh://{user}@{host}"))
}

/// The peering's own federation.toml, when `goatbuilder federate allocate`
/// generated it (`federation_toml: rendered`):
/// ops/federation/<environment>/peerings/<with>/federation.toml, starting with
/// the generated marker. Phase 3 then reads it with `init --declaration`, and
/// nothing is copied into the goat context.
pub fn peering_declaration(repo: &Path, peering: &config::Peering) -> Option<std::path::PathBuf> {
    let p = repo.join(peering_declaration_rel(peering));
    let text = std::fs::read_to_string(&p).ok()?;
    text.starts_with(super::peering_files::MARKER).then_some(p)
}

fn peering_declaration_rel(peering: &config::Peering) -> String {
    format!(
        "ops/federation/{}/peerings/{}/federation.toml",
        peering.environment, peering.with
    )
}

fn existing(p: std::path::PathBuf) -> Option<String> {
    p.is_file().then(|| p.display().to_string())
}

/// Every input phases 1-14 read from the environment that the deployment can
/// supply, in the order preflight.py reports them. `peering` is the declared
/// entry (C1 has already found it); `alloc` the allocated values, when known.
pub fn derive_inputs(
    repo: &Path,
    peering: &config::Peering,
    work: &str,
    edge_group: &str,
    alloc: Option<&Allocated>,
) -> Vec<Input> {
    let with = peering.with.as_str();
    let site = peering.site.as_str();
    let core_group = alloc
        .and_then(|a| a.core_group.clone())
        .unwrap_or_else(|| format!("{with}_core"));
    let gv = |k: &str| group_var(repo, edge_group, k);
    let at = |k: &str| format!("group_vars/{edge_group}: {k}");
    let mut v = vec![
        input(
            "FED_WORK",
            Some(work.to_string()),
            "--work, else $FED_WORK, else /var/lib/goat-federation/<peering>",
        ),
        input(
            "FED_COUNTERPARTY_ID",
            Some(with.to_string()),
            "goatbuilder.yaml peerings: with",
        ),
        input(
            "FED_CORE_GROUP",
            Some(core_group.clone()),
            "the allocated core group, else <with>_core",
        ),
        input(
            "FED_SLOT_ENV",
            Some(peering.environment.clone()),
            "goatbuilder.yaml peerings: environment",
        ),
        input(
            "GOAT_AWS_PROFILE",
            Some(crate::settings::aws_profile(repo)),
            "goatbuilder.yaml deploy.aws_profile, else default",
        ),
    ];
    let a = alloc.cloned().unwrap_or_default();
    v.push(input(
        "FED_WG_INTERFACE",
        a.wg_interface,
        "allocated wg_interface",
    ));
    v.push(input(
        "FED_WG_LISTEN_PORT",
        a.wg_listen_port.map(|p| p.to_string()),
        "allocated wg_listen_port",
    ));
    v.push(input(
        "FED_EDGE_LISTEN_PORT",
        a.edge_listen_port.map(|p| p.to_string()),
        "allocated edge_listen_port",
    ));
    v.push(input(
        "FED_REPUBLISHER_METRICS_PORT",
        a.republisher_metrics_port.map(|p| p.to_string()),
        "allocated republisher_metrics_port",
    ));
    v.push(input(
        "FED_SIGNING_KEY",
        gv("goat_federation_signing_key"),
        at("goat_federation_signing_key"),
    ));
    let no_dir = gv("goat_federation_no_directory")
        .filter(|b| b == "true")
        .map(|_| "1".to_string());
    v.push(input(
        "FED_NO_DIRECTORY",
        no_dir,
        at("goat_federation_no_directory"),
    ));
    v.push(input(
        "FED_DIRECTORY_URL",
        gv("goat_federation_directory_url"),
        at("goat_federation_directory_url"),
    ));
    v.push(input(
        "FED_PACKAGE_URL",
        gv("goat_federation_package_url"),
        at("goat_federation_package_url"),
    ));
    // Our CA, the way the deployment already declares it for the portal role
    // (portal_ca_root_src, portal_ca_key_src, portal_age_identity_src), unless
    // the edge group names another.
    let (ca_cert, ca_src) = match gv("goat_federation_ca_cert") {
        Some(c) => (Some(c), at("goat_federation_ca_cert")),
        None => (
            existing(repo.join(format!("ops/ansible/enrollment/ca/{site}/root-cert.pem"))),
            format!("ops/ansible/enrollment/ca/{site}/root-cert.pem"),
        ),
    };
    v.push(input("FED_CA_CERT", ca_cert, ca_src));
    v.push(input(
        "FED_CA_KEY",
        gv("goat_federation_ca_key"),
        at("goat_federation_ca_key"),
    ));
    let state = crate::settings::state_dir_abs(repo);
    let (age_key, age_src) = match gv("goat_federation_ca_key_age") {
        Some(k) => (Some(k), at("goat_federation_ca_key_age")),
        None => (
            existing(Path::new(&state).join(format!("{site}-ca/ca.key.age"))),
            format!("<deploy.state_dir>/{site}-ca/ca.key.age"),
        ),
    };
    v.push(input("FED_CA_KEY_AGE", age_key, age_src));
    let home = std::env::var("HOME").unwrap_or_default();
    let (ident, ident_src) = match gv("goat_federation_age_identity") {
        Some(i) => (Some(i), at("goat_federation_age_identity")),
        None => (
            existing(Path::new(&home).join(".config/age/keys.txt")),
            "~/.config/age/keys.txt (the portal role's identity)".to_string(),
        ),
    };
    v.push(input("FED_AGE_IDENTITY", ident, ident_src));
    // Phase 8 opens the age-encrypted key only when the deployment opts in.
    let from_age = gv("goat_federation_ca_key_from_age")
        .filter(|b| b == "true")
        .map(|_| "1".to_string());
    v.push(input(
        "FED_CA_KEY_FROM_AGE",
        from_age,
        at("goat_federation_ca_key_from_age (true opts phase 8 in to opening the age-encrypted CA key)"),
    ));
    let sans = core_names(repo, peering, edge_group, &core_group);
    v.push(input(
        "FED_CORE_SAN",
        (!sans.is_empty()).then(|| sans.join(",")),
        core_names_sources(edge_group, &core_group),
    ));
    // The core certificate's CN: the name the edge pins for its core, from the
    // one core-identity function (FED_CORE_CN in the environment overrides it).
    let (core_cn, cn_src) =
        super::site_values::phase8_core_cn(repo, peering, edge_group, &core_group);
    v.push(input("FED_CORE_CN", core_cn, cn_src));
    let decl = peering_declaration(repo, peering);
    v.push(input(
        "FED_DECLARATION",
        decl.map(|p| p.display().to_string()),
        format!(
            "{} (generated by goatbuilder federate allocate for federation_toml: rendered; else phase 3 reads the goat context's federation.toml)",
            peering_declaration_rel(peering)
        ),
    ));
    v.push(input(
        "FED_ROUTER_DOCKER_HOST",
        docker_host_of(repo, &core_group),
        format!("the one host of inventory group [{core_group}] (ansible_host, ansible_user)"),
    ));
    v
}

/// The derived values the operator has not set, as (name, value).
pub fn unset_derived(
    inputs: &[Input],
    env: impl Fn(&str) -> Option<String>,
) -> Vec<(&'static str, String)> {
    inputs
        .iter()
        .filter(|i| env(i.name).map(|v| v.is_empty()).unwrap_or(true))
        .filter_map(|i| i.value.clone().map(|v| (i.name, v)))
        .collect()
}

/// Put every derived value the operator has not set into this process's
/// environment, so the plan's commands, phase 1 and every helper read it.
/// Called only for a real run: a dry run and a mock change nothing, and a dry
/// run's commands stay word for word the script's (the parity gate).
pub fn export(inputs: &[Input]) {
    for (k, v) in unset_derived(inputs, |k| std::env::var(k).ok()) {
        std::env::set_var(k, v);
    }
}

/// The dry run's rows: each phase-1 check, then each input with its value and
/// source, marked as what a real run would export or what the operator set.
pub fn plan_rows(inputs: &[Input]) -> Vec<Check> {
    let mut rows: Vec<Check> = CHECKS
        .iter()
        .map(|(id, what, code)| Check {
            name: "preflight-check".into(),
            ok: true,
            detail: format!("{id}: {what} ({code})"),
            fix: None,
        })
        .collect();
    for i in inputs {
        let set = std::env::var(i.name).ok().filter(|v| !v.is_empty());
        let detail = match (&set, &i.value) {
            (Some(v), _) => format!("{}={v}  (set in the environment; it wins)", i.name),
            (None, Some(v)) => format!("{}={v}  (from {})", i.name, i.source),
            (None, None) => format!("{} not derived; declare {}", i.name, i.source),
        };
        rows.push(Check {
            name: "preflight-input".into(),
            ok: true,
            detail,
            fix: None,
        });
    }
    rows
}

/// The outcome of one re-run of the slot-list check.
// The phase-10 restart step calls this; it is wired by that step's change.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct SlotRecheck {
    /// PASS, FAIL, BLOCKED, SKIP or NOT APPLICABLE.
    pub outcome: String,
    pub detail: String,
    /// GB-134 (drift), GB-135 (registry unreadable) or GB-130 (input missing).
    pub code: Option<String>,
}

// The phase-10 restart step calls this; it is wired by that step's change.
#[allow(dead_code)]
impl SlotRecheck {
    /// Whether the restart may go ahead: the list matches, or this deployment
    /// keeps no lazy slot storage at all.
    pub fn allows_restart(&self) -> bool {
        self.outcome == "PASS" || self.outcome == "NOT APPLICABLE"
    }
}

/// The slot-list check alone, for the phase-10 step to call immediately before
/// the core router restarts: the 9 Nov run fixed a 316/461 drift, and a slot
/// minted between the fix and the restart drifted it again (461/462). Runs
/// `preflight.py <peering> --work <work> --only slot-anchors` with this
/// process's environment (call after [`export`]) and reads its result file.
// The phase-10 restart step calls this; it is wired by that step's change.
#[allow(dead_code)]
pub fn recheck_slot_anchors(repo: &Path, peering: &str, work: &str) -> Result<SlotRecheck> {
    let script = repo.join("ops/federation/standup/preflight.py");
    // Its own result file, removed first: a stale answer (or the full phase-1
    // result) must never stand in for this run's.
    let result = Path::new(work).join("preflight/slot-recheck.json");
    let _ = std::fs::remove_file(&result);
    let status = Command::new("python3")
        .arg(&script)
        .args([
            peering,
            "--work",
            work,
            "--only",
            "slot-anchors",
            "--result",
        ])
        .arg(&result)
        .current_dir(repo)
        .status()
        .with_context(|| format!("python3 {}", script.display()))?;
    let r = read_slot_result(&result)?;
    anyhow::ensure!(
        status.success() == r.allows_restart(),
        "the slot recheck exited {status} but its result says {}; not trusting either",
        r.outcome
    );
    Ok(r)
}

// The phase-10 restart step calls this; it is wired by that step's change.
#[allow(dead_code)]
fn read_slot_result(path: &Path) -> Result<SlotRecheck> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("no preflight result at {}", path.display()))?;
    let doc: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("{} is not JSON", path.display()))?;
    let r = doc["results"]
        .as_array()
        .and_then(|rs| rs.iter().find(|r| r["check"] == "slot-anchors"))
        .with_context(|| format!("{} has no slot-anchors result", path.display()))?;
    Ok(SlotRecheck {
        outcome: r["outcome"].as_str().unwrap_or("").to_string(),
        detail: r["detail"].as_str().unwrap_or("").to_string(),
        code: r["code"]
            .as_str()
            .filter(|c| !c.is_empty())
            .map(String::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// From its declared fields, as the config loader reads it: a field added
    /// to the declaration later takes its default here too.
    fn peering() -> config::Peering {
        serde_json::from_value(serde_json::json!({
            "with": "xab",
            "environment": "zagros",
            "site": "alpha",
            "edge_class": "edge",
        }))
        .expect("a peering declaration")
    }

    fn get<'a>(v: &'a [Input], name: &str) -> &'a Input {
        v.iter().find(|i| i.name == name).unwrap()
    }

    /// A deployment with nothing declared derives only what the declaration
    /// itself says; every deployment fact stays unset and names its source.
    #[test]
    fn nothing_deployment_specific_is_defaulted() {
        let repo = tempfile::tempdir().unwrap();
        let v = derive_inputs(repo.path(), &peering(), "/w", "xab_edge", None);
        assert_eq!(get(&v, "FED_COUNTERPARTY_ID").value.as_deref(), Some("xab"));
        assert_eq!(get(&v, "FED_SLOT_ENV").value.as_deref(), Some("zagros"));
        assert_eq!(get(&v, "FED_CORE_GROUP").value.as_deref(), Some("xab_core"));
        for n in [
            "FED_SIGNING_KEY",
            "FED_DIRECTORY_URL",
            "FED_CA_CERT",
            "FED_CA_KEY",
            "FED_CORE_SAN",
            "FED_CORE_CN",
            "FED_ROUTER_DOCKER_HOST",
            "FED_WG_LISTEN_PORT",
        ] {
            assert_eq!(get(&v, n).value, None, "{n} was defaulted");
            assert!(!get(&v, n).source.is_empty(), "{n} names no source");
        }
    }

    /// The deployment's own declarations supply the rest: group_vars, the
    /// inventory, the dialled endpoints, the portal's CA, the allocation.
    #[test]
    fn the_deployment_supplies_what_was_exported_by_hand() {
        let repo = tempfile::tempdir().unwrap();
        let r = repo.path();
        std::fs::create_dir_all(r.join("ops/ansible/group_vars/xab_edge")).unwrap();
        std::fs::write(
            r.join("ops/ansible/group_vars/xab_edge/federation.yml"),
            "goat_federation_signing_key: /etc/goat-federation/signing.key\ngoat_federation_directory_url: https://directory.example.org\ngoat_federation_ca_key: '{{ vault_ca_key }}'\n",
        )
        .unwrap();
        std::fs::write(r.join("ops/ansible/group_vars/xab_edge/deployment.yml"), "goat_zenoh_edge_core_endpoints:\n  - \"tls/192.0.2.20:7447\"\n  - \"tls/core.example.org:7447\"\n").unwrap();
        std::fs::create_dir_all(r.join("ops/ansible/inventory")).unwrap();
        std::fs::write(
            r.join("ops/ansible/inventory/peering-edges.ini"),
            "[xab_core]\ncore-1 ansible_host=192.0.2.20 ansible_user=goat\n",
        )
        .unwrap();
        std::fs::create_dir_all(r.join("ops/ansible/enrollment/ca/alpha")).unwrap();
        std::fs::write(r.join("ops/ansible/enrollment/ca/alpha/root-cert.pem"), "x").unwrap();
        let alloc = Allocated {
            wg_listen_port: Some(51830),
            wg_interface: Some("wg-fed1".into()),
            ..Default::default()
        };
        let v = derive_inputs(r, &peering(), "/w", "xab_edge", Some(&alloc));
        assert_eq!(
            get(&v, "FED_SIGNING_KEY").value.as_deref(),
            Some("/etc/goat-federation/signing.key")
        );
        assert_eq!(
            get(&v, "FED_DIRECTORY_URL").value.as_deref(),
            Some("https://directory.example.org")
        );
        assert_eq!(
            get(&v, "FED_CA_KEY").value,
            None,
            "a templated value is not a value"
        );
        assert_eq!(
            get(&v, "FED_CORE_SAN").value.as_deref(),
            Some("192.0.2.20,core.example.org")
        );
        assert_eq!(
            get(&v, "FED_ROUTER_DOCKER_HOST").value.as_deref(),
            Some("ssh://goat@192.0.2.20")
        );
        assert!(get(&v, "FED_CA_CERT")
            .value
            .as_deref()
            .unwrap()
            .ends_with("enrollment/ca/alpha/root-cert.pem"));
        assert_eq!(
            get(&v, "FED_WG_LISTEN_PORT").value.as_deref(),
            Some("51830")
        );
        assert_eq!(
            get(&v, "FED_WG_INTERFACE").value.as_deref(),
            Some("wg-fed1")
        );
    }

    /// The core certificate's names come from where the deployment declares
    /// them: the edge's endpoints in any group_vars file (the rendered
    /// peering-deployment.yml too), the core's republisher endpoint, the
    /// declaration's hosts, then the core inventory host; once each, in order.
    #[test]
    fn the_core_names_are_read_from_every_declaration_that_holds_one() {
        let repo = tempfile::tempdir().unwrap();
        let r = repo.path();
        let gv = r.join("ops/ansible/group_vars");
        std::fs::create_dir_all(gv.join("xab_edge")).unwrap();
        std::fs::create_dir_all(gv.join("xab_core")).unwrap();
        std::fs::create_dir_all(gv.join("all")).unwrap();
        std::fs::write(
            gv.join("xab_edge/peering-deployment.yml"),
            "goat_zenoh_edge_core_endpoints:\n  - \"tls/core.example.org:7447#root_ca_certificate_file=/x/our-ca.pem\"\n  - \"tls/192.0.2.20:7447\"\n",
        )
        .unwrap();
        std::fs::write(
            gv.join("xab_core/peering-deployment.yml"),
            "goat_fed_republisher_core_endpoint: \"tls/192.0.2.20:7447\"\n",
        )
        .unwrap();
        std::fs::write(
            gv.join("all/site.yml"),
            "goat_fed_republisher_core_endpoint: \"tls/{{ core_host }}:7447\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(r.join("ops/ansible/inventory")).unwrap();
        std::fs::write(
            r.join("ops/ansible/inventory/peering-edges.ini"),
            "[xab_core]\ncore-1 ansible_host=192.0.2.21\n",
        )
        .unwrap();
        let mut p = peering();
        p.hosts = Some(config::PeeringHosts {
            edge: "192.0.2.30".into(),
            core: "192.0.2.20".into(),
            user: None,
        });
        assert_eq!(
            core_names(r, &p, "xab_edge", "xab_core"),
            ["core.example.org", "192.0.2.20", "192.0.2.21"]
        );
        let v = derive_inputs(r, &p, "/w", "xab_edge", None);
        assert_eq!(
            get(&v, "FED_CORE_SAN").value.as_deref(),
            Some("core.example.org,192.0.2.20,192.0.2.21")
        );
        // Only the inventory knows the core: that is the name.
        std::fs::remove_dir_all(&gv).unwrap();
        assert_eq!(core_names(r, &peering(), "xab_edge", "xab_core"), ["192.0.2.21"]);
        // Nothing declares one: unset, and the source names every place looked.
        std::fs::remove_dir_all(r.join("ops/ansible/inventory")).unwrap();
        let v = derive_inputs(r, &peering(), "/w", "xab_edge", None);
        let san = get(&v, "FED_CORE_SAN");
        assert_eq!(san.value, None);
        for place in [
            "group_vars/xab_edge/",
            "group_vars/xab_core/",
            "peerings[].hosts.core",
            "[xab_core]",
        ] {
            assert!(san.source.contains(place), "{}", san.source);
        }
    }

    /// Phase 3's declaration is the peering's generated federation.toml when
    /// allocate wrote one; a file there without the generated marker is not it.
    #[test]
    fn the_peering_declaration_is_the_generated_file_only() {
        let repo = tempfile::tempdir().unwrap();
        let r = repo.path();
        let v = derive_inputs(r, &peering(), "/w", "xab_edge", None);
        assert_eq!(get(&v, "FED_DECLARATION").value, None);
        assert!(get(&v, "FED_DECLARATION")
            .source
            .contains("ops/federation/zagros/peerings/xab/federation.toml"));
        let f = r.join("ops/federation/zagros/peerings/xab/federation.toml");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, "sovereign_id = \"xaa\"\n").unwrap();
        assert_eq!(peering_declaration(r, &peering()), None);
        std::fs::write(
            &f,
            format!("{}: x\nsovereign_id = \"xaa\"\n", crate::federate::peering_files::MARKER),
        )
        .unwrap();
        assert_eq!(peering_declaration(r, &peering()), Some(f.clone()));
        let v = derive_inputs(r, &peering(), "/w", "xab_edge", None);
        assert_eq!(
            get(&v, "FED_DECLARATION").value.as_deref(),
            Some(f.display().to_string().as_str())
        );
    }

    /// A declared value that would break out of the phase commands' quoting is
    /// not exported.
    #[test]
    fn a_value_with_shell_characters_is_not_derived() {
        let i = input(
            "FED_SIGNING_KEY",
            Some("/k'; touch /tmp/x; '".into()),
            "group_vars/xab_edge: goat_federation_signing_key",
        );
        assert_eq!(i.value, None);
        assert!(i.source.contains("was not used"), "{}", i.source);
        assert_eq!(
            input(
                "FED_CA_CERT",
                Some("/etc/goat ca/root-cert.pem".into()),
                "x"
            )
            .value
            .as_deref(),
            Some("/etc/goat ca/root-cert.pem")
        );
    }

    /// Two core hosts: which one the router runs on is not guessed.
    #[test]
    fn an_ambiguous_core_group_derives_no_docker_host() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join("ops/ansible/inventory")).unwrap();
        std::fs::write(
            repo.path().join("ops/ansible/inventory/peering-edges.ini"),
            "[xab_core]\ncore-1 ansible_user=goat\ncore-2 ansible_user=goat\n",
        )
        .unwrap();
        assert_eq!(docker_host_of(repo.path(), "xab_core"), None);
    }

    /// An explicitly set variable wins over a derived one (the script avenue).
    #[test]
    fn an_explicit_variable_overrides_a_derived_one() {
        let inputs = vec![
            input("FED_SIGNING_KEY", Some("/derived".into()), "x"),
            input("FED_CA_CERT", Some("/derived-ca".into()), "y"),
            input("FED_CA_KEY", None, "z"),
        ];
        let set = |k: &str| (k == "FED_SIGNING_KEY").then(|| "/mine".to_string());
        assert_eq!(
            unset_derived(&inputs, set),
            vec![("FED_CA_CERT", "/derived-ca".to_string())]
        );
    }

    #[test]
    fn plan_rows_list_every_check_and_input() {
        let rows = plan_rows(&[input(
            "FED_GB_TEST_UNSET_INPUT",
            None,
            "group_vars/xab_edge: k",
        )]);
        assert_eq!(
            rows.iter().filter(|r| r.name == "preflight-check").count(),
            CHECKS.len()
        );
        assert!(
            rows.iter().any(|r| r.detail
                == "FED_GB_TEST_UNSET_INPUT not derived; declare group_vars/xab_edge: k"),
            "{rows:?}"
        );
    }

    /// The same check list as preflight_live.py, id for id and code for code.
    #[test]
    fn the_check_list_matches_the_python_checks() {
        let py = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../ops/federation/standup/preflight_live.py"),
        )
        .unwrap();
        let re = regex::Regex::new(r#"\("([a-z-]+)", "[^"]+", "(GB-\d{3})"\)"#).unwrap();
        let theirs: Vec<(String, String)> = re
            .captures_iter(&py)
            .map(|c| (c[1].to_string(), c[2].to_string()))
            .collect();
        let ours: Vec<(String, String)> = CHECKS
            .iter()
            .map(|(i, _, c)| (i.to_string(), c.to_string()))
            .collect();
        assert_eq!(ours, theirs);
    }

    #[test]
    fn a_slot_result_is_read_and_graded() {
        let work = tempfile::tempdir().unwrap();
        let path = work.path().join("slot-recheck.json");
        let write = |outcome: &str, code: &str| {
            std::fs::write(
                &path,
                format!(r#"{{"schema":"goat-federation-preflight/v1","results":[{{"check":"slot-anchors","outcome":"{outcome}","detail":"live slots 462 · committed 461","code":"{code}"}}]}}"#),
            )
            .unwrap();
        };
        write("FAIL", "GB-134");
        let r = read_slot_result(&path).unwrap();
        assert_eq!(r.code.as_deref(), Some("GB-134"));
        assert!(!r.allows_restart());
        write("NOT APPLICABLE", "");
        assert!(read_slot_result(&path).unwrap().allows_restart());
        write("SKIP", "");
        assert!(
            !read_slot_result(&path).unwrap().allows_restart(),
            "a skip is not a pass"
        );
    }

    /// Opening the age-encrypted CA key in phase 8 is opt-in: derived only
    /// from goat_federation_ca_key_from_age: true, otherwise left unset.
    #[test]
    fn opening_the_ca_key_from_age_is_opt_in() {
        let repo = tempfile::tempdir().unwrap();
        let v = derive_inputs(repo.path(), &peering(), "/w", "xab_edge", None);
        assert_eq!(get(&v, "FED_CA_KEY_FROM_AGE").value, None);
        let gv = repo.path().join("ops/ansible/group_vars/xab_edge");
        std::fs::create_dir_all(&gv).unwrap();
        std::fs::write(gv.join("fed.yml"), "goat_federation_ca_key_from_age: true\n").unwrap();
        let v = derive_inputs(repo.path(), &peering(), "/w", "xab_edge", None);
        assert_eq!(get(&v, "FED_CA_KEY_FROM_AGE").value.as_deref(), Some("1"));
    }
}
