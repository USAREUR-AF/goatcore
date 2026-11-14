//! The plan's host firewall entries (docs/design/federation-plan.md, "Host
//! firewall"): for each edge and core host with `goat_host_firewall_baseline`
//! true, the ruleset the baseline would load and its hash, from the same
//! renderer the role runs (ops/ansible/roles/goat-host-firewall/files/
//! goat-host-firewall-render.py), before anything is applied.
//!
//! The input is assembled the way the role assembles `_hf_input`
//! (roles/goat-host-firewall/tasks/main.yml) and the plays build each peering's
//! entry (peering-wg.yml, peering-edge.yml, peering-republisher.yml), from the
//! same variables: inventory host vars, host_vars, the group's vars, its parent
//! group's and `all`, the role's default, and apply's rendered transport file.
//! The fields the role reads from the live host (bridge_interfaces,
//! ephemeral_ports, uplink_interfaces, must_reach, listening_ss) are left out:
//! they are checks only and never change the ruleset or its hash, so the plan's
//! hash equals the one the deploy prints for the same declaration.
//!
//! A value that resolves only inside a play (a template beyond
//! `{{ var | default(x) }}`), a missing agreement or counterparty package, or a
//! renderer refusal makes the host BLOCKED, naming why. Nothing is guessed.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serde_yaml::Value as Y;

use super::hostalloc::agreement_of;

/// The renderer, relative to the repository.
pub const RENDERER: &str =
    "ops/ansible/roles/goat-host-firewall/files/goat-host-firewall-render.py";

/// The fields of `_hf_input.host` read from the live host. They never change
/// the ruleset or its hash, and the plan cannot read them, so it leaves them out.
#[cfg_attr(not(test), allow(dead_code))]
pub const LIVE_HOST_FIELDS: &[&str] =
    &["bridge_interfaces", "ephemeral_ports", "uplink_interfaces"];

/// The top-level live fields of `_hf_input`, left out for the same reason.
#[cfg_attr(not(test), allow(dead_code))]
pub const LIVE_INPUT_FIELDS: &[&str] = &["must_reach", "listening_ss", "listening"];

/// One host's entry in the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostFirewall {
    pub host: String,
    /// `ansible_host`, else the inventory name: a host that moves is a changed input.
    pub address: String,
    pub group: String,
    /// NOT APPLICABLE (baseline off), PASS (rendered), or BLOCKED.
    pub outcome: String,
    pub detail: String,
    /// The renderer's refusal code (GB-210..215), when it refused.
    #[serde(default)]
    pub code: Option<String>,
    /// The ruleset's sha256, as the renderer and the role print it.
    #[serde(default)]
    pub sha256: Option<String>,
    /// The nft ruleset text the baseline would load.
    #[serde(default)]
    pub ruleset: Option<String>,
}

/// The peering inventory as Ansible reads it for variables: each group's
/// direct hosts, its children, its `[group:vars]`, and each host line's vars.
#[derive(Default)]
struct Inventory {
    members: std::collections::BTreeMap<String, Vec<String>>,
    children: std::collections::BTreeMap<String, Vec<String>>,
    group_vars: std::collections::BTreeMap<String, Vec<(String, Y)>>,
    host_vars: std::collections::BTreeMap<String, Vec<(String, Y)>>,
}

fn ini_value(v: &str) -> Y {
    let t = v.trim();
    if (t.starts_with('"') && t.ends_with('"') && t.len() >= 2)
        || (t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2)
    {
        return Y::String(t[1..t.len() - 1].to_string());
    }
    serde_yaml::from_str::<Y>(t).unwrap_or_else(|_| Y::String(t.to_string()))
}

fn parse_inventory(repo: &Path) -> Inventory {
    let text = super::plan::peering_inventory_text(repo);
    let mut inv = Inventory::default();
    let mut section: Option<(String, String)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest.split(']').next().unwrap_or_default().trim();
            section = Some(match name.split_once(':') {
                Some((g, k)) => (g.to_string(), k.to_string()),
                None => (name.to_string(), String::new()),
            });
            continue;
        }
        let Some((g, kind)) = &section else { continue };
        let body = line.split(" #").next().unwrap_or(line);
        match kind.as_str() {
            "" => {
                let mut words = body.split_whitespace();
                let Some(name) = words.next() else { continue };
                inv.members
                    .entry(g.clone())
                    .or_default()
                    .push(name.to_string());
                for w in words {
                    if let Some((k, v)) = w.split_once('=') {
                        inv.host_vars
                            .entry(name.to_string())
                            .or_default()
                            .push((k.to_string(), ini_value(v)));
                    }
                }
            }
            "children" => inv
                .children
                .entry(g.clone())
                .or_default()
                .push(body.trim().to_string()),
            "vars" => {
                if let Some((k, v)) = body.split_once('=') {
                    inv.group_vars
                        .entry(g.clone())
                        .or_default()
                        .push((k.trim().to_string(), ini_value(v)));
                }
            }
            _ => {}
        }
    }
    inv
}

impl Inventory {
    fn parents(&self, g: &str) -> Vec<String> {
        self.children
            .iter()
            .filter(|(_, cs)| cs.iter().any(|c| c == g))
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// Ansible's depth: `all` is 0, a group with no parent 1, else one more
    /// than its deepest parent.
    fn depth(&self, g: &str, seen: &mut Vec<String>) -> u32 {
        if seen.iter().any(|s| s == g) {
            return 1;
        }
        seen.push(g.to_string());
        let ps = self.parents(g);
        if ps.is_empty() {
            1
        } else {
            1 + ps.iter().map(|p| self.depth(p, seen)).max().unwrap_or(0)
        }
    }

    /// Every group the host is in, directly or through children, most
    /// specific first (deeper first; at one depth, the later name first, as
    /// Ansible applies them in name order and the last wins), then `all`.
    fn groups_of(&self, host: &str) -> Vec<String> {
        let mut gs: Vec<String> = self
            .members
            .iter()
            .filter(|(_, hs)| hs.iter().any(|h| h == host))
            .map(|(g, _)| g.clone())
            .collect();
        let mut i = 0;
        while i < gs.len() {
            for p in self.parents(&gs[i]) {
                if !gs.contains(&p) && p != "all" {
                    gs.push(p);
                }
            }
            i += 1;
        }
        let mut keyed: Vec<(u32, String)> = gs
            .into_iter()
            .map(|g| (self.depth(&g, &mut Vec::new()), g))
            .collect();
        keyed.sort();
        keyed.reverse();
        let mut out: Vec<String> = keyed.into_iter().map(|(_, g)| g).collect();
        out.push("all".to_string());
        out
    }
}

/// Where a host's variables come from, in Ansible's order of precedence (the
/// first that sets a key wins): apply's rendered file (the plays load it with
/// include_vars), host_vars/, the host's inventory line, then each group from
/// the most specific (its group_vars/ files, then its `[group:vars]`), then the
/// role's default.
struct Scope<'a> {
    repo: &'a Path,
    inv: Inventory,
    host: String,
    groups: Vec<String>,
    /// apply's rendered group_vars, when the play loads them and they exist.
    applied: Option<PathBuf>,
    /// The roles whose defaults the play has in scope: goat-host-firewall and
    /// the play's own role.
    roles: Vec<&'static str>,
    /// Vars files that exist and do not parse. Ansible would use them or
    /// fail, so the plan does not fall through to a lower layer: any entry
    /// here makes the input an error.
    broken: std::cell::RefCell<Vec<String>>,
}

