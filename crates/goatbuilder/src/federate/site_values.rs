//! The site values a rendered deployment half needs (`peerings[].deployment:
//! rendered`), each read from the declaration that already owns it.
//!
//! A value set in the hand-written group_vars of the environment's groups (then
//! `all`) is taken as it is. Otherwise each required value is derived:
//!
//! - `goat_fed_republisher_core_endpoint`: `tls/<core address>:<listen port>`,
//!   the core host of the peering (the address its inventory line carries) and
//!   the core router's `goat_zenoh_listen_port` (group_vars, else the
//!   `goat-zenoh` role default);
//! - `peering_spire_server_addr`: the same core address, because the rendered
//!   edge values name that host as `peering_spire_server_host`;
//! - `goat_fed_republisher_image`: the pin as a play reads it (group_vars, else
//!   the role default, else the `fleet:` value of the `ops/pins/fleet-pins.yaml`
//!   component whose `var:` it is, through the environment's deviations);
//! - `goat_zenoh_edge_image`: the same for its own variable, else the fabric
//!   image the core router runs (`zenoh_router_image`), resolved the same way;
//! - `goat_zenoh_edge_core_cn`: the common name of the core certificate phase 8
//!   mints, [`core_cn`].
//!
//! A required value none of these yields is refused with GB-232, naming every
//! place that was read. Optional values are never derived.

use std::path::Path;

use serde_yaml::Value;

use super::alloc::PeeringValues;
use crate::config::Peering;

/// One required value: the variable, and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    pub var: &'static str,
    pub value: Value,
    pub from: String,
}

/// Everything read for the peering's environment: the group names (then
/// `all`) whose hand-written group_vars are consulted.
pub struct Scope<'a> {
    pub repo: &'a Path,
    pub peering: &'a Peering,
    pub values: &'a PeeringValues,
    pub groups: Vec<String>,
    /// Reads one variable from the hand-written group_vars of `groups`, then
    /// `all`: (value, the file it was read from).
    pub hand: &'a dyn Fn(&str) -> Option<(Value, String)>,
}

/// A plain string that resolves as it stands, or carries only `{{ ghcr_org }}`
/// as a pin does (Ansible fills that from group_vars/all.yml).
fn pinned_image(v: &Value) -> Option<String> {
    let s = v.as_str()?.trim();
    let rest = s.replace("{{ ghcr_org }}", "");
    (!s.is_empty() && s.contains("@sha256:") && !rest.contains("{{")).then(|| s.to_string())
}

