//! Phase 3's measurements, made from the operator's machine.
//!
//! `goat federate init` measures the host it runs on: the federation interface,
//! the transports, the PQC layer, and (through `DOCKER_HOST`) the core router.
//! Run that way, the operator copies the `goat` binary and a signing key to the
//! edge, gives the edge Docker access to the core, and copies the package back.
//!
//! Here the same derivations (`goat_federation::derive`) read the edge and the
//! core over SSH instead, reaching each host exactly as the plays do: the
//! inventory the plays are given (`inventory/peering-edges.ini`, read with
//! `ansible-inventory` so group_vars and host_vars apply), its address, user,
//! port, key and SSH options, and its privilege escalation. Nothing is copied
//! to either host. The package is signed where this runs, with a key resolved
//! here.
//!
//! The privilege boundary is derive's: each command sent to a host is checked
//! against `derive::is_read_only_probe` first, and anything else is refused
//! before SSH is started. Phase 3 reads; it never writes on a host.
//!
//! The core router is chosen in this order: the operator's `--router-container`;
//! the name the deployment's own variables give (the goat-zenoh role names its
//! container `{{ zenoh_stack }}-router`, and `zenoh_stack` defaults to
//! `{{ site_id }}-zenoh`); otherwise the discovery of #239, run on the core host.
//! A named router is never replaced by a discovered one.
//!
//! Refusals carry catalog codes (ops/errors/catalog.yaml): GB-110 the inventory
//! does not name a host this can reach, GB-111 SSH to a host failed, GB-112
//! the host wants a sudo password, GB-113 no core router to measure.
//!
//! docs/design/federation-router-discovery.md is the design.

use std::path::Path;
use std::process::Command;

use anyhow::Result;
use goat_federation::derive::{self, Measured, Probe, RouterTarget};
use serde_json::Value;

/// The inventory the peering plays are given (relative to ops/ansible).
pub const INVENTORY: &str = "inventory/peering-edges.ini";

/// How the operator names the core router by hand.
pub const ROUTER_FLAG: &str = "--router-container";

/// The container runtimes asked on the core host, in order.
const RUNTIMES: &[&str] = &["docker", "podman"];

/// One host, as the plays reach it.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// The inventory hostname.
    pub name: String,
    /// `ansible_host`, else the inventory hostname.
    pub address: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub key: Option<String>,
    /// `ansible_ssh_common_args` then `ansible_ssh_extra_args`, split as a
    /// shell would (a jump host is `-o ProxyJump=…` here).
    pub ssh_args: Vec<String>,
    /// `sudo -n` before each read. Ansible's `become`; ops/ansible/ansible.cfg
    /// sets it for every play, so it is on unless the host turns it off.
    pub become_root: Option<String>,
    /// `ansible_connection=local`: this machine is the host.
    pub local: bool,
    /// Every variable the inventory gives the host (group_vars and host_vars).
    pub vars: Value,
}

impl Target {
    /// `user@address`, or the address.
    pub fn destination(&self) -> String {
        match &self.user {
            Some(u) => format!("{u}@{}", self.address),
            None => self.address.clone(),
        }
    }
}

fn coded(code: &str, msg: String) -> anyhow::Error {
    goat_errors::coded(code, msg).into()
}

// ---------------------------------------------------------------------------
// The inventory
// ---------------------------------------------------------------------------

