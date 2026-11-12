//! Every value `goatbuilder federate <peering>` uses, with its source: what the
//! command line, the environment and the declaration give the run, the plan's
//! own values (the working directory's files, the groups and inventories the
//! plays are limited to), phase 1's derived inputs (federate/preflight.rs and
//! federate/declared.rs), and, for a peering `goatbuilder federate allocate`
//! has recorded, its allocated values (federate/alloc.rs), the machines taken
//! from the site inventory (federate/site_hosts.rs) and the site values of a
//! rendered half (federate/site_values.rs).
//!
//! The run's value is the one listed: each entry repeats the rule the run
//! itself applies, and the test below runs the plan and finds every value of
//! every command line here, so a value the plan gains without a source fails it.

use std::path::Path;

use anyhow::{anyhow, Result};

use super::{push, rel, yaml_line, Input, Seg, Source};
use crate::config;
use crate::federate::{self, alloc, preflight, Options};

/// Reads one variable of the operator's environment, as it was before the run
/// exported anything: a value set there wins, and its source is the
/// environment.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The declaration and where it is, for naming lines in it.
struct Decl<'a> {
    path: String,
    text: String,
    cfg: &'a config::Config,
    index: usize,
}

impl Decl<'_> {
    fn peering_line(&self, rest: &[Seg]) -> Option<usize> {
        let mut path = vec![Seg::Key("peerings"), Seg::Index(self.index)];
        path.extend_from_slice(rest);
        yaml_line(&self.text, &path)
    }

    /// `peerings[<i>].<key...>` in goatbuilder.yaml, at its line.
    fn peering(&self, keys: &[&str]) -> Source {
        let segs: Vec<Seg> = keys.iter().map(|k| Seg::Key(k)).collect();
        Source::file(
            self.path.clone(),
            self.peering_line(&segs),
            format!("peerings[{}].{}", self.index, keys.join(".")),
        )
    }

    fn at(&self, path: &[Seg], key: String) -> Source {
        Source::file(self.path.clone(), yaml_line(&self.text, path), key)
    }
}

fn set(env: Env, name: &str) -> Option<String> {
    env(name).filter(|v| !v.is_empty())
}

/// The operator's value when set, else `value` from `source`.
fn env_or(
    v: &mut Vec<Input>,
    env: Env,
    name: &str,
    var: &str,
    value: String,
    source: Source,
) -> String {
    match set(env, var) {
        Some(x) => {
            push(v, Input::new(name, x.clone(), Source::env(var)));
            x
        }
        None => {
            push(v, Input::new(name, value.clone(), source));
            value
        }
    }
}

