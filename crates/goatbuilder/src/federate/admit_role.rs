//! Phase 10's admission on a core deployed by the goat-zenoh role
//! (docs/design/goatbuilder.md, "Admission on a goat-zenoh role core").
//!
//! A core the goat-zenoh role deploys is already declared: an inventory group
//! of the site inventory whose group_vars configure the role, the play that
//! runs the role (the fleet pin of the component whose image variable those
//! group_vars set, `deploy_playbook:`), and the role's own `site_id`. This
//! finds that declaration, so admission restarts the core by running the
//! role's play on that group with `-e target=<group> -e site_id=<site_id>`
//! (`--check --diff` for the dry run), with no restart command typed for it.
//!
//! Used only when the deployment names no route of its own (FED_CORE_PLAY or
//! FED_CORE_RESTART stay overrides). Nothing found: admission asks for those
//! settings as before. Found but not resolvable (several groups, no
//! `site_id`, no play): refused with GB-236, naming what was looked for and
//! where.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The core router as the goat-zenoh role's declarations give it.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleCore {
    /// The inventory group the play targets.
    pub group: String,
    /// The inventory file that defines it, relative to `ops/ansible/`.
    pub inventory: String,
    /// The role's `site_id`.
    pub site_id: String,
    /// The play that runs the role, relative to `ops/ansible/`.
    pub play: String,
}

/// Every top-level key the group's group_vars set: group_vars/<group>.yml,
/// .yaml, and each yml/yaml file in group_vars/<group>/.
fn group_keys(repo: &Path, group: &str) -> BTreeSet<String> {
    let gv = repo.join("ops/ansible/group_vars");
    let mut files = vec![
        gv.join(format!("{group}.yml")),
        gv.join(format!("{group}.yaml")),
    ];
    if let Ok(rd) = std::fs::read_dir(gv.join(group)) {
        files.extend(
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml")),
        );
    }
    let mut keys = BTreeSet::new();
    for f in files {
        let Some(doc) = std::fs::read_to_string(&f)
            .ok()
            .and_then(|t| serde_yaml::from_str::<serde_yaml::Value>(&t).ok())
        else {
            continue;
        };
        if let Some(m) = doc.as_mapping() {
            keys.extend(m.keys().filter_map(|k| k.as_str().map(String::from)));
        }
    }
    keys
}