/// `ansible-inventory --list` over the plays' inventory, with the playbook
/// directory's group_vars and host_vars applied, as a play sees them.
pub fn read_inventory(repo: &Path, inventories: &[&str]) -> Result<Value> {
    let mut args: Vec<String> = inventories
        .iter()
        .flat_map(|i| ["-i".to_string(), i.to_string()])
        .collect();
    args.extend(["--playbook-dir".into(), ".".into(), "--list".into()]);
    let shown = format!("cd ops/ansible && ansible-inventory {}", args.join(" "));
    let out = crate::push::ansible_cmd(repo, "ansible-inventory", &args)
        .output()
        .map_err(|e| coded("GB-110", format!("could not run ansible-inventory ({e}); phase 3 reads the hosts from the inventory the plays use: {shown}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(coded(
            "GB-110",
            format!(
                "ansible-inventory refused the plays' inventory (exit {}): {}; ran: {shown}",
                out.status.code().unwrap_or(1),
                err.trim()
            ),
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|_| {
        coded(
            "GB-110",
            format!("ansible-inventory printed something that is not JSON; ran: {shown}"),
        )
    })
}

/// The hosts of `group` and of its child groups, as a play's `hosts:` sees them.
fn group_hosts(inv: &Value, group: &str) -> Vec<String> {
    let strs = |v: Option<&Value>| -> Vec<String> {
        v.and_then(|h| h.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let (mut hosts, mut seen, mut todo) = (
        Vec::<String>::new(),
        std::collections::HashSet::new(),
        vec![group.to_string()],
    );
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
    hosts
}

fn truthy(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Some(true),
            "false" | "no" | "off" | "0" => Some(false),
            _ => None,
        },
        Value::Number(n) => n.as_i64().map(|n| n != 0),
        _ => None,
    }
}

/// The one host of `group` that phase 3 measures. `pick` names an inventory
/// host when the group has several; a group of several with no pick is
/// refused rather than guessed.
pub fn target_in(inv: &Value, group: &str, pick: Option<&str>, flag: &str) -> Result<Target> {
    let hosts = group_hosts(inv, group);
    let name = match (pick, hosts.as_slice()) {
        (Some(p), _) if hosts.iter().any(|h| h == p) => p.to_string(),
        (Some(p), _) => return Err(coded("GB-110", format!("{flag} {p}: not a host of inventory group {group} in {INVENTORY} (it has: {})", if hosts.is_empty() { "none".into() } else { hosts.join(", ") }))),
        (None, [one]) => one.clone(),
        (None, []) => return Err(coded("GB-110", format!("inventory group {group} has no host in {INVENTORY}; phase 3 reaches the host the plays reach, so the inventory must name it"))),
        (None, many) => return Err(coded("GB-110", format!("inventory group {group} has {} hosts ({}); name the one to measure with {flag} <host>", many.len(), many.join(", ")))),
    };
    let vars = inv
        .get("_meta")
        .and_then(|m| m.get("hostvars"))
        .and_then(|h| h.get(&name))
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let s = |k: &str| {
        vars.get(k).and_then(|v| match v {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    };
    let local = match s("ansible_connection").as_deref() {
        None | Some("ssh") | Some("smart") | Some("paramiko") => false,
        Some("local") => true,
        Some(other) => return Err(coded("GB-110", format!("inventory host {name} uses ansible_connection={other}; phase 3 measures a host over ssh, or this machine with ansible_connection=local"))),
    };
    let address = s("ansible_host").unwrap_or_else(|| name.clone());
    if address.starts_with('-') || address.chars().any(|c| c.is_whitespace()) {
        return Err(coded(
            "GB-110",
            format!(
                "inventory host {name} has ansible_host `{address}`, which is not a host address"
            ),
        ));
    }
    let user = s("ansible_user").or_else(|| s("ansible_ssh_user"));
    if user.as_deref().is_some_and(|u| {
        u.starts_with('-') || u.contains('@') || u.chars().any(|c| c.is_whitespace())
    }) {
        return Err(coded(
            "GB-110",
            format!(
                "inventory host {name} has ansible_user `{}`, which is not a user name",
                user.unwrap_or_default()
            ),
        ));
    }
    let port = match s("ansible_port").or_else(|| s("ansible_ssh_port")) {
        Some(p) => Some(p.parse::<u16>().map_err(|_| {
            coded(
                "GB-110",
                format!("inventory host {name} has ansible_port `{p}`, which is not a port"),
            )
        })?),
        None => None,
    };
    let mut ssh_args = Vec::new();
    for k in ["ansible_ssh_common_args", "ansible_ssh_extra_args"] {
        if let Some(a) = s(k) {
            ssh_args.extend(shlex::split(&a).ok_or_else(|| {
                coded(
                    "GB-110",
                    format!("inventory host {name}: {k} does not split as shell words: {a}"),
                )
            })?);
        }
    }
    let become_on = vars.get("ansible_become").and_then(truthy).unwrap_or(true);
    let become_root = become_on.then(|| s("ansible_become_user").unwrap_or_else(|| "root".into()));
    Ok(Target {
        name,
        address,
        user,
        port,
        key: s("ansible_ssh_private_key_file").or_else(|| s("ansible_private_key_file")),
        ssh_args,
        become_root,
        local,
        vars,
    })
}

// ---------------------------------------------------------------------------
// The core router
// ---------------------------------------------------------------------------

/// Which core router phase 3 measures, and why that one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouterChoice {
    /// The operator named it.
    Explicit(String),
    /// The deployment's own variables name it.
    Declared { name: String, from: String },
    /// Nothing names it: discover it on the core host. `why` says what was
    /// looked for first.
    Discover { why: String },
}

impl RouterChoice {
    pub fn target(&self) -> RouterTarget<'_> {
        match self {
            RouterChoice::Explicit(name) => RouterTarget::Named {
                name,
                from: ROUTER_FLAG,
            },
            RouterChoice::Declared { name, from } => RouterTarget::Named { name, from },
            RouterChoice::Discover { .. } => RouterTarget::Discover,
        }
    }
}

fn container_name(v: &str) -> bool {
    !v.is_empty()
        && v.starts_with(|c: char| c.is_ascii_alphanumeric())
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
}

/// `{{ site_id }}` (any spacing) replaced by `site_id`; `None` when a template
/// is left that this cannot resolve.
fn resolve_site_id(text: &str, site_id: Option<&str>) -> Option<String> {
    let re = regex::Regex::new(r"\{\{\s*site_id\s*\}\}").expect("regex");
    let out = match site_id {
        Some(id) => re.replace_all(text, regex::NoExpand(id)).to_string(),
        None if re.is_match(text) => return None,
        None => text.to_string(),
    };
    (!out.contains("{{") && !out.contains("{%")).then_some(out)
}

/// The precedence: the operator's `--router-container`, then the core host's
/// variables, then discovery. The goat-zenoh role names its container
/// `{{ zenoh_stack }}-router` and defaults `zenoh_stack` to
/// `{{ site_id }}-zenoh` (roles/goat-zenoh/defaults/main.yml); role defaults
/// are not inventory variables, so a host with `site_id` and no `zenoh_stack`
/// takes the role's default.
pub fn choose_router(explicit: Option<&str>, core: &Target) -> Result<RouterChoice> {
    if let Some(name) = explicit {
        if !container_name(name) {
            return Err(coded(
                "GB-113",
                format!("{ROUTER_FLAG} `{name}` is not a container name or ID"),
            ));
        }
        return Ok(RouterChoice::Explicit(name.to_string()));
    }
    let var = |k: &str| {
        core.vars
            .get(k)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
    };
    // A site_id that is itself a template resolves nothing.
    let site_id = var("site_id").filter(|s| !s.contains("{{") && !s.contains("{%"));
    let host = &core.name;
    let (stack, from) = match (var("zenoh_stack"), site_id) {
        (Some(stack), _) => match resolve_site_id(stack, site_id) {
            Some(s) => (s, format!("zenoh_stack in the inventory's variables for {host}")),
            None => return Ok(RouterChoice::Discover { why: format!("zenoh_stack for {host} is `{stack}`, a template phase 3 does not resolve") }),
        },
        (None, Some(id)) => (format!("{id}-zenoh"), format!("site_id in the inventory's variables for {host} (the goat-zenoh role's default zenoh_stack)")),
        (None, None) => return Ok(RouterChoice::Discover { why: format!("the inventory gives {host} neither zenoh_stack nor site_id") }),
    };
    let name = format!("{stack}-router");
    if !container_name(&name) {
        return Err(coded(
            "GB-113",
            format!("{from} gives the core router name `{name}`, which is not a container name"),
        ));
    }
    Ok(RouterChoice::Declared { name, from })
}

// ---------------------------------------------------------------------------
// The core's SPIRE server
// ---------------------------------------------------------------------------

/// The SPIRE server config phase 3 reads the trust domain from (#247):
/// `{{ spire_config_dir }}/server.conf`, the spire role's layout, with the
/// role's default `/opt/spire/config` when the core host's variables do not set
/// it. A value this cannot resolve (a template) still names a path, so the
/// read fails and says why instead of a default standing in silently.
pub fn spire_conf(core: &Target) -> String {
    let dir = core
        .vars
        .get("spire_config_dir")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("/opt/spire/config");
    format!("{}/server.conf", dir.trim_end_matches('/'))
}

// ---------------------------------------------------------------------------
// Reaching a host
// ---------------------------------------------------------------------------

/// A word for a POSIX shell: bare when it is plainly safe, single-quoted
/// otherwise. The remote side of SSH is always a shell.
fn quote(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./:=@%+,-".contains(c))
    {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// Runs derive's read-only commands on one [`Target`]: through `ssh`, or here
/// when the inventory says the host is this machine.
pub struct HostProbe {
    pub target: Target,
    /// The ssh client; `ssh` on PATH. Tests put a recorded one here.
    pub ssh: String,
}

impl HostProbe {
    pub fn new(target: Target) -> Self {
        HostProbe {
            target,
            ssh: "ssh".into(),
        }
    }

    /// The command line on the host, privilege escalation included.
    fn on_host(&self, bin: &str, args: &[&str]) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(user) = &self.target.become_root {
            v.extend(["sudo".to_string(), "-n".into()]);
            if user != "root" {
                v.extend(["-u".to_string(), user.clone()]);
            }
            v.push("--".into());
        }
        v.push(bin.to_string());
        v.extend(args.iter().map(|a| a.to_string()));
        v
    }

    /// The full argument vector run on this machine. Over SSH the host's
    /// command line is one quoted string, after `--` and the destination, so no
    /// part of it can be read as an `ssh` option.
    pub fn argv(&self, bin: &str, args: &[&str]) -> Vec<String> {
        let host = self.on_host(bin, args);
        if self.target.local {
            return host;
        }
        let t = &self.target;
        let mut v: Vec<String> = [
            self.ssh.as_str(),
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "StrictHostKeyChecking=accept-new",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if let Some(p) = t.port {
            v.extend(["-p".into(), p.to_string()]);
        }
        if let Some(k) = &t.key {
            v.extend(["-i".into(), k.clone()]);
        }
        v.extend(t.ssh_args.iter().cloned());
        v.extend(["--".into(), t.destination()]);
        // C messages, so a remote `cat` failure reads the same on every host.
        let line = std::iter::once("LC_ALL=C".to_string())
            .chain(host.iter().map(|w| quote(w)))
            .collect::<Vec<_>>()
            .join(" ");
        v.push(line);
        v
    }

    fn exec(&self, bin: &str, args: &[&str], answer_on_failure: bool) -> Result<String, String> {
        if !derive::is_read_only_probe(bin, args) {
            return Err(format!(
                "refused before it reached {}: `{bin} {}` is not one of phase 3's read-only probes",
                self.target.name,
                args.join(" ")
            ));
        }
        let argv = self.argv(bin, args);
        crate::trace::cmd(Command::new(&argv[0]).args(&argv[1..]));
        let out = Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .map_err(|e| format!("{}: {e}", argv[0]))?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if out.status.success() {
            return Ok(stdout);
        }
        if !self.target.local && out.status.code() == Some(255) {
            return Err(format!(
                "ssh to {} ({}) failed, or the command there exited 255: {stderr}",
                self.target.name,
                self.target.destination()
            ));
        }
        if answer_on_failure && !stdout.trim().is_empty() {
            return Ok(stdout);
        }
        Err(if stderr.is_empty() {
            format!("{bin} exited {}", out.status)
        } else {
            stderr
        })
    }

    /// One harmless command (`true`, under the same escalation), so an
    /// unreachable host or a sudo that wants a password is refused with its
    /// fix before anything is measured.
    pub fn reach(&self) -> Result<()> {
        let argv = self.argv("true", &[]);
        let shown = argv.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ");
        crate::trace::cmd(Command::new(&argv[0]).args(&argv[1..]));
        let out = Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .map_err(|e| {
                coded(
                    "GB-111",
                    format!(
                        "could not start `{}` to reach {}: {e}",
                        argv[0], self.target.name
                    ),
                )
            })?;
        if out.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if err.contains("a password is required") || err.contains("a terminal is required") {
            return Err(coded(
                "GB-112",
                format!(
                    "{} ({}) answered, but sudo there wants a password: {err}; ran: {shown}",
                    self.target.name,
                    self.target.destination()
                ),
            ));
        }
        Err(coded(
            "GB-111",
            format!(
                "could not reach {} ({}) over ssh: {err}; ran: {shown}",
                self.target.name,
                self.target.destination()
            ),
        ))
    }
}

impl Probe for HostProbe {
    fn run(&self, bin: &str, args: &[&str]) -> Result<String, String> {
        self.exec(bin, args, false)
    }
    fn run_reading_output(&self, bin: &str, args: &[&str]) -> Result<String, String> {
        self.exec(bin, args, true)
    }
    fn read_file(&self, path: &str) -> std::io::Result<String> {
        use std::io::{Error, ErrorKind};
        self.exec("cat", &["--", path], false).map_err(|e| {
            // ssh's own failure ("Permission denied (publickey)", a jump
            // host's "No such file or directory") is never the file's.
            let kind = if e.starts_with("ssh to ") {
                ErrorKind::Other
            } else if e.contains("Permission denied") {
                ErrorKind::PermissionDenied
            } else if e.contains("No such file or directory") {
                ErrorKind::NotFound
            } else {
                ErrorKind::Other
            };
            Error::new(kind, e)
        })
    }
    fn place(&self) -> String {
        if self.target.local {
            format!("on {} (this machine)", self.target.name)
        } else {
            format!("on {} ({})", self.target.name, self.target.destination())
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 3
// ---------------------------------------------------------------------------

/// The two hosts phase 3 measures.
pub struct Hosts {
    pub edge: Target,
    pub core: Target,
}

/// The edge and the core, from the plays' inventory.
pub fn hosts(
    inv: &Value,
    edge_group: &str,
    core_group: &str,
    edge_pick: Option<&str>,
    core_pick: Option<&str>,
) -> Result<Hosts> {
    Ok(Hosts {
        edge: target_in(inv, edge_group, edge_pick, "--edge-host")?,
        core: target_in(inv, core_group, core_pick, "--core-host")?,
    })
}

/// A router the core cannot show is refused here, with what to supply,
/// instead of becoming two underived values: the operator can act on it.
/// A version probe that fails on a router that WAS found stays an underived
/// value for `init` to report, as it always has.
fn router_refusal(m: &Measured, choice: &RouterChoice, core: &Target) -> Result<()> {
    let Err(why) = &m.fabric_fips_image.outcome else {
        return Ok(());
    };
    let looked = match choice {
        RouterChoice::Discover { why } => format!("{why}, so the router was discovered; "),
        _ => String::new(),
    };
    Err(coded(
        "GB-113",
        format!(
            "no core router to measure on {} ({}): {looked}{why}",
            core.name,
            core.destination()
        ),
    ))
}

/// Measure the edge and the core for phase 3. Both hosts are reached first,
/// so a host that cannot be reached is one refusal with its fix, not a list of
/// underived values. `interface` is the declaration's federation interface,
/// when the package offers a transport.
pub fn measure(
    hosts: &Hosts,
    router_flag: Option<&str>,
    interface: Option<&str>,
) -> Result<(Measured, RouterChoice)> {
    let choice = choose_router(router_flag, &hosts.core)?;
    let (edge, core) = (
        HostProbe::new(hosts.edge.clone()),
        HostProbe::new(hosts.core.clone()),
    );
    edge.reach()?;
    core.reach()?;
    let m = derive::measure(
        &edge,
        &core,
        choice.target(),
        RUNTIMES,
        ROUTER_FLAG,
        interface,
        Some(&spire_conf(&hosts.core)),
    );
    router_refusal(&m, &choice, &hosts.core)?;
    Ok((m, choice))
}

/// The edge and the core named by the plays' inventory in this checkout.
pub fn resolve(
    repo: &Path,
    edge_group: &str,
    core_group: &str,
    edge_pick: Option<&str>,
    core_pick: Option<&str>,
) -> Result<Hosts> {
    let inv = read_inventory(repo, &[INVENTORY])?;
    hosts(&inv, edge_group, core_group, edge_pick, core_pick)
}

/// The measurement `goat_federation::verbs::run_init_measured` takes: the edge
/// and the core over SSH, with the router chosen by [`choose_router`]. `init`
/// then reads identity and the declaration from the operator's own profile and
/// signs where this runs.
pub fn over_ssh(
    hosts: Hosts,
    router_flag: Option<String>,
) -> impl Fn(Option<&str>) -> Result<Measured> {
    move |interface| measure(&hosts, router_flag.as_deref(), interface).map(|(m, _)| m)
}

// ---------------------------------------------------------------------------
// `goatbuilder federate init --over-ssh`, and the driver's phase 3
// ---------------------------------------------------------------------------

/// The flags that make `goatbuilder federate init` measure the edge and the
/// core over SSH instead of this machine. Without `--over-ssh`, init measures
/// this machine exactly as `goat federate init` does.
#[derive(clap::Args, Debug, Clone, Default)]
pub struct MeasureArgs {
    /// init: measure this peering's edge and core over SSH from this machine,
    /// reaching the hosts of inventory groups <PEERING>_edge and <PEERING>_core
    /// in ops/ansible/inventory/peering-edges.ini as the plays reach them.
    /// Only read-only commands run there; nothing is copied to either host.
    #[arg(long, value_name = "PEERING", global = true)]
    pub over_ssh: Option<String>,
    /// init --over-ssh: the edge's inventory group (default <PEERING>_edge).
    #[arg(long, value_name = "GROUP", global = true)]
    pub ssh_edge_group: Option<String>,
    /// init --over-ssh: the core's inventory group (default <PEERING>_core).
    #[arg(long, value_name = "GROUP", global = true)]
    pub ssh_core_group: Option<String>,
    /// init --over-ssh: the edge host to measure, when its group has several.
    #[arg(long, value_name = "HOST", global = true)]
    pub edge_host: Option<String>,
    /// init --over-ssh: the core host to measure, when its group has several.
    #[arg(long, value_name = "HOST", global = true)]
    pub core_host: Option<String>,
    /// init --over-ssh: the core router container, by hand (default: the name
    /// the core host's variables give, else the one discovery finds there).
    #[arg(long = "router-container", value_name = "NAME", global = true)]
    pub router_container: Option<String>,
}

impl MeasureArgs {
    pub fn is_empty(&self) -> bool {
        self.over_ssh.is_none()
            && self.ssh_edge_group.is_none()
            && self.ssh_core_group.is_none()
            && self.edge_host.is_none()
            && self.core_host.is_none()
            && self.router_container.is_none()
    }

    /// The peering to measure over SSH; `None` measures this machine. A
    /// host, group or router flag without `--over-ssh` is refused rather than
    /// ignored, because ignoring it would measure the wrong machine.
    pub fn peering(&self) -> Result<Option<&str>> {
        let Some(p) = self.over_ssh.as_deref() else {
            if self.is_empty() {
                return Ok(None);
            }
            return Err(anyhow::anyhow!(
                "--ssh-edge-group, --ssh-core-group, --edge-host, --core-host and \
                 --router-container choose what `init --over-ssh <peering>` measures; \
                 without --over-ssh, init measures this machine"
            ));
        };
        let plain = |v: &str| {
            !v.is_empty()
                && !v.starts_with('-')
                && v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        };
        for (flag, v) in [
            ("--over-ssh", Some(p)),
            ("--ssh-edge-group", self.ssh_edge_group.as_deref()),
            ("--ssh-core-group", self.ssh_core_group.as_deref()),
            ("--edge-host", self.edge_host.as_deref()),
            ("--core-host", self.core_host.as_deref()),
        ] {
            if let Some(v) = v.filter(|v| !plain(v)) {
                return Err(coded(
                    "GB-110",
                    format!("{flag} `{v}` is not an inventory name (letters, digits, `_`, `.`, `-`)"),
                ));
            }
        }
        Ok(Some(p))
    }

    /// The edge and core groups for `peering`.
    pub fn groups(&self, peering: &str) -> (String, String) {
        (
            self.ssh_edge_group
                .clone()
                .unwrap_or_else(|| format!("{peering}_edge")),
            self.ssh_core_group
                .clone()
                .unwrap_or_else(|| format!("{peering}_core")),
        )
    }
}

/// `goatbuilder federate init --over-ssh <peering>`: the hosts are resolved
/// from the plays' inventory in `repo`, measured over SSH, and the package is
/// built and signed here by the library's `init`.
pub fn run_init(
    repo: &Path,
    site: &goat_federation::verbs::Site,
    a: &goat_federation::verbs::InitArgs,
    m: &MeasureArgs,
    out: &mut dyn std::io::Write,
) -> Result<i32> {
    let peering = m
        .peering()?
        .ok_or_else(|| anyhow::anyhow!("run_init over SSH without --over-ssh"))?;
    let (edge_group, core_group) = m.groups(peering);
    let hosts = resolve(
        repo,
        &edge_group,
        &core_group,
        m.edge_host.as_deref(),
        m.core_host.as_deref(),
    )?;
    let measure = over_ssh(hosts, m.router_container.clone());
    goat_federation::verbs::run_init_measured(site, a, out, &measure)
}

/// Whether the driver's phase 3 measures over SSH (`FED_MEASURE=ssh`) instead
/// of the machine the driver runs on (unset, empty or `here`, the default).
pub fn driver_measures_over_ssh() -> bool {
    std::env::var("FED_MEASURE").ok().as_deref() == Some("ssh")
}

/// Any other value of `FED_MEASURE` is refused, as peering-standup.sh refuses
/// it: a misspelt `ssh` would otherwise measure the wrong machine.
pub fn check_driver_measure() -> Result<()> {
    match std::env::var("FED_MEASURE").ok().as_deref() {
        None | Some("") | Some("here") | Some("ssh") => Ok(()),
        Some(other) => Err(anyhow::anyhow!(
            "FED_MEASURE is here or ssh, not '{other}'"
        )),
    }
}

/// The words after `goatbuilder federate` for phase 3 measured over SSH,
/// after init's own words (`--with-transport ... --out <file>`): the peering,
/// the edge group the run uses, the router when `FED_ROUTER_CONTAINER` names
/// one, and the repository whose inventory names the hosts.
/// ops/scripts/peering-standup.sh prints the same words.
pub fn phase3_words(
    mut init_words: Vec<String>,
    peering: &str,
    edge_group: &str,
    router: Option<&str>,
    repo: &Path,
) -> Vec<String> {
    init_words.extend([
        "--over-ssh".to_string(),
        peering.to_string(),
        "--ssh-edge-group".into(),
        edge_group.to_string(),
    ]);
    if let Some(r) = router.filter(|r| !r.is_empty()) {
        init_words.extend(["--router-container".to_string(), r.to_string()]);
    }
    init_words.extend(["--repo".to_string(), repo.display().to_string()]);
    init_words
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn code(e: &anyhow::Error) -> Option<String> {
        e.downcast_ref::<goat_errors::Coded>()
            .map(|c| c.code.clone())
    }

    /// What `ansible-inventory --list` prints for the harness-shaped peering
    /// inventory: the edge in `xab_edge`, the core in `xab_core` via a parent.
    fn inventory() -> Value {
        json!({
            "_meta": {"hostvars": {
                "xaa-edge": {"ansible_host": "192.0.2.10", "ansible_user": "goat", "ansible_port": 2222,
                             "ansible_ssh_private_key_file": "~/.ssh/fed", "ansible_ssh_common_args": "-o ProxyJump=jump@198.51.100.7 -o 'UserKnownHostsFile=/tmp/known hosts'"},
                "xaa-core": {"ansible_host": "192.0.2.20", "ansible_user": "goat", "site_id": "xaa"},
                "xaa-core-2": {"ansible_host": "192.0.2.21"},
                "station": {"ansible_connection": "local", "ansible_become": false}
            }},
            "all": {"children": ["peering_edges", "peering_cores", "ungrouped"]},
            "peering_edges": {"children": ["xab_edge"]},
            "xab_edge": {"hosts": ["xaa-edge"]},
            "peering_cores": {"children": ["xab_core"]},
            "xab_core": {"hosts": ["xaa-core"]},
            "two_cores": {"hosts": ["xaa-core", "xaa-core-2"]},
            "here": {"hosts": ["station"]},
            "empty": {"hosts": []}
        })
    }

    #[test]
    fn a_host_is_reached_with_the_inventorys_address_user_port_key_and_jump() {
        let t = target_in(&inventory(), "peering_edges", None, "--edge-host").unwrap();
        assert_eq!(
            (
                t.name.as_str(),
                t.address.as_str(),
                t.user.as_deref(),
                t.port
            ),
            ("xaa-edge", "192.0.2.10", Some("goat"), Some(2222))
        );
        let argv = HostProbe::new(t).argv("wg", &["show", "interfaces"]);
        assert_eq!(
            argv,
            [
                "ssh",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-p",
                "2222",
                "-i",
                "~/.ssh/fed",
                "-o",
                "ProxyJump=jump@198.51.100.7",
                "-o",
                "UserKnownHostsFile=/tmp/known hosts",
                "--",
                "goat@192.0.2.10",
                "LC_ALL=C sudo -n -- wg show interfaces"
            ]
        );
    }

    #[test]
    fn the_host_command_line_is_quoted_so_it_cannot_become_shell() {
        let t = target_in(&inventory(), "xab_core", None, "--core-host").unwrap();
        let argv = HostProbe::new(t).argv(
            "docker",
            &[
                "ps",
                "--no-trunc",
                "--format",
                "{{.ID}}\t{{.Label \"org.goat.component\"}}",
            ],
        );
        assert_eq!(argv.last().unwrap(), "LC_ALL=C sudo -n -- docker ps --no-trunc --format '{{.ID}}\t{{.Label \"org.goat.component\"}}'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote("$(id)"), "'$(id)'");
    }

    #[test]
    fn a_local_host_runs_here_and_become_follows_the_inventory() {
        let t = target_in(&inventory(), "here", None, "--edge-host").unwrap();
        assert!(t.local && t.become_root.is_none());
        assert_eq!(
            HostProbe::new(t).argv("wg", &["show", "interfaces"]),
            ["wg", "show", "interfaces"]
        );
        let mut inv = inventory();
        inv["_meta"]["hostvars"]["station"] =
            json!({"ansible_connection": "local", "ansible_become_user": "zenoh"});
        let t = target_in(&inv, "here", None, "--edge-host").unwrap();
        assert_eq!(
            HostProbe::new(t).argv("cat", &["--", "/proc/sys/crypto/fips_enabled"]),
            [
                "sudo",
                "-n",
                "-u",
                "zenoh",
                "--",
                "cat",
                "--",
                "/proc/sys/crypto/fips_enabled"
            ]
        );
    }

    #[test]
    fn a_command_outside_the_read_only_list_never_starts_ssh() {
        // The address is unroutable on purpose: reaching it would hang, not fail.
        let t = target_in(&inventory(), "xab_core", None, "--core-host").unwrap();
        let p = HostProbe::new(t);
        let why = p.run("docker", &["rm", "-f", "coreid"]).unwrap_err();
        assert!(
            why.starts_with("refused before it reached xaa-core"),
            "{why}"
        );
        assert!(p
            .run("systemctl", &["restart", "rosenpass"])
            .unwrap_err()
            .contains("read-only"));
    }

    #[test]
    fn inventory_problems_are_refused_with_gb_110_naming_the_fix() {
        let inv = inventory();
        let e = target_in(&inv, "empty", None, "--edge-host").unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("GB-110"));
        let e = target_in(&inv, "two_cores", None, "--core-host").unwrap_err();
        assert!(
            code(&e).as_deref() == Some("GB-110") && e.to_string().contains("--core-host <host>"),
            "{e}"
        );
        assert_eq!(
            target_in(&inv, "two_cores", Some("xaa-core-2"), "--core-host")
                .unwrap()
                .address,
            "192.0.2.21"
        );
        let e = target_in(&inv, "xab_core", Some("xaa-edge"), "--core-host").unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("GB-110"));
        let mut odd = inventory();
        odd["_meta"]["hostvars"]["xaa-edge"]["ansible_connection"] = json!("community.aws.aws_ssm");
        assert_eq!(
            code(&target_in(&odd, "xab_edge", None, "--edge-host").unwrap_err()).as_deref(),
            Some("GB-110")
        );
        odd["_meta"]["hostvars"]["xaa-edge"] = json!({"ansible_host": "-oProxyCommand=id"});
        assert_eq!(
            code(&target_in(&odd, "xab_edge", None, "--edge-host").unwrap_err()).as_deref(),
            Some("GB-110")
        );
    }

    fn core_with(vars: Value) -> Target {
        let mut inv = inventory();
        inv["_meta"]["hostvars"]["xaa-core"] = vars;
        target_in(&inv, "xab_core", None, "--core-host").unwrap()
    }

    #[test]
    fn router_precedence_is_the_flag_then_the_deployment_then_discovery() {
        let both = core_with(json!({"site_id": "xaa", "zenoh_stack": "fabric-a"}));
        // The operator's flag wins over what the deployment declares.
        assert_eq!(
            choose_router(Some("custom-router"), &both).unwrap(),
            RouterChoice::Explicit("custom-router".into())
        );
        // zenoh_stack, as the role uses it.
        assert!(
            matches!(choose_router(None, &both).unwrap(), RouterChoice::Declared { name, .. } if name == "fabric-a-router")
        );
        // zenoh_stack written in terms of site_id resolves.
        let templ = core_with(json!({"site_id": "xaa", "zenoh_stack": "{{site_id}}-fabric"}));
        assert!(
            matches!(choose_router(None, &templ).unwrap(), RouterChoice::Declared { name, .. } if name == "xaa-fabric-router")
        );
        // site_id alone takes the role's default zenoh_stack.
        let site = core_with(json!({"site_id": "xaa"}));
        assert!(
            matches!(choose_router(None, &site).unwrap(), RouterChoice::Declared { name, from } if name == "xaa-zenoh-router" && from.contains("site_id"))
        );
        // Nothing declared, or a template this cannot resolve: discovery.
        assert!(matches!(
            choose_router(None, &core_with(json!({}))).unwrap(),
            RouterChoice::Discover { .. }
        ));
        let other = core_with(json!({"zenoh_stack": "{{ fabric_name }}"}));
        assert!(
            matches!(choose_router(None, &other).unwrap(), RouterChoice::Discover { why } if why.contains("does not resolve"))
        );
        // A site_id that is itself a template resolves nothing: discovery.
        let t = core_with(json!({"site_id": "{{ lookup('env', 'SITE') }}"}));
        assert!(matches!(
            choose_router(None, &t).unwrap(),
            RouterChoice::Discover { .. }
        ));
        // A `$` in site_id is text, not a capture reference.
        let d = core_with(json!({"site_id": "a$1", "zenoh_stack": "{{ site_id }}-z"}));
        assert!(
            matches!(choose_router(None, &d).unwrap_err().downcast_ref::<goat_errors::Coded>(), Some(c) if c.message.contains("`a$1-z-router`"))
        );
        // A flag that is an option or an expression is refused.
        assert_eq!(
            code(&choose_router(Some("--rm"), &site).unwrap_err()).as_deref(),
            Some("GB-113")
        );
    }

    #[test]
    fn the_role_still_names_its_router_the_way_choose_router_assumes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/ansible/roles/goat-zenoh");
        let defaults = std::fs::read_to_string(root.join("defaults/main.yml")).unwrap();
        assert!(
            defaults
                .lines()
                .any(|l| l.starts_with("zenoh_stack: \"{{ site_id }}-zenoh\"")),
            "goat-zenoh's zenoh_stack default changed; update choose_router"
        );
        let compose =
            std::fs::read_to_string(root.join("templates/docker-compose.yml.j2")).unwrap();
        assert!(
            compose.contains("container_name: {{ zenoh_stack }}-router"),
            "goat-zenoh's container name changed; update choose_router"
        );
    }

    #[test]
    fn the_spire_config_follows_the_role_layout_and_the_core_hosts_variables() {
        let mut core = Target {
            name: "core-a".into(),
            address: "core-a".into(),
            user: None,
            port: None,
            key: None,
            ssh_args: vec![],
            become_root: None,
            local: false,
            vars: serde_json::json!({}),
        };
        assert_eq!(spire_conf(&core), derive::SPIRE_SERVER_CONF);
        core.vars = serde_json::json!({"spire_config_dir": "/etc/spire/server/"});
        assert_eq!(spire_conf(&core), "/etc/spire/server/server.conf");
        // A template is not resolved and not replaced by the default: the read
        // is refused and says so.
        core.vars = serde_json::json!({"spire_config_dir": "{{ spire_root }}/config"});
        assert!(!derive::spire_conf_path(&spire_conf(&core)));
        let role = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../ops/ansible/roles/spire/defaults/main.yml"
        ))
        .unwrap();
        assert!(
            role.contains("spire_config_dir: \"/opt/spire/config\""),
            "the spire role's config dir changed; update derive::SPIRE_SERVER_CONF"
        );
    }

    fn measured(fips: Result<bool, String>) -> Measured {
        let d = |field: &'static str, outcome| derive::Derived {
            field,
            outcome,
            source: String::new(),
        };
        Measured {
            transports: derive::Derived {
                field: "t",
                outcome: Ok(vec!["wireguard".into()]),
                source: String::new(),
            },
            fabric_fips_image: d("f", fips),
            fabric_version: derive::Derived {
                field: "v",
                outcome: Err("x".into()),
                source: String::new(),
            },
            pqc_capable: derive::Derived {
                field: "p",
                outcome: Ok(false),
                source: String::new(),
            },
            public_key: None,
            interface_mtu: None,
            spire_trust_domain: None,
        }
    }

    #[test]
    fn a_core_with_no_router_is_refused_with_gb_113_and_a_found_one_is_not() {
        let core = core_with(json!({}));
        let choice = RouterChoice::Discover {
            why: "the inventory gives xaa-core neither zenoh_stack nor site_id".into(),
        };
        let e = router_refusal(
            &measured(Err(
                "no running core router found. Set --router-container".into()
            )),
            &choice,
            &core,
        )
        .unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("GB-113"));
        assert!(
            e.to_string().contains("neither zenoh_stack nor site_id")
                && e.to_string().contains("--router-container"),
            "{e}"
        );
        // Found, version unreadable: left for init to report as underived.
        assert!(router_refusal(&measured(Ok(true)), &choice, &core).is_ok());
    }

    /// A recorded ssh client: prints `stderr` and exits `code`, whatever it is
    /// asked. The tests then need no network, no sshd and no ssh config.
    fn recorded_ssh(dir: &Path, code: i32, stderr: &str) -> HostProbe {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(format!("ssh-{code}-{}", stderr.len()));
        std::fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s\\n' '{stderr}' >&2\nexit {code}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut p =
            HostProbe::new(target_in(&inventory(), "xab_core", None, "--core-host").unwrap());
        p.ssh = path.display().to_string();
        p
    }

    #[test]
    fn an_unreachable_host_is_gb_111_and_a_sudo_password_is_gb_112_both_naming_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let e = recorded_ssh(
            dir.path(),
            255,
            "ssh: connect to host 192.0.2.20 port 22: Connection refused",
        )
        .reach()
        .unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("GB-111"), "{e}");
        assert!(
            e.to_string().contains("port 22: Connection refused")
                && e.to_string()
                    .contains("goat@192.0.2.20 'LC_ALL=C sudo -n -- true'"),
            "{e}"
        );
        let e = recorded_ssh(dir.path(), 1, "sudo: a password is required")
            .reach()
            .unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("GB-112"), "{e}");
    }

    #[test]
    fn a_remote_read_keeps_the_files_answer_apart_from_ssh_failing() {
        use std::io::ErrorKind;
        let dir = tempfile::tempdir().unwrap();
        let kind = |code, stderr| {
            recorded_ssh(dir.path(), code, stderr)
                .read_file("/proc/sys/crypto/fips_enabled")
                .unwrap_err()
                .kind()
        };
        assert_eq!(
            kind(1, "cat: /proc/sys/crypto/fips_enabled: Permission denied"),
            ErrorKind::PermissionDenied
        );
        assert_eq!(
            kind(
                1,
                "cat: /proc/sys/crypto/fips_enabled: No such file or directory"
            ),
            ErrorKind::NotFound
        );
        // ssh's own words are not the file's.
        assert_eq!(
            kind(255, "goat@192.0.2.20: Permission denied (publickey)."),
            ErrorKind::Other
        );
        assert_eq!(
            kind(255, "/usr/bin/nc: No such file or directory"),
            ErrorKind::Other
        );
        assert_eq!(
            kind(1, "sudo: sorry, user goat is not allowed to execute cat"),
            ErrorKind::Other
        );
    }

    /// A checkout whose `ops/scripts/with-aws` prints `inventory` as
    /// `ansible-inventory --list` would, so `run_init` reads hosts the way it
    /// does on a real checkout without ansible installed.
    fn checkout_with_inventory(inventory: &Value) -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("ops/ansible/inventory")).unwrap();
        std::fs::create_dir_all(d.path().join("ops/scripts")).unwrap();
        std::fs::write(d.path().join("inventory.json"), inventory.to_string()).unwrap();
        let script = d.path().join("ops/scripts/with-aws");
        std::fs::write(&script, "#!/bin/sh\ncat ../../inventory.json\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        d
    }

    fn init_args() -> goat_federation::verbs::InitArgs {
        goat_federation::verbs::InitArgs {
            with_transport: false,
            with_data: false,
            signing_key: None,
            out: None,
            json: false,
        }
    }

    /// A context directory with a declaration and nothing else: init reads
    /// the declaration before it asks for the measurement.
    fn site(dir: &Path) -> goat_federation::verbs::Site {
        std::fs::write(
            dir.join("federation.toml"),
            "sovereign_id = \"xaa\"\nterm_days = 30\ncut_contact = \"ops@example.org\"\n",
        )
        .unwrap();
        goat_federation::verbs::Site {
            context_dir: dir.to_path_buf(),
            cert_path: dir.join("mtls.cert.pem"),
            ca_roots_path: dir.join("ca-roots.pem"),
        }
    }

    #[test]
    fn a_host_flag_without_over_ssh_is_refused_not_ignored() {
        assert_eq!(MeasureArgs::default().peering().unwrap(), None);
        let m = MeasureArgs {
            edge_host: Some("xaa-edge".into()),
            ..Default::default()
        };
        let e = m.peering().unwrap_err();
        assert!(e.to_string().contains("without --over-ssh"), "{e}");
        let m = MeasureArgs {
            over_ssh: Some("xab".into()),
            core_host: Some("-oProxyCommand=x".into()),
            ..Default::default()
        };
        assert_eq!(code(&m.peering().unwrap_err()).as_deref(), Some("GB-110"));
    }

    #[test]
    fn the_groups_follow_the_peering_unless_named() {
        let m = MeasureArgs {
            over_ssh: Some("xab".into()),
            ..Default::default()
        };
        assert_eq!(m.groups("xab"), ("xab_edge".into(), "xab_core".into()));
        let m = MeasureArgs {
            over_ssh: Some("xab".into()),
            ssh_edge_group: Some("edge-x".into()),
            ..Default::default()
        };
        assert_eq!(m.groups("xab"), ("edge-x".into(), "xab_core".into()));
    }

    #[test]
    fn phase_3_words_name_the_peering_group_router_and_checkout() {
        let w = phase3_words(
            vec!["init".into(), "--out".into(), "/w/ours.canonical.json".into()],
            "xab",
            "xab_edge",
            Some("xaa-zenoh-router"),
            Path::new("/repo"),
        );
        assert_eq!(
            w.join(" "),
            "init --out /w/ours.canonical.json --over-ssh xab --ssh-edge-group xab_edge \
             --router-container xaa-zenoh-router --repo /repo"
        );
        let w = phase3_words(vec!["init".into()], "xab", "xab_edge", Some(""), Path::new("/r"));
        assert!(!w.contains(&"--router-container".to_string()), "{w:?}");
    }

    /// The whole path of `init --over-ssh`, short of a real host: the flags
    /// pick the groups, the inventory names the hosts, the router comes from
    /// the core host's variables, and the probe runs on the host the
    /// inventory names (here this machine, `ansible_connection=local`). No
    /// container of that name runs here, so the refusal is GB-113 and names
    /// the core host and the variable the name came from.
    #[test]
    fn init_over_ssh_reads_the_inventory_and_measures_the_named_hosts() {
        let inv = json!({
            "_meta": {"hostvars": {
                "station": {"ansible_connection": "local", "ansible_become": false,
                            "zenoh_stack": "gb-w3-test-no-such"}
            }},
            "all": {"children": ["xab_edge", "xab_core"]},
            "xab_edge": {"hosts": ["station"]},
            "xab_core": {"hosts": ["station"]}
        });
        let repo = checkout_with_inventory(&inv);
        let ctx = tempfile::tempdir().unwrap();
        let site = site(ctx.path());
        let m = MeasureArgs {
            over_ssh: Some("xab".into()),
            ..Default::default()
        };
        let mut out = Vec::new();
        let e = run_init(repo.path(), &site, &init_args(), &m, &mut out).unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("GB-113"), "{e:#}");
        let text = format!("{e:#}");
        assert!(text.contains("on station"), "{text}");

        // A core group the inventory does not have is GB-110, naming it.
        let m = MeasureArgs {
            over_ssh: Some("xab".into()),
            ssh_core_group: Some("no_such_core".into()),
            ..Default::default()
        };
        let e = run_init(repo.path(), &site, &init_args(), &m, &mut out).unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("GB-110"), "{e:#}");
        assert!(format!("{e:#}").contains("no_such_core"), "{e:#}");
    }
}
