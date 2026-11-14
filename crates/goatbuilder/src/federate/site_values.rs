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
//! - `peering_wg_extra_ports`: `[peering_spire_bundle_port]` (group_vars, else
//!   the `goat-peering-spire` role default), the one port the tunnel must pass
//!   for SPIRE federation;
//! - `goat_zenoh_edge_acl_trace`: true, because phase 14's strict acceptance
//!   reads the edge's ACL trace;
//! - `goat_zenoh_edge_core_cn`: the core's own name in the site inventory
//!   ([`core_cn`]), which phase 8 also mints as the core certificate's CN
//!   ([`phase8_core_cn`]).
//!
//! A required value none of these yields is refused with GB-232, naming every
//! place that was read. The other optional values are never derived.

use std::path::Path;

use serde_yaml::Value;

use super::alloc::PeeringValues;
use crate::config::Peering;
use crate::inputs::Source;

/// One required value: the variable, and where it came from (`from`, the words
/// the rendered half writes above it; `source`, the same as a structured
/// source with its line).
#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    pub var: &'static str,
    pub value: Value,
    pub from: String,
    pub source: Source,
}

/// `key` in the YAML file `rel` (relative to `repo`), at its line.
fn file_source(repo: &Path, rel: &str, path: &[crate::inputs::Seg], key: &str) -> Source {
    let text = std::fs::read_to_string(repo.join(rel)).unwrap_or_default();
    Source::file(rel, crate::inputs::yaml_line(&text, path), key)
}

fn key_source(repo: &Path, rel: &str, key: &str) -> Source {
    file_source(repo, rel, &[crate::inputs::Seg::Key(key)], key)
}