/// The inputs of `goatbuilder federate <o.with>`. `env` reads the operator's
/// environment (before the run exports phase 1's inputs).
pub fn inputs(repo: &Path, o: &Options, env: Env) -> Result<Vec<Input>> {
    let cfg_path = match o.mock {
        Some(m) => m.join("goatbuilder.yaml"),
        None => repo.join("ops/goatbuilder/goatbuilder.yaml"),
    };
    let cfg = config::load(&cfg_path)?;
    let cfg_dir = cfg_path.parent().unwrap_or(repo).to_path_buf();
    let index = cfg
        .peerings
        .iter()
        .position(|p| p.with == o.with && o.env.map(|e| p.environment == e).unwrap_or(true))
        .ok_or_else(|| {
            anyhow!(
                "peering '{}' is not declared in {} peerings: — declare it first (C1)",
                o.with,
                cfg_path.display()
            )
        })?;
    let peering = &cfg.peerings[index];
    let d = Decl {
        path: rel(repo, &cfg_path),
        text: std::fs::read_to_string(&cfg_path).unwrap_or_default(),
        cfg: &cfg,
        index,
    };
    let mut v: Vec<Input> = Vec::new();

    // What the command line and the declaration give the run.
    v.push(Input::new("peering", o.with, Source::arg("<peering>")));
    v.push(Input::new(
        "repo",
        repo.display().to_string(),
        Source::arg("-C, else the current directory"),
    ));
    if let Some(e) = o.env {
        v.push(Input::new("environment", e, Source::arg("--env")));
    }
    let work = match o.work {
        Some(w) => {
            v.push(Input::new("work", w, Source::arg("--work")));
            w.to_string()
        }
        None => env_or(
            &mut v,
            env,
            "work",
            "FED_WORK",
            format!("{}/{}", alloc::DEFAULT_WORK_ROOT, o.with),
            Source::default(format!(
                "{}/<peering>, the driver's working-directory root",
                alloc::DEFAULT_WORK_ROOT
            )),
        ),
    };
    let edge_group = match o.edge_group {
        Some(g) => {
            v.push(Input::new("edge_group", g, Source::arg("--edge-group")));
            g.to_string()
        }
        None => {
            let g = format!("{}_edge", o.with);
            v.push(Input::new(
                "edge_group",
                g.clone(),
                Source::derived(format!("the peering '{}'", o.with), "<peering>_edge"),
            ));
            g
        }
    };
    v.push(Input::new(
        "core_group",
        format!("{}_core", o.with),
        Source::derived(
            format!("the peering '{}'", o.with),
            "<peering>_core (the core the republisher play is limited to)",
        ),
    ));
    match o.r#as {
        Some(a) => v.push(Input::new("as", a, Source::arg("--as"))),
        None => {
            if let Some(a) = federate::our_id(&work) {
                v.push(Input::new(
                    "as",
                    a,
                    Source::derived(
                        format!(
                            "the goat context's federation.toml, else {work}/ours.canonical.json"
                        ),
                        "its sovereign_id",
                    ),
                ));
            }
        }
    }

    // The plan's own values: the working directory's files and the plays' inventories.
    let at_work = |f: &str| {
        Source::derived(
            format!("the working directory {work}"),
            format!("<work>/{f}"),
        )
    };
    let counterparty = env_or(
        &mut v,
        env,
        "counterparty",
        "FED_COUNTERPARTY",
        format!("{work}/theirs.canonical.json"),
        at_work("theirs.canonical.json"),
    );
    let keyring = env_or(
        &mut v,
        env,
        "keyring",
        "FED_KEYRING",
        format!("{work}/keys.txt"),
        at_work("keys.txt"),
    );
    let _ = counterparty;
    for f in ["ours.canonical.json", "agreement.json", "applied", "pki"] {
        v.push(Input::new(
            f.trim_end_matches(".json").trim_end_matches(".canonical"),
            format!("{work}/{f}"),
            at_work(f),
        ));
    }
    v.push(Input::new("OBJC_DISABLE_INITIALIZE_FORK_SAFETY", "YES", Source::default("set on every play, so Ansible's forked workers run on macOS; it changes nothing elsewhere")));
    v.push(Input::new(
        "inventory",
        "inventory/peering-edges.ini",
        Source::default(
            "the hand-written peering inventory, ops/ansible/inventory/peering-edges.ini",
        ),
    ));
    let own = format!("inventory/peering-{edge_group}.ini");
    if repo.join("ops/ansible").join(&own).is_file() {
        v.push(Input::new(
            "peering_inventory",
            own,
            Source::derived(
                format!("the edge group {edge_group}"),
                "inventory/peering-<edge group>.ini, written by goatbuilder federate allocate",
            ),
        ));
    }
    let aws = crate::settings::aws_profile(repo);
    let aws_src = if set(env, "GOAT_AWS_PROFILE").is_some() {
        Source::env("GOAT_AWS_PROFILE")
    } else {
        let decl = repo.join("ops/goatbuilder/goatbuilder.yaml");
        let text = std::fs::read_to_string(&decl).unwrap_or_default();
        match yaml_line(&text, &[Seg::Key("deploy"), Seg::Key("aws_profile")]) {
            Some(n) => Source::file(rel(repo, &decl), Some(n), "deploy.aws_profile"),
            None => Source::default("default, the profile the AWS tools themselves fall back to"),
        }
    };
    v.push(Input::new("aws_profile", aws.clone(), aws_src.clone()));
    v.push(Input::new("GOAT_AWS_PROFILE", aws, aws_src));
    match set(env, "GOAT_BIN") {
        Some(b) => v.push(Input::new("goat", b, Source::env("GOAT_BIN"))),
        None => v.push(Input::new(
            "goat",
            "goat",
            Source::derived("PATH", "the first goat on it, else goat"),
        )),
    }
    // Phase 6: each `sovereign=key` line of the keyring, at its line.
    if let Ok(text) = std::fs::read_to_string(&keyring) {
        for (i, line) in text.lines().enumerate() {
            let clean: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            if clean.is_empty() || clean.starts_with('#') {
                continue;
            }
            let who = clean.split('=').next().unwrap_or_default().to_string();
            v.push(Input::new(
                format!("key.{who}"),
                clean,
                Source::file(
                    keyring.clone(),
                    Some(i + 1),
                    "<sovereign_id>=<verifying key>, read aloud and recorded at phase 5",
                ),
            ));
        }
    }
    // Phase 11: the transport the agreement chose.
    let agreement = format!("{work}/agreement.json");
    if let Some(kind) = std::fs::read_to_string(&agreement)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|j| j.get("transport")?.get("kind")?.as_str().map(String::from))
    {
        v.push(Input::new(
            "transport",
            kind,
            Source::file(agreement.clone(), None, "transport.kind"),
        ));
    }
    if let Some(c) = set(env, "FED_ROUTER_CONTAINER") {
        v.push(Input::new(
            "FED_ROUTER_CONTAINER",
            c,
            Source::env("FED_ROUTER_CONTAINER"),
        ));
    }

    // Phase 1's inputs, which a real run exports for every phase.
    for i in preflight::derive_inputs(repo, &cfg, peering, &work, &edge_group, None) {
        if i.name == "FED_WORK" {
            v.push(Input::new(
                "FED_WORK",
                work.clone(),
                Source::derived("work (above)", "the run's working directory"),
            ));
            continue;
        }
        // Inputs that hold a path or a host address, which their names may make
        // look like a credential (FED_SIGNING_KEY is the path to our key).
        let make = |name: &'static str, value: String, src: Source| -> Input {
            if PATH_INPUTS.contains(&name) {
                Input::path(name, value, src)
            } else if ADDRESS_INPUTS.contains(&name) {
                Input::address(name, value, src)
            } else {
                Input::new(name, value, src)
            }
        };
        let input = match (set(env, i.name), &i.value) {
            (Some(x), _) => make(i.name, x, Source::env(i.name)),
            (None, Some(x)) => make(
                i.name,
                x.clone(),
                classify(repo, &d, &edge_group, &i.source),
            ),
            (None, None) => Input::unset(i.name, i.source.clone()),
        };
        push(&mut v, input);
    }
    // The core router's daemon reaches phase 3 as DOCKER_HOST.
    if let Some(h) = v
        .iter()
        .find(|i| i.name == "FED_ROUTER_DOCKER_HOST")
        .and_then(|i| i.value().map(String::from))
    {
        v.push(Input::address(
            "DOCKER_HOST",
            h,
            Source::derived("FED_ROUTER_DOCKER_HOST (above)", "phase 3's DOCKER_HOST"),
        ));
    }

    // The values `goatbuilder federate allocate` recorded for this peering.
    allocated(repo, &d, &cfg_dir, peering, &mut v)?;
    Ok(v)
}

