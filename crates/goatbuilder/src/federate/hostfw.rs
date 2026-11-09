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
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serde_yaml::Value as Y;

use super::hostalloc::{agreement_of, work_of_other};
use super::plan::{group_raw, inventory_hosts};
use crate::config;

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

/// Where a host's variables come from, in Ansible's order of precedence (the
/// first that sets a key wins): inventory host vars, host_vars, then each
/// group from the most specific, then the files apply rendered for that group.
struct Scope<'a> {
    repo: &'a Path,
    host: String,
    groups: Vec<String>,
    /// apply's rendered group_vars for this host's peering group, when there.
    applied: Option<PathBuf>,
}

fn yaml_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn last_in(files: &[PathBuf], key: &str) -> Option<Y> {
    let mut found = None;
    for f in files {
        let Some(doc) = std::fs::read_to_string(f)
            .ok()
            .and_then(|t| serde_yaml::from_str::<Y>(&t).ok())
        else {
            continue;
        };
        if let Some(v) = doc.get(key).filter(|v| !v.is_null()) {
            found = Some(v.clone());
        }
    }
    found
}

/// A host's own variable from its inventory line (`key=value`).
fn inline_host_var(repo: &Path, host: &str, key: &str) -> Option<Y> {
    let text =
        std::fs::read_to_string(repo.join("ops/ansible/inventory/peering-edges.ini")).ok()?;
    let mut found = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let body = line.split(" #").next().unwrap_or(line);
        let mut words = body.split_whitespace();
        if words.next() != Some(host) {
            continue;
        }
        for w in words {
            if let Some(v) = w.strip_prefix(&format!("{key}=")) {
                let v = v.trim_matches(|c| c == '"' || c == '\'');
                found =
                    Some(serde_yaml::from_str::<Y>(v).unwrap_or_else(|_| Y::String(v.to_string())));
            }
        }
    }
    found
}

