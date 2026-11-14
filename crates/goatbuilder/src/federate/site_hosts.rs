//! A peering's machines, taken from the site inventory when the peering does
//! not declare `hosts:` (docs/design/goatbuilder.md, "Hosts from the site
//! inventory").
//!
//! The site inventory already names this side's edge and core: an INI file
//! under `ops/ansible/inventory/` whose groups are the site's classes. For a
//! peering with no `hosts:`, `goatbuilder federate allocate` looks for the
//! group of the peering's `edge_class` and the group of the site's one other
//! class (the core), each holding exactly one machine, and uses their
//! `ansible_host` (else the inventory name) and `ansible_user` as `hosts:`.
//!
//! Where it looks: the environment's declared `inventory:` alone when there is
//! one; otherwise every `*.ini` in `ops/ansible/inventory/` except the peering
//! inventories (`peering-*.ini`). A group matches when its name is the class,
//! `<site>_<class>`, `<environment>_<class>` or `<environment>_<site>_<class>`.
//! Nothing found: the peering stays without hosts, as before. More than one
//! candidate, or one that is not a single machine per class: refused (GB-234),
//! naming the candidates and where it looked. A peering whose edge or core
//! group the hand-written `peering-edges.ini` already defines is left alone,
//! because that file stays the peering's inventory.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::alloc::PeeringValues;
use crate::config::{Config, Peering, PeeringHosts};

/// One machine as an inventory names it.
#[derive(Debug, Clone, PartialEq)]
pub struct Machine {
    /// The inventory name.
    pub name: String,
    /// Its `ansible_host`, else the inventory name.
    pub addr: String,
    pub user: Option<String>,
}

/// Machines, each with the inventory group it is listed under.
type Grouped = Vec<(String, Machine)>;

/// The host lines of every plain `[group]` section of an INI inventory. The
/// one INI reader for the site inventory: site_values.rs reads the core's
/// address and name with it too.
pub fn groups(text: &str) -> Vec<(String, Vec<Machine>)> {
    let mut out: Vec<(String, Vec<Machine>)> = Vec::new();
    let mut current: Option<usize> = None;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with(';') {
            continue;
        }
        if let Some(name) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            // `:children` and `:vars` sections list groups and variables, not hosts.
            current = (!name.contains(':')).then(|| {
                out.push((name.to_string(), Vec::new()));
                out.len() - 1
            });
            continue;
        }
        let Some(i) = current else { continue };
        let mut words = l.split_whitespace();
        let Some(name) = words.next() else { continue };
        let mut m = Machine {
            name: name.to_string(),
            addr: name.to_string(),
            user: None,
        };
        for w in words {
            if let Some(v) = w.strip_prefix("ansible_host=") {
                m.addr = v.to_string();
            } else if let Some(v) = w.strip_prefix("ansible_user=") {
                m.user = Some(v.to_string());
            }
        }
        out[i].1.push(m);
    }
    out
}

/// The hand-written site inventories: every `ops/ansible/inventory/*.ini`
/// except the peering inventories (`peering-*.ini`) and a file
/// `goatbuilder federate allocate` generated, in name order, as (path relative
/// to the checkout, text). site_values.rs reads the core's name from these.
pub fn site_inventories(repo: &Path) -> Vec<(String, String)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(repo.join("ops/ansible/inventory"))
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    files.sort();
    files
        .into_iter()
        .filter_map(|f| {
            let name = f.file_name()?.to_str()?.to_string();
            if !name.ends_with(".ini") || name.starts_with("peering-") {
                return None;
            }
            let text = std::fs::read_to_string(&f).ok()?;
            (!text.starts_with(super::peering_files::MARKER))
                .then(|| (format!("ops/ansible/inventory/{name}"), text))
        })
        .collect()
}