/// A pin as a play would read it: the hand-written group_vars, else the one
/// role default that sets it, else the fleet-pins component whose `var:` it is
/// (the environment's deviation, else `fleet:`). `looked` collects each place.
fn pin(s: &Scope, var: &str, looked: &mut Vec<String>) -> Option<(Value, String)> {
    if let Some((v, f)) = (s.hand)(var) {
        return Some((v, f));
    }
    looked.push(format!(
        "{var} in group_vars of {} or all",
        groups_text(&s.groups)
    ));
    let paths = crate::pins::Paths::new(s.repo);
    if let Ok(defaults) = crate::pins::role_defaults(&paths) {
        let hits: Vec<(String, String)> = defaults
            .get(var)
            .map(|h| {
                h.iter()
                    .filter_map(|(role, v)| {
                        let j = serde_yaml::to_value(v).ok()?;
                        pinned_image(&j).map(|i| (role.clone(), i))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let [(role, image)] = hits.as_slice() {
            return Some((
                Value::String(image.clone()),
                format!("ops/ansible/roles/{role}/defaults/main.yml"),
            ));
        }
    }
    looked.push(format!(
        "a digest-pinned default of {var} in ops/ansible/roles/*/defaults"
    ));
    let table = paths.table();
    looked.push(format!(
        "the component of ops/pins/fleet-pins.yaml whose var is {var}"
    ));
    // The merged table (with the lite sidecar's deviations) when a goatnet is
    // listed; a checkout that lists none still carries every component's fleet pin.
    let components = match crate::pins::load_table(&table) {
        Ok(t) => t.components,
        Err(_) => crate::pins::read_yaml(&table)
            .ok()?
            .get("components")?
            .as_object()?
            .clone(),
    };
    let (name, spec) = components
        .iter()
        .find(|(_, c)| c.get("var").and_then(|v| v.as_str()) == Some(var))?;
    let env = s.peering.environment.as_str();
    let member = s
        .groups
        .iter()
        .find(|g| crate::pins::deviation_scope(spec, g, env).is_some())
        .map(String::as_str)
        .unwrap_or("");
    let (v, dev) = crate::pins::resolve(spec, member, env);
    let image = v.and_then(|v| v.as_str().map(String::from))?;
    let at = match dev {
        Some(_) => format!("ops/pins/fleet-pins.yaml components.{name} deviation"),
        None => format!("ops/pins/fleet-pins.yaml components.{name}.fleet"),
    };
    Some((Value::String(image), at))
}

fn groups_text(groups: &[String]) -> String {
    if groups.is_empty() {
        "environments[].groups (none declared)".to_string()
    } else {
        groups.join(", ")
    }
}

/// The address of the peering's core host: `peerings[].hosts.core`, else the
/// `ansible_host` of the one host of `[<core_group>]` in the hand-written
/// peering inventory.
pub fn core_address(s: &Scope, looked: &mut Vec<String>) -> Option<(String, String)> {
    if let Some(h) = &s.peering.hosts {
        return Some((
            h.core.clone(),
            "goatbuilder.yaml peerings[].hosts.core".into(),
        ));
    }
    looked.push(format!(
        "peerings[].hosts.core of peering '{}' in goatbuilder.yaml",
        s.values.peering
    ));
    let inv = "ops/ansible/inventory/peering-edges.ini";
    looked.push(format!(
        "the one host of [{}] in {inv}",
        s.values.core_group
    ));
    let text = std::fs::read_to_string(s.repo.join(inv)).ok()?;
    let head = format!("[{}]", s.values.core_group);
    let mut inside = false;
    let mut hosts = Vec::new();
    for l in text.lines().map(str::trim) {
        if l.is_empty() || l.starts_with('#') || l.starts_with(';') {
            continue;
        }
        if l.starts_with('[') {
            inside = l == head;
            continue;
        }
        if inside {
            hosts.push(l.to_string());
        }
    }
    let [line] = hosts.as_slice() else {
        return None;
    };
    let mut words = line.split_whitespace();
    let mut addr = words.next()?.to_string();
    for w in words {
        if let Some(v) = w.strip_prefix("ansible_host=") {
            addr = v.to_string();
        }
    }
    Some((addr, format!("[{}] in {inv}", s.values.core_group)))
}

/// The core router's listen port: `goat_zenoh_listen_port` in group_vars, else
/// the `goat-zenoh` role default.
fn core_port(s: &Scope, looked: &mut Vec<String>) -> Option<(u16, String)> {
    let num = |v: &Value| -> Option<u16> {
        v.as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .or_else(|| v.as_str().and_then(|t| t.trim().parse().ok()))
    };
    if let Some((v, f)) = (s.hand)("goat_zenoh_listen_port") {
        if let Some(p) = num(&v) {
            return Some((p, f));
        }
    }
    let role = "ops/ansible/roles/goat-zenoh/defaults/main.yml";
    looked.push(format!(
        "goat_zenoh_listen_port in group_vars of {} or all, or in {role}",
        groups_text(&s.groups)
    ));
    let doc: Value =
        serde_yaml::from_str(&std::fs::read_to_string(s.repo.join(role)).ok()?).ok()?;
    num(doc.get("goat_zenoh_listen_port")?).map(|p| (p, role.to_string()))
}

/// The common name of the core certificate phase 8 mints (FED_CORE_CN), which
/// the edge pins as the core link's ACL subject: the same function feeds both.
/// From the inventory name of the peering's core host.
pub fn core_cn(s: &Scope, looked: &mut Vec<String>) -> Option<(String, String)> {
    let _ = looked;
    (!s.values.core_host_name.is_empty()).then(|| {
        (
            s.values.core_host_name.clone(),
            format!(
                "the inventory name of the core host of [{}]",
                s.values.core_group
            ),
        )
    })
}

/// The required site values, each from its owner; `Err` lists what could not be
/// derived and every place that was read for it.
pub fn required(s: &Scope) -> Result<Vec<Site>, Vec<(&'static str, Vec<String>)>> {
    let mut out = Vec::new();
    let mut missing: Vec<(&'static str, Vec<String>)> = Vec::new();
    let mut put = |var: &'static str, got: Option<(Value, String)>, looked: Vec<String>| match got {
        Some((value, from)) => out.push(Site { var, value, from }),
        None => missing.push((var, looked)),
    };

    // The core's address and port: the republisher's endpoint and the SPIRE server.
    for var in [
        "goat_fed_republisher_core_endpoint",
        "peering_spire_server_addr",
    ] {
        let mut looked = Vec::new();
        let got = (s.hand)(var).or_else(|| {
            looked.push(format!(
                "{var} in group_vars of {} or all",
                groups_text(&s.groups)
            ));
            let (addr, from) = core_address(s, &mut looked)?;
            if var == "peering_spire_server_addr" {
                return Some((Value::String(addr), from));
            }
            let (port, pfrom) = core_port(s, &mut looked)?;
            Some((
                Value::String(format!("tls/{addr}:{port}")),
                format!("{from}; port {pfrom}"),
            ))
        });
        put(var, got, looked);
    }

    let mut looked = Vec::new();
    let got = pin(s, "goat_fed_republisher_image", &mut looked);
    put("goat_fed_republisher_image", got, looked);

    let mut looked = Vec::new();
    let got = pin(s, "goat_zenoh_edge_image", &mut looked).or_else(|| {
        pin(s, "zenoh_router_image", &mut looked).map(|(v, f)| {
            (
                v,
                format!("the fabric image the core runs (zenoh_router_image, {f})"),
            )
        })
    });
    put("goat_zenoh_edge_image", got, looked);

    let mut looked = Vec::new();
    let got = (s.hand)("goat_zenoh_edge_core_cn").or_else(|| {
        looked.push(format!(
            "goat_zenoh_edge_core_cn in group_vars of {} or all",
            groups_text(&s.groups)
        ));
        core_cn(s, &mut looked).map(|(v, f)| (Value::String(v), f))
    });
    put("goat_zenoh_edge_core_cn", got, looked);

    if missing.is_empty() {
        Ok(out)
    } else {
        Err(missing)
    }
}
