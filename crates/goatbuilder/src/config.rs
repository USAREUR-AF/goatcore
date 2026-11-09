//! The config: loaded from YAML, validated against the embedded versioned schema,
//! then against the cross-field rules JSON Schema cannot express. These rules are
//! the same ones `ops/goatbuilder/validate.py` enforces; the parity test in this
//! module runs both over this repository's real config.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use jsonschema::{Draft, JSONSchema};
use serde::Deserialize;
use serde_json::Value;

use crate::{Check, Report, EXIT_REFUSED};

/// The schema the binary validates against, fixed at build time (schema: goatbuilder/v1).
pub const SCHEMA_JSON: &str =
    include_str!("../../../ops/goatbuilder/schema/goatbuilder-v1.schema.json");

/// The typed view the verbs act on. Only the fields a verb reads are modelled;
/// the schema is the authority on shape and it has already run by the time this
/// is built.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub schema: String,
    #[serde(default)]
    pub preset: Option<String>,
    /// `None` only under `preset: lite` with no checkout config to take it from: the
    /// schema requires it of every other config, and no lite verb reads it.
    #[serde(default)]
    pub repo: Option<Repo>,
    /// Empty only in that same case: the verbs read components from `ops/pins/`.
    #[serde(default)]
    pub components: BTreeMap<String, Component>,
    /// Explicit, or derived from the `lite:` block by `derive_lite` before this is built.
    pub environments: Vec<Environment>,
    /// Explicit, or derived from the `lite:` block by `derive_lite` before this is built.
    pub promotion: Promotion,
    #[serde(default)]
    pub peerings: Vec<Peering>,
    /// The declared secrets posture (ADR 1204); `env-file` under a lite config that declares none.
    #[serde(default)]
    pub secrets: Option<Secrets>,
    /// The goatlite preset's four facts (ADR 1213); `Some` only under `preset: lite`.
    #[serde(default)]
    pub lite: Option<Lite>,
}

/// The learner's `lite:` block (docs/design/goatlite.md §2): what a goatlite
/// is derived from. Only the fields the binary reads are modelled; the schema
/// is the authority on shape.
#[derive(Debug, Clone, Deserialize)]
pub struct Lite {
    pub name: String,
    pub host: String,
    pub ssh_user: String,
    pub admin: String,
    #[serde(default)]
    pub zone: Option<String>,
    #[serde(default)]
    pub disk_encryption: Option<bool>,
    /// The LUKS data volume's device; the renderer's (default /dev/nvme0n1).
    #[serde(default)]
    pub data_disk: Option<String>,
    /// Path to the administrator's SSH public key; the renderer's (default the first ~/.ssh/id_*.pub).
    #[serde(default)]
    pub ssh_pubkey: Option<String>,
    #[serde(default)]
    pub internet: Option<LiteInternet>,
}

impl Lite {
    /// The derived environment name and inventory group stem: `<name>` with `-` as `_`.
    pub fn env_name(&self) -> String {
        self.name.replace('-', "_")
    }

    /// The goatnet's zone: declared, or `<name>.lan`.
    pub fn zone(&self) -> String {
        self.zone.clone().unwrap_or_else(|| format!("{}.lan", self.name))
    }
}

/// Growing out of lite (goatlite.md §6). ops/scripts/goatlite-render.py renders the
/// internet surface from it; the binary reads `aws_profile` and makes the standup's
/// profile internet. The rest is the renderer's (validated by the schema).
#[derive(Debug, Clone, Deserialize)]
pub struct LiteInternet {
    pub zone: String,
    #[serde(default)]
    pub aws_profile: Option<String>,
    #[serde(default)]
    pub cloudflare_token_ref: Option<String>,
}