/// The hand-written group_vars reader the rendered halves use: `var` in the
/// files of each of `groups`, then `all` (`group_vars/<g>.yml|.yaml` and every
/// YAML file of `group_vars/<g>/`, the last in name order winning, a generated
/// file skipped), as (value, the file relative to the checkout).
pub fn hand_reader<'a>(
    repo: &'a Path,
    groups: &'a [String],
) -> impl Fn(&str) -> Option<(Value, String)> + 'a {
    move |var: &str| {
        let files = |g: &str| -> Vec<std::path::PathBuf> {
            let gv = repo.join("ops/ansible/group_vars");
            let mut out: Vec<std::path::PathBuf> = std::fs::read_dir(gv.join(g))
                .map(|it| it.flatten().map(|e| e.path()).collect())
                .unwrap_or_default();
            for ext in ["yml", "yaml"] {
                out.push(gv.join(format!("{g}.{ext}")));
            }
            out.retain(|f| {
                matches!(f.extension().and_then(|e| e.to_str()), Some("yml" | "yaml"))
                    && std::fs::read_to_string(f)
                        .map(|t| !t.starts_with(super::peering_files::MARKER))
                        .unwrap_or(false)
            });
            out.sort();
            out
        };
        let read = |f: &Path| -> Option<(Value, String)> {
            let v: Value = serde_yaml::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
            let rel = f.strip_prefix(repo).unwrap_or(f).display().to_string();
            v.get(var).cloned().map(|v| (v, rel))
        };
        groups
            .iter()
            .map(String::as_str)
            .chain(["all"])
            .find_map(|g| files(g).iter().rev().find_map(|f| read(f)))
    }
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
fn pin(s: &Scope, var: &str, looked: &mut Vec<String>) -> Option<(Value, String, Source)> {
    if let Some((v, f)) = (s.hand)(var) {
        let src = key_source(s.repo, &f, var);
        return Some((v, f, src));
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
            let file = format!("ops/ansible/roles/{role}/defaults/main.yml");
            let src = key_source(s.repo, &file, var);
            return Some((Value::String(image.clone()), file, src));
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
    let table_rel = table
        .strip_prefix(s.repo)
        .unwrap_or(&table)
        .display()
        .to_string();
    let (at, src) = match dev {
        Some(_) => (
            format!("ops/pins/fleet-pins.yaml components.{name} deviation"),
            Source::derived(
                format!("{table_rel} components.{name}"),
                format!("its deviation for environment {env}"),
            ),
        ),
        None => {
            use crate::inputs::Seg::Key;
            (
                format!("ops/pins/fleet-pins.yaml components.{name}.fleet"),
                file_source(
                    s.repo,
                    &table_rel,
                    &[Key("components"), Key(name), Key("fleet")],
                    &format!("components.{name}.fleet"),
                ),
            )
        }
    };
    Some((Value::String(image), at, src))
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
/// peering inventory. The one core-identity source: the rendered halves, the
/// core certificate's name (`core_cn`) and its SANs read the core from here.
// When the peering's hosts come from the site inventory, `goatbuilder federate
// allocate` has already put them in `peering.hosts` (site_hosts::fill).
pub fn core_address(
    repo: &Path,
    peering: &Peering,
    core_group: &str,
    looked: &mut Vec<String>,
) -> Option<(String, String)> {
    if let Some(h) = &peering.hosts {
        return Some((
            h.core.clone(),
            "goatbuilder.yaml peerings[].hosts.core".into(),
        ));
    }
    looked.push(format!(
        "peerings[].hosts.core of peering '{}' in goatbuilder.yaml",
        peering.with
    ));
    let inv = "ops/ansible/inventory/peering-edges.ini and the per-peering peering-*.ini";
    looked.push(format!("the one host of [{core_group}] in {inv}"));
    let text = super::plan::peering_inventory_text(repo);
    let hosts: Vec<(String, String)> = super::site_hosts::groups(&text)
        .into_iter()
        .filter(|(g, _)| g == core_group)
        .flat_map(|(_, ms)| ms.into_iter().map(|m| (m.name, m.addr)))
        .collect();
    let [(_, addr)] = hosts.as_slice() else {
        return None;
    };
    Some((addr.clone(), format!("[{core_group}] in {inv}")))
}

/// The core's own name in the site inventory: the inventory name of the host
/// whose address is the core address, in the hand-written inventories of
/// `ops/ansible/inventory/` other than the peering inventories (which give a
/// machine one name per peering). One name per core, so every peering on that
/// core mints and pins the same certificate name.
// The same files and reader as site_hosts.rs, which resolves peerings[].hosts.
// TODO: a declared environments[].inventory narrows site_hosts to that file;
// this has no config in hand and still reads every site inventory.
pub fn core_cn(
    repo: &Path,
    peering: &Peering,
    core_group: &str,
    looked: &mut Vec<String>,
) -> Option<(String, String)> {
    let (addr, _) = core_address(repo, peering, core_group, looked)?;
    looked.push(format!(
        "the site inventory's name for the core host {addr} in ops/ansible/inventory/*.ini (not peering-*.ini)"
    ));
    let mut names: Vec<(String, String)> = Vec::new();
    for (file, text) in super::site_hosts::site_inventories(repo) {
        for (_, ms) in super::site_hosts::groups(&text) {
            for m in ms {
                if m.addr == addr && !names.iter().any(|(n, _)| *n == m.name) {
                    names.push((m.name, file.clone()));
                }
            }
        }
    }
    match names.as_slice() {
        [(name, file)] => Some((
            name.clone(),
            format!("the core host {addr}'s name in {file}"),
        )),
        [] => None,
        many => {
            looked.push(format!(
                "(the core host {addr} has several names there: {}; give it one)",
                many.iter()
                    .map(|(n, f)| format!("{n} in {f}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            None
        }
    }
}

/// The CN phase 8 mints for the core certificate (FED_CORE_CN, which stays an
/// override): the value the edge pins as its core subject
/// (`goat_zenoh_edge_core_cn` in its group_vars, the rendered half included),
/// else `core_cn`. So the certificate and the edge's ACL subject cannot differ.
pub fn phase8_core_cn(
    repo: &Path,
    peering: &Peering,
    edge_group: &str,
    core_group: &str,
) -> (Option<String>, String) {
    if let Some(cn) = super::preflight::group_var(repo, edge_group, "goat_zenoh_edge_core_cn") {
        return (
            Some(cn),
            format!("group_vars/{edge_group}: goat_zenoh_edge_core_cn (the edge's core subject)"),
        );
    }
    let mut looked = vec![format!(
        "goat_zenoh_edge_core_cn in group_vars/{edge_group}"
    )];
    match core_cn(repo, peering, core_group, &mut looked) {
        Some((cn, from)) => (Some(cn), from),
        None => (None, looked.join("; ")),
    }
}

/// A port as a play reads it: `var` in the hand-written group_vars of the
/// environment's groups or all, else the `role`'s default.
fn role_port(
    s: &Scope,
    var: &str,
    role: &str,
    looked: &mut Vec<String>,
) -> Option<(u16, String, Source)> {
    let num = |v: &Value| -> Option<u16> {
        v.as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .or_else(|| v.as_str().and_then(|t| t.trim().parse().ok()))
    };
    if let Some((v, f)) = (s.hand)(var) {
        if let Some(p) = num(&v) {
            let src = key_source(s.repo, &f, var);
            return Some((p, f, src));
        }
    }
    let file = format!("ops/ansible/roles/{role}/defaults/main.yml");
    looked.push(format!(
        "{var} in group_vars of {} or all, or in {file}",
        groups_text(&s.groups)
    ));
    let doc: Value =
        serde_yaml::from_str(&std::fs::read_to_string(s.repo.join(&file)).ok()?).ok()?;
    let src = key_source(s.repo, &file, var);
    num(doc.get(var)?).map(|p| (p, file, src))
}

/// The core router's listen port: `goat_zenoh_listen_port` in group_vars, else
/// the `goat-zenoh` role default.
fn core_port(s: &Scope, looked: &mut Vec<String>) -> Option<(u16, String, Source)> {
    role_port(s, "goat_zenoh_listen_port", "goat-zenoh", looked)
}

/// The required site values, each from its owner; `Err` lists what could not be
/// derived and every place that was read for it.
pub fn required(s: &Scope) -> Result<Vec<Site>, Vec<(&'static str, Vec<String>)>> {
    let (out, missing) = required_all(s);
    if missing.is_empty() {
        Ok(out)
    } else {
        Err(missing)
    }
}

/// [`required`], with both what was derived and what was not, for listing
/// every value with its source (`goatbuilder inputs`).
pub fn required_all(s: &Scope) -> (Vec<Site>, Vec<(&'static str, Vec<String>)>) {
    let mut out = Vec::new();
    let mut missing: Vec<(&'static str, Vec<String>)> = Vec::new();
    let mut put =
        |var: &'static str, got: Option<(Value, String, Source)>, looked: Vec<String>| match got {
            Some((value, from, source)) => out.push(Site {
                var,
                value,
                from,
                source,
            }),
            None => missing.push((var, looked)),
        };
    // A value the hand-written group_vars set, at its line.
    let hand = |var: &str| -> Option<(Value, String, Source)> {
        let (v, f) = (s.hand)(var)?;
        let src = key_source(s.repo, &f, var);
        Some((v, f, src))
    };

    // The core's address and port: the republisher's endpoint and the SPIRE server.
    for var in [
        "goat_fed_republisher_core_endpoint",
        "peering_spire_server_addr",
    ] {
        let mut looked = Vec::new();
        let got = hand(var).or_else(|| {
            looked.push(format!(
                "{var} in group_vars of {} or all",
                groups_text(&s.groups)
            ));
            let (addr, from) = core_address(s.repo, s.peering, &s.values.core_group, &mut looked)?;
            if var == "peering_spire_server_addr" {
                let src = Source::derived(from.clone(), "the core host's address");
                return Some((Value::String(addr), from, src));
            }
            let (port, pfrom, _) = core_port(s, &mut looked)?;
            let from = format!("{from}; port {pfrom}");
            let src = Source::derived(from.clone(), "tls/<core address>:<goat_zenoh_listen_port>");
            Some((Value::String(format!("tls/{addr}:{port}")), from, src))
        });
        put(var, got, looked);
    }

    let mut looked = Vec::new();
    let got = pin(s, "goat_fed_republisher_image", &mut looked);
    put("goat_fed_republisher_image", got, looked);

    let mut looked = Vec::new();
    let got = pin(s, "goat_zenoh_edge_image", &mut looked).or_else(|| {
        pin(s, "zenoh_router_image", &mut looked).map(|(v, f, src)| {
            (
                v,
                format!("the fabric image the core runs (zenoh_router_image, {f})"),
                Source::derived(
                    src.render(),
                    "the fabric image the core runs (zenoh_router_image)",
                ),
            )
        })
    });
    put("goat_zenoh_edge_image", got, looked);

    let mut looked = Vec::new();
    let got = hand("goat_zenoh_edge_core_cn").or_else(|| {
        looked.push(format!(
            "goat_zenoh_edge_core_cn in group_vars of {} or all",
            groups_text(&s.groups)
        ));
        core_cn(s.repo, s.peering, &s.values.core_group, &mut looked).map(|(v, f)| {
            let src = Source::derived(f.clone(), "the core host's one inventory name");
            (Value::String(v), f, src)
        })
    });
    put("goat_zenoh_edge_core_cn", got, looked);

    // The tunnel firewall drops every port it is not told about, and the
    // rendered half turns SPIRE federation on: the one extra port is this
    // peering's SPIRE bundle endpoint (goat-peering-spire asserts it is listed).
    let mut looked = Vec::new();
    let got = hand("peering_wg_extra_ports").or_else(|| {
        looked.push(format!(
            "peering_wg_extra_ports in group_vars of {} or all",
            groups_text(&s.groups)
        ));
        let (port, from, src) = role_port(
            s,
            "peering_spire_bundle_port",
            "goat-peering-spire",
            &mut looked,
        )?;
        Some((
            Value::Sequence(vec![Value::Number(port.into())]),
            format!("the SPIRE bundle endpoint port (peering_spire_bundle_port, {from})"),
            Source::derived(
                src.render(),
                "[peering_spire_bundle_port], the one port the tunnel passes for SPIRE federation",
            ),
        ))
    });
    put("peering_wg_extra_ports", got, looked);

    // Phase 14's strict acceptance reads the edge's ACL trace (acceptance.sh:
    // the refusal of an unoffered set is recorded there, beside the allow), so
    // the rendered half turns it on unless group_vars says otherwise.
    let got = hand("goat_zenoh_edge_acl_trace").or_else(|| {
        let why = "phase 14's strict acceptance reads the edge's ACL trace (ops/federation/standup/acceptance.sh)";
        Some((Value::Bool(true), why.to_string(), Source::default(format!("true: {why}"))))
    });
    put("goat_zenoh_edge_acl_trace", got, Vec::new());

    (out, missing)
}