/// The variables the goat-zenoh role defines (its defaults).
fn role_vars(repo: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(repo.join("ops/ansible/roles/goat-zenoh/defaults/main.yml"))
        .ok()
        .and_then(|t| serde_yaml::from_str::<serde_yaml::Value>(&t).ok())
        .and_then(|d| {
            d.as_mapping().map(|m| {
                m.keys()
                    .filter_map(|k| k.as_str().map(String::from))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// The deploy play of the fleet pin whose `var` the group sets, as
/// `goatbuilder deploy` finds it (push::find_play).
fn play_for(repo: &Path, keys: &BTreeSet<String>) -> Option<String> {
    let text = std::fs::read_to_string(repo.join("ops/pins/fleet-pins.yaml")).ok()?;
    let pins: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let ansible = repo.join("ops/ansible");
    let s =
        |v: Option<&serde_yaml::Value>| v.and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let mut found: Vec<PathBuf> = Vec::new();
    for (name, comp) in pins.get("components")?.as_mapping()? {
        let var = s(comp.get("var"));
        if var.is_empty() || !keys.contains(&var) {
            continue;
        }
        let name = name.as_str().unwrap_or_default();
        let ledger = Some(s(comp.get("ledger_component")))
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| name.to_string());
        if let crate::push::Play::Found(p) =
            crate::push::find_play(&ansible, &s(comp.get("deploy_playbook")), &ledger, name)
        {
            if !found.contains(&p) {
                found.push(p);
            }
        }
    }
    match found.as_slice() {
        [one] => one
            .strip_prefix(&ansible)
            .ok()
            .map(|p| p.display().to_string()),
        _ => None,
    }
}

/// The addresses of the hosts of `group` in the peering inventories
/// (`ops/ansible/inventory/peering-*.ini`): the peering's own core.
fn peering_core_addresses(repo: &Path, group: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Ok(rd) = std::fs::read_dir(repo.join("ops/ansible/inventory")) else {
        return out;
    };
    for f in rd.filter_map(|e| e.ok().map(|e| e.path())) {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if !(name.starts_with("peering-") && name.ends_with(".ini")) {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        for (g, ms) in super::site_hosts::groups(&text) {
            if g == group {
                out.extend(ms.into_iter().map(|m| m.addr));
            }
        }
    }
    out
}

fn refuse(why: String) -> String {
    format!("our core router's goat-zenoh role declarations do not resolve (GB-236): {why}")
}

/// The goat-zenoh role core of this peering, from the site inventories and
/// their group_vars. `Ok(None)`: no group configures the role, so the
/// deployment's own settings are asked for. `Err`: found but not resolvable,
/// with what was looked for and where (GB-236). `core_group` is the
/// peering's core group, whose address picks one role group among several.
pub fn find(repo: &Path, core_group: &str) -> Result<Option<RoleCore>, String> {
    let role = role_vars(repo);
    if role.is_empty() {
        return Ok(None);
    }
    let looked = "the groups of ops/ansible/inventory/*.ini (not peering-*.ini) whose group_vars set a variable of roles/goat-zenoh/defaults/main.yml";
    // (group, inventory file, its machines' addresses, its group_vars keys)
    let mut cands: Vec<(String, String, BTreeSet<String>, BTreeSet<String>)> = Vec::new();
    for (file, text) in super::site_hosts::site_inventories(repo) {
        for (g, ms) in super::site_hosts::groups(&text) {
            if ms.is_empty() || cands.iter().any(|(c, ..)| *c == g) {
                continue;
            }
            let keys = group_keys(repo, &g);
            if keys.iter().any(|k| role.contains(k)) {
                let rel = file.trim_start_matches("ops/ansible/").to_string();
                cands.push((g, rel, ms.into_iter().map(|m| m.addr).collect(), keys));
            }
        }
    }
    if cands.len() > 1 {
        let all = cands
            .iter()
            .map(|(g, f, ..)| format!("[{g}] in {f}"))
            .collect::<Vec<_>>()
            .join(", ");
        let ours = peering_core_addresses(repo, core_group);
        cands.retain(|(_, _, addrs, _)| addrs.iter().any(|a| ours.contains(a)));
        if cands.len() != 1 {
            return Err(refuse(format!(
                "{all} configure the goat-zenoh role, and the address of the peering's core [{core_group}] in ops/ansible/inventory/peering-*.ini picks {} of them. Looked in {looked}",
                if cands.is_empty() { "none".to_string() } else { cands.len().to_string() }
            )));
        }
    }
    let Some((group, inventory, _, keys)) = cands.pop() else {
        return Ok(None);
    };
    let site_id = super::preflight::group_var(repo, &group, "site_id")
        .or_else(|| group.strip_suffix("_hosts").map(String::from))
        .filter(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        });
    let Some(site_id) = site_id else {
        return Err(refuse(format!(
            "[{group}] in {inventory} configures the role, but nothing gives its site_id: declare site_id in group_vars/{group}/ (the role's mandatory variable), or name the group <site>_hosts"
        )));
    };
    let Some(play) = play_for(repo, &keys) else {
        return Err(refuse(format!(
            "[{group}] in {inventory} configures the role, but no single fleet pin in ops/pins/fleet-pins.yaml whose var group_vars/{group}/ sets names a deploy play that exists (deploy_playbook:, else <ledger>-deploy.yml or <component>-deploy.yml)"
        )));
    };
    Ok(Some(RoleCore {
        group,
        inventory,
        site_id,
        play,
    }))
}

impl RoleCore {
    /// The role's play on the core group, from `ops/ansible`, as admission's
    /// dry run (`check`) or restart.
    pub fn play_cmd(&self, repo: &Path, check: bool) -> String {
        let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
        format!(
            "cd {} && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws {} -- ../scripts/ansible-preflight.sh -i {} -e target={} -e site_id={} {}{}",
            q(&repo.join("ops/ansible").display().to_string()),
            q(&crate::settings::aws_profile(repo)),
            q(&self.inventory),
            q(&self.group),
            q(&self.site_id),
            q(&self.play),
            if check { " --check --diff" } else { "" }
        )
    }

    pub fn route(&self) -> String {
        format!(
            "the goat-zenoh role's play {} on [{}] in {} (site_id {})",
            self.play, self.group, self.inventory, self.site_id
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkout(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let base: &[(&str, &str)] = &[
            ("ops/ansible/roles/goat-zenoh/defaults/main.yml", "zenoh_router_image: \"x\"\nzenoh_sovereign_peerings: []\n"),
            ("ops/ansible/zenoh-deploy.yml", "- hosts: \"{{ target | mandatory }}\"\n"),
            ("ops/pins/fleet-pins.yaml", "components:\n  zenoh-router:\n    var: zenoh_router_image\n    ledger_component: zagros-zenoh\n    deploy_playbook: zenoh-deploy.yml\n  other:\n    var: other_image\n"),
        ];
        for (rel, text) in base.iter().chain(files.iter()) {
            let f = d.path().join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        d
    }

    const INV: (&str, &str) = (
        "ops/ansible/inventory/core-router.ini",
        "[core_router]\nzagros-core ansible_host=192.0.2.10 ansible_user=goat\n",
    );
    const GV: (&str, &str) = (
        "ops/ansible/group_vars/core_router/zenoh.yml",
        "zenoh_router_image: \"ghcr.io/example-org/zenoh@sha256:00\"\nsite_id: zagros\n",
    );

    #[test]
    fn the_role_core_comes_from_the_group_whose_vars_configure_the_role() {
        let r = checkout(&[
            INV,
            GV,
            (
                "ops/ansible/inventory/zagros-site.ini",
                "[edge]\nzagros-edge ansible_host=192.0.2.20\n",
            ),
        ]);
        let c = find(r.path(), "xab_core").unwrap().unwrap();
        assert_eq!(
            c,
            RoleCore {
                group: "core_router".into(),
                inventory: "inventory/core-router.ini".into(),
                site_id: "zagros".into(),
                play: "zenoh-deploy.yml".into()
            }
        );
        let cmd = c.play_cmd(r.path(), true);
        assert!(cmd.contains("-i 'inventory/core-router.ini' -e target='core_router' -e site_id='zagros' 'zenoh-deploy.yml' --check --diff"), "{cmd}");
        assert!(!c.play_cmd(r.path(), false).contains("--check"));
    }

    #[test]
    fn no_group_configuring_the_role_finds_nothing() {
        let r = checkout(&[INV]);
        assert_eq!(find(r.path(), "xab_core").unwrap(), None);
    }

    #[test]
    fn site_id_from_a_site_hosts_group_name_else_refused() {
        let r = checkout(&[
            (
                "ops/ansible/inventory/mgmt.ini",
                "[zagros_hosts]\nh1 ansible_host=192.0.2.10\n",
            ),
            (
                "ops/ansible/group_vars/zagros_hosts/zenoh.yml",
                "zenoh_router_image: x\n",
            ),
        ]);
        assert_eq!(
            find(r.path(), "xab_core").unwrap().unwrap().site_id,
            "zagros"
        );
        let r = checkout(&[
            INV,
            (
                "ops/ansible/group_vars/core_router/zenoh.yml",
                "zenoh_router_image: x\n",
            ),
        ]);
        let e = find(r.path(), "xab_core").unwrap_err();
        assert!(
            e.contains("(GB-236)") && e.contains("declare site_id in group_vars/core_router/"),
            "{e}"
        );
    }

    #[test]
    fn several_role_groups_are_narrowed_by_the_peerings_core_address_or_refused() {
        let two = ("ops/ansible/inventory/core-router.ini", "[core_router]\nzagros-core ansible_host=192.0.2.10\n[spare_router]\nspare ansible_host=192.0.2.11\n");
        let gv2 = (
            "ops/ansible/group_vars/spare_router.yml",
            "zenoh_router_image: y\nsite_id: spare\n",
        );
        let r = checkout(&[two, GV, gv2]);
        let e = find(r.path(), "xab_core").unwrap_err();
        assert!(
            e.contains("(GB-236)")
                && e.contains("[core_router] in inventory/core-router.ini")
                && e.contains("[spare_router]"),
            "{e}"
        );
        let r = checkout(&[
            two,
            GV,
            gv2,
            (
                "ops/ansible/inventory/peering-xab_edge.ini",
                "[xab_core]\nzagros-core-xab ansible_host=192.0.2.10\n",
            ),
        ]);
        assert_eq!(
            find(r.path(), "xab_core").unwrap().unwrap().group,
            "core_router"
        );
    }

    #[test]
    fn no_deploy_play_for_the_groups_image_variable_is_refused() {
        let r = checkout(&[
            INV,
            (
                "ops/ansible/group_vars/core_router/zenoh.yml",
                "zenoh_sovereign_peerings: []\nsite_id: zagros\n",
            ),
        ]);
        let e = find(r.path(), "xab_core").unwrap_err();
        assert!(e.contains("(GB-236)") && e.contains("fleet pin"), "{e}");
    }
}