/// The declared secrets posture (schema `secrets:`); the store is what the verbs
/// read — the rest of the block is posture.py's (ADR 1204).
#[derive(Debug, Clone, Deserialize)]
pub struct Secrets {
    pub store: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Repo {
    pub source: String,
    pub image_namespace: String,
    #[serde(default)]
    pub signing: Option<String>,
    /// The repository whose release workflow signs the goatbuilder binaries
    /// this project installs. A deployment whose declarations live in one
    /// repository and whose goatbuilder comes from another names the second
    /// here. Omitted: `source`.
    #[serde(default)]
    pub release_source: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Component {
    #[serde(default)]
    pub image_workflow: Option<String>,
    #[serde(default)]
    pub var: Option<String>,
    pub declared_in: String,
    #[serde(default)]
    pub ledger: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Environment {
    pub name: String,
    pub converger: String,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default)]
    pub inventory: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub sites: Vec<Site>,
    /// UDP ports this environment's edge hosts already hold, which a peering's
    /// tunnel or carrier never takes. Absent: NetBird's 51820.
    #[serde(default)]
    pub peering_edge_reserved_udp: Option<Vec<u16>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Site {
    pub name: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub joins: Option<String>,
    #[serde(default)]
    pub converger: Option<String>,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default)]
    pub classes: BTreeMap<String, Class>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Class {
    #[serde(default)]
    pub converger: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Promotion {
    pub path: Vec<Stage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Stage {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub environment: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Peering {
    pub with: String,
    pub environment: String,
    pub site: String,
    pub edge_class: String,
    /// Step 1's tunnel: `wireguard` or `ipsec` (the FIPS-validated IPsec
    /// transport). Absent: wireguard. The one field for the transport.
    #[serde(default)]
    pub tunnel: Option<String>,
    /// Release sets this peering offers, narrowing the `[data] release` of the
    /// goat context's federation.toml, which owns them. Absent: all of them.
    /// Checked by federate/declaration.rs: it may narrow, never widen.
    #[serde(default)]
    pub offer: Option<Vec<String>>,
    /// Pins that override allocation, same keys as the allocated values.
    #[serde(default)]
    pub values: crate::federate::declaration::PeeringPins,
    /// The machines' addresses, for the rendered peering inventory.
    #[serde(default)]
    pub hosts: Option<PeeringHosts>,
    /// Present when a peering forwarder relays the partner to our edge (ADR 1218).
    #[serde(default)]
    #[allow(dead_code)] // read where the forwarder is rendered and applied
    pub forwarder: Option<PeeringForwarder>,
}

/// Where the edge and the core are, as the inventory reaches them.
#[derive(Debug, Clone, Deserialize)]
pub struct PeeringHosts {
    pub edge: String,
    pub core: String,
    #[serde(default)]
    pub user: Option<String>,
}

/// The peering forwarder in front of the edge (docs/design/peering-forwarder.md).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // read where the forwarder is rendered and applied
pub struct PeeringForwarder {
    /// The forwarder's public host the partner and our edge dial.
    pub endpoint: String,
    /// The inventory group the forwarder is in (default `peering_forwarders`).
    #[serde(default)]
    pub group: Option<String>,
    /// The forwarder's address as the inventory reaches it (for the rendered inventory).
    #[serde(default)]
    pub ansible_host: Option<String>,
}

/// Words `goatbuilder federate <word>` takes, or is reserved to take, as a
/// subcommand, so a peering declared with one could never be run: the six
/// federation verbs, clap's `help`, the stages of the one command (`status`,
/// `plan`, `deploy`, `test`), and `allocate`. The one list; ops/goatbuilder/
/// validate.py's RESERVED_PEERING_NAMES mirrors it.
pub const RESERVED_PEERING_NAMES: &[&str] = &[
    "init", "lint", "compose", "diff", "verify", "apply", "help", "status", "plan", "deploy",
    "test", "allocate",
];

/// Read a YAML config file into a JSON value (the form the schema validates).
pub fn load_value(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read config {}", path.display()))?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&text)
        .with_context(|| format!("{} is not valid YAML", path.display()))?;
    serde_json::to_value(yaml)
        .with_context(|| format!("{} does not convert to JSON", path.display()))
}

/// The checkout's own config a lite file beside it may take `repo` and
/// `components` from: `<dir of path>/ops/goatbuilder/goatbuilder.yaml`, when it
/// exists and is not the file itself.
pub fn checkout_for(path: &Path) -> Option<Value> {
    let dir = path.parent()?;
    let own = dir.join("ops/goatbuilder/goatbuilder.yaml");
    if !own.is_file() {
        return None;
    }
    let same = std::fs::canonicalize(&own).ok() == std::fs::canonicalize(path).ok();
    if same {
        return None;
    }
    load_value(&own).ok()
}

/// Every problem with the config, as `where: what` lines; empty means valid.
/// Shape errors come first; the cross-field rules run only on a well-shaped config.
/// A lite config is judged without a checkout config here; `problems_in` takes one.
#[cfg(test)]
pub fn problems(value: &Value) -> Result<Vec<String>> {
    problems_in(value, None)
}

/// `problems`, with the checkout's own config a lite file may take `repo` and
/// `components` from.
#[allow(dead_code)] // the record-free form, for callers without a config path
pub fn problems_in(value: &Value, checkout: Option<&Value>) -> Result<Vec<String>> {
    problems_at(value, checkout, None)
}

/// `problems_in`, reading the peering allocation record in `record_dir` (the
/// config file's directory), so a pin on a recorded value is reported too.
pub fn problems_at(
    value: &Value,
    checkout: Option<&Value>,
    record_dir: Option<&Path>,
) -> Result<Vec<String>> {
    let schema: Value = serde_json::from_str(SCHEMA_JSON).context("embedded schema is not JSON")?;
    let compiled = JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&schema)
        .map_err(|e| goat_errors::coded("GB-025", format!("embedded schema does not compile: {e}")))?;
    let mut out: Vec<String> = Vec::new();
    if let Err(errors) = compiled.validate(value) {
        let mut shaped: Vec<(String, String)> = errors
            .map(|e| {
                let path = e.instance_path.to_string();
                let path = if path.is_empty() {
                    "(root)".to_string()
                } else {
                    path.trim_start_matches('/').to_string()
                };
                (path, e.to_string())
            })
            .collect();
        shaped.sort();
        out.extend(shaped.into_iter().map(|(p, m)| format!("{p}: {m}")));
    }
    if !out.is_empty() {
        return Ok(out);
    }
    out.extend(lite_problems(value));
    if !out.is_empty() {
        return Ok(out);
    }
    let cfg: Config = serde_json::from_value(derive_lite(value, checkout))
        .context("config does not match the typed model")?;
    out.extend(cross_field_at(&cfg, record_dir));
    Ok(out)
}

/// The preset-lite rules (ADR 1213) on the config as written, before any
/// derivation: under `lite`, `environments` may be absent only when a `lite:`
/// block stands in for them; a `lite:` block beside explicit environments must
/// name one of them (its derived environment `<name with - as _>`, or a site
/// `<name>`). `repo` and `components` are taken from the checkout's config when it
/// has one and may be absent when it does not. The same rules, in the same words,
/// as validate.py.
pub fn lite_problems(value: &Value) -> Vec<String> {
    if value.get("preset").and_then(Value::as_str) != Some("lite") {
        return Vec::new();
    }
    let lite = value.get("lite");
    let envs = value.get("environments").and_then(Value::as_array);
    match (envs, lite) {
        (None, None) => vec!["(root): preset lite: declare environments or a lite: block".to_string()],
        (Some(envs), Some(lite)) => {
            let name = lite.get("name").and_then(Value::as_str).unwrap_or("");
            if lite_env(envs, name).is_some() {
                Vec::new()
            } else {
                vec![format!("lite: lite.name '{name}' names no declared environment")]
            }
        }
        _ => Vec::new(),
    }
}

/// The declared environment a `lite:` block names: the one called
/// `<name with - as _>`, or the one carrying a site called `<name>`.
fn lite_env<'a>(envs: &'a [Value], name: &str) -> Option<&'a str> {
    let name_us = name.replace('-', "_");
    envs.iter().find_map(|e| {
        let n = e.get("name").and_then(Value::as_str)?;
        let site = e
            .get("sites")
            .and_then(Value::as_array)
            .map(|ss| ss.iter().any(|s| s.get("name").and_then(Value::as_str) == Some(name)))
            .unwrap_or(false);
        (n == name_us || site).then_some(n)
    })
}

/// The preset's derivation (docs/design/goatlite.md §3, §4): under `preset: lite`
/// with a `lite:` block, fill in what the config leaves out — one environment
/// `<name with - as _>` (push-ansible, transport direct, group `<name_us>_hosts`,
/// one founder site `<name>` with class `core`), one rung, and the `env-file`
/// secrets store. An explicit field is never overwritten. The same defaults
/// validate.py's `derive_lite` writes; a config that is not lite is returned as is.
pub fn derive_lite(value: &Value, checkout: Option<&Value>) -> Value {
    let mut out = value.clone();
    let Some(obj) = out.as_object_mut() else { return out };
    if obj.get("preset").and_then(Value::as_str) != Some("lite") {
        return out;
    }
    // repo and components: the repository the learner cloned declares them in
    // its own ops/goatbuilder/goatbuilder.yaml; the lite config reuses them.
    if let Some(co) = checkout {
        for key in ["repo", "components"] {
            if !obj.contains_key(key) {
                if let Some(v) = co.get(key) {
                    obj.insert(key.into(), v.clone());
                }
            }
        }
    }
    let Some(lite) = obj.get("lite").cloned() else { return out };
    let Some(name) = lite.get("name").and_then(Value::as_str).map(str::to_string) else { return out };
    let name_us = name.replace('-', "_");
    if !obj.contains_key("environments") {
        obj.insert(
            "environments".into(),
            serde_json::json!([{
                "name": name_us,
                "converger": "push-ansible",
                "transport": "direct",
                "groups": [format!("{name_us}_hosts")],
                "sites": [{"name": name, "role": "founder", "classes": {"core": {}}}]
            }]),
        );
    }
    if !obj.contains_key("promotion") {
        // The one rung is the goatnet itself: the environment the lite: block names.
        let env_name = obj
            .get("environments")
            .and_then(Value::as_array)
            .and_then(|envs| lite_env(envs, &name))
            .unwrap_or(name_us.as_str())
            .to_string();
        obj.insert(
            "promotion".into(),
            serde_json::json!({"path": [{"name": env_name, "kind": "rung", "environment": env_name}]}),
        );
    }
    if !obj.contains_key("secrets") {
        obj.insert("secrets".into(), serde_json::json!({"store": "env-file"}));
    }
    out
}

/// The rules JSON Schema cannot express — identical in intent to validate.py's.
#[allow(dead_code)] // the record-free form, kept for callers without a config path
pub fn cross_field(cfg: &Config) -> Vec<String> {
    cross_field_at(cfg, None)
}

/// `cross_field`, with the peering allocation record read from `record_dir`.
pub fn cross_field_at(cfg: &Config, record_dir: Option<&Path>) -> Vec<String> {
    let mut out = Vec::new();
    let names: Vec<&str> = cfg.environments.iter().map(|e| e.name.as_str()).collect();
    let mut dupes: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| names.iter().filter(|m| m == &n).count() > 1)
        .collect();
    dupes.sort();
    dupes.dedup();
    if !dupes.is_empty() {
        out.push(format!("environments: duplicate name(s) {dupes:?}"));
    }
    let mut sites_of: HashMap<&str, HashMap<&str, &Site>> = HashMap::new();
    for (ei, env) in cfg.environments.iter().enumerate() {
        let site_names: Vec<&str> = env.sites.iter().map(|s| s.name.as_str()).collect();
        let mut sd: Vec<&str> = site_names
            .iter()
            .copied()
            .filter(|n| site_names.iter().filter(|m| m == &n).count() > 1)
            .collect();
        sd.sort();
        sd.dedup();
        if !sd.is_empty() {
            out.push(format!(
                "environments/{ei}/sites: duplicate site name(s) {sd:?}"
            ));
        }
        let map: HashMap<&str, &Site> = env.sites.iter().map(|s| (s.name.as_str(), s)).collect();
        for (si, s) in env.sites.iter().enumerate() {
            if s.role.as_deref() == Some("joiner") {
                let target = s.joins.as_deref().and_then(|j| map.get(j));
                let founder = target
                    .map(|t| {
                        t.role.as_deref().unwrap_or("founder") == "founder" && t.name != s.name
                    })
                    .unwrap_or(false);
                if !founder {
                    out.push(format!(
                        "environments/{ei}/sites/{si}: joiner '{}' must join a founder site in '{}', not '{}'",
                        s.name,
                        env.name,
                        s.joins.as_deref().unwrap_or("")
                    ));
                }
            }
        }
        sites_of.insert(env.name.as_str(), map);
    }
    for (i, stage) in cfg.promotion.path.iter().enumerate() {
        if let Some(env) = stage.environment.as_deref() {
            if !names.contains(&env) {
                out.push(format!(
                    "promotion/path/{i}: stage '{}' names undeclared environment '{env}'",
                    stage.name
                ));
            }
        }
    }
    for (pi, p) in cfg.peerings.iter().enumerate() {
        let Some(sites) = sites_of.get(p.environment.as_str()) else {
            out.push(format!(
                "peerings/{pi} (with '{}'): names undeclared environment '{}'",
                p.with, p.environment
            ));
            continue;
        };
        let Some(site) = sites.get(p.site.as_str()) else {
            out.push(format!(
                "peerings/{pi} (with '{}'): names undeclared site '{}' in '{}'",
                p.with, p.site, p.environment
            ));
            continue;
        };
        if !site.classes.contains_key(&p.edge_class) {
            out.push(format!(
                "peerings/{pi} (with '{}'): edge_class '{}' is not a declared class at site '{}'",
                p.with, p.edge_class, p.site
            ));
        }
    }
    // Reserved names and the offer's shape (docs/design/goatbuilder.md, "The
    // peering declaration").
    out.extend(crate::federate::declaration::problems(cfg));
    // Pins that are malformed or collide, and partners declared twice
    // (docs/design/goatbuilder.md, "Per-peering values").
    out.extend(match record_dir {
        Some(d) => crate::federate::alloc::declaration_problems_in(cfg, d),
        None => crate::federate::alloc::declaration_problems(cfg),
    });
    out
}