/// Every vars file under `dir`, recursively, in name order, hidden names left
/// out (as Ansible reads a vars directory).
fn yaml_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        // Ansible's vars loaders skip hidden files and directories.
        if p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if p.is_dir() {
            out.extend(yaml_files(&p));
        } else if p
            .extension()
            .is_some_and(|x| x == "yml" || x == "yaml" || x == "json")
        {
            out.push(p);
        }
    }
    out
}

/// The vars files of one host or group in a vars directory: `<name>`,
/// `<name>.yml|.yaml|.json`, and everything under `<name>/`.
fn named_files(dir: &Path, name: &str) -> Vec<PathBuf> {
    let mut files = vec![
        dir.join(name),
        dir.join(format!("{name}.yml")),
        dir.join(format!("{name}.yaml")),
        dir.join(format!("{name}.json")),
    ];
    files.retain(|f| f.is_file());
    files.extend(yaml_files(&dir.join(name)));
    files
}

fn inventory_value(kv: Option<&Vec<(String, Y)>>, key: &str) -> Option<Y> {
    kv.and_then(|kv| kv.iter().rev().find(|(k, _)| k == key))
        .map(|(_, v)| v.clone())
}

impl Scope<'_> {
    /// The last of `files` that sets `key` (a null counts: it is set). A file
    /// that does not parse is recorded in `broken`.
    fn last_in(&self, files: &[PathBuf], key: &str) -> Option<Y> {
        let mut found = None;
        for f in files {
            let Ok(text) = std::fs::read_to_string(f) else {
                continue;
            };
            match serde_yaml::from_str::<Y>(&text) {
                Ok(doc) => {
                    if let Some(v) = doc.get(key) {
                        found = Some(v.clone());
                    }
                }
                Err(e) => {
                    let msg = format!("{} does not parse ({e})", f.display());
                    let mut b = self.broken.borrow_mut();
                    if !b.contains(&msg) {
                        b.push(msg);
                    }
                }
            }
        }
        found
    }

    /// The first source that sets `key`, in Ansible's precedence, highest
    /// first: apply's include_vars; playbook host_vars/, inventory host_vars/,
    /// the inventory line; then, for the groups other than `all` (deepest
    /// first), every playbook group_vars/, then every inventory group_vars/,
    /// then every inventory `[group:vars]`; then the same three for `all`.
    fn raw(&self, key: &str) -> Option<Y> {
        let pb = self.repo.join("ops/ansible");
        let inv_dir = pb.join("inventory");
        if let Some(v) = self
            .applied
            .as_ref()
            .and_then(|d| self.last_in(&yaml_files(d), key))
        {
            return Some(v);
        }
        for dir in [pb.join("host_vars"), inv_dir.join("host_vars")] {
            if let Some(v) = self.last_in(&named_files(&dir, &self.host), key) {
                return Some(v);
            }
        }
        if let Some(v) = inventory_value(self.inv.host_vars.get(&self.host), key) {
            return Some(v);
        }
        let (all, others): (Vec<&String>, Vec<&String>) =
            self.groups.iter().partition(|g| g.as_str() == "all");
        for layer in [others, all] {
            for g in &layer {
                if let Some(v) = self.last_in(&named_files(&pb.join("group_vars"), g), key) {
                    return Some(v);
                }
            }
            for g in &layer {
                if let Some(v) = self.last_in(&named_files(&inv_dir.join("group_vars"), g), key) {
                    return Some(v);
                }
            }
            for g in &layer {
                if let Some(v) = inventory_value(self.inv.group_vars.get(*g), key) {
                    return Some(v);
                }
            }
        }
        None
    }

    /// A default from any role the play has in scope (the `role` hint first).
    fn role_default(&self, role: Option<&str>, key: &str) -> Option<Y> {
        let mut roles: Vec<&str> = role.into_iter().collect();
        roles.extend(self.roles.iter().copied().filter(|r| Some(*r) != role));
        roles.into_iter().find_map(|r| {
            self.last_in(
                &[self
                    .repo
                    .join(format!("ops/ansible/roles/{r}/defaults/main.yml"))],
                key,
            )
        })
    }

    /// An error naming every vars file that did not parse, if any did.
    fn check_parsed(&self) -> Result<(), String> {
        let b = self.broken.borrow();
        if b.is_empty() {
            Ok(())
        } else {
            Err(b.join("; "))
        }
    }

    /// The value of `key`, else the role's default, else `fallback`, evaluated.
    /// A declared null is refused: Ansible keeps it as None, which no field
    /// here reads the way the plan could repeat.
    fn get(&self, key: &str, role: Option<&str>, fallback: Y) -> Result<Y, String> {
        let v = match self.raw(key) {
            Some(Y::Null) => {
                return Err(format!(
                    "{key} is declared empty (null); give it a value or remove it"
                ))
            }
            Some(v) => v,
            None => self.role_default(role, key).unwrap_or(fallback),
        };
        self.eval(&v, key, 0, role)
    }

    fn lookup(&self, name: &str, role: Option<&str>) -> Option<Y> {
        self.raw(name).or_else(|| self.role_default(role, name))
    }

    /// A Jinja literal or variable inside `default(...)`.
    fn default_arg(
        &self,
        arg: &str,
        key: &str,
        depth: u32,
        role: Option<&str>,
    ) -> Result<Y, String> {
        let a = arg.trim();
        if (a.starts_with('\'') && a.ends_with('\'') || a.starts_with('"') && a.ends_with('"'))
            && a.len() >= 2
        {
            return Ok(Y::String(a[1..a.len() - 1].to_string()));
        }
        match a {
            "true" | "True" => return Ok(Y::Bool(true)),
            "false" | "False" => return Ok(Y::Bool(false)),
            "none" | "None" => return Ok(Y::Null),
            "[]" => return Ok(Y::Sequence(vec![])),
            "{}" => return Ok(Y::Mapping(Default::default())),
            _ => {}
        }
        if let Ok(n) = a.parse::<i64>() {
            return Ok(Y::Number(n.into()));
        }
        if regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$")
            .expect("regex")
            .is_match(a)
        {
            return match self.lookup(a, role) {
                Some(x) => self.eval(&x, a, depth + 1, role),
                None => Err(format!("{key} falls back to {a}, which nothing declares")),
            };
        }
        Err(format!(
            "{key} has default({a}), which the plan does not evaluate outside a play"
        ))
    }

    fn eval(&self, v: &Y, key: &str, depth: u32, role: Option<&str>) -> Result<Y, String> {
        if depth > 6 {
            return Err(format!("{key} refers to itself"));
        }
        let s = match v {
            Y::String(s) => s,
            Y::Sequence(_) | Y::Mapping(_) => {
                return if serde_yaml::to_string(v).unwrap_or_default().contains("{{") {
                    Err(format!("{key} holds a template inside a list or mapping, which resolves only inside a play"))
                } else {
                    Ok(v.clone())
                };
            }
            _ => return Ok(v.clone()),
        };
        if !s.contains("{{") && !s.contains("{%") {
            return Ok(v.clone());
        }
        let expr = regex::Regex::new(r"\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*(?:\|\s*default\(([^()]*)\))?\s*(?:\|\s*(int|bool|trim|string))?\s*\}\}").expect("regex");
        let one = |c: &regex::Captures| -> Result<Y, String> {
            let inner = match self.lookup(&c[1], role) {
                Some(Y::Null) if c.get(2).is_none() => Y::Null,
                Some(x) => self.eval(&x, &c[1], depth + 1, role)?,
                None => match c.get(2) {
                    Some(d) => self.default_arg(d.as_str(), key, depth, role)?,
                    None => return Err(format!("{key} needs {}, which nothing declares", &c[1])),
                },
            };
            Ok(match c.get(3).map(|m| m.as_str()) {
                Some("int") => Y::Number(int_of(&inner).unwrap_or(0).into()),
                Some("bool") => Y::Bool(truthy(&inner)),
                Some("trim") => Y::String(jinja_str(&inner)?.trim().to_string()),
                Some("string") => Y::String(jinja_str(&inner)?),
                _ => inner,
            })
        };
        // A string that is one expression keeps the expression's type, as a
        // single-expression template does in Ansible.
        if let Some(c) = expr.captures(s) {
            if c.get(0).map(|m| m.as_str()) == Some(s.trim()) {
                return one(&c);
            }
        }
        let mut out = String::new();
        let mut last = 0;
        for c in expr.captures_iter(s) {
            let m = c.get(0).expect("match");
            out.push_str(&s[last..m.start()]);
            out.push_str(&jinja_str(&one(&c)?)?);
            last = m.end();
        }
        out.push_str(&s[last..]);
        if out.contains("{{") || out.contains("{%") {
            return Err(format!(
                "{key} is a template that resolves only inside a play"
            ));
        }
        Ok(Y::String(out))
    }
}

