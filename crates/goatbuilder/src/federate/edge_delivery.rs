//! Before phase 12: whether the peering material phase 8 wrote reaches the
//! edge host.
//!
//! Phase 8 mints the leaves, and places our CA beside them, in `<work>/pki` on
//! the machine the driver runs on. The edge router mounts each of those files
//! by path on the edge host. When the driver runs on the edge host (the
//! inventory marks it `ansible_connection=local`, or names it by a loopback
//! address or by this machine's own name), those paths are the same files. When
//! it does not, as from an operator's workspace, only the role's own install
//! (`goat_zenoh_edge_install_certs`) copies them there. So when the edge group's
//! vars point the edge at files in `<work>/pki`, the edge is another machine,
//! and the install is off, phase 12 is refused first, with the reason and the
//! one setting that fixes it (GB-272): otherwise the edge starts with nothing to
//! mount and never reaches the core, which its inward-link check reports only
//! after the fact. Files the operator placed on the edge by hand, at paths of
//! their own, are not this case and are not refused.

use std::path::Path;

/// One host of the edge group, as the peering inventory names it.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeHost {
    pub name: String,
    pub address: String,
    pub local: bool,
}

/// The direct hosts of `group` in the peering inventory text, each with its
/// address (`ansible_host`, else its name) and whether it is this machine
/// (`ansible_connection=local`).
pub fn hosts_in(text: &str, group: &str) -> Vec<EdgeHost> {
    let mut out = Vec::new();
    let mut here = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            here = rest.split(']').next().unwrap_or_default().trim() == group;
            continue;
        }
        if !here {
            continue;
        }
        let body = line.split(" #").next().unwrap_or(line);
        let mut words = body.split_whitespace();
        let Some(name) = words.next() else { continue };
        let mut address = name.to_string();
        let mut local = false;
        for w in words {
            match w.split_once('=') {
                Some(("ansible_host", v)) => address = v.trim_matches(['"', '\'']).to_string(),
                Some(("ansible_connection", v)) => local = v.trim_matches(['"', '\'']) == "local",
                _ => {}
            }
        }
        if !out.iter().any(|h: &EdgeHost| h.name == name) {
            out.push(EdgeHost {
                name: name.to_string(),
                address,
                local,
            });
        }
    }
    out
}

/// Whether `h` is the machine this runs on: marked local, named by a loopback
/// address or `localhost`, or by one of `names` (this machine's own names).
pub fn is_here(h: &EdgeHost, names: &[String]) -> bool {
    let a = h.address.to_ascii_lowercase();
    h.local
        || a == "localhost"
        || a == "::1"
        || a.starts_with("127.")
        || names.iter().any(|n| n.eq_ignore_ascii_case(&a))
}

/// The edge variables naming a file the edge router mounts.
pub const MOUNTED: &[&str] = &[
    "goat_zenoh_edge_our_ca_path",
    "goat_zenoh_edge_partner_ca_path",
    "goat_zenoh_edge_outward_cert",
    "goat_zenoh_edge_outward_key",
    "goat_zenoh_edge_inward_cert",
    "goat_zenoh_edge_inward_key",
];

/// The refusal phase 12 gets, or `None` when the material reaches the edge:
/// the role installs it, no mounted file is one phase 8 wrote in `<work>/pki`,
/// or every edge host is this machine. `mounted` is each of [`MOUNTED`] the edge
/// group's vars set, with its value.
pub fn refusal(
    hosts: &[EdgeHost],
    names: &[String],
    installs: bool,
    mounted: &[(String, String)],
    edge_group: &str,
    work: &str,
) -> Option<String> {
    if installs {
        return None;
    }
    let pki = format!("{}/pki/", work.trim_end_matches('/'));
    let from_here: Vec<&(String, String)> = mounted
        .iter()
        .filter(|(_, v)| v.starts_with(&pki))
        .collect();
    if from_here.is_empty() {
        return None;
    }
    let away: Vec<&EdgeHost> = hosts.iter().filter(|h| !is_here(h, names)).collect();
    if away.is_empty() {
        return None;
    }
    let named = away
        .iter()
        .map(|h| format!("{} ({})", h.name, h.address))
        .collect::<Vec<_>>()
        .join(", ");
    let files = from_here
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "GB-272: phase 12 deploys the edge router of {edge_group} on {named}, which is not this machine, \
         and goat_zenoh_edge_install_certs is off for {edge_group}: the edge mounts {files}, which phase 8 \
         placed in {pki} on this machine and nothing copies to that host, so the edge would start with \
         nothing to mount and never reach the core. Set goat_zenoh_edge_install_certs: true for \
         {edge_group} (the role then installs the leaves, the partner's CA and our CA's certificate \
         from this machine, never a CA key), run goatbuilder federate allocate again when the \
         deployment is rendered, and resume from phase 12. If this machine is that host, mark it \
         ansible_connection=local in the inventory instead."
    ))
}

/// This machine's own names, as an inventory may name it: its host name, short
/// and full, lower-cased.
pub fn own_names() -> Vec<String> {
    let mut v = Vec::new();
    for f in ["/proc/sys/kernel/hostname", "/etc/hostname"] {
        if let Ok(s) = std::fs::read_to_string(f) {
            let s = s.trim().to_ascii_lowercase();
            if !s.is_empty() {
                if let Some((short, _)) = s.split_once('.') {
                    v.push(short.to_string());
                }
                v.push(s);
                break;
            }
        }
    }
    v
}