/// Load, validate, and return the typed config — the entry every verb uses.
/// The GitHub owner this checkout belongs to, from `repo: source:`. A fork or
/// another organisation declares its own, so nothing below compiles ours in.
pub fn owner(repo: &Path) -> Option<String> {
    let v = load_value(&repo.join("ops/goatbuilder/goatbuilder.yaml")).ok()?;
    let source = v.get("repo")?.get("source")?.as_str()?.to_string();
    Some(source.split('/').next()?.to_string())
}

/// The 1Password account `secrets: op_account:` pins, where one is declared.
pub fn op_account(repo: &Path) -> Option<String> {
    let v = load_value(&repo.join("ops/goatbuilder/goatbuilder.yaml")).ok()?;
    Some(v.get("secrets")?.get("op_account")?.as_str()?.to_string())
}

pub fn load(path: &Path) -> Result<Config> {
    let value = load_value(path)?;
    let checkout = checkout_for(path);
    let problems = problems_at(&value, checkout.as_ref(), path.parent())?;
    if !problems.is_empty() {
        return Err(anyhow!(
            "{} does not validate:\n  {}",
            path.display(),
            problems.join("\n  ")
        ));
    }
    serde_json::from_value(derive_lite(&value, checkout.as_ref())).context("config does not match the typed model")
}

