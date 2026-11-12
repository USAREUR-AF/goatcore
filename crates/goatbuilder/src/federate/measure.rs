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
//! With two drivers (a site where no one machine reaches both hosts), the
//! measurement runs in two halves: `--side edge` measures only the edge and
//! writes [`EdgeHalf`] to a file, and `--side core` measures only the core and
//! [`join`]s that file's half before init signs. GB-114 refuses an edge half
//! that cannot be joined. The one-machine run is the two halves joined.
//!
//! docs/design/federation-router-discovery.md is the design.

use std::path::Path;

use anyhow::Result;
use goat_federation::derive::{self, Measured, Probe, RouterTarget};
use serde_json::Value;

/// The inventory the peering plays are given (relative to ops/ansible).
pub const INVENTORY: &str = "inventory/peering-edges.ini";

/// How the operator names the core router by hand.
pub const ROUTER_FLAG: &str = "--router-container";
/// How a refusal tells the operator to name the core router: the flag of
/// `goatbuilder federate init --over-ssh`, and the variable the one command
/// (`goatbuilder federate <partner>`) passes to it as that flag.
pub const ROUTER_KNOB: &str =
    "--router-container (FED_ROUTER_CONTAINER for goatbuilder federate <partner>)";

/// The schema of the file phase 3's edge half is carried in, from the edge
/// driver to the controller.
pub const EDGE_HALF_SCHEMA: &str = "goatbuilder.phase3-edge-half/v1";

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
        crate::trace::cmd(crate::federate::runenv::command(&argv[0]).args(&argv[1..]));
        let out = crate::federate::runenv::command(&argv[0])
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
        crate::trace::cmd(crate::federate::runenv::command(&argv[0]).args(&argv[1..]));
        let out = crate::federate::runenv::command(&argv[0])
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
fn router_refusal(
    fips: &derive::Derived<bool>,
    choice: &RouterChoice,
    core: &Target,
) -> Result<()> {
    let Err(why) = &fips.outcome else {
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

/// What phase 3 measures on the edge: the transports, the PQC layer, and the
/// federation interface's public key and MTU (`goat_federation::derive`'s edge
/// reads, in its order).
#[derive(Debug, Clone)]
pub struct EdgeHalf {
    pub transports: derive::Derived<Vec<String>>,
    pub pqc_capable: derive::Derived<bool>,
    pub public_key: Option<derive::Derived<String>>,
    pub interface_mtu: Option<i64>,
}

/// What phase 3 measures on the core: the router's FIPS image and version,
/// and the SPIRE server's trust domain.
#[derive(Debug, Clone)]
pub struct CoreHalf {
    pub fabric_fips_image: derive::Derived<bool>,
    pub fabric_version: derive::Derived<String>,
    pub spire_trust_domain: Option<derive::Derived<String>>,
}

/// The edge half, read through `edge`.
pub fn edge_half(edge: &dyn Probe, interface: Option<&str>) -> EdgeHalf {
    EdgeHalf {
        transports: derive::transports_on(edge),
        pqc_capable: derive::pqc_capable_on(edge),
        public_key: interface.map(|i| derive::transport_public_key_on(edge, i)),
        interface_mtu: interface.and_then(|i| derive::interface_mtu_on(edge, i)),
    }
}

/// The core half, read through `core`.
pub fn core_half(core: &dyn Probe, router: RouterTarget, spire_conf: Option<&str>) -> CoreHalf {
    let (fabric_fips_image, fabric_version) =
        derive::fabric_capabilities_on(core, router, RUNTIMES, ROUTER_KNOB);
    CoreHalf {
        fabric_fips_image,
        fabric_version,
        spire_trust_domain: spire_conf.map(|p| derive::spire_trust_domain_on(core, p)),
    }
}

/// The two halves, as the one measurement `init` signs.
pub fn join(edge: EdgeHalf, core: CoreHalf) -> Measured {
    Measured {
        transports: edge.transports,
        fabric_fips_image: core.fabric_fips_image,
        fabric_version: core.fabric_version,
        pqc_capable: edge.pqc_capable,
        public_key: edge.public_key,
        interface_mtu: edge.interface_mtu,
        spire_trust_domain: core.spire_trust_domain,
    }
}

/// Measure the edge and the core for phase 3, from one machine that reaches
/// both. Both hosts are reached first, so a host that cannot be reached is one
/// refusal with its fix, not a list of underived values. `interface` is the
/// declaration's federation interface, when the package offers a transport.
/// The reads are the same, in the same order, as `derive::measure` makes:
/// the core half, then the edge half.
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
    let c = core_half(&core, choice.target(), Some(&spire_conf(&hosts.core)));
    router_refusal(&c.fabric_fips_image, &choice, &hosts.core)?;
    Ok((join(edge_half(&edge, interface), c), choice))
}

/// Phase 3's edge half alone, on a machine that reaches the edge: only the
/// edge is reached.
pub fn measure_edge(edge: &Target, interface: Option<&str>) -> Result<EdgeHalf> {
    let probe = HostProbe::new(edge.clone());
    probe.reach()?;
    Ok(edge_half(&probe, interface))
}

/// Phase 3's core half alone, on a machine that reaches the core: only the
/// core is reached, and the router is chosen as [`measure`] chooses it.
pub fn measure_core(core: &Target, router_flag: Option<&str>) -> Result<CoreHalf> {
    let choice = choose_router(router_flag, core)?;
    let probe = HostProbe::new(core.clone());
    probe.reach()?;
    let c = core_half(&probe, choice.target(), Some(&spire_conf(core)));
    router_refusal(&c.fabric_fips_image, &choice, core)?;
    Ok(c)
}

/// The plays' inventory for this peering: peering-edges.ini, and this
/// peering's own file when `allocate` rendered one.
fn peering_inventory(repo: &Path, edge_group: &str) -> Result<Value> {
    let own = format!("inventory/peering-{edge_group}.ini");
    if repo.join("ops/ansible").join(&own).is_file() {
        read_inventory(repo, &[INVENTORY, &own])
    } else {
        read_inventory(repo, &[INVENTORY])
    }
}

/// The edge and the core named by the plays' inventory in this checkout.
pub fn resolve(
    repo: &Path,
    edge_group: &str,
    core_group: &str,
    edge_pick: Option<&str>,
    core_pick: Option<&str>,
) -> Result<Hosts> {
    let inv = peering_inventory(repo, edge_group)?;
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
// The edge half, carried from the edge driver to the controller
// ---------------------------------------------------------------------------

/// A probe that answers nothing. Asking derive's edge reads through it gives
/// each read's field name, the `&'static str` an imported half is rebuilt
/// with, so the names here cannot drift from derive's.
struct Nothing;

impl Probe for Nothing {
    fn run(&self, _: &str, _: &[&str]) -> Result<String, String> {
        Err("not asked".into())
    }
    fn run_reading_output(&self, _: &str, _: &[&str]) -> Result<String, String> {
        Err("not asked".into())
    }
    fn read_file(&self, _: &str) -> std::io::Result<String> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not asked",
        ))
    }
}

