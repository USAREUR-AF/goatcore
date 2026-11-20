//! The data router's stack name, as every role a joining site's core node runs
//! reads it (GB-361).
//!
//! `zenoh_stack` names the router's directories, its container
//! (`<stack>-router`) and its compose network (`<stack>_default`). The router's
//! role (goat-zenoh) defaults it to `{{ site_id }}-zenoh`. Each health
//! publisher's role reads it with a default of its own, `goat-zenoh` today, or
//! requires it (`| mandatory`). Ansible gives each role only its own defaults,
//! so a site whose group_vars do not set `zenoh_stack` gets a router under one
//! name and publishers that look for another. This module resolves the value
//! as each role would, as `federate/admit_role.rs` resolves the router role's
//! declarations, and refuses a disagreement before any phase runs.

use std::path::{Path, PathBuf};

/// A role's reading of `zenoh_stack` when the group sets none.
#[derive(Debug, PartialEq)]
enum Unset {
    /// The role's own default, with `{{ site_id }}` put in.
    Default(String),
    /// The role requires it (`| mandatory`).
    Required,
}

/// group_vars/<group>.yml, then each file in group_vars/<group>/ in name
/// order, as Ansible merges them (the last that sets a key wins).
fn group_files(repo: &Path, group: &str) -> Vec<PathBuf> {
    let gv = repo.join("ops/ansible/group_vars");
    let mut files = vec![
        gv.join(format!("{group}.yml")),
        gv.join(format!("{group}.yaml")),
    ];
    if let Ok(rd) = std::fs::read_dir(gv.join(group)) {
        let mut more: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml"))
            .collect();
        more.sort();
        files.extend(more);
    }
    files
}

/// The group's own `zenoh_stack`, as written (templated or not), or None.
fn group_value(repo: &Path, group: &str) -> Option<String> {
    let mut found = None;
    for f in group_files(repo, group) {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
            continue;
        };
        if let Some(v) = doc.get("zenoh_stack").and_then(|v| v.as_str()) {
            found = Some(v.to_string());
        }
    }
    found
}

/// How `role` reads `zenoh_stack` when the group sets none, from its
/// defaults; None when the checkout has no defaults for it or the role does
/// not read it.
fn role_unset(repo: &Path, role: &str, site_id: &str) -> Option<Unset> {
    let text =
        std::fs::read_to_string(repo.join(format!("ops/ansible/roles/{role}/defaults/main.yml")))
            .ok()?;
    if role == "goat-zenoh" {
        let doc: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
        let v = doc.get("zenoh_stack")?.as_str()?;
        return Some(Unset::Default(v.replace("{{ site_id }}", site_id)));
    }
    let re = regex::Regex::new(
        r#"zenoh_stack\s*\|\s*(?:default\(\s*['"]([^'"]*)['"]\s*\)|(mandatory))"#,
    )
    .expect("regex");
    let c = re.captures(&text)?;
    Some(match c.get(1) {
        Some(d) => Unset::Default(d.as_str().replace("{{ site_id }}", site_id)),
        None => Unset::Required,
    })
}