/// The `validate` verb's report: one check per problem, or one passing check.
pub fn validate_report(path: &Path) -> Result<Report> {
    let value = load_value(path)?;
    let problems = problems_at(&value, checkout_for(path).as_ref(), path.parent())?;
    let checks = if problems.is_empty() {
        vec![Check {
            name: "config".into(),
            ok: true,
            detail: format!("{} validates against {}", path.display(), schema_version()),
            fix: None,
        }]
    } else {
        problems
            .into_iter()
            .map(|p| Check {
                name: "config".into(),
                ok: false,
                detail: p,
                fix: None,
            })
            .collect()
    };
    Ok(Report::new("validate", checks, EXIT_REFUSED))
}

fn schema_version() -> String {
    serde_json::from_str::<Value>(SCHEMA_JSON)
        .ok()
        .and_then(|s| {
            s.get("properties")?
                .get("schema")?
                .get("const")?
                .as_str()
                .map(String::from)
        })
        .unwrap_or_else(|| "goatbuilder/v1".to_string())
}

/// Whether this checkout declares a deployment of its own: the private
/// repository does (`ops/goatbuilder/goatbuilder.yaml`); a public clone of
/// goatcore does not, by design, and the tests that read that deployment
/// return early there.
#[cfg(test)]
pub(crate) fn checkout_declares_a_deployment() -> bool {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ops/goatbuilder/goatbuilder.yaml")
        .is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal() -> Value {
        json!({
            "schema": "goatbuilder/v1",
            "preset": "lite",
            "repo": {"source": "example-org/example-repo", "image_namespace": "ghcr.io/example-org"},
            "components": {"example-service": {"declared_in": "ops/pins/fleet-pins.yaml"}},
            "environments": [{"name": "dev", "converger": "push-ansible"}],
            "promotion": {"path": [{"name": "dev", "kind": "rung", "environment": "dev"}]}
        })
    }

    fn with_sites() -> Value {
        let mut v = minimal();
        v["environments"][0]["sites"] = json!([
            {"name": "alpha", "role": "founder", "classes": {"core": {}, "edge": {}}},
            {"name": "bravo", "role": "joiner", "joins": "alpha", "classes": {"pod": {"converger": "pull-agent"}}}
        ]);
        v
    }

    fn rejected(v: &Value, fragment: &str) {
        let p = problems(v).unwrap();
        assert!(!p.is_empty(), "expected a validation error, got none");
        assert!(
            p.iter().any(|x| x.contains(fragment)),
            "{fragment:?} not in {p:?}"
        );
    }

    #[test]
    fn minimal_validates() {
        assert_eq!(problems(&minimal()).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn sites_and_peering_validate() {
        let mut v = with_sites();
        v["peerings"] = json!([{"with": "velmara", "environment": "dev", "site": "alpha", "edge_class": "edge",
            "tunnel": "wireguard", "trust": "spire-federation", "ingress_prefix": "from/velmara/"}]);
        assert_eq!(problems(&v).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn unknown_field_is_refused() {
        let mut v = minimal();
        v["surprise"] = json!(1);
        rejected(&v, "surprise");
    }

    #[test]
    fn wrong_schema_version_is_refused() {
        let mut v = minimal();
        v["schema"] = json!("goatbuilder/v2");
        rejected(&v, "schema");
    }

    #[test]
    fn converger_outside_vocabulary_is_refused() {
        let mut v = minimal();
        v["environments"][0]["converger"] = json!("ssh-by-hand");
        rejected(&v, "converger");
    }

    /// An address on one operator's own network is not a transport (ADR 1114).
    #[test]
    fn transport_on_one_operators_network_is_refused() {
        let mut v = minimal();
        v["environments"][0]["transport"] = json!("tailscale");
        rejected(&v, "transport");
        v["environments"][0]["transport"] = json!("ssm");
        assert_eq!(problems(&v).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn soak_gate_requires_min_days() {
        let mut v = minimal();
        v["promotion"]["path"][0]["gates"] = json!([{"id": "soak"}]);
        rejected(&v, "min_days");
    }

    #[test]
    fn stage_naming_undeclared_environment_is_refused() {
        let mut v = minimal();
        v["promotion"]["path"][0]["environment"] = json!("prod");
        rejected(&v, "undeclared environment 'prod'");
    }

    #[test]
    fn joiner_must_join_a_declared_founder() {
        let mut v = with_sites();
        v["environments"][0]["sites"][1]["joins"] = json!("charlie");
        rejected(&v, "must join a founder site");
    }

    #[test]
    fn joiner_requires_joins() {
        let mut v = with_sites();
        v["environments"][0]["sites"][1]
            .as_object_mut()
            .unwrap()
            .remove("joins");
        rejected(&v, "joins");
    }

    #[test]
    fn peering_edge_class_must_be_declared() {
        let mut v = with_sites();
        v["peerings"] = json!([{"with": "velmara", "environment": "dev", "site": "alpha", "edge_class": "router"}]);
        rejected(&v, "edge_class 'router'");
    }

    #[test]
    fn peering_to_undeclared_site_is_refused() {
        let mut v = with_sites();
        v["peerings"] = json!([{"with": "velmara", "environment": "dev", "site": "delta", "edge_class": "edge"}]);
        rejected(&v, "undeclared site 'delta'");
    }

    /// The learner's file (goatlite.md §2): four facts, no environments, no promotion.
    fn goatlite() -> Value {
        json!({
            "schema": "goatbuilder/v1",
            "preset": "lite",
            "lite": {"name": "pasture", "host": "pasture-box.lan", "ssh_user": "ubuntu", "admin": "admin@example.org"},
            "repo": {"source": "example-org/goatnet-from-scratch", "image_namespace": "ghcr.io/example-org"},
            "components": {"netbird-mgmt": {"declared_in": "declared/pins.yaml"}}
        })
    }

    fn typed(v: &Value) -> Config {
        assert_eq!(problems(v).unwrap(), Vec::<String>::new());
        serde_json::from_value(derive_lite(v, None)).unwrap()
    }

    /// ADR 1213 D2/D3: a four-field lite config validates and derives one
    /// environment, one founder site with class core, one rung and the env-file store.
    #[test]
    fn a_four_field_goatlite_config_derives_its_environment_site_and_rung() {
        let cfg = typed(&goatlite());
        assert_eq!(cfg.environments.len(), 1);
        let env = &cfg.environments[0];
        assert_eq!(env.name, "pasture");
        assert_eq!(env.converger, "push-ansible");
        assert_eq!(env.transport.as_deref(), Some("direct"));
        assert_eq!(env.groups, vec!["pasture_hosts".to_string()]);
        assert_eq!(env.sites.len(), 1);
        assert_eq!(env.sites[0].name, "pasture");
        assert_eq!(env.sites[0].role.as_deref(), Some("founder"));
        assert!(env.sites[0].classes.contains_key("core") && env.sites[0].classes.len() == 1);
        assert_eq!(cfg.promotion.path.len(), 1);
        assert_eq!((cfg.promotion.path[0].name.as_str(), cfg.promotion.path[0].kind.as_str(), cfg.promotion.path[0].environment.as_deref()), ("pasture", "rung", Some("pasture")));
        assert_eq!(cfg.secrets.as_ref().map(|s| s.store.as_str()), Some("env-file"));
        let lite = cfg.lite.as_ref().unwrap();
        assert_eq!((lite.env_name().as_str(), lite.zone().as_str(), lite.admin.as_str()), ("pasture", "pasture.lan", "admin@example.org"));
        assert!(lite.disk_encryption.is_none() && lite.internet.is_none());
        // The derived shape validates as a full declaration in its own right.
        let mut full = derive_lite(&goatlite(), None);
        full["preset"] = json!("full");
        assert_eq!(problems(&full).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn a_hyphenated_lite_name_derives_an_underscored_environment_and_a_hyphenated_site() {
        let mut v = goatlite();
        v["lite"]["name"] = json!("my-pasture");
        let cfg = typed(&v);
        assert_eq!(cfg.environments[0].name, "my_pasture");
        assert_eq!(cfg.environments[0].groups, vec!["my_pasture_hosts".to_string()]);
        assert_eq!(cfg.environments[0].sites[0].name, "my-pasture");
        assert_eq!(cfg.promotion.path[0].environment.as_deref(), Some("my_pasture"));
        assert_eq!(cfg.lite.unwrap().zone(), "my-pasture.lan");
    }

    /// An explicit field always wins over a derived one.
    #[test]
    fn explicit_fields_win_over_the_lite_derivation() {
        let mut v = goatlite();
        v["environments"] = json!([{"name": "pasture", "converger": "pull-agent", "sites": [{"name": "pasture", "classes": {"core": {}, "edge": {}}}]}]);
        v["secrets"] = json!({"store": "sops"});
        v["lite"]["zone"] = json!("pasture.home.arpa");
        let cfg = typed(&v);
        assert_eq!(cfg.environments[0].converger, "pull-agent");
        assert!(cfg.environments[0].transport.is_none(), "nothing is added inside an explicit environment");
        assert_eq!(cfg.environments[0].sites[0].classes.len(), 2);
        assert_eq!(cfg.secrets.unwrap().store, "sops");
        assert_eq!(cfg.promotion.path[0].environment.as_deref(), Some("pasture"), "the rung still derives");
        assert_eq!(cfg.lite.unwrap().zone(), "pasture.home.arpa");
        // An explicit environment under another name, carrying the site, is the one the rung names.
        let mut v = goatlite();
        v["environments"] = json!([{"name": "dev", "converger": "push-ansible", "sites": [{"name": "pasture"}]}]);
        let cfg = typed(&v);
        assert_eq!(cfg.promotion.path[0].environment.as_deref(), Some("dev"));
    }

    /// The full preset is untouched by the derivation and still requires everything.
    #[test]
    fn the_full_preset_is_untouched_and_still_requires_environments_and_promotion() {
        let v = minimal();
        let mut full = v.clone();
        full["preset"] = json!("full");
        assert_eq!(derive_lite(&full, None), full);
        assert!(typed(&full).lite.is_none());
        let mut v = goatlite();
        v["preset"] = json!("full");
        rejected(&v, "\"environments\" is a required property");
        rejected(&v, "\"promotion\" is a required property");
        v.as_object_mut().unwrap().remove("preset");
        rejected(&v, "\"environments\" is a required property");
    }

    /// The learner's file is four facts: repo and components come from the
    /// checkout's own config, and a checkout with none (the open-source export)
    /// still validates. An explicit block wins over the checkout's.
    #[test]
    fn a_four_fact_lite_config_takes_repo_and_components_from_the_checkout() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let mut four = goatlite();
        let obj = four.as_object_mut().unwrap();
        obj.remove("repo");
        obj.remove("components");
        assert_eq!(problems(&four).unwrap(), Vec::<String>::new(), "no checkout config: four facts still validate");
        let own = load_value(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/goatbuilder.yaml")).unwrap();
        assert_eq!(problems_in(&four, Some(&own)).unwrap(), Vec::<String>::new());
        let cfg: Config = serde_json::from_value(derive_lite(&four, Some(&own))).unwrap();
        assert_eq!(cfg.repo.unwrap().source, own["repo"]["source"].as_str().unwrap());
        assert!(cfg.components.contains_key("microscope"), "the checkout's components");
        assert_eq!(cfg.environments[0].name, "pasture", "the lite: block still derives the rest");
        // An explicit block wins over the checkout's.
        let cfg: Config = serde_json::from_value(derive_lite(&goatlite(), Some(&own))).unwrap();
        assert_eq!(cfg.repo.unwrap().source, "example-org/goatnet-from-scratch");
        assert_eq!(cfg.components.len(), 1);
        // The four-fact file at a checkout root loads through `load`, finding the checkout config beside it.
        let root = std::env::temp_dir().join(format!("gb-lite-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ops/goatbuilder")).unwrap();
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/goatbuilder.yaml"), root.join("ops/goatbuilder/goatbuilder.yaml")).unwrap();
        std::fs::write(root.join("goatlite.yaml"), "schema: goatbuilder/v1\npreset: lite\nlite:\n  name: pasture\n  host: 192.0.2.50\n  ssh_user: ubuntu\n  admin: me@example.org\n").unwrap();
        let cfg = load(&root.join("goatlite.yaml")).unwrap();
        assert!(cfg.components.contains_key("microscope") && cfg.lite.is_some() && cfg.secrets.unwrap().store == "env-file");
        assert!(checkout_for(&root.join("ops/goatbuilder/goatbuilder.yaml")).is_none(), "the checkout config never takes itself as a checkout");
        std::fs::remove_file(root.join("ops/goatbuilder/goatbuilder.yaml")).unwrap();
        let cfg = load(&root.join("goatlite.yaml")).unwrap();
        assert!(cfg.repo.is_none() && cfg.components.is_empty());
        assert_eq!(cfg.environments[0].name, "pasture");
        assert_eq!(cfg.secrets.unwrap().store, "env-file");
        // The other direction: only preset lite may leave them out.
        let mut full = four.clone();
        full["preset"] = json!("full");
        rejected(&full, "\"repo\" is a required property");
        rejected(&full, "\"components\" is a required property");
        let mut no_block = four.clone();
        no_block.as_object_mut().unwrap().remove("lite");
        rejected(&no_block, "preset lite: declare environments or a lite: block");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn lite_with_neither_environments_nor_a_lite_block_is_refused() {
        let mut v = goatlite();
        v.as_object_mut().unwrap().remove("lite");
        rejected(&v, "preset lite: declare environments or a lite: block");
        // minimal() is lite with explicit environments and no lite: block — still fine.
        assert_eq!(problems(&minimal()).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn a_lite_block_beside_environments_must_name_one_of_them() {
        let mut v = goatlite();
        v["environments"] = json!([{"name": "dev", "converger": "push-ansible"}]);
        rejected(&v, "lite.name 'pasture' names no declared environment");
    }

    #[test]
    fn the_lite_block_is_fail_closed() {
        let mut v = goatlite();
        v["lite"]["admin"] = json!("not-an-email");
        rejected(&v, "lite/admin");
        let mut v = goatlite();
        v["lite"]["port"] = json!(22);
        rejected(&v, "port");
        let mut v = goatlite();
        v["lite"]["internet"] = json!({"aws_profile": "pasture"});
        rejected(&v, "zone");
        let mut v = goatlite();
        v["lite"]["internet"] = json!({"zone": "pasture.example.org", "aws_profile": "pasture", "cloudflare_token_ref": "CLOUDFLARE_API_TOKEN_PASTURE"});
        v["lite"]["disk_encryption"] = json!(false);
        let cfg = typed(&v);
        assert_eq!(cfg.lite.as_ref().unwrap().internet.as_ref().unwrap().zone, "pasture.example.org");
        assert_eq!(cfg.lite.unwrap().disk_encryption, Some(false));
        let mut v = goatlite();
        v["lite"]["internet"] = json!({"zone": "pasture.example.org", "acme_email": "ops@example.org",
            "relays": [{"name": "a", "region": "us-east-2"}], "mesh_ip": "192.0.2.7"});
        assert_eq!(typed(&v).lite.unwrap().internet.unwrap().zone, "pasture.example.org");
        let mut v = goatlite();
        v["lite"]["internet"] = json!({"zone": "pasture.example.org", "relays": [{"name": "a"}]});
        rejected(&v, "region");
    }

    /// Parity with validate.py: both accept this repository's generated config.
    #[test]
    fn this_repositorys_config_validates() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/goatbuilder.yaml");
        let cfg = load(&path).expect("the generated config validates");
        assert_eq!(cfg.schema, "goatbuilder/v1");
        assert!(cfg.components.contains_key("microscope"));
        let names: Vec<&str> = cfg.promotion.path.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["dev", "preprod", "prod"]);
    }
}