/// Phase 1's inputs that hold a path: kept as they are, whatever their names.
const PATH_INPUTS: &[&str] = &[
    "FED_SIGNING_KEY",
    "FED_CA_CERT",
    "FED_CA_KEY",
    "FED_CA_KEY_AGE",
    "FED_AGE_IDENTITY",
    "FED_DECLARATION",
    "SSL_CERT_FILE",
];

/// Phase 1's inputs that hold a host address built from the inventory.
const ADDRESS_INPUTS: &[&str] = &["FED_ROUTER_DOCKER_HOST"];

/// A phase-1 input's source, from the words preflight.rs and declared.rs give
/// it, as a structured [`Source`] with the line where there is one.
fn classify(repo: &Path, d: &Decl, edge_group: &str, source: &str) -> Source {
    let gv = format!("group_vars/{edge_group}: ");
    if let Some(rest) = source.strip_prefix(&gv) {
        let key = rest.split_whitespace().next().unwrap_or(rest);
        return match group_var_at(repo, edge_group, key) {
            Some((file, line)) => Source::file(file, line, key),
            None => Source::file(format!("ops/ansible/group_vars/{edge_group}"), None, key),
        };
    }
    if let Some(key) = source.strip_prefix("goatbuilder.yaml peerings: ") {
        return d.peering(&[key]);
    }
    if let Some(rest) = source.strip_prefix("goatbuilder.yaml secrets.federation.") {
        let key = rest.split_whitespace().next().unwrap_or(rest);
        return d.at(
            &[Seg::Key("secrets"), Seg::Key("federation"), Seg::Key(key)],
            format!(
                "secrets.federation.{key} ({})",
                rest.trim_start_matches(key).trim()
            ),
        );
    }
    if let Some(rest) = source.strip_prefix("goatbuilder.yaml environments[") {
        // environments[<name>].peering_directory.<key>
        if let Some((name, path)) = rest.split_once("].") {
            let idx = d
                .cfg
                .environments
                .iter()
                .position(|e| e.name == name)
                .unwrap_or(0);
            let mut segs = vec![Seg::Key("environments"), Seg::Index(idx)];
            segs.extend(path.split('.').map(Seg::Key));
            return d.at(&segs, format!("environments[{name}].{path}"));
        }
    }
    if source.starts_with("goatbuilder.yaml deploy.aws_profile") {
        return Source::file(
            d.path.clone(),
            yaml_line(&d.text, &[Seg::Key("deploy"), Seg::Key("aws_profile")]),
            "deploy.aws_profile",
        );
    }
    if source.starts_with("the allocated core group") {
        return Source::derived(
            format!("peerings[{}].with", d.index),
            "<with>_core (no allocation passed to phase 1)",
        );
    }
    if source.starts_with("~/") {
        return Source::default(source.to_string());
    }
    // A path read where the deployment keeps it, a name read from the
    // inventory, or the core's address: each source already says which.
    Source::derived(
        source.to_string(),
        "the rule named there (federate/preflight.rs)",
    )
}