/// The inventories to look in for a peering, as (path, text), and where that
/// is, for a refusal: the environment's declared `inventory:` alone, else
/// [`site_inventories`].
fn candidates(repo: &Path, cfg: &Config, p: &Peering) -> (Vec<(String, String)>, String) {
    let env = cfg.environments.iter().find(|e| e.name == p.environment);
    if let Some(inv) = env.and_then(|e| e.inventory.as_deref()) {
        let rel = format!("ops/ansible/{inv}");
        let text = std::fs::read_to_string(repo.join(&rel)).ok();
        return (
            text.map(|t| vec![(rel.clone(), t)]).unwrap_or_default(),
            rel,
        );
    }
    (
        site_inventories(repo),
        "ops/ansible/inventory/*.ini (not peering-*.ini)".to_string(),
    )
}

/// The site's core class: its one class other than the edge class.
fn core_class(cfg: &Config, p: &Peering) -> Option<String> {
    let site = cfg
        .environments
        .iter()
        .find(|e| e.name == p.environment)?
        .sites
        .iter()
        .find(|s| s.name == p.site)?;
    let others: Vec<&String> = site
        .classes
        .keys()
        .filter(|c| **c != p.edge_class)
        .collect();
    match others.as_slice() {
        [one] => Some((*one).clone()),
        _ => None,
    }
}

fn group_names(p: &Peering, class: &str) -> [String; 4] {
    let (e, s) = (&p.environment, &p.site);
    [
        class.to_string(),
        format!("{s}_{class}"),
        format!("{e}_{class}"),
        format!("{e}_{s}_{class}"),
    ]
}

/// What one inventory file says about the class: the machines of every
/// matching group, each with the group it came from.
fn matching(text: &str, names: &[String]) -> Vec<(String, Machine)> {
    groups(text)
        .into_iter()
        .filter(|(g, _)| names.contains(g))
        .flat_map(|(g, ms)| ms.into_iter().map(move |m| (g.clone(), m)))
        .collect()
}

/// Whether the hand-written peering inventory already defines the peering's
/// edge or core group.
fn hand_written_defines(repo: &Path, groups_of: &[&str]) -> bool {
    let Ok(text) = std::fs::read_to_string(repo.join("ops/ansible/inventory/peering-edges.ini"))
    else {
        return false;
    };
    if text.starts_with(super::peering_files::MARKER) {
        return false;
    }
    groups(&text)
        .iter()
        .any(|(g, _)| groups_of.contains(&g.as_str()))
}

fn refuse(p: &Peering, looked: &str, why: String) -> anyhow::Error {
    goat_errors::coded(
        "GB-234",
        format!(
            "peering '{}' declares no hosts:, and the site inventory does not name one edge and one core for it (GB-234): {why}. Looked in {looked} for the groups of site '{}' classes. Declare hosts: {{edge: <address>, core: <address>}} on the peering, or make the site inventory name one machine per class",
            p.with, p.site
        ),
    )
    .into()
}

/// The hosts the site inventory gives a peering that declares none, or `None`
/// when it gives nothing (the peering is then as before).
pub fn derive(
    repo: &Path,
    cfg: &Config,
    p: &Peering,
    edge_group: &str,
    core_group: &str,
) -> Result<Option<PeeringHosts>> {
    Ok(derive_at(repo, cfg, p, edge_group, core_group)?.map(|f| f.hosts))
}

/// The machines [`derive`] took, and where: the inventory file, and each
/// machine's group, inventory name and line there.
#[derive(Debug, Clone)]
pub struct Found {
    pub hosts: PeeringHosts,
    pub file: String,
    pub edge: (String, Machine, Option<usize>),
    pub core: (String, Machine, Option<usize>),
}