/// A value as Jinja prints it into a string.
fn jinja_str(v: &Y) -> Result<String, String> {
    Ok(match v {
        Y::String(s) => s.clone(),
        Y::Number(n) => n.to_string(),
        Y::Bool(true) => "True".into(),
        Y::Bool(false) => "False".into(),
        Y::Null => "None".into(),
        _ => {
            return Err(
                "a list or mapping is interpolated into a string, which the plan does not repeat"
                    .into(),
            )
        }
    })
}

fn scalar(v: &Y) -> Option<String> {
    match v {
        Y::String(s) => Some(s.clone()),
        Y::Number(n) => Some(n.to_string()),
        Y::Bool(b) => Some(b.to_string()),
        Y::Null => Some(String::new()),
        _ => None,
    }
}

fn int_of(v: &Y) -> Option<i64> {
    match v {
        Y::Bool(b) => Some(*b as i64),
        _ => scalar(v).and_then(|s| s.trim().parse().ok()),
    }
}

/// Ansible's `bool`: true, yes, on, y, t, 1 (any case) are true.
fn truthy(v: &Y) -> bool {
    match v {
        Y::Bool(b) => *b,
        _ => scalar(v).is_some_and(|s| {
            matches!(
                s.trim().to_lowercase().as_str(),
                "true" | "yes" | "on" | "y" | "t" | "1"
            )
        }),
    }
}