/// Refuse, with GB-361, a joining site whose data router and declared health
/// publishers would read `zenoh_stack` differently. `group` is the site's
/// inventory group (`<site>_hosts`), `publishers` the roles phase 13 runs on
/// the node. A group value is every role's, so a plain one always agrees; a
/// templated one, or a checkout without the roles' defaults, gives no verdict
/// here and the plays decide.
pub(crate) fn check(
    repo: &Path,
    group: &str,
    site_id: &str,
    publishers: &[&str],
) -> anyhow::Result<()> {
    if group_value(repo, group).is_some() {
        return Ok(());
    }
    let Some(Unset::Default(router)) = role_unset(repo, "goat-zenoh", site_id) else {
        return Ok(());
    };
    let mut off: Vec<String> = Vec::new();
    for p in publishers {
        match role_unset(repo, p, site_id) {
            Some(Unset::Default(v)) if v != router => off.push(format!("{p} ({v})")),
            Some(Unset::Required) => off.push(format!("{p} (no default: the role requires it)")),
            _ => {}
        }
    }
    if off.is_empty() {
        return Ok(());
    }
    Err(goat_errors::coded(
        "GB-361",
        format!(
            "the data router and the health publishers at {site_id} would name its Zenoh stack differently: group_vars/{group}/ sets no zenoh_stack, so the router's role (goat-zenoh) takes '{router}', and {} read their own default, a router container and network the site's router does not have. Add `zenoh_stack: {router}` to group_vars/{group}/ so every role reads one value",
            off.join(", ")
        ),
    )
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn repo() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        write(
            t.path(),
            "ops/ansible/roles/goat-zenoh/defaults/main.yml",
            "zenoh_stack: \"{{ site_id }}-zenoh\"   # a shared role must not name a goatnet\n",
        );
        write(
            t.path(),
            "ops/ansible/roles/fabric-health/defaults/main.yml",
            "fabric_health_zenoh_network: \"{{ zenoh_stack | default('goat-zenoh') }}_default\"\n",
        );
        write(
            t.path(),
            "ops/ansible/roles/fabric-crawler/defaults/main.yml",
            "fabric_crawler_zenoh_stack: \"{{ zenoh_stack | mandatory }}\"\n",
        );
        write(
            t.path(),
            "ops/ansible/roles/time-health/defaults/main.yml",
            "time_health_x: \"{{ zenoh_stack | default('gamma-zenoh') }}\"\n",
        );
        write(
            t.path(),
            "ops/ansible/group_vars/gamma_hosts/vars.yml",
            "site_id: gamma\n",
        );
        t
    }

    fn code(e: &anyhow::Error) -> Option<String> {
        e.downcast_ref::<goat_errors::Coded>()
            .map(|c| c.code.clone())
    }

    #[test]
    fn a_publisher_with_another_default_is_refused_and_named() {
        let t = repo();
        let e = check(t.path(), "gamma_hosts", "gamma", &["fabric-health"]).unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("GB-361"), "{e}");
        let m = e.to_string();
        assert!(
            m.contains("fabric-health (goat-zenoh)")
                && m.contains("'gamma-zenoh'")
                && m.contains("zenoh_stack: gamma-zenoh"),
            "{m}"
        );
    }

    #[test]
    fn a_publisher_that_requires_it_is_refused_when_unset() {
        let t = repo();
        let e = check(t.path(), "gamma_hosts", "gamma", &["fabric-crawler"]).unwrap_err();
        assert!(
            e.to_string()
                .contains("fabric-crawler (no default: the role requires it)"),
            "{e}"
        );
    }

    #[test]
    fn a_publisher_whose_default_matches_the_router_passes() {
        let t = repo();
        check(t.path(), "gamma_hosts", "gamma", &["time-health"]).unwrap();
    }

    #[test]
    fn a_group_value_is_every_roles_so_it_passes() {
        let t = repo();
        write(
            t.path(),
            "ops/ansible/group_vars/gamma_hosts/zenoh.yml",
            "zenoh_stack: gamma-zenoh\n",
        );
        check(
            t.path(),
            "gamma_hosts",
            "gamma",
            &["fabric-health", "fabric-crawler"],
        )
        .unwrap();
        write(
            t.path(),
            "ops/ansible/group_vars/gamma_hosts/zenoh.yml",
            "zenoh_stack: \"{{ site_id }}-fabric\"\n",
        );
        check(t.path(), "gamma_hosts", "gamma", &["fabric-health"]).unwrap();
    }

    #[test]
    fn a_checkout_without_the_roles_gives_no_verdict() {
        let t = tempfile::tempdir().unwrap();
        check(t.path(), "gamma_hosts", "gamma", &["fabric-health"]).unwrap();
    }

    #[test]
    fn the_repositorys_own_roles_read_it_as_this_module_does() {
        let r = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert_eq!(
            role_unset(&r, "goat-zenoh", "gamma"),
            Some(Unset::Default("gamma-zenoh".into()))
        );
        assert_eq!(
            role_unset(&r, "fabric-crawler", "gamma"),
            Some(Unset::Required)
        );
        for p in [
            "fabric-health",
            "mesh-health",
            "presence-health",
            "time-health",
            "consistency-health",
        ] {
            assert!(
                role_unset(&r, p, "gamma").is_some(),
                "{p} reads zenoh_stack"
            );
        }
    }
}