/// The check, read from the checkout: the peering inventory, and whether the
/// edge group's group_vars turn the role's install on.
pub fn check(repo: &Path, edge_group: &str, work: &str) -> Option<String> {
    let hosts = hosts_in(&super::plan::peering_inventory_text(repo), edge_group);
    let installs = super::peering_files::installs_certs(repo, edge_group);
    let mounted: Vec<(String, String)> = MOUNTED
        .iter()
        .filter_map(|k| {
            super::preflight::group_var(repo, edge_group, k).map(|v| (k.to_string(), v))
        })
        .collect();
    refusal(&hosts, &own_names(), installs, &mounted, edge_group, work)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INV: &str = "[peering_edges:children]\nxab_edge\n\n\
                       [xab_edge]\nxaa-edge-xab ansible_host=192.0.2.20 ansible_user=goat\n\n\
                       [xab_core]\nxaa-core-xab ansible_host=192.0.2.10 ansible_user=goat\n";

    /// A driver on a workspace: the edge is another machine, the role's install
    /// is off, so the material phase 8 wrote here would never reach it.
    #[test]
    fn a_workspace_driver_with_the_install_off_is_refused_before_phase_12() {
        let hosts = hosts_in(INV, "xab_edge");
        assert_eq!(
            hosts,
            vec![EdgeHost {
                name: "xaa-edge-xab".into(),
                address: "192.0.2.20".into(),
                local: false
            }]
        );
        let r = refusal(
            &hosts,
            &["workspace-1".into()],
            false,
            &work_paths(),
            "xab_edge",
            "/var/lib/goat-federation/xab",
        )
        .expect("refused");
        assert!(r.starts_with("GB-272: "), "{r}");
        for part in [
            "xaa-edge-xab (192.0.2.20)",
            "/var/lib/goat-federation/xab/pki/",
            "goat_zenoh_edge_our_ca_path, goat_zenoh_edge_outward_cert",
            "goat_zenoh_edge_install_certs: true",
            "never a CA key",
            "ansible_connection=local",
        ] {
            assert!(r.contains(part), "{part}: {r}");
        }
    }

    /// With the install on, the role copies the material from this machine:
    /// nothing to refuse.
    #[test]
    fn a_workspace_driver_with_the_install_on_proceeds() {
        let hosts = hosts_in(INV, "xab_edge");
        assert_eq!(
            refusal(
                &hosts,
                &[],
                true,
                &work_paths(),
                "xab_edge",
                "/var/lib/goat-federation/xab"
            ),
            None
        );
    }

    /// Files placed on the edge by hand, at paths outside <work>/pki, are the
    /// operator's: nothing to refuse.
    #[test]
    fn files_placed_by_hand_outside_the_working_directory_proceed() {
        let hosts = hosts_in(INV, "xab_edge");
        let hand = vec![(
            "goat_zenoh_edge_our_ca_path".to_string(),
            "/etc/goat/our-ca.pem".to_string(),
        )];
        assert_eq!(
            refusal(
                &hosts,
                &[],
                false,
                &hand,
                "xab_edge",
                "/var/lib/goat-federation/xab"
            ),
            None
        );
        assert_eq!(
            refusal(
                &hosts,
                &[],
                false,
                &[],
                "xab_edge",
                "/var/lib/goat-federation/xab"
            ),
            None
        );
    }

    /// The driver on the edge host: the paths are the same files.
    #[test]
    fn a_driver_on_the_edge_host_proceeds() {
        for inv in [
            "[xab_edge]\nxaa-edge-xab ansible_host=127.0.0.1 ansible_connection=local\n",
            "[xab_edge]\nxaa-edge-xab ansible_host=127.0.0.1\n",
            "[xab_edge]\nedge-1.example.org\n",
        ] {
            let hosts = hosts_in(inv, "xab_edge");
            assert_eq!(hosts.len(), 1, "{inv}");
            assert_eq!(
                refusal(
                    &hosts,
                    &["edge-1.example.org".into()],
                    false,
                    &work_paths(),
                    "xab_edge",
                    "/var/lib/goat-federation/xab"
                ),
                None,
                "{inv}"
            );
        }
    }

    /// Another group's hosts and comments are not the edge's.
    #[test]
    fn only_the_edge_groups_own_hosts_count() {
        let inv = "[xac_edge]\nother ansible_host=192.0.2.30\n# a note\n[xab_edge]\nme ansible_connection=local # here\n";
        let hosts = hosts_in(inv, "xab_edge");
        assert_eq!(
            hosts,
            vec![EdgeHost {
                name: "me".into(),
                address: "me".into(),
                local: true
            }]
        );
        assert_eq!(
            refusal(
                &hosts,
                &[],
                false,
                &work_paths(),
                "xab_edge",
                "/var/lib/goat-federation/xab"
            ),
            None
        );
    }

    fn work_paths() -> Vec<(String, String)> {
        vec![
            (
                "goat_zenoh_edge_our_ca_path".into(),
                "/var/lib/goat-federation/xab/pki/our-ca.pem".into(),
            ),
            (
                "goat_zenoh_edge_outward_cert".into(),
                "/var/lib/goat-federation/xab/pki/peering-outward.pem".into(),
            ),
        ]
    }
}
