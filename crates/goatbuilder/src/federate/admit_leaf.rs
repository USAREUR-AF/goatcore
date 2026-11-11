//! Where phase 10 places our core's router leaf on a goat-zenoh role core:
//! where the role reads it (docs/design/goatbuilder.md, "Admission on a
//! goat-zenoh role core").
//!
//! The role reads the router leaf on the controller from
//! `zenoh_router_leaf_cert_path` and `zenoh_router_leaf_key_path`, which
//! default to `'router-cert.pem' | goat_leaf(goat_router_leaf_site)`:
//! `$GOAT_LEAF_DIR`, else the site manifest's `out_dir`, else
//! `<goat_controller_state_dir>/<site>-leaves`
//! (`ops/ansible/filter_plugins/goat_leaf.py`). Admission places phase 8's
//! certificate and key there, so the role's next run deploys them. The same
//! filter computes the paths here, so the two never disagree.
//!
//! A path the core group declares plainly is taken as it is; a templated one
//! other than the role's own default is not guessed (None: admission then
//! places no leaf, as without FED_CORE_CERT). FED_CORE_CERT and FED_CORE_KEY
//! stay overrides.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::admit_role::RoleCore;
use super::preflight::group_var;

/// A leaf's path, and where it came from.
pub type Leaf = (PathBuf, String);

/// `goat_leaf(<file>, <site>, <root>)` from this checkout's filter plugin.
fn goat_leaf(repo: &Path, file: &str, site: &str, root: Option<&str>) -> Option<PathBuf> {
    let plugins = repo.join("ops/ansible/filter_plugins");
    let out = Command::new("python3")
        .arg("-I")
        .arg("-c")
        .arg(
            "import sys; sys.path.insert(0, sys.argv[1]); import goat_leaf; \
             print(goat_leaf.goat_leaf(sys.argv[2], sys.argv[3], sys.argv[4] or None))",
        )
        .arg(&plugins)
        .arg(file)
        .arg(site)
        .arg(root.unwrap_or(""))
        .current_dir(repo)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!p.is_empty()).then(|| PathBuf::from(p))
}

/// (certificate, key) where the role reads our core's router leaf, with
/// where each came from; None for one that cannot be resolved.
pub fn router_leaf(repo: &Path, core: &RoleCore) -> (Option<Leaf>, Option<Leaf>) {
    let g = &core.group;
    let site = group_var(repo, g, "goat_router_leaf_site").unwrap_or_else(|| core.site_id.clone());
    let root = group_var(repo, g, "goat_controller_state_dir");
    let one = |var: &str, file: &str| {
        if let Some(p) = group_var(repo, g, var).filter(|p| !p.trim().is_empty()) {
            return Some((PathBuf::from(p), format!("group_vars/{g}: {var}")));
        }
        // A templated value in the group's own files is not the role default.
        if declared_templated(repo, g, var) {
            return None;
        }
        goat_leaf(repo, file, &site, root.as_deref()).map(|p| {
            (
                p,
                format!("the goat-zenoh role's {var}: goat_leaf({file}, {site})"),
            )
        })
    };
    (
        one("zenoh_router_leaf_cert_path", "router-cert.pem"),
        one("zenoh_router_leaf_key_path", "router-key.pem"),
    )
}

/// The group's own group_vars set `var` with a `{{ }}` template.
fn declared_templated(repo: &Path, group: &str, var: &str) -> bool {
    let gv = repo.join("ops/ansible/group_vars");
    let mut files = vec![
        gv.join(format!("{group}.yml")),
        gv.join(format!("{group}.yaml")),
    ];
    if let Ok(rd) = std::fs::read_dir(gv.join(group)) {
        files.extend(rd.filter_map(|e| e.ok().map(|e| e.path())));
    }
    files.iter().any(|f| {
        std::fs::read_to_string(f)
            .ok()
            .and_then(|t| serde_yaml::from_str::<serde_yaml::Value>(&t).ok())
            .and_then(|d| {
                d.get(var)
                    .and_then(|v| v.as_str())
                    .map(|s| s.contains("{{"))
            })
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let fp = d.path().join("ops/ansible/filter_plugins");
        std::fs::create_dir_all(&fp).unwrap();
        std::fs::copy(
            real.join("ops/ansible/filter_plugins/goat_leaf.py"),
            fp.join("goat_leaf.py"),
        )
        .unwrap();
        std::fs::create_dir_all(d.path().join("ops/ansible/group_vars/core_router")).unwrap();
        d
    }

    fn core() -> RoleCore {
        RoleCore {
            group: "core_router".into(),
            inventory: "inventory/core-router.ini".into(),
            site_id: "zagros".into(),
            play: "zenoh-deploy.yml".into(),
            machines: 1,
        }
    }

    #[test]
    fn the_leaf_goes_where_the_role_reads_it() {
        let d = repo();
        let gv = d.path().join("ops/ansible/group_vars");
        std::fs::write(
            gv.join("all.yml"),
            "goat_controller_state_dir: /srv/example-state\n",
        )
        .unwrap();
        let (cert, key) = router_leaf(d.path(), &core());
        assert_eq!(
            cert.unwrap().0,
            PathBuf::from("/srv/example-state/zagros-leaves/router-cert.pem")
        );
        assert_eq!(
            key.unwrap().0,
            PathBuf::from("/srv/example-state/zagros-leaves/router-key.pem")
        );
        // A site that serves another site's leaf, and the group's own state root.
        std::fs::write(
            gv.join("core_router/leaf.yml"),
            "goat_router_leaf_site: zagros-b\ngoat_controller_state_dir: /srv/other\n",
        )
        .unwrap();
        let (cert, _) = router_leaf(d.path(), &core());
        assert_eq!(
            cert.unwrap().0,
            PathBuf::from("/srv/other/zagros-b-leaves/router-cert.pem")
        );
    }

    #[test]
    fn a_declared_path_wins_and_a_templated_one_is_not_guessed() {
        let d = repo();
        let gv = d.path().join("ops/ansible/group_vars/core_router");
        std::fs::write(
            gv.join("leaf.yml"),
            "zenoh_router_leaf_cert_path: /etc/example/router.pem\nzenoh_router_leaf_key_path: \"{{ some_dir }}/router-key.pem\"\n",
        )
        .unwrap();
        let (cert, key) = router_leaf(d.path(), &core());
        let (p, from) = cert.unwrap();
        assert_eq!(p, PathBuf::from("/etc/example/router.pem"));
        assert!(from.contains("group_vars/core_router"), "{from}");
        assert!(key.is_none());
    }
}