fn to_json(v: &Y) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn ints(v: &Y, key: &str) -> Result<Value, String> {
    match v {
        Y::Sequence(xs) => xs
            .iter()
            .map(|x| {
                int_of(x)
                    .map(Value::from)
                    .ok_or_else(|| format!("{key}: {x:?} is not a number"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        other => Err(format!("{key} is not a list ({other:?})")),
    }
}

/// `_hf_input.host` without the live fields, as the role assembles it.
fn host_input(s: &Scope, profile: &str) -> Result<Value, String> {
    let fw = Some("goat-host-firewall");
    let g = |k: &str| s.get(k, fw, Y::Null);
    let ssh_port = int_of(&g("goat_host_firewall_ssh_port")?)
        .ok_or("goat_host_firewall_ssh_port is not a number")?;
    // The role's default for the QUIC port is a conditional, so it is read here
    // as the role writes it: goat_zenoh_quic_port (7448) when goat_zenoh_quic_enabled.
    let fabric_udp = match s.raw("goat_host_firewall_fabric_udp_port") {
        Some(Y::Null) => return Err("goat_host_firewall_fabric_udp_port is declared empty (null); give it a value or remove it".into()),
        Some(v) => s.eval(
            &v,
            "goat_host_firewall_fabric_udp_port",
            0,
            Some("goat-host-firewall"),
        )?,
        None => {
            if truthy(&s.get("goat_zenoh_quic_enabled", None, Y::Bool(false))?) {
                s.get("goat_zenoh_quic_port", None, Y::Number(7448.into()))?
            } else {
                Y::String(String::new())
            }
        }
    };
    let fabric_udp = match scalar(&fabric_udp).map(|x| x.trim().to_string()) {
        Some(x) if !x.is_empty() => Value::from(
            x.parse::<i64>()
                .map_err(|_| format!("fabric UDP port {x:?} is not a number"))?,
        ),
        _ => Value::Null,
    };
    let dns = g("goat_host_firewall_mesh_dns_port")?;
    let dns = match scalar(&dns) {
        Some(x) if !x.is_empty() => {
            Value::from(int_of(&dns).ok_or("goat_host_firewall_mesh_dns_port is not a number")?)
        }
        _ => Value::Null,
    };
    // The core play sets these itself (peering-republisher.yml): the SPIRE
    // bundle port only on the SPIRE server's host, and never any sources.
    let (spire_port, spire_sources) = if profile == "core" {
        let on = truthy(&s.get("peering_spire_enabled", None, Y::Bool(false))?)
            && scalar(&s.get("peering_spire_server_host", None, Y::String(String::new()))?)
                .as_deref()
                == Some(s.host.as_str());
        let port = if on {
            Value::from(
                int_of(&s.get("peering_spire_bundle_port", None, Y::Number(8443.into()))?)
                    .ok_or("peering_spire_bundle_port is not a number")?,
            )
        } else {
            json!("")
        };
        (port, json!([]))
    } else {
        (
            to_json(&g("goat_host_firewall_spire_bundle_port")?),
            to_json(&g("goat_host_firewall_spire_bundle_sources")?),
        )
    };
    Ok(json!({
        "ssh_port": ssh_port,
        "management_sources": to_json(&g("goat_host_firewall_management_sources")?),
        "mesh_interface": to_json(&g("goat_host_firewall_mesh_interface")?),
        "mesh_udp_ports": ints(&g("goat_host_firewall_mesh_udp_ports")?, "goat_host_firewall_mesh_udp_ports")?,
        "fabric_tcp_port": int_of(&g("goat_host_firewall_fabric_tcp_port")?).ok_or("goat_host_firewall_fabric_tcp_port is not a number")?,
        "fabric_udp_port": fabric_udp,
        "fabric_sources": to_json(&g("goat_host_firewall_fabric_sources")?),
        "mesh_dns_port": dns,
        "closed_listeners": to_json(&g("goat_host_firewall_closed_listeners")?),
        "management_prefix_floor": to_json(&g("goat_host_firewall_management_prefix_floor")?),
        "spire_bundle_port": spire_port,
        "spire_bundle_sources": spire_sources,
    }))
}

/// A literal IPv4 `addr:port` gives its address; anything else gives "" (the
/// plays leave a name unfiltered rather than guess).
fn literal_addr(endpoint: &str) -> String {
    let re = regex::Regex::new(r"^([0-9.]+)(?::[0-9]+)?$").expect("regex");
    re.captures(endpoint)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

/// The counterparty's offered endpoint, from its package (`transport_offer.endpoint`).
fn offered_endpoint(package: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(package).map_err(|_| {
        format!(
            "no counterparty package at {} (phase 5 places it)",
            package.display()
        )
    })?;
    let v: Value =
        serde_json::from_str(&text).map_err(|_| format!("{} is not JSON", package.display()))?;
    Ok(v["transport_offer"]["endpoint"]
        .as_str()
        .unwrap_or_default()
        .to_string())
}

/// The SPIRE fields both transport plays add to a peering entry.
fn spire_fields(s: &Scope, entry: &mut serde_json::Map<String, Value>) -> Result<(), String> {
    let on = truthy(&s.get("peering_spire_enabled", None, Y::Bool(false))?);
    entry.insert(
        "spire_outbound_port".into(),
        if on {
            Value::from(
                int_of(&s.get("peering_spire_outbound_port", None, Y::Number(18443.into()))?)
                    .ok_or("peering_spire_outbound_port is not a number")?,
            )
        } else {
            json!("")
        },
    );
    let server = scalar(&s.get("peering_spire_server_addr", None, Y::String(String::new()))?)
        .unwrap_or_default();
    let literal = regex::Regex::new(r"^[0-9.]+$")
        .expect("regex")
        .is_match(&server);
    entry.insert(
        "spire_server_addr".into(),
        json!(if literal { server } else { String::new() }),
    );
    Ok(())
}

/// One peering's entry, as peering-wg.yml or peering-edge.yml builds it.
fn peering_entry(s: &Scope, transport: &str, package: &Path) -> Result<Value, String> {
    let mut e = serde_json::Map::new();
    let get = |k: &str, role: &str| s.get(k, Some(role), Y::Null);
    match transport {
        "wireguard" => {
            let wg = "goat-peering-wg";
            let iface = scalar(&get("peering_wg_interface", wg)?).unwrap_or_default();
            let remote = scalar(&get("peering_wg_remote_link_ip", wg)?).unwrap_or_default();
            if remote.is_empty() {
                return Err(format!("the far link address of {iface} is not known until phase 9 renders the transport file"));
            }
            let fwd = scalar(&get("peering_wg_forwarder_endpoint", wg)?).unwrap_or_default();
            let fwd_re = regex::Regex::new(r"^[0-9.]+:[0-9]+$").expect("regex");
            e.insert("name".into(), json!(iface));
            e.insert("transport".into(), json!("wireguard"));
            e.insert(
                "listen_port".into(),
                json!(int_of(&get("peering_wg_listen_port", wg)?)
                    .ok_or("peering_wg_listen_port is not a number")?),
            );
            let endpoint = offered_endpoint(package)?;
            e.insert(
                "peer_addr".into(),
                json!(if fwd_re.is_match(&endpoint) {
                    literal_addr(&endpoint)
                } else {
                    String::new()
                }),
            );
            e.insert("tunnel_interface".into(), json!(iface));
            e.insert("remote_link_ip".into(), json!(remote));
            e.insert(
                "edge_port".into(),
                json!(int_of(&get("peering_wg_fabric_port", wg)?)
                    .ok_or("peering_wg_fabric_port is not a number")?),
            );
            e.insert(
                "extra_tcp_ports".into(),
                ints(
                    &get("peering_wg_extra_ports", wg)?,
                    "peering_wg_extra_ports",
                )?,
            );
            spire_fields(s, &mut e)?;
            e.insert(
                "via_forwarder".into(),
                json!(truthy(&get("peering_wg_via_forwarder", wg)?)),
            );
            e.insert(
                "carrier_interface".into(),
                to_json(&get("peering_wg_carrier_interface", wg)?),
            );
            e.insert(
                "carrier_remote_ip".into(),
                to_json(&get("peering_wg_carrier_remote_ip", wg)?),
            );
            e.insert(
                "carrier_listen_port".into(),
                json!(int_of(&get("peering_wg_carrier_listen_port", wg)?)
                    .ok_or("peering_wg_carrier_listen_port is not a number")?),
            );
            e.insert(
                "forwarder_addr".into(),
                json!(if fwd_re.is_match(&fwd) {
                    fwd.rsplit_once(':')
                        .map(|(a, _)| a.to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                }),
            );
        }
        "ipsec-fips" => {
            let ip = "goat-peering-edge";
            let name = scalar(&get("peering_edge_link_ifname", ip)?).unwrap_or_default();
            let ike = int_of(&get("peering_edge_ike_port", ip)?)
                .ok_or("peering_edge_ike_port is not a number")?;
            let mut ports = vec![ike];
            if ike != 4500 {
                ports.push(4500);
            }
            let remote_addr = scalar(&get("peering_edge_remote_addr", ip)?).unwrap_or_default();
            e.insert("name".into(), json!(name));
            e.insert("transport".into(), json!("ipsec-fips"));
            e.insert("ike_ports".into(), json!(ports));
            e.insert("peer_addr".into(), json!(literal_addr(&remote_addr)));
            e.insert("tunnel_interface".into(), json!(name));
            e.insert(
                "remote_link_ip".into(),
                to_json(&get("peering_edge_remote_link_ip", ip)?),
            );
            e.insert(
                "edge_port".into(),
                json!(int_of(&s.get(
                    "goat_zenoh_edge_listen_port",
                    None,
                    Y::Number(7448.into())
                )?)
                .ok_or("goat_zenoh_edge_listen_port is not a number")?),
            );
            e.insert(
                "extra_tcp_ports".into(),
                ints(
                    &s.get("peering_wg_extra_ports", None, Y::Sequence(vec![]))?,
                    "peering_wg_extra_ports",
                )?,
            );
            spire_fields(s, &mut e)?;
        }
        "" => {
            return Err(
                "no agreement yet, so the transport (and the tunnel the rules admit) is not known"
                    .into(),
            )
        }
        other => {
            return Err(format!(
                "the agreement chose transport '{other}', which has no play"
            ))
        }
    }
    Ok(Value::Object(e))
}

/// Everything the input for one host needs from the plan.
pub struct HostContext<'a> {
    pub repo: &'a Path,
    pub our_edge_group: &'a str,
    pub our_work: &'a Path,
    /// The counterparty package this run uses (FED_COUNTERPARTY, else <work>/theirs.canonical.json).
    pub our_package: PathBuf,
    /// Phase 1's probe answers, one `<inventory host>.json` per host, with the
    /// peerings the baseline stored on each edge (`host_firewall_peerings`).
    pub probe_dir: PathBuf,
}

/// The roles in scope in the play that applies the baseline to this kind of host.
fn play_roles(profile: &str, transport: &str) -> Vec<&'static str> {
    match (profile, transport) {
        ("core", _) => vec!["goat-host-firewall", "goat-fed-republisher"],
        (_, "ipsec-fips") => vec!["goat-host-firewall", "goat-peering-edge"],
        _ => vec!["goat-host-firewall", "goat-peering-wg"],
    }
}

fn scope<'a>(
    repo: &'a Path,
    host: &str,
    applied: Option<PathBuf>,
    roles: Vec<&'static str>,
) -> Scope<'a> {
    let inv = parse_inventory(repo);
    let groups = inv.groups_of(host);
    Scope {
        repo,
        inv,
        host: host.to_string(),
        groups,
        applied,
        roles,
        broken: Default::default(),
    }
}

/// The peerings the baseline stored on this edge, as phase 1's probe read them.
fn stored_peerings(probe_dir: &Path, host: &str) -> Result<Vec<Value>, String> {
    let f = probe_dir.join(format!("{host}.json"));
    let text = std::fs::read_to_string(&f).map_err(|_| {
        format!("the peerings stored on {host} were not read (phase 1's live probe reads them; it did not run, or the host did not answer)")
    })?;
    let doc: Value =
        serde_json::from_str(&text).map_err(|_| format!("{} is not JSON", f.display()))?;
    match &doc["host_firewall_peerings"] {
        Value::Array(xs) => Ok(xs.clone()),
        _ => Err(format!(
            "the probe of {host} did not read the peerings stored on it"
        )),
    }
}

/// The renderer input for one host, without the live fields. An edge host
/// carries the peerings the baseline stored on it, with this run's entry in
/// place of its stored copy, exactly as the role assembles them.
pub fn input(c: &HostContext, host: &str, profile: &str) -> Result<Value, String> {
    let transport = agreement_of(c.our_work).map(|(k, _)| k).unwrap_or_default();
    // peering-wg.yml and peering-republisher.yml load apply's rendered values
    // with include_vars; peering-edge.yml (ipsec-fips) does not.
    let applied = c.our_work.join(format!(
        "applied/ops/ansible/group_vars/{}",
        c.our_edge_group
    ));
    let loads_applied = profile == "core" || transport == "wireguard";
    // The play's include_vars fails when the directory is not there, so the
    // deploy would stop; the plan does not render as if it would not.
    if loads_applied && !applied.is_dir() {
        return Err(format!(
            "apply has not rendered {} (phase 9); the play loads it before the baseline",
            applied.display()
        ));
    }
    let s = scope(
        c.repo,
        host,
        (loads_applied && applied.is_dir()).then_some(applied),
        play_roles(profile, &transport),
    );
    let host_v = host_input(&s, profile)?;
    let mut peerings = Vec::new();
    if profile == "edge" {
        let ours = peering_entry(&s, &transport, &c.our_package)?;
        let name = ours["name"].clone();
        peerings = stored_peerings(&c.probe_dir, host)?
            .into_iter()
            .filter(|p| p["name"] != name)
            .collect();
        peerings.push(ours);
    }
    let override_v = to_json(&s.get(
        "goat_host_firewall_override",
        Some("goat-host-firewall"),
        Y::Mapping(Default::default()),
    )?);
    s.check_parsed()?;
    Ok(json!({
        "schema": "goat-host-firewall-input/v1",
        "profile": profile,
        "host": host_v,
        "peerings": peerings,
        "override": override_v,
    }))
}

/// What the renderer said.
#[derive(Debug, Clone, PartialEq)]
pub enum Render {
    Ruleset {
        sha256: String,
        ruleset: String,
    },
    Refused {
        code: Option<String>,
        message: String,
    },
}

/// Run the renderer on one input, as the role does (`--input - --format json`).
pub fn render(repo: &Path, input: &Value) -> Render {
    let script = repo.join(RENDERER);
    if !script.is_file() {
        return Render::Refused {
            code: None,
            message: format!("the renderer {RENDERER} is not in this checkout"),
        };
    }
    let child = crate::federate::runenv::command("python3")
        .arg(&script)
        .args(["--input", "-", "--format", "json"])
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            return Render::Refused {
                code: None,
                message: format!("python3 could not run: {e}"),
            }
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write as _;
        let _ = stdin.write_all(input.to_string().as_bytes());
    }
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            return Render::Refused {
                code: None,
                message: format!("the renderer did not finish: {e}"),
            }
        }
    };
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        let code = regex::Regex::new(r"\b(GB-2[0-9]{2})\b")
            .expect("regex")
            .captures(&stderr)
            .map(|c| c[1].to_string());
        return Render::Refused {
            code,
            message: if stderr.is_empty() {
                format!("exit {}", out.status)
            } else {
                stderr
            },
        };
    }
    let doc: Value = match serde_json::from_slice(&out.stdout) {
        Ok(d) => d,
        Err(e) => {
            return Render::Refused {
                code: None,
                message: format!("the renderer printed no JSON: {e}"),
            }
        }
    };
    match (doc["sha256"].as_str(), doc["ruleset"].as_str()) {
        (Some(h), Some(r)) => Render::Ruleset {
            sha256: h.to_string(),
            ruleset: r.to_string(),
        },
        _ => Render::Refused {
            code: None,
            message: "the renderer's JSON has no sha256 or ruleset".into(),
        },
    }
}