/// Where a group's own plain value is set: the last of the group's vars files
/// that sets it (preflight::group_var's order), and the line.
fn group_var_at(repo: &Path, group: &str, key: &str) -> Option<(String, Option<usize>)> {
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
    let mut found = None;
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
            continue;
        };
        if doc.get(key).is_some() {
            found = Some((rel(repo, &f), super::yaml_key_line(&text, key)));
        }
    }
    found
}

/// The allocated values, machines and site values of a peering the allocation
/// record holds (the run must then use them: peering_files::driver_agrees).
/// Nothing for a peering not yet allocated: its plays read hand-written files.
fn allocated(
    repo: &Path,
    d: &Decl,
    cfg_dir: &Path,
    peering: &config::Peering,
    v: &mut Vec<Input>,
) -> Result<()> {
    let ctx = alloc::AllocContext::load(cfg_dir)?;
    let key = format!("{}/{}", peering.environment, peering.with);
    if !ctx.record.peerings.contains_key(&key) {
        return Ok(());
    }
    let values = alloc::allocate_in(d.cfg, Some(&peering.environment), &peering.with, &ctx)?;
    let record_path = cfg_dir.join(alloc::RECORD_FILE);
    let record_text = std::fs::read_to_string(&record_path).unwrap_or_default();
    for (field, value, src) in alloc::field_sources(
        peering,
        &values,
        &ctx,
        &d.peering_source_fn(),
        &rel(repo, &record_path),
        &record_text,
    ) {
        v.push(Input::new(format!("values.{field}"), value, src));
    }
    // The machines: declared hosts:, else the site inventory's.
    match &peering.hosts {
        Some(h) => {
            v.push(Input::new(
                "hosts.edge",
                h.edge.clone(),
                d.peering(&["hosts", "edge"]),
            ));
            v.push(Input::new(
                "hosts.core",
                h.core.clone(),
                d.peering(&["hosts", "core"]),
            ));
            if let Some(u) = &h.user {
                v.push(Input::new(
                    "hosts.user",
                    u.clone(),
                    d.peering(&["hosts", "user"]),
                ));
            }
        }
        None => {
            if let Some(found) = federate::site_hosts::derive_at(
                repo,
                d.cfg,
                peering,
                &values.edge_group,
                &values.core_group,
            )? {
                for i in found.inputs() {
                    v.push(i);
                }
            }
        }
    }
    // The site values of a rendered half.
    if federate::switches::deployment(repo, peering, &values).rendered {
        let groups: Vec<String> = d
            .cfg
            .environments
            .iter()
            .find(|e| e.name == peering.environment)
            .map(|e| e.groups.clone())
            .unwrap_or_default();
        let hand = federate::site_values::hand_reader(repo, &groups);
        let scope = federate::site_values::Scope {
            repo,
            peering,
            values: &values,
            groups: groups.clone(),
            hand: &hand,
        };
        let (site, missing) = federate::site_values::required_all(&scope);
        for s in site {
            let shown = match &s.value {
                serde_yaml::Value::String(x) => x.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            v.push(Input::new(s.var, shown, s.source));
        }
        for (var, looked) in missing {
            v.push(Input::unset(
                var,
                format!("looked in: {}", looked.join("; ")),
            ));
        }
    }
    Ok(())
}

impl Decl<'_> {
    /// For alloc::field_sources: the line of `peerings[<i>].values.<field>`.
    fn peering_source_fn(&self) -> impl Fn(&str) -> Source + '_ {
        move |field: &str| self.peering(&["values", field])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(line: &str) -> Vec<(bool, String)> {
        super::super::command_values(
            line,
            &[
                "--work",
                "--edge-group",
                "--signing-key",
                "--declaration",
                "--key",
                "--as",
                "--ssh-edge-group",
                "--router-container",
                "--url",
                "--ours",
                "--package-url",
                "--their-id",
                "--theirs",
                "--keyring",
                "--ca",
            ],
        )
    }

    struct World {
        _root: crate::testdir::Held<std::path::PathBuf>,
        repo: std::path::PathBuf,
        mock: std::path::PathBuf,
        work: String,
    }

    /// The tutorial-3 peering `hello-ally`, with what the plays read on its
    /// hosts, a keyring and an agreement, as tests/federate_world builds it.
    fn world() -> World {
        let root = crate::testdir::path("goatbuilder-inputs-");
        let repo = root.join("repo");
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mock = repo.join("ops/goatbuilder");
        std::fs::create_dir_all(&mock).unwrap();
        std::fs::copy(
            real.join(
                "ops/goatbuilder/fixtures/goatnet-from-scratch/03-boundary-node/goatbuilder.yaml",
            ),
            mock.join("goatbuilder.yaml"),
        )
        .unwrap();
        let w = |p: &str, t: &str| {
            let p = repo.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        w("ops/ansible/group_vars/hello-ally_edge/peering.yml", "peering_wg_interface: wg-fed1\npeering_wg_listen_port: 51821\ngoat_federation_signing_key: /srv/fed/signing.key\n");
        w(
            "ops/ansible/group_vars/hello-ally_core/peering.yml",
            "goat_fed_republisher_metrics_port: 9464\n",
        );
        w("ops/ansible/inventory/peering-edges.ini", "[hello-ally_edge]\nedge-1 ansible_host=192.0.2.10 ansible_user=goat\n\n[hello-ally_core]\ncore-1 ansible_host=192.0.2.11 ansible_user=goat\n");
        let work = root.join("work/hello-ally");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(
            work.join("keys.txt"),
            "# recorded at phase 5\nxaa=QUJD\nxab=QUJE\n",
        )
        .unwrap();
        std::fs::write(
            work.join("agreement.json"),
            r#"{"transport":{"kind":"wireguard"}}"#,
        )
        .unwrap();
        let work = work.display().to_string();
        World {
            _root: root,
            repo,
            mock,
            work,
        }
    }

    fn opts<'a>(w: &'a World, dry: bool) -> Options<'a> {
        Options {
            with: "hello-ally",
            env: None,
            r#as: Some("xaa"),
            edge_group: None,
            work: Some(&w.work),
            mock: Some(&w.mock),
            from: 1,
            to: 99,
            dry_run: dry,
            table: false,
        }
    }

    /// Check (b) of the legibility conformance test, for federate, on the
    /// library: every value in every command the plan prints has an entry in
    /// `inputs` with a source.
    #[test]
    fn every_value_in_a_federate_command_has_a_sourced_input() {
        let w = world();
        let o = opts(&w, true);
        let report = crate::federate::federate(&w.repo, &o).unwrap();
        // Each phase's command; a MANUAL phase's is prose for the operator, not a command line.
        let mut cmds: Vec<&str> = Vec::new();
        let mut manual = false;
        for c in &report.checks {
            match c.name.as_str() {
                "phase" => manual = c.detail.contains("[MANUAL]"),
                "cmd" if !manual => cmds.push(c.detail.as_str()),
                _ => {}
            }
        }
        assert_eq!(cmds.len(), 12, "{cmds:?}");
        let env = |k: &str| std::env::var(k).ok();
        let inputs = inputs(&w.repo, &o, &env).unwrap();
        let mut missing = Vec::new();
        for c in &cmds {
            for (from_env, val) in values(c) {
                let ok = inputs.iter().any(|i| {
                    i.sourced()
                        && if from_env {
                            i.name == val
                        } else {
                            i.value() == Some(val.as_str())
                        }
                });
                if !ok {
                    missing.push(val);
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "values on a command line with no sourced input: {missing:?}\ninputs: {:#?}",
            inputs.iter().map(Input::line).collect::<Vec<_>>()
        );
    }

    /// Each input names a structured source; a file source names a line where
    /// the file has one; nothing renders empty.
    #[test]
    fn federate_inputs_carry_lines() {
        let w = world();
        let env = |_: &str| None;
        let inputs = inputs(&w.repo, &opts(&w, true), &env).unwrap();
        let get = |n: &str| {
            inputs
                .iter()
                .find(|i| i.name == n)
                .unwrap_or_else(|| panic!("{n}: {inputs:#?}"))
        };
        assert_eq!(
            get("FED_SIGNING_KEY").source,
            Source::file(
                "ops/ansible/group_vars/hello-ally_edge/peering.yml",
                Some(3),
                "goat_federation_signing_key"
            )
        );
        assert_eq!(
            get("FED_COUNTERPARTY_ID").source.render(),
            "ops/goatbuilder/goatbuilder.yaml:51 peerings[0].with"
        );
        assert_eq!(
            get("key.xab").source.render(),
            format!(
                "{}/keys.txt:3 <sovereign_id>=<verifying key>, read aloud and recorded at phase 5",
                w.work
            )
        );
        assert_eq!(get("work").source, Source::arg("--work"));
        assert_eq!(super::super::dangling(&inputs), Vec::<String>::new());
        for i in &inputs {
            assert!(i.sourced() || i.unset.is_some(), "{i:?}");
        }
    }

    /// The operator's environment wins, and its source says so.
    #[test]
    fn an_operator_value_is_an_environment_override() {
        let w = world();
        let key = w.repo.join("elsewhere-signing.key");
        std::fs::write(&key, "x").unwrap();
        let path = key.display().to_string();
        let env = |k: &str| (k == "FED_SIGNING_KEY").then(|| path.clone());
        let inputs = inputs(&w.repo, &opts(&w, true), &env).unwrap();
        let i = inputs.iter().find(|i| i.name == "FED_SIGNING_KEY").unwrap();
        assert_eq!(i.value(), Some(path.as_str()));
        assert_eq!(i.source, Source::env("FED_SIGNING_KEY"));
        // The key itself in the variable, not its path: held back.
        let env = |k: &str| (k == "FED_SIGNING_KEY").then(|| "0".repeat(64));
        let held = super::inputs(&w.repo, &opts(&w, true), &env).unwrap();
        let i = held.iter().find(|i| i.name == "FED_SIGNING_KEY").unwrap();
        assert!(i.secret && i.value().is_none() && i.note.is_some(), "{i:?}");
    }

    /// An allocated peering (`peering-values.yaml` holds it), declared
    /// `deployment: rendered`, with no `hosts:`: every allocated value names the
    /// record's line or the pin's, the machines name their inventory line, and
    /// every site value names the file and line it was read from or the rule
    /// it was derived by.
    #[test]
    fn allocated_hosts_and_site_values_carry_their_sources() {
        let root = crate::testdir::path("goatbuilder-inputs-alloc-");
        let repo = root.join("repo");
        let w = |p: &str, t: &str| {
            let p = repo.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        let decl = "\
schema: goatbuilder/v1
repo:
  source: example-org/zagros
  image_namespace: ghcr.io/example-org
components:
  zenoh-router:
    declared_in: declared/pins.yaml
environments:
  - name: zagros
    converger: push-ansible
    sites:
      - name: alpha
        classes:
          core: {}
          edge: {}
peerings:
  - with: xab
    environment: zagros
    site: alpha
    edge_class: edge
    deployment: rendered
    values:
      wg_listen_port: 51831
promotion:
  path:
    - name: zagros
      kind: rung
      environment: zagros
";
        w("ops/goatbuilder/goatbuilder.yaml", decl);
        w("ops/ansible/inventory/zagros.ini", "[alpha_core]\ncore-a ansible_host=192.0.2.21 ansible_user=goat\n\n[alpha_edge]\nedge-a ansible_host=192.0.2.20 ansible_user=goat\n");
        // The peering's own inventory, as `goatbuilder federate allocate` writes it.
        w("ops/ansible/inventory/peering-xab_edge.ini", "[xab_edge]\nzagros-edge-xab ansible_host=192.0.2.20 ansible_user=goat\n\n[xab_core]\nzagros-core-xab ansible_host=192.0.2.21 ansible_user=goat\n");
        w("ops/ansible/group_vars/all.yml", "goat_zenoh_listen_port: 7447\ngoat_fed_republisher_image: ghcr.io/example-org/republisher@sha256:aaaa\ngoat_zenoh_edge_image: ghcr.io/example-org/zenoh@sha256:bbbb\npeering_spire_bundle_port: 8443\n");
        let cfg = config::load(&repo.join("ops/goatbuilder/goatbuilder.yaml")).unwrap();
        let values = alloc::allocate_all(&cfg, &alloc::AllocContext::default()).unwrap();
        let record = alloc::record_of(
            &values,
            &alloc::Record {
                schema: alloc::RECORD_SCHEMA.into(),
                peerings: Default::default(),
            },
        );
        w(
            "ops/goatbuilder/peering-values.yaml",
            &alloc::record_text(&record),
        );
        let work = root.join("work").display().to_string();
        let o = Options {
            with: "xab",
            env: None,
            r#as: Some("xaa"),
            edge_group: None,
            work: Some(&work),
            mock: None,
            from: 1,
            to: 99,
            dry_run: true,
            table: false,
        };
        let env = |_: &str| None;
        let inputs = inputs(&repo, &o, &env).unwrap();
        let get = |n: &str| {
            inputs.iter().find(|i| i.name == n).unwrap_or_else(|| {
                panic!(
                    "{n}: {:#?}",
                    inputs.iter().map(Input::line).collect::<Vec<_>>()
                )
            })
        };
        let render = |n: &str| get(n).source.render();

        // The pin: its line in goatbuilder.yaml.
        assert_eq!(get("values.wg_listen_port").value(), Some("51831"));
        assert_eq!(
            render("values.wg_listen_port"),
            "ops/goatbuilder/goatbuilder.yaml:23 peerings[0].values.wg_listen_port"
        );
        // Recorded: its line in the record.
        let rec =
            std::fs::read_to_string(repo.join("ops/goatbuilder/peering-values.yaml")).unwrap();
        let line = rec
            .lines()
            .position(|l| l.trim_start().starts_with("wg_interface:"))
            .unwrap()
            + 1;
        assert_eq!(
            render("values.wg_interface"),
            format!(
                "allocated: ops/goatbuilder/peering-values.yaml:{line} zagros/xab.wg_interface"
            )
        );
        assert!(matches!(
            get("values.cert_dir").source,
            Source::Allocated { recorded: true, .. }
        ));
        // The machines, from the site inventory's lines.
        assert_eq!(get("hosts.edge").value(), Some("192.0.2.20"));
        assert!(
            render("hosts.edge").starts_with(
                "derived from ops/ansible/inventory/zagros.ini:5 [alpha_edge] edge-a by "
            ),
            "{}",
            render("hosts.edge")
        );
        assert!(
            render("hosts.core").starts_with(
                "derived from ops/ansible/inventory/zagros.ini:2 [alpha_core] core-a by "
            ),
            "{}",
            render("hosts.core")
        );
        assert_eq!(get("hosts.user").value(), Some("goat"));
        // The site values: read at a line, or derived by a rule.
        assert_eq!(
            render("goat_fed_republisher_image"),
            "ops/ansible/group_vars/all.yml:2 goat_fed_republisher_image"
        );
        assert_eq!(get("goat_zenoh_edge_acl_trace").value(), Some("true"));
        assert!(matches!(
            get("goat_zenoh_edge_acl_trace").source,
            Source::Default { .. }
        ));
        assert!(
            matches!(get("peering_wg_extra_ports").source, Source::Derived { .. }),
            "{}",
            render("peering_wg_extra_ports")
        );
        assert!(
            render("peering_wg_extra_ports")
                .contains("ops/ansible/group_vars/all.yml:4 peering_spire_bundle_port"),
            "{}",
            render("peering_wg_extra_ports")
        );
        // The core's address from the peering inventory, its port from all.yml.
        assert_eq!(
            get("goat_fed_republisher_core_endpoint").value(),
            Some("tls/192.0.2.21:7447")
        );
        let ep = render("goat_fed_republisher_core_endpoint");
        assert!(
            ep.contains("[xab_core]")
                && ep.contains("ops/ansible/group_vars/all.yml")
                && ep.ends_with("by tls/<core address>:<goat_zenoh_listen_port>"),
            "{ep}"
        );
        // The core's name: the site inventory's name for that address.
        assert_eq!(get("goat_zenoh_edge_core_cn").value(), Some("core-a"));
        // Nothing listed lacks a source except what nothing gave.
        for i in &inputs {
            assert!(
                i.sourced() || (i.unset.is_some() && i.value().is_none()),
                "{i:?}"
            );
        }
    }
}