fn derived_json<T: serde::Serialize>(d: &derive::Derived<T>) -> Value {
    match &d.outcome {
        Ok(v) => serde_json::json!({"field": d.field, "ok": v, "source": d.source}),
        Err(e) => serde_json::json!({"field": d.field, "err": e, "source": d.source}),
    }
}

fn derived_from<T: serde::de::DeserializeOwned>(
    v: &Value,
    field: &'static str,
) -> Option<derive::Derived<T>> {
    if v.get("field")?.as_str()? != field {
        return None;
    }
    let source = v.get("source")?.as_str()?.to_string();
    let outcome = match (v.get("ok"), v.get("err")) {
        (Some(ok), None) => Ok(serde_json::from_value(ok.clone()).ok()?),
        (None, Some(Value::String(e))) => Err(e.clone()),
        _ => return None,
    };
    Some(derive::Derived {
        field,
        outcome,
        source,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Write the edge half for `peering`, measured on `edge` for `interface`, to
/// `path`; returns the file's sha256, which the controller is shown when it
/// reads the file back.
pub fn write_edge_half(
    path: &Path,
    peering: &str,
    edge: &Target,
    interface: Option<&str>,
    half: &EdgeHalf,
) -> Result<String> {
    let v = serde_json::json!({
        "schema": EDGE_HALF_SCHEMA,
        "peering": peering,
        "edge_host": edge.name,
        "interface": interface,
        "transports": derived_json(&half.transports),
        "pqc_capable": derived_json(&half.pqc_capable),
        "public_key": half.public_key.as_ref().map(derived_json),
        "interface_mtu": half.interface_mtu,
    });
    let text = serde_json::to_string_pretty(&v)? + "\n";
    std::fs::write(path, &text)
        .map_err(|e| anyhow::anyhow!("could not write the edge half to {}: {e}", path.display()))?;
    Ok(sha256_hex(text.as_bytes()))
}

/// An edge half read back: what was measured, for which interface, on which
/// host, and the file's sha256.
#[derive(Debug, Clone)]
pub struct ImportedEdgeHalf {
    pub half: EdgeHalf,
    pub interface: Option<String>,
    pub edge_host: String,
    pub sha256: String,
}

/// Read an edge half for `peering`. Anything else is refused (GB-114): a
/// file that does not read, another schema, another peering, or a field that
/// is not what the edge half carries.
pub fn read_edge_half(path: &Path, peering: &str) -> Result<ImportedEdgeHalf> {
    let bad = |why: String| {
        coded(
            "GB-114",
            format!(
                "the edge half {} cannot be used for peering {peering} (GB-114): {why}",
                path.display()
            ),
        )
    };
    let bytes = std::fs::read(path).map_err(|e| bad(format!("it could not be read: {e}")))?;
    let v: Value =
        serde_json::from_slice(&bytes).map_err(|e| bad(format!("it is not JSON: {e}")))?;
    if v["schema"] != EDGE_HALF_SCHEMA {
        return Err(bad(format!(
            "its schema is {}, not {EDGE_HALF_SCHEMA}",
            v["schema"]
        )));
    }
    if v["peering"] != peering {
        return Err(bad(format!("it was measured for peering {}", v["peering"])));
    }
    let n = Nothing;
    let field = |k: &str| bad(format!("its {k} is not an edge measurement"));
    let transports = derived_from(&v["transports"], derive::transports_on(&n).field)
        .ok_or_else(|| field("transports"))?;
    let pqc_capable = derived_from(&v["pqc_capable"], derive::pqc_capable_on(&n).field)
        .ok_or_else(|| field("pqc_capable"))?;
    let public_key = match &v["public_key"] {
        Value::Null => None,
        pk => Some(
            derived_from(pk, derive::transport_public_key_on(&n, "wg0").field)
                .ok_or_else(|| field("public_key"))?,
        ),
    };
    let interface_mtu = match &v["interface_mtu"] {
        Value::Null => None,
        m => Some(m.as_i64().ok_or_else(|| field("interface_mtu"))?),
    };
    Ok(ImportedEdgeHalf {
        half: EdgeHalf {
            transports,
            pqc_capable,
            public_key,
            interface_mtu,
        },
        interface: v["interface"].as_str().map(String::from),
        edge_host: v["edge_host"].as_str().unwrap_or_default().to_string(),
        sha256: sha256_hex(&bytes),
    })
}

/// The federation interface init will ask the measurement for: the
/// declaration's `[transport] interface` when the package offers a transport,
/// as `run_init_measured` reads it. The edge half is measured for it.
fn declared_interface(
    site: &goat_federation::verbs::Site,
    a: &goat_federation::verbs::InitArgs,
) -> Result<Option<String>> {
    if !a.with_transport {
        return Ok(None);
    }
    let path = a
        .declaration
        .clone()
        .unwrap_or_else(|| site.declaration_path());
    let text = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("could not read the declaration {}: {e}", path.display()))?;
    let doc: toml::Value =
        toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    Ok(doc
        .get("transport")
        .and_then(|t| t.get("interface"))
        .and_then(|i| i.as_str())
        .map(String::from))
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
    /// init --over-ssh: measure one half of phase 3 on this machine, for a
    /// peering no one machine reaches both hosts of
    /// (docs/design/federation-router-discovery.md): `edge` measures only the edge
    /// and writes the result to --half-out; `core` measures only the core,
    /// joins the edge half read from --edge-half, and builds the package.
    /// Without it, both hosts are measured here, as always.
    #[arg(long, value_name = "SIDE", value_parser = ["edge", "core"], global = true)]
    pub side: Option<String>,
    /// init --over-ssh --side edge: the file the edge half is written to.
    #[arg(long, value_name = "FILE", global = true)]
    pub half_out: Option<std::path::PathBuf>,
    /// init --over-ssh --side core: the edge half the edge driver wrote.
    #[arg(long, value_name = "FILE", global = true)]
    pub edge_half: Option<std::path::PathBuf>,
    /// init --over-ssh --side core: the edge half's sha256 as the edge driver
    /// printed it, read from there and confirmed here (the full digest, or its
    /// first 32 hex digits; spaces and dashes are ignored).
    #[arg(long, value_name = "SHA256", global = true)]
    pub edge_half_sha256: Option<String>,
}

impl MeasureArgs {
    pub fn is_empty(&self) -> bool {
        self.over_ssh.is_none()
            && self.ssh_edge_group.is_none()
            && self.ssh_core_group.is_none()
            && self.edge_host.is_none()
            && self.core_host.is_none()
            && self.router_container.is_none()
            && self.side.is_none()
            && self.half_out.is_none()
            && self.edge_half.is_none()
            && self.edge_half_sha256.is_none()
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
                "--ssh-edge-group, --ssh-core-group, --edge-host, --core-host, \
                 --router-container, --side, --half-out and --edge-half choose what \
                 `init --over-ssh <peering>` measures; \
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
                    format!(
                        "{flag} `{v}` is not an inventory name (letters, digits, `_`, `.`, `-`)"
                    ),
                ));
            }
        }
        match (self.side.as_deref(), &self.half_out, &self.edge_half) {
            (None, None, None) | (Some("edge"), Some(_), None) | (Some("core"), None, Some(_)) => {}
            _ => {
                return Err(anyhow::anyhow!(
                    "--side edge takes --half-out <file> (where the edge half is written), and                      --side core takes --edge-half <file> (the edge half to join); neither file                      is read or written without its side"
                ))
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
///
/// With `--side edge`, only the edge is resolved and reached; its half is
/// written to `--half-out` and nothing is built or signed. With `--side core`,
/// only the core is resolved and reached; the edge half is read from
/// `--edge-half`, joined, and the package is built and signed here.
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
    match m.side.as_deref() {
        None => {
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
        Some("edge") => {
            let path = m.half_out.as_ref().expect("checked by peering()");
            let inv = peering_inventory(repo, &edge_group)?;
            let edge = target_in(&inv, &edge_group, m.edge_host.as_deref(), "--edge-host")?;
            let interface = declared_interface(site, a)?;
            let half = measure_edge(&edge, interface.as_deref())?;
            let digest = write_edge_half(path, peering, &edge, interface.as_deref(), &half)?;
            writeln!(
                out,
                "phase 3, edge half of peering {peering}: measured on {} and written to {} (sha256 {digest}). Nothing was built or signed.\n\
                 Carry the file to the controller, then run there: goatbuilder federate init --over-ssh {peering} --side core --edge-half <file> (with the same init flags)",
                edge.name,
                path.display()
            )?;
            Ok(0)
        }
        Some(_) => {
            let path = m.edge_half.as_ref().expect("checked by peering()");
            let imported = read_edge_half(path, peering)?;
            let inv = peering_inventory(repo, &edge_group)?;
            let core = target_in(&inv, &core_group, m.core_host.as_deref(), "--core-host")?;
            eprintln!(
                "phase 3: the edge half of peering {peering} from {} (measured on {}, sha256 {}), joined with the core measured here",
                path.display(),
                imported.edge_host,
                imported.sha256
            );
            let router = m.router_container.clone();
            let measure = move |interface: Option<&str>| -> Result<Measured> {
                if interface != imported.interface.as_deref() {
                    return Err(coded(
                        "GB-114",
                        format!(
                            "the edge half {} cannot be used for peering {peering} (GB-114): it was measured for interface {:?}, and this package asks for {interface:?}. Measure the edge half again with the same declaration and init flags",
                            path.display(),
                            imported.interface
                        ),
                    ));
                }
                let c = measure_core(&core, router.as_deref())?;
                Ok(join(imported.half.clone(), c))
            };
            goat_federation::verbs::run_init_measured(site, a, out, &measure)
        }
    }
}

/// Whether the driver's phase 3 measures over SSH (`FED_MEASURE` unset, empty
/// or `ssh`, the default) or the machine the driver runs on (`here`).
pub fn driver_measures_over_ssh() -> bool {
    crate::federate::runenv::var("FED_MEASURE").ok().as_deref() != Some("here")
}

/// Any other value of `FED_MEASURE` is refused, as peering-standup.sh refuses
/// it: a misspelt `ssh` would otherwise measure the wrong machine.
pub fn check_driver_measure() -> Result<()> {
    match crate::federate::runenv::var("FED_MEASURE").ok().as_deref() {
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
            ))
            .fabric_fips_image,
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
        assert!(router_refusal(&measured(Ok(true)).fabric_fips_image, &choice, &core).is_ok());
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
            declaration: None,
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

    /// `init --over-ssh` with nothing naming the core router: discovery runs on
    /// the core host (here this machine), finds no router or several, and the
    /// GB-113 refusal names both ways to pick one, the flag and the variable
    /// the one command passes as it.
    #[test]
    fn a_router_discovery_refusal_names_the_flag_and_the_variable() {
        let inv = json!({
            "_meta": {"hostvars": {
                "station": {"ansible_connection": "local", "ansible_become": false}
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
        match run_init(repo.path(), &site, &init_args(), &m, &mut out) {
            Err(e) if code(&e).as_deref() == Some("GB-113") => {
                let text = format!("{e:#}");
                assert!(
                    text.contains("--router-container") && text.contains("FED_ROUTER_CONTAINER"),
                    "{text}"
                );
            }
            Err(e) => panic!("expected GB-113, got {e:#}"),
            Ok(_) => eprintln!("one core router runs on this machine: nothing to refuse, skipped"),
        }
        assert!(ROUTER_KNOB.contains(ROUTER_FLAG) && ROUTER_KNOB.contains("FED_ROUTER_CONTAINER"));
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

    // --- phase 3 in two halves (docs/design/federation-two-drivers.md) -------

    /// An inventory whose `reachable` group is this machine and whose
    /// `unreachable` group is a host ssh can never reach (its ProxyCommand
    /// fails at once, so the test neither hangs nor needs a network).
    fn split_inventory(edge_reachable: bool) -> Value {
        let here = json!({"ansible_connection": "local", "ansible_become": false, "zenoh_stack": "gb-b10-test-no-such"});
        let gone = json!({"ansible_host": "192.0.2.99", "ansible_user": "goat", "ansible_become": false,
                          "ansible_ssh_common_args": "-o ProxyCommand=false"});
        let (e, c) = if edge_reachable {
            (here, gone)
        } else {
            (gone, here)
        };
        json!({
            "_meta": {"hostvars": {"edge-x": e, "core-x": c}},
            "all": {"children": ["xab_edge", "xab_core"]},
            "xab_edge": {"hosts": ["edge-x"]},
            "xab_core": {"hosts": ["core-x"]}
        })
    }

    /// B10: the edge driver reaches the edge and not the core. Phase 3's edge
    /// half runs there alone and writes its result for the controller; the
    /// core is never asked. Today phase 3 reaches both hosts in one call, so
    /// this run is refused (GB-111) at the core.
    #[test]
    fn phase_3_measures_the_edge_half_with_only_the_edge_reachable() {
        let repo = checkout_with_inventory(&split_inventory(true));
        let ctx = tempfile::tempdir().unwrap();
        let site = site(ctx.path());
        let half = ctx.path().join("edge-half.json");
        let m = MeasureArgs {
            over_ssh: Some("xab".into()),
            side: Some("edge".into()),
            half_out: Some(half.clone()),
            ..Default::default()
        };
        let mut out = Vec::new();
        let rc = run_init(repo.path(), &site, &init_args(), &m, &mut out)
            .unwrap_or_else(|e| panic!("the edge half needs only the edge: {e:#}"));
        assert_eq!(rc, 0);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&half).unwrap()).unwrap();
        assert_eq!(v["schema"], EDGE_HALF_SCHEMA);
        assert_eq!(v["peering"], "xab");
        assert_eq!(v["edge_host"], "edge-x");
        assert!(
            v["transports"]["source"]
                .as_str()
                .unwrap()
                .contains("edge-x"),
            "{v}"
        );
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("sha256") && text.contains("--side core --edge-half"),
            "{text}"
        );
    }

    /// B10: the controller reaches the core and not the edge. Phase 3's core
    /// half measures the core alone and joins the edge half it is given; the
    /// edge is never asked. No router runs here under the test's name, so the
    /// core half's own refusal (GB-113, on core-x) is the proof that it got
    /// past the edge without reaching it.
    #[test]
    fn phase_3_joins_an_edge_half_and_measures_the_core_with_only_the_core_reachable() {
        let ctx = tempfile::tempdir().unwrap();
        let site = site(ctx.path());
        let half = ctx.path().join("edge-half.json");
        // The edge driver's half, made where the edge is reachable.
        let edge_repo = checkout_with_inventory(&split_inventory(true));
        let m = MeasureArgs {
            over_ssh: Some("xab".into()),
            side: Some("edge".into()),
            half_out: Some(half.clone()),
            ..Default::default()
        };
        run_init(edge_repo.path(), &site, &init_args(), &m, &mut Vec::new()).unwrap();
        // The controller, where only the core is.
        let repo = checkout_with_inventory(&split_inventory(false));
        let m = MeasureArgs {
            over_ssh: Some("xab".into()),
            side: Some("core".into()),
            edge_half: Some(half.clone()),
            ..Default::default()
        };
        let e = run_init(repo.path(), &site, &init_args(), &m, &mut Vec::new()).unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("GB-113"), "{e:#}");
        assert!(format!("{e:#}").contains("core-x"), "{e:#}");
        // An edge half for another peering, or altered, is refused before the core is asked.
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&half).unwrap()).unwrap();
        v["peering"] = json!("xac");
        std::fs::write(&half, v.to_string()).unwrap();
        let e = run_init(repo.path(), &site, &init_args(), &m, &mut Vec::new()).unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("GB-114"), "{e:#}");
        assert!(format!("{e:#}").contains("xac"), "{e:#}");
    }

    /// A probe that answers each read from a table and records what it was
    /// asked, in order, under its name.
    struct Table {
        name: &'static str,
        log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }

    impl Probe for Table {
        fn run(&self, bin: &str, args: &[&str]) -> Result<String, String> {
            self.log
                .borrow_mut()
                .push(format!("{} run {bin} {}", self.name, args.join(" ")));
            match bin {
                "wg" => Ok("wg-fed0\n".into()),
                "docker" if args.first() == Some(&"inspect") => {
                    Ok("abc\ttrue\tghcr.io/example-org/zenoh@sha256:1111\n".into())
                }
                _ => Err(format!("{bin}: not on this host")),
            }
        }
        fn run_reading_output(&self, bin: &str, args: &[&str]) -> Result<String, String> {
            self.log
                .borrow_mut()
                .push(format!("{} out {bin} {}", self.name, args.join(" ")));
            Ok("inactive\n".into())
        }
        fn read_file(&self, path: &str) -> std::io::Result<String> {
            self.log
                .borrow_mut()
                .push(format!("{} read {path}", self.name));
            match path {
                "/proc/sys/crypto/fips_enabled" => Ok("1\n".into()),
                p if p.ends_with("/mtu") => Ok("1420\n".into()),
                _ => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "absent")),
            }
        }
        fn place(&self) -> String {
            format!("on {}", self.name)
        }
    }

    /// The one-machine run is unchanged: the two halves, joined, are what
    /// `derive::measure` measures, from the same reads made in the same order.
    #[test]
    fn the_two_halves_joined_are_the_one_machine_measurement() {
        let probes = || {
            let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            (
                Table {
                    name: "edge",
                    log: log.clone(),
                },
                Table {
                    name: "core",
                    log: log.clone(),
                },
                log,
            )
        };
        let router = RouterTarget::Named {
            name: "xaa-zenoh-router",
            from: "the test",
        };
        let (e, c, whole_log) = probes();
        let whole = derive::measure(
            &e,
            &c,
            router,
            RUNTIMES,
            ROUTER_KNOB,
            Some("wg-fed0"),
            Some("/etc/spire/server.conf"),
        );
        let (e, c, split_log) = probes();
        let core = core_half(&c, router, Some("/etc/spire/server.conf"));
        let split = join(edge_half(&e, Some("wg-fed0")), core);
        assert_eq!(format!("{whole:?}"), format!("{split:?}"));
        assert_eq!(*whole_log.borrow(), *split_log.borrow());
        assert!(
            whole_log.borrow().iter().any(|l| l.starts_with("edge "))
                && whole_log.borrow().iter().any(|l| l.starts_with("core "))
        );
    }

    /// The edge half survives the file it is carried in: what is read back is
    /// what was measured, field names included, and the digest is the file's.
    #[test]
    fn an_edge_half_reads_back_as_it_was_measured() {
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let half = edge_half(&Table { name: "edge", log }, Some("wg-fed0"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("half.json");
        let edge = target_in(&inventory(), "xab_edge", None, "--edge-host").unwrap();
        let digest = write_edge_half(&path, "xab", &edge, Some("wg-fed0"), &half).unwrap();
        let back = read_edge_half(&path, "xab").unwrap();
        assert_eq!(format!("{:?}", back.half), format!("{half:?}"));
        assert_eq!(
            (
                back.interface.as_deref(),
                back.edge_host.as_str(),
                back.sha256.as_str()
            ),
            (Some("wg-fed0"), "xaa-edge", digest.as_str())
        );
        // Another schema, a field that is not the edge's, and a missing file are GB-114.
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["transports"]["field"] = json!("capabilities.fabric_version");
        std::fs::write(&path, v.to_string()).unwrap();
        assert_eq!(
            code(&read_edge_half(&path, "xab").unwrap_err()).as_deref(),
            Some("GB-114")
        );
        v["schema"] = json!("something/v9");
        std::fs::write(&path, v.to_string()).unwrap();
        assert_eq!(
            code(&read_edge_half(&path, "xab").unwrap_err()).as_deref(),
            Some("GB-114")
        );
        assert_eq!(
            code(&read_edge_half(&dir.path().join("none.json"), "xab").unwrap_err()).as_deref(),
            Some("GB-114")
        );
    }

    #[test]
    fn a_side_takes_its_own_file_and_no_other() {
        let m = |side: Option<&str>, out: bool, half: bool| MeasureArgs {
            over_ssh: Some("xab".into()),
            side: side.map(String::from),
            half_out: out.then(|| "h.json".into()),
            edge_half: half.then(|| "h.json".into()),
            ..Default::default()
        };
        assert!(m(Some("edge"), true, false).peering().is_ok());
        assert!(m(Some("core"), false, true).peering().is_ok());
        for bad in [
            m(Some("edge"), false, false),
            m(Some("core"), false, false),
            m(Some("edge"), false, true),
            m(None, true, false),
            m(None, false, true),
        ] {
            assert!(bad
                .peering()
                .unwrap_err()
                .to_string()
                .contains("--side edge takes --half-out"));
        }
        let e = MeasureArgs {
            side: Some("edge".into()),
            ..Default::default()
        }
        .peering()
        .unwrap_err();
        assert!(e.to_string().contains("without --over-ssh"), "{e}");
    }

    /// The edge half carries the edge's WireGuard public key, which the core
    /// side signs into our package. A half changed after the edge driver wrote
    /// it, here its key, is refused (GB-114) against the digest the edge driver
    /// printed and the operator confirms, before the core is asked.
    #[test]
    fn a_half_whose_key_was_changed_in_transit_is_refused() {
        let ctx = tempfile::tempdir().unwrap();
        let site = site(ctx.path());
        let half = ctx.path().join("edge-half.json");
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut measured = edge_half(&Table { name: "edge", log }, None);
        measured.public_key = Some(derive::Derived { field: derive::transport_public_key_on(&Nothing, "wg0").field, outcome: Ok("Ym9yaW5nIGtleSBmb3IgYSB0ZXN0IG9ubHkgMDAwMDA=".into()), source: "wg show wg-fed0 public-key on edge-x".into() });
        let edge = target_in(&split_inventory(true), "xab_edge", None, "--edge-host").unwrap();
        let digest = write_edge_half(&half, "xab", &edge, None, &measured).unwrap();
        // In transit, someone puts their own key in the half.
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&half).unwrap()).unwrap();
        v["public_key"]["ok"] = json!("YXR0YWNrZXIncyBrZXkgaW5zdGVhZCBvZiBvdXJzMDA=");
        std::fs::write(&half, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        let repo = checkout_with_inventory(&split_inventory(false));
        let m = MeasureArgs { over_ssh: Some("xab".into()), side: Some("core".into()), edge_half: Some(half.clone()), edge_half_sha256: Some(digest.clone()), ..Default::default() };
        let e = run_init(repo.path(), &site, &init_args(), &m, &mut Vec::new()).unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("GB-114"), "{e:#}");
        assert!(format!("{e:#}").contains(&digest[..32]), "names what was confirmed: {e:#}");
    }
}