impl Found {
    /// `hosts.edge`, `hosts.core` and `hosts.user`, each derived from its
    /// inventory line.
    pub fn inputs(&self) -> Vec<crate::inputs::Input> {
        use crate::inputs::{Input, Source};
        let from = |(g, m, line): &(String, Machine, Option<usize>)| {
            format!(
                "{}{} [{g}] {}",
                self.file,
                line.map(|n| format!(":{n}")).unwrap_or_default(),
                m.name
            )
        };
        let rule =
            "the one machine of the site class's group: its ansible_host, else its inventory name";
        let mut v = vec![
            Input::new(
                "hosts.edge",
                self.hosts.edge.clone(),
                Source::derived(from(&self.edge), rule),
            ),
            Input::new(
                "hosts.core",
                self.hosts.core.clone(),
                Source::derived(from(&self.core), rule),
            ),
        ];
        if let Some(u) = &self.hosts.user {
            v.push(Input::new(
                "hosts.user",
                u.clone(),
                Source::derived(
                    format!("{} and {}", from(&self.edge), from(&self.core)),
                    "their one ansible_user",
                ),
            ));
        }
        v
    }
}

/// [`derive`], with where each machine was found.
pub fn derive_at(
    repo: &Path,
    cfg: &Config,
    p: &Peering,
    edge_group: &str,
    core_group: &str,
) -> Result<Option<Found>> {
    if p.hosts.is_some() || hand_written_defines(repo, &[edge_group, core_group]) {
        return Ok(None);
    }
    let Some(core) = core_class(cfg, p) else {
        return Ok(None);
    };
    let (files, looked) = candidates(repo, cfg, p);
    let edge_names = group_names(p, &p.edge_class);
    let core_names = group_names(p, &core);
    let mut found: Vec<(String, Grouped, Grouped)> = Vec::new();
    for (rel, text) in &files {
        let (e, c) = (matching(text, &edge_names), matching(text, &core_names));
        if !e.is_empty() || !c.is_empty() {
            found.push((rel.clone(), e, c));
        }
    }
    let describe = |ms: &[(String, Machine)]| {
        ms.iter()
            .map(|(g, m)| format!("{} in [{g}]", m.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (file, edges, cores) = match found.as_slice() {
        [] => return Ok(None),
        [one] => one,
        many => {
            let list = many
                .iter()
                .map(|(f, e, c)| format!("{f} ({})", describe(&[e.clone(), c.clone()].concat())))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(refuse(
                p,
                &looked,
                format!("{} files name them: {list}", many.len()),
            ));
        }
    };
    let one = |ms: &[(String, Machine)], class: &str| -> Result<(String, Machine)> {
        match ms {
            [(g, m)] => Ok((g.clone(), m.clone())),
            [] => Err(refuse(
                p,
                &looked,
                format!(
                    "{file} names no machine for class '{class}' (groups {})",
                    group_names(p, class).join(", ")
                ),
            )),
            _ => Err(refuse(
                p,
                &looked,
                format!(
                    "{file} names several machines for class '{class}': {}",
                    describe(ms)
                ),
            )),
        }
    };
    let ((eg, e), (cg, c)) = (one(edges, &p.edge_class)?, one(cores, &core)?);
    if e.user != c.user {
        let who = |u: &Option<String>| u.clone().unwrap_or_else(|| "no ansible_user".into());
        return Err(refuse(
            p,
            &looked,
            format!(
                "{file} reaches the edge as {} and the core as {}, and hosts: has one user",
                who(&e.user),
                who(&c.user)
            ),
        ));
    }
    let user = e.user.clone();
    let text = &files
        .iter()
        .find(|(f, _)| f == file)
        .expect("found in one of the files")
        .1;
    let (el, cl) = (
        crate::inputs::ini_host_line(text, &eg, &e.name),
        crate::inputs::ini_host_line(text, &cg, &c.name),
    );
    Ok(Some(Found {
        hosts: PeeringHosts {
            edge: e.addr.clone(),
            core: c.addr.clone(),
            user,
        },
        file: file.clone(),
        edge: (eg, e, el),
        core: (cg, c, cl),
    }))
}

/// `cfg` with `hosts:` filled from the site inventory on every peering that
/// declares none and whose site inventory names its machines. `values` are the
/// allocated values, in the order of `cfg.peerings` (their edge and core
/// groups). The rows say what was taken.
pub fn fill(
    repo: &Path,
    mut cfg: Config,
    values: &[PeeringValues],
) -> Result<(Config, Vec<String>)> {
    let mut rows = Vec::new();
    let snapshot = cfg.clone();
    for (p, v) in cfg.peerings.iter_mut().zip(values.iter()) {
        if let Some(h) = derive(repo, &snapshot, p, &v.edge_group, &v.core_group)? {
            rows.push(format!(
                "peering '{}': hosts from the site inventory: edge {}, core {}{}",
                p.with,
                h.edge,
                h.core,
                h.user
                    .as_deref()
                    .map(|u| format!(", user {u}"))
                    .unwrap_or_default()
            ));
            p.hosts = Some(h);
        }
    }
    Ok((cfg, rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(env: serde_json::Value, peerings: serde_json::Value) -> Config {
        serde_json::from_value(serde_json::json!({
            "schema": "goatbuilder/v1",
            "environments": [env],
            "promotion": {"path": [{"name": "zagros", "kind": "rung", "environment": "zagros"}]},
            "peerings": peerings,
        }))
        .unwrap()
    }

    fn env() -> serde_json::Value {
        serde_json::json!({"name": "zagros", "converger": "push-ansible",
            "sites": [{"name": "home", "role": "founder", "classes": {"core": {}, "edge": {}}}]})
    }

    fn peering() -> serde_json::Value {
        serde_json::json!([{"with": "xab", "environment": "zagros", "site": "home", "edge_class": "edge"}])
    }

    fn checkout(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (rel, text) in files {
            let f = d.path().join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        d
    }

    const SITE: &str = "# zagros site\n[core]\nzagros-core ansible_host=192.0.2.10 ansible_user=goat\n\n[edge]\nzagros-edge ansible_host=192.0.2.20 ansible_user=goat\n";

    fn run(repo: &Path, c: &Config) -> Result<Option<PeeringHosts>> {
        derive(repo, c, &c.peerings[0], "xab_edge", "xab_core")
    }

    #[test]
    fn the_site_inventory_gives_the_edge_the_core_and_the_user() {
        let repo = checkout(&[("ops/ansible/inventory/zagros-site.ini", SITE)]);
        let h = run(repo.path(), &cfg(env(), peering())).unwrap().unwrap();
        assert_eq!(
            (h.edge.as_str(), h.core.as_str(), h.user.as_deref()),
            ("192.0.2.20", "192.0.2.10", Some("goat"))
        );
    }

    #[test]
    fn prefixed_group_names_match_and_the_inventory_name_is_the_address_without_ansible_host() {
        let text = "[zagros_home_edge]\nedge-1.example.org\n[home_core]\ncore-1.example.org\n[edge:vars]\nx=1\n";
        let repo = checkout(&[("ops/ansible/inventory/site.ini", text)]);
        let h = run(repo.path(), &cfg(env(), peering())).unwrap().unwrap();
        assert_eq!(
            (h.edge.as_str(), h.core.as_str(), h.user),
            ("edge-1.example.org", "core-1.example.org", None)
        );
    }

    #[test]
    fn nothing_found_declared_hosts_or_a_hand_written_peering_inventory_change_nothing() {
        // No site inventory at all.
        let repo = checkout(&[]);
        assert!(
            run(repo.path(), &cfg(env(), peering())).unwrap().is_none(),
            "nothing derived"
        );
        // Only the peering inventories, which are never the site inventory.
        let repo = checkout(&[
            (
                "ops/ansible/inventory/peering-edges.ini",
                "[xab_edge]\ne ansible_host=192.0.2.9\n[xab_core]\nc\n",
            ),
            ("ops/ansible/inventory/peering-other.ini", SITE),
        ]);
        assert!(
            run(repo.path(), &cfg(env(), peering())).unwrap().is_none(),
            "nothing derived"
        );
        // hosts: declared stays the explicit form.
        let repo = checkout(&[("ops/ansible/inventory/zagros-site.ini", SITE)]);
        let mut p = peering();
        p[0]["hosts"] = serde_json::json!({"edge": "198.51.100.2", "core": "198.51.100.1"});
        assert!(
            run(repo.path(), &cfg(env(), p)).unwrap().is_none(),
            "nothing derived"
        );
        // A hand-written peering-edges.ini that defines this peering stays its inventory.
        let repo = checkout(&[
            ("ops/ansible/inventory/zagros-site.ini", SITE),
            (
                "ops/ansible/inventory/peering-edges.ini",
                "[xab_edge]\nzagros-edge-xab ansible_host=192.0.2.20\n",
            ),
        ]);
        assert!(
            run(repo.path(), &cfg(env(), peering())).unwrap().is_none(),
            "nothing derived"
        );
        // A site whose classes do not name one core class.
        let mut e = env();
        e["sites"][0]["classes"] = serde_json::json!({"edge": {}, "core": {}, "pod": {}});
        let repo = checkout(&[("ops/ansible/inventory/zagros-site.ini", SITE)]);
        assert!(
            run(repo.path(), &cfg(e, peering())).unwrap().is_none(),
            "nothing derived"
        );
    }

    #[test]
    fn the_declared_inventory_is_the_only_file_read() {
        let mut e = env();
        e["inventory"] = serde_json::json!("inventory/chosen.ini");
        let repo = checkout(&[
            ("ops/ansible/inventory/zagros-site.ini", SITE),
            (
                "ops/ansible/inventory/chosen.ini",
                "[edge]\ne2 ansible_host=203.0.113.2\n[core]\nc2 ansible_host=203.0.113.1\n",
            ),
        ]);
        let h = run(repo.path(), &cfg(e, peering())).unwrap().unwrap();
        assert_eq!(
            (h.edge.as_str(), h.core.as_str()),
            ("203.0.113.2", "203.0.113.1")
        );
    }

    fn refused(repo: &Path, c: &Config) -> String {
        let e = run(repo, c).unwrap_err().to_string();
        assert!(e.contains("(GB-234)"), "{e}");
        e
    }

    #[test]
    fn two_candidate_files_are_refused_naming_both_and_where_it_looked() {
        let repo = checkout(&[
            ("ops/ansible/inventory/a-site.ini", SITE),
            ("ops/ansible/inventory/b-site.ini", SITE),
        ]);
        let e = refused(repo.path(), &cfg(env(), peering()));
        assert!(
            e.contains("ops/ansible/inventory/a-site.ini")
                && e.contains("ops/ansible/inventory/b-site.ini"),
            "{e}"
        );
        assert!(
            e.contains("zagros-edge in [edge]")
                && e.contains("Looked in ops/ansible/inventory/*.ini"),
            "{e}"
        );
    }

    #[test]
    fn several_machines_none_for_a_class_or_two_users_are_refused() {
        let several = "[edge]\ne1 ansible_host=192.0.2.21\ne2 ansible_host=192.0.2.22\n[core]\nc ansible_host=192.0.2.10\n";
        let repo = checkout(&[("ops/ansible/inventory/site.ini", several)]);
        let e = refused(repo.path(), &cfg(env(), peering()));
        assert!(
            e.contains("several machines for class 'edge'")
                && e.contains("e1 in [edge], e2 in [edge]"),
            "{e}"
        );
        let no_core = "[edge]\ne1 ansible_host=192.0.2.21\n";
        let repo = checkout(&[("ops/ansible/inventory/site.ini", no_core)]);
        assert!(
            refused(repo.path(), &cfg(env(), peering())).contains("no machine for class 'core'")
        );
        let users = "[edge]\ne ansible_host=192.0.2.20 ansible_user=goat\n[core]\nc ansible_host=192.0.2.10 ansible_user=ops\n";
        let repo = checkout(&[("ops/ansible/inventory/site.ini", users)]);
        assert!(
            refused(repo.path(), &cfg(env(), peering())).contains("as goat and the core as ops")
        );
    }
}