impl Scope<'_> {
    fn raw(&self, key: &str) -> Option<Y> {
        if let Some(v) = inline_host_var(self.repo, &self.host, key) {
            return Some(v);
        }
        let hv = self.repo.join("ops/ansible/host_vars");
        let mut files = vec![
            hv.join(format!("{}.yml", self.host)),
            hv.join(format!("{}.yaml", self.host)),
        ];
        files.extend(yaml_files(&hv.join(&self.host)));
        if let Some(v) = last_in(&files, key) {
            return Some(v);
        }
        for g in &self.groups {
            if let Some(v) = group_raw(self.repo, g, key).filter(|v| !v.is_null()) {
                return Some(v);
            }
        }
        self.applied
            .as_ref()
            .and_then(|d| last_in(&yaml_files(d), key))
    }

    /// The value of `key`, else the role's default, else `fallback`, with a
    /// template evaluated when it is `{{ var | default(x) }}` (optionally `| int`
    /// or `| bool`) or a string with such expressions in it.
    fn get(&self, key: &str, role: Option<&str>, fallback: Y) -> Result<Y, String> {
        let v = match self.raw(key) {
            Some(v) => v,
            None => self.role_default(role, key).unwrap_or(fallback),
        };
        self.eval(&v, key, 0, role)
    }

    fn role_default(&self, role: Option<&str>, key: &str) -> Option<Y> {
        let f = self
            .repo
            .join(format!("ops/ansible/roles/{}/defaults/main.yml", role?));
        last_in(&[f], key)
    }

    fn eval(&self, v: &Y, key: &str, depth: u32, role: Option<&str>) -> Result<Y, String> {
        let Y::String(s) = v else {
            return Ok(v.clone());
        };
        if !s.contains("{{") {
            return Ok(v.clone());
        }
        if depth > 4 {
            return Err(format!("{key} refers to itself"));
        }
        let expr = regex::Regex::new(
            r"\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*(?:\|\s*default\(\s*([^()]*?)\s*\))?\s*(?:\|\s*(int|bool|trim))?\s*\}\}",
        )
        .expect("regex");
        let one = |c: &regex::Captures| -> Result<Y, String> {
            let inner = match self.raw(&c[1]).or_else(|| self.role_default(role, &c[1])) {
                Some(x) => self.eval(&x, &c[1], depth + 1, role)?,
                None => match c.get(2) {
                    Some(d) => serde_yaml::from_str::<Y>(d.as_str()).unwrap_or(Y::Null),
                    None => return Err(format!("{key} needs {}, which nothing declares", &c[1])),
                },
            };
            Ok(match c.get(3).map(|m| m.as_str()) {
                Some("int") => scalar(&inner)
                    .and_then(|s| s.trim().parse::<i64>().ok())
                    .map(|n| Y::Number(n.into()))
                    .unwrap_or(Y::Number(0.into())),
                Some("bool") => Y::Bool(scalar(&inner).is_some_and(|s| {
                    matches!(s.trim().to_lowercase().as_str(), "true" | "yes" | "1")
                })),
                Some("trim") => Y::String(scalar(&inner).unwrap_or_default().trim().to_string()),
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
            out.push_str(&scalar(&one(&c)?).unwrap_or_default());
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
    scalar(v).and_then(|s| s.trim().parse().ok())
}

fn truthy(v: &Y) -> bool {
    scalar(v).is_some_and(|s| matches!(s.trim().to_lowercase().as_str(), "true" | "yes" | "1"))
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
        Y::Null => Ok(json!([])),
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
    pub peerings: &'a [config::Peering],
    pub ours: &'a config::Peering,
    pub our_edge_group: &'a str,
    pub our_work: &'a Path,
    /// The counterparty package this run uses (FED_COUNTERPARTY, else <work>/theirs.canonical.json).
    pub our_package: PathBuf,
}

fn scope<'a>(
    repo: &'a Path,
    host: &str,
    group: &str,
    parent: &str,
    applied: Option<PathBuf>,
) -> Scope<'a> {
    Scope {
        repo,
        host: host.to_string(),
        groups: vec![group.to_string(), parent.to_string(), "all".to_string()],
        applied,
    }
}

/// The renderer input for one host, without the live fields. For an edge host
/// it carries every declared peering whose edge group names the same machine,
/// as the role carries every peering stored on the host.
pub fn input(
    c: &HostContext,
    host: &str,
    address: &str,
    group: &str,
    profile: &str,
) -> Result<Value, String> {
    let parent = if profile == "core" {
        "peering_cores"
    } else {
        "peering_edges"
    };
    let s = scope(c.repo, host, group, parent, None);
    let host_v = host_input(&s, profile)?;
    let mut peerings = Vec::new();
    if profile == "edge" {
        for p in c.peerings {
            let mine = p.with == c.ours.with && p.environment == c.ours.environment;
            let edge_group = if mine {
                c.our_edge_group.to_string()
            } else {
                format!("{}_edge", p.with)
            };
            let Some((name, _)) = inventory_hosts(c.repo, &edge_group)
                .into_iter()
                .find(|(_, a)| a == address)
            else {
                continue;
            };
            let work = if mine {
                Some(c.our_work.to_path_buf())
            } else {
                work_of_other(c.our_work, &p.with)
            };
            let Some(work) = work else {
                return Err(format!("{} shares this machine, and its working directory (agreement, package) is not found", p.with));
            };
            let transport = agreement_of(&work).map(|(k, _)| k).unwrap_or_default();
            let package = if mine {
                c.our_package.clone()
            } else {
                work.join("theirs.canonical.json")
            };
            let applied = work.join(format!("applied/ops/ansible/group_vars/{edge_group}"));
            let ps = scope(
                c.repo,
                &name,
                &edge_group,
                "peering_edges",
                applied.is_dir().then_some(applied),
            );
            peerings.push(
                peering_entry(&ps, &transport, &package)
                    .map_err(|e| format!("peering {}: {e}", p.with))?,
            );
        }
    }
    Ok(json!({
        "schema": "goat-host-firewall-input/v1",
        "profile": profile,
        "host": host_v,
        "peerings": peerings,
        "override": to_json(&s.get("goat_host_firewall_override", Some("goat-host-firewall"), Y::Mapping(Default::default()))?),
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
    let child = Command::new("python3")
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

/// Whether the baseline is on for a host (`goat_host_firewall_baseline`).
pub fn baseline_on(repo: &Path, host: &str, group: &str, parent: &str) -> bool {
    scope(repo, host, group, parent, None)
        .get(
            "goat_host_firewall_baseline",
            Some("goat-host-firewall"),
            Y::Bool(false),
        )
        .map(|v| truthy(&v))
        .unwrap_or(false)
}

/// The plan's entry for one host.
pub fn entry(
    c: &HostContext,
    host: &str,
    address: &str,
    group: &str,
    profile: &str,
) -> HostFirewall {
    let parent = if profile == "core" {
        "peering_cores"
    } else {
        "peering_edges"
    };
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
    if !baseline_on(c.repo, host, group, parent) {
        return base(
            "NOT APPLICABLE",
            "host firewall baseline: off (goat_host_firewall_baseline is not true); nothing is rendered or applied".into(),
        );
    }
    let input = match input(c, host, address, group, profile) {
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

    fn p(with: &str) -> config::Peering {
        config::Peering {
            with: with.into(),
            environment: "zagros".into(),
            site: "alpha".into(),
            edge_class: "edge".into(),
        }
    }

    /// A repository with the renderer and the role defaults it is read with,
    /// one edge and one core host for `velmara`, a work directory with a
    /// WireGuard agreement, apply's rendered transport file and the partner's
    /// package, and the baseline on for both groups.
    struct Fixture {
        repo: tempfile::TempDir,
        work: tempfile::TempDir,
    }

    fn write(path: PathBuf, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn fixture() -> Fixture {
        let repo = tempfile::tempdir().unwrap();
        let r = repo.path();
        for f in [
            RENDERER,
            "ops/ansible/roles/goat-host-firewall/defaults/main.yml",
            "ops/ansible/roles/goat-peering-wg/defaults/main.yml",
            "ops/ansible/roles/goat-peering-edge/defaults/main.yml",
        ] {
            std::fs::create_dir_all(r.join(f).parent().unwrap()).unwrap();
            std::fs::copy(real().join(f), r.join(f)).unwrap();
        }
        write(
            r.join("ops/ansible/inventory/peering-edges.ini"),
            "[velmara_edge]\nedge-a ansible_host=192.0.2.10\n[velmara_core]\ncore-a ansible_host=192.0.2.20\n",
        );
        write(
            r.join("ops/ansible/group_vars/all/fw.yml"),
            "goat_host_firewall_baseline: true\ngoat_host_firewall_management_sources: [\"198.51.100.10/32\"]\n",
        );
        write(
            r.join("ops/ansible/group_vars/velmara_edge/peering.yml"),
            "peering_wg_listen_port: 51823\n",
        );
        let work = tempfile::tempdir().unwrap();
        let w = work.path();
        write(
            w.join("agreement.json"),
            r#"{"transport":{"kind":"wireguard","link_subnet":"169.254.10.0/31"}}"#,
        );
        write(
            w.join("applied/ops/ansible/group_vars/velmara_edge/federation-velmara-transport.yml"),
            "goat_peering_local_link_ip: \"169.254.10.0\"\ngoat_peering_remote_link_ip: \"169.254.10.1\"\n",
        );
        write(
            w.join("theirs.canonical.json"),
            r#"{"transport_offer":{"endpoint":"203.0.113.7:51820"}}"#,
        );
        Fixture { repo, work }
    }

    fn ctx<'a>(
        f: &'a Fixture,
        all: &'a [config::Peering],
        ours: &'a config::Peering,
    ) -> HostContext<'a> {
        HostContext {
            repo: f.repo.path(),
            peerings: all,
            ours,
            our_edge_group: "velmara_edge",
            our_work: f.work.path(),
            our_package: f.work.path().join("theirs.canonical.json"),
        }
    }

    /// Baseline off: NOT APPLICABLE, and the renderer is not called.
    #[test]
    fn off_is_not_applicable() {
        let f = fixture();
        write(
            f.repo.path().join("ops/ansible/group_vars/all/fw.yml"),
            "goat_host_firewall_baseline: false\n",
        );
        let ours = p("velmara");
        let all = [ours.clone()];
        let e = entry(
            &ctx(&f, &all, &ours),
            "edge-a",
            "192.0.2.10",
            "velmara_edge",
            "edge",
        );
        assert_eq!(e.outcome, "NOT APPLICABLE");
        assert!(e.detail.starts_with("host firewall baseline: off"));
        assert!(e.sha256.is_none() && e.ruleset.is_none());
    }

    /// Baseline on: the plan records the ruleset and the hash the renderer
    /// printed for this input, in the same run; the input carries the
    /// peering as peering-wg.yml builds it.
    #[test]
    fn on_records_the_ruleset_and_its_hash() {
        let f = fixture();
        let ours = p("velmara");
        let all = [ours.clone()];
        let c = ctx(&f, &all, &ours);
        let e = entry(&c, "edge-a", "192.0.2.10", "velmara_edge", "edge");
        assert_eq!(e.outcome, "PASS", "{}", e.detail);
        let i = input(&c, "edge-a", "192.0.2.10", "velmara_edge", "edge").unwrap();
        let Render::Ruleset { sha256, ruleset } = render(f.repo.path(), &i) else {
            panic!("refused")
        };
        assert_eq!(e.sha256.as_deref(), Some(sha256.as_str()));
        assert_eq!(e.ruleset.as_deref(), Some(ruleset.as_str()));
        let peer = &i["peerings"][0];
        assert_eq!(peer["name"], "wg-fed0");
        assert_eq!(peer["listen_port"], 51823);
        assert_eq!(peer["peer_addr"], "203.0.113.7");
        assert_eq!(peer["remote_link_ip"], "169.254.10.1");
        assert_eq!(peer["edge_port"], 7448);
        let core = entry(&c, "core-a", "192.0.2.20", "velmara_core", "core");
        assert_eq!(core.outcome, "PASS", "{}", core.detail);
        assert_ne!(core.sha256, e.sha256);
    }

    /// The plan's hash equals the hash the deploy prints for the same
    /// declaration: the role adds the live-host fields (bridges, ephemeral
    /// range, uplinks, the managing session, the listening sockets) to the same
    /// input, and the renderer's hash does not move.
    #[test]
    fn the_plan_hash_equals_the_deploy_hash() {
        let f = fixture();
        let ours = p("velmara");
        let all = [ours.clone()];
        let c = ctx(&f, &all, &ours);
        for (host, addr, group, profile) in [
            ("edge-a", "192.0.2.10", "velmara_edge", "edge"),
            ("core-a", "192.0.2.20", "velmara_core", "core"),
        ] {
            let planned = entry(&c, host, addr, group, profile);
            let mut deploy = input(&c, host, addr, group, profile).unwrap();
            deploy["host"]["bridge_interfaces"] = json!(["br-5f1e2d3c4b5a"]);
            deploy["host"]["ephemeral_ports"] = json!([32768, 60999]);
            deploy["host"]["uplink_interfaces"] = json!(["eth0"]);
            deploy["must_reach"] =
                json!([{"saddr": "198.51.100.10", "proto": "tcp", "dport": 22, "iif": "eth0"}]);
            deploy["listening_ss"] =
                json!("tcp LISTEN 0 128 0.0.0.0:22 0.0.0.0:* users:((\"sshd\",pid=1,fd=3))\n");
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
            f.repo.path().join("ops/ansible/group_vars/all/fw.yml"),
            "goat_host_firewall_baseline: true\ngoat_host_firewall_management_sources: []\n",
        );
        let ours = p("velmara");
        let all = [ours.clone()];
        let e = entry(
            &ctx(&f, &all, &ours),
            "edge-a",
            "192.0.2.10",
            "velmara_edge",
            "edge",
        );
        assert_eq!(
            (e.outcome.as_str(), e.code.as_deref()),
            ("BLOCKED", Some("GB-210")),
            "{}",
            e.detail
        );
    }

    /// Before the agreement, the tunnel the rules admit is not known: BLOCKED,
    /// saying so, rather than a ruleset built on a guess.
    #[test]
    fn no_agreement_is_blocked() {
        let f = fixture();
        std::fs::remove_file(f.work.path().join("agreement.json")).unwrap();
        let ours = p("velmara");
        let all = [ours.clone()];
        let e = entry(
            &ctx(&f, &all, &ours),
            "edge-a",
            "192.0.2.10",
            "velmara_edge",
            "edge",
        );
        assert_eq!(e.outcome, "BLOCKED");
        assert!(e.detail.contains("no agreement yet"), "{}", e.detail);
    }

    /// The core play's SPIRE values: the bundle port only on the SPIRE
    /// server's host, and never any sources.
    #[test]
    fn the_core_spire_bundle_port_is_the_plays() {
        let f = fixture();
        write(
            f.repo
                .path()
                .join("ops/ansible/group_vars/velmara_core/spire.yml"),
            "peering_spire_enabled: true\npeering_spire_server_host: core-a\n",
        );
        let ours = p("velmara");
        let all = [ours.clone()];
        let c = ctx(&f, &all, &ours);
        let i = input(&c, "core-a", "192.0.2.20", "velmara_core", "core").unwrap();
        assert_eq!(i["host"]["spire_bundle_port"], 8443);
        assert_eq!(i["host"]["spire_bundle_sources"], json!([]));
        let other = input(&c, "core-b", "192.0.2.21", "velmara_core", "core").unwrap();
        assert_eq!(other["host"]["spire_bundle_port"], "");
    }

    /// Two declared peerings on one edge machine are both in the input, as the
    /// role carries every peering stored on the host.
    #[test]
    fn every_peering_on_the_machine_is_carried() {
        let f = fixture();
        let root = f.work.path().parent().unwrap().to_path_buf();
        let meadow = root.join("meadow");
        write(
            meadow.join("agreement.json"),
            r#"{"transport":{"kind":"wireguard","link_subnet":"169.254.10.2/31"}}"#,
        );
        write(
            meadow.join("applied/ops/ansible/group_vars/meadow_edge/t.yml"),
            "goat_peering_remote_link_ip: \"169.254.10.3\"\n",
        );
        write(
            meadow.join("theirs.canonical.json"),
            r#"{"transport_offer":{"endpoint":"partner.example.org:51820"}}"#,
        );
        let inv = f
            .repo
            .path()
            .join("ops/ansible/inventory/peering-edges.ini");
        let text = std::fs::read_to_string(&inv).unwrap()
            + "[meadow_edge]\nedge-m ansible_host=192.0.2.10\n";
        std::fs::write(&inv, text).unwrap();
        write(
            f.repo
                .path()
                .join("ops/ansible/group_vars/meadow_edge/p.yml"),
            "peering_wg_interface: wg-fed1\npeering_wg_listen_port: 51825\n",
        );
        let ours = p("velmara");
        let all = [ours.clone(), p("meadow")];
        // Ours sits beside meadow's directory, as work_of_other looks for it.
        let our_dir = root.join("velmara");
        std::fs::create_dir_all(&our_dir).unwrap();
        for e in std::fs::read_dir(f.work.path()).unwrap() {
            let e = e.unwrap();
            let _ = std::process::Command::new("cp")
                .arg("-a")
                .arg(e.path())
                .arg(&our_dir)
                .status();
        }
        let c = HostContext {
            our_work: &our_dir,
            our_package: our_dir.join("theirs.canonical.json"),
            ..ctx(&f, &all, &ours)
        };
        let i = input(&c, "edge-a", "192.0.2.10", "velmara_edge", "edge").unwrap();
        let names: Vec<&str> = i["peerings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["wg-fed0", "wg-fed1"]);
        assert_eq!(
            i["peerings"][1]["peer_addr"], "",
            "a name is not filtered on, as the play does"
        );
        let _ = std::fs::remove_dir_all(&our_dir);
        let _ = std::fs::remove_dir_all(&meadow);
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

    /// The plan builds exactly the fields the role and the plays build: a field
    /// added there and not here would make the two hashes differ.
    #[test]
    fn the_input_has_the_fields_the_role_and_plays_build() {
        let f = fixture();
        let ours = p("velmara");
        let all = [ours.clone()];
        let c = ctx(&f, &all, &ours);
        let i = input(&c, "edge-a", "192.0.2.10", "velmara_edge", "edge").unwrap();
        let ours_keys = |v: &Value| {
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
        assert_eq!(ours_keys(&i), want, "top-level fields");
        let mut want = keys(&role_input["host"]);
        for k in LIVE_HOST_FIELDS {
            want.remove(*k);
        }
        assert_eq!(ours_keys(&i["host"]), want, "host fields");

        let play_peering = |play: &str| {
            let doc = yaml(play);
            let tasks = doc[0]["post_tasks"].as_sequence().unwrap().clone();
            let t = tasks
                .iter()
                .find(|t| t["vars"]["goat_host_firewall_profile"].as_str() == Some("edge"))
                .unwrap()
                .clone();
            keys(&t["vars"]["goat_host_firewall_peering"])
        };
        assert_eq!(
            ours_keys(&i["peerings"][0]),
            play_peering("ops/ansible/peering-wg.yml"),
            "wireguard peering fields"
        );
        std::fs::write(
            f.work.path().join("agreement.json"),
            r#"{"transport":{"kind":"ipsec-fips","link_subnet":"169.254.10.0/31"}}"#,
        )
        .unwrap();
        let i = input(&c, "edge-a", "192.0.2.10", "velmara_edge", "edge").unwrap();
        assert_eq!(
            ours_keys(&i["peerings"][0]),
            play_peering("ops/ansible/peering-edge.yml"),
            "ipsec peering fields"
        );
    }
}