/// Whether the baseline is on for a host (`goat_host_firewall_baseline`). An
/// expression the plan cannot evaluate is an error, never "off".
pub fn baseline_on(repo: &Path, host: &str) -> Result<bool, String> {
    let s = scope(repo, host, None, vec!["goat-host-firewall"]);
    let v = s.get(
        "goat_host_firewall_baseline",
        Some("goat-host-firewall"),
        Y::Bool(false),
    )?;
    s.check_parsed()?;
    Ok(truthy(&v))
}

/// The plan's entry for one host.
pub fn entry(
    c: &HostContext,
    host: &str,
    address: &str,
    group: &str,
    profile: &str,
) -> HostFirewall {
    let base = |outcome: &str, detail: String| HostFirewall {
        host: host.to_string(),
        address: address.to_string(),
        group: group.to_string(),
        outcome: outcome.into(),
        detail,
        code: None,
        sha256: None,
        ruleset: None,
    };
    match baseline_on(c.repo, host) {
        Ok(false) => {
            return base(
                "NOT APPLICABLE",
                "host firewall baseline: off (goat_host_firewall_baseline is not true); nothing is rendered or applied".into(),
            )
        }
        Err(why) => return base("BLOCKED", format!("cannot tell whether the host firewall baseline is on: {why}")),
        Ok(true) => {}
    }
    let input = match input(c, host, profile) {
        Ok(i) => i,
        Err(why) => {
            return base(
                "BLOCKED",
                format!("host firewall baseline: on, and its input cannot be built: {why}"),
            )
        }
    };
    match render(c.repo, &input) {
        Render::Ruleset { sha256, ruleset } => HostFirewall {
            sha256: Some(sha256.clone()),
            ruleset: Some(ruleset),
            ..base(
                "PASS",
                format!("host firewall baseline: on; {profile} ruleset rendered, sha256 {sha256}"),
            )
        },
        Render::Refused { code, message } => HostFirewall {
            code: code.clone(),
            ..base(
                "BLOCKED",
                format!("host firewall baseline: on, and the renderer refused: {message}"),
            )
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    const FIXTURE: &str = "ops/federation/standup/fixtures/host-firewall";
    /// The fixture's variants: WireGuard (precedence, stored peerings, core
    /// SPIRE port), ipsec-fips (edge SPIRE forward, QUIC, empty DNS port,
    /// override), and WireGuard behind a forwarder.
    const VARIANTS: [&str; 3] = ["wireguard", "ipsec", "forwarder"];

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

    fn write(path: PathBuf, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// The shared fixture in a fresh repository and working directory, with
    /// the renderer and the defaults of every role the plays have in scope.
    struct Fixture {
        repo: tempfile::TempDir,
        work: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        variant("wireguard")
    }

    fn variant(name: &str) -> Fixture {
        let repo = tempfile::tempdir().unwrap();
        copy_dir(&real().join(FIXTURE).join(name).join("repo"), repo.path());
        for f in [
            RENDERER,
            "ops/ansible/roles/goat-host-firewall/defaults/main.yml",
            "ops/ansible/roles/goat-peering-wg/defaults/main.yml",
            "ops/ansible/roles/goat-peering-edge/defaults/main.yml",
            "ops/ansible/roles/goat-fed-republisher/defaults/main.yml",
        ] {
            std::fs::create_dir_all(repo.path().join(f).parent().unwrap()).unwrap();
            std::fs::copy(real().join(f), repo.path().join(f)).unwrap();
        }
        let work = tempfile::tempdir().unwrap();
        copy_dir(&real().join(FIXTURE).join(name).join("work"), work.path());
        Fixture { repo, work }
    }

    fn ctx(f: &Fixture) -> HostContext<'_> {
        HostContext {
            repo: f.repo.path(),
            our_edge_group: "velmara_edge",
            our_work: f.work.path(),
            our_package: f.work.path().join("theirs.canonical.json"),
            probe_dir: f.work.path().join("preflight/probe"),
        }
    }

    const HOSTS: [(&str, &str, &str, &str); 2] = [
        ("edge-a", "192.0.2.10", "velmara_edge", "edge"),
        ("core-a", "192.0.2.20", "velmara_core", "core"),
    ];

    /// The plan builds exactly the recorded input for each host. The Ansible
    /// side (tests/test_host_firewall_plan_input.py) evaluates the role's and
    /// the plays' own expressions on the same fixture and checks the renderer
    /// gives the same hash for its input as for this one: together, the plan's
    /// hash equals the deploy's for this declaration.
    #[test]
    fn the_plan_builds_the_recorded_input() {
        for v in VARIANTS {
            let f = variant(v);
            for (host, _, _, profile) in HOSTS {
                let got =
                    input(&ctx(&f), host, profile).unwrap_or_else(|e| panic!("{v}/{host}: {e}"));
                let path = real()
                    .join(FIXTURE)
                    .join(v)
                    .join(format!("expected/{host}.json"));
                if crate::federate::runenv::var_os("GOAT_RECORD_HOST_FIREWALL").is_some() {
                    write(
                        path.clone(),
                        &(serde_json::to_string_pretty(&got).unwrap() + "\n"),
                    );
                }
                let want: Value =
                    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                assert_eq!(
                    got,
                    want,
                    "{v}/{host}: the plan's input differs from {}",
                    path.display()
                );
                let e = entry(&ctx(&f), host, "x", "x", profile);
                assert_eq!(e.outcome, "PASS", "{v}/{host} renders: {}", e.detail);
            }
        }
    }

    /// The precedence the fixture exercises: ansible_port from three group
    /// layers,
    /// host_vars over the inventory line (mesh interface), apply's rendered
    /// file over a stale group_vars value (far link address), the SPIRE port on
    /// the SPIRE server's core host only, and the stored peerings with this
    /// run's entry in place of its stale stored copy.
    #[test]
    fn the_input_follows_ansible_precedence() {
        let f = fixture();
        let c = ctx(&f);
        let edge = input(&c, "edge-a", "edge").unwrap();
        // The playbook group_vars of a shallower group outrank the deeper
        // group's inventory group_vars and [group:vars].
        assert_eq!(edge["host"]["ssh_port"], 2400);
        assert_eq!(edge["host"]["mesh_interface"], "wt0");
        let names: Vec<&str> = edge["peerings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["wg-fed1", "wg-fed0"]);
        let ours = &edge["peerings"][1];
        assert_eq!(ours["remote_link_ip"], "169.254.10.1");
        assert_eq!(
            ours["listen_port"], 51823,
            "this run's entry, not the stale stored one"
        );
        assert_eq!(ours["peer_addr"], "203.0.113.7");
        let core = input(&c, "core-a", "core").unwrap();
        assert_eq!(core["host"]["spire_bundle_port"], 8443);
        assert_eq!(core["host"]["spire_bundle_sources"], json!([]));
        assert_eq!(core["peerings"], json!([]));
    }

    /// Baseline off: NOT APPLICABLE, and the renderer is not called.
    #[test]
    fn off_is_not_applicable() {
        let f = fixture();
        write(
            f.repo
                .path()
                .join("ops/ansible/group_vars/all/host-firewall.yml"),
            "goat_host_firewall_baseline: false\n",
        );
        let e = entry(&ctx(&f), "edge-a", "192.0.2.10", "velmara_edge", "edge");
        assert_eq!(e.outcome, "NOT APPLICABLE");
        assert!(e.detail.starts_with("host firewall baseline: off"));
        assert!(e.sha256.is_none() && e.ruleset.is_none());
    }

    /// Baseline on: the plan records the ruleset and the hash the renderer
    /// printed for this input, in the same run.
    #[test]
    fn on_records_the_ruleset_and_its_hash() {
        let f = fixture();
        let c = ctx(&f);
        for (host, addr, group, profile) in HOSTS {
            let e = entry(&c, host, addr, group, profile);
            assert_eq!(e.outcome, "PASS", "{host}: {}", e.detail);
            let Render::Ruleset { sha256, ruleset } =
                render(f.repo.path(), &input(&c, host, profile).unwrap())
            else {
                panic!("refused")
            };
            assert_eq!(e.sha256.as_deref(), Some(sha256.as_str()));
            assert_eq!(e.ruleset.as_deref(), Some(ruleset.as_str()));
        }
    }

    /// The fields the role reads from the live host never move the hash, so
    /// leaving them out of the plan keeps its hash the deploy's.
    #[test]
    fn the_live_host_fields_do_not_move_the_hash() {
        let f = fixture();
        let c = ctx(&f);
        for (host, addr, group, profile) in HOSTS {
            let planned = entry(&c, host, addr, group, profile);
            let mut deploy = input(&c, host, profile).unwrap();
            deploy["host"]["bridge_interfaces"] = json!(["br-5f1e2d3c4b5a"]);
            deploy["host"]["ephemeral_ports"] = json!([32768, 60999]);
            deploy["host"]["uplink_interfaces"] = json!(["eth0"]);
            let ssh = deploy["host"]["ssh_port"].clone();
            deploy["must_reach"] =
                json!([{"saddr": "198.51.100.10", "proto": "tcp", "dport": ssh, "iif": "eth0"}]);
            deploy["listening_ss"] = json!(format!(
                "tcp LISTEN 0 128 0.0.0.0:{ssh} 0.0.0.0:* users:((\"sshd\",pid=1,fd=3))\n"
            ));
            let Render::Ruleset { sha256, .. } = render(f.repo.path(), &deploy) else {
                panic!("deploy-side input refused")
            };
            assert_eq!(
                planned.sha256.as_deref(),
                Some(sha256.as_str()),
                "{profile}"
            );
        }
    }

    /// A refusal becomes BLOCKED with the renderer's code: no management
    /// source is GB-210.
    #[test]
    fn a_refusal_is_blocked_with_its_code() {
        let f = fixture();
        write(
            f.repo
                .path()
                .join("ops/ansible/group_vars/all/host-firewall.yml"),
            "goat_host_firewall_baseline: true\ngoat_host_firewall_management_sources: []\n",
        );
        let e = entry(&ctx(&f), "edge-a", "192.0.2.10", "velmara_edge", "edge");
        assert_eq!(
            (e.outcome.as_str(), e.code.as_deref()),
            ("BLOCKED", Some("GB-210")),
            "{}",
            e.detail
        );
    }

    /// What the plan cannot know is BLOCKED, saying why: no agreement yet, or
    /// no probe answer with the peerings stored on the edge.
    #[test]
    fn what_the_plan_cannot_know_is_blocked() {
        let f = fixture();
        std::fs::remove_file(f.work.path().join("preflight/probe/edge-a.json")).unwrap();
        let e = entry(&ctx(&f), "edge-a", "192.0.2.10", "velmara_edge", "edge");
        assert_eq!(e.outcome, "BLOCKED");
        assert!(
            e.detail.contains("stored on edge-a were not read"),
            "{}",
            e.detail
        );
        let core = entry(&ctx(&f), "core-a", "192.0.2.20", "velmara_core", "core");
        assert_eq!(
            core.outcome, "PASS",
            "a core host carries no peerings: {}",
            core.detail
        );
        std::fs::remove_file(f.work.path().join("agreement.json")).unwrap();
        let e = entry(&ctx(&f), "edge-a", "192.0.2.10", "velmara_edge", "edge");
        assert!(e.detail.contains("no agreement yet"), "{}", e.detail);
    }

    /// The evaluator never guesses: an undefined variable in the switch is
    /// BLOCKED (not "off"), a declared null is BLOCKED, `default(var)` reads
    /// the variable, and `on` is true as Ansible's bool reads it.
    #[test]
    fn the_evaluator_fails_closed() {
        let f = fixture();
        let fw = f
            .repo
            .path()
            .join("ops/ansible/group_vars/all/host-firewall.yml");
        let base = "goat_host_firewall_management_sources: [\"198.51.100.0/24\"]\n";
        write(
            fw.clone(),
            &format!("{base}goat_host_firewall_baseline: \"{{{{ site_hardening }}}}\"\n"),
        );
        let e = entry(&ctx(&f), "edge-a", "192.0.2.10", "velmara_edge", "edge");
        assert_eq!(e.outcome, "BLOCKED", "{}", e.detail);
        write(
            fw.clone(),
            &format!("{base}goat_host_firewall_baseline: on\ngoat_host_firewall_mesh_dns_port:\n"),
        );
        let e = entry(&ctx(&f), "edge-a", "192.0.2.10", "velmara_edge", "edge");
        assert_eq!(
            e.outcome, "BLOCKED",
            "on is true, and a null is refused: {}",
            e.detail
        );
        assert!(e.detail.contains("declared empty"), "{}", e.detail);
        write(fw, &format!("{base}goat_host_firewall_baseline: true\nsite_ssh: 2200\ngoat_host_firewall_ssh_port: \"{{{{ jump_port | default(site_ssh) }}}}\"\n"));
        let i = input(&ctx(&f), "edge-a", "edge").unwrap();
        assert_eq!(i["host"]["ssh_port"], 2200);
    }

    fn yaml(path: &str) -> Y {
        serde_yaml::from_str(&std::fs::read_to_string(real().join(path)).unwrap()).unwrap()
    }

    fn keys(m: &Y) -> std::collections::BTreeSet<String> {
        m.as_mapping()
            .unwrap()
            .keys()
            .map(|k| k.as_str().unwrap().to_string())
            .collect()
    }

    fn baseline_task(play: &str, profile: &str) -> Y {
        let doc = yaml(play);
        doc[0]["post_tasks"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|t| t["vars"]["goat_host_firewall_profile"].as_str() == Some(profile))
            .expect("the play's baseline task")
            .clone()
    }

    /// The plan builds exactly the fields the role and the plays build, and
    /// the core play passes no variable the plan does not set: a field or a
    /// play variable added there and not here fails this test.
    #[test]
    fn the_input_has_the_fields_the_role_and_plays_build() {
        let f = fixture();
        let c = ctx(&f);
        let i = input(&c, "edge-a", "edge").unwrap();
        let ours = |v: &Value| {
            v.as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        };
        let tasks = yaml("ops/ansible/roles/goat-host-firewall/tasks/main.yml");
        let assemble = tasks
            .as_sequence()
            .unwrap()
            .iter()
            .find(|t| t["name"].as_str() == Some("host-firewall | assemble the input"))
            .expect("the role's assemble task");
        let role_input = &assemble["ansible.builtin.set_fact"]["_hf_input"];
        let mut want = keys(role_input);
        for k in LIVE_INPUT_FIELDS {
            want.remove(*k);
        }
        assert_eq!(ours(&i), want, "top-level fields");
        let mut want = keys(&role_input["host"]);
        for k in LIVE_HOST_FIELDS {
            want.remove(*k);
        }
        assert_eq!(ours(&i["host"]), want, "host fields");
        assert_eq!(
            ours(&i["peerings"][1]),
            keys(
                &baseline_task("ops/ansible/peering-wg.yml", "edge")["vars"]
                    ["goat_host_firewall_peering"]
            ),
            "wireguard peering fields"
        );
        let core_vars = keys(&baseline_task("ops/ansible/peering-republisher.yml", "core")["vars"]);
        let handled: std::collections::BTreeSet<String> = [
            "goat_host_firewall_profile",
            "goat_host_firewall_spire_bundle_port",
            "goat_host_firewall_spire_bundle_sources",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(core_vars, handled, "the core play's variables");
        for play in ["ops/ansible/peering-wg.yml", "ops/ansible/peering-edge.yml"] {
            let v = keys(&baseline_task(play, "edge")["vars"]);
            let handled: std::collections::BTreeSet<String> =
                ["goat_host_firewall_profile", "goat_host_firewall_peering"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
            assert_eq!(v, handled, "{play}'s variables");
        }
        std::fs::write(
            f.work.path().join("agreement.json"),
            r#"{"transport":{"kind":"ipsec-fips","link_subnet":"169.254.10.0/31"}}"#,
        )
        .unwrap();
        let i = input(&c, "edge-a", "edge").unwrap();
        let last = i["peerings"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(
            ours(&last),
            keys(
                &baseline_task("ops/ansible/peering-edge.yml", "edge")["vars"]
                    ["goat_host_firewall_peering"]
            ),
            "ipsec peering fields"
        );
    }

    /// A vars file that does not parse is an error, never skipped for a
    /// lower layer (serde_yaml refuses a duplicate key that PyYAML would take).
    #[test]
    fn a_vars_file_that_does_not_parse_is_blocked() {
        let f = fixture();
        write(
            f.repo
                .path()
                .join("ops/ansible/group_vars/velmara_edge/dup.yml"),
            "peering_wg_listen_port: 1\npeering_wg_listen_port: 2\n",
        );
        let e = entry(&ctx(&f), "edge-a", "192.0.2.10", "velmara_edge", "edge");
        assert_eq!(e.outcome, "BLOCKED", "{}", e.detail);
        assert!(e.detail.contains("does not parse"), "{}", e.detail);
    }

    /// A hidden file in a vars directory (`.old.yml`) is not read, as Ansible
    /// does not read it.
    #[test]
    fn a_hidden_vars_file_is_not_read() {
        let f = fixture();
        write(
            f.repo
                .path()
                .join("ops/ansible/group_vars/velmara_edge/.old.yml"),
            "peering_wg_listen_port: 1\n",
        );
        write(
            f.repo
                .path()
                .join("ops/ansible/group_vars/velmara_edge/.backup/p.yml"),
            "peering_wg_listen_port: 2\n",
        );
        let i = input(&ctx(&f), "edge-a", "edge").unwrap();
        assert_eq!(i["peerings"][1]["listen_port"], 51823);
    }
}
