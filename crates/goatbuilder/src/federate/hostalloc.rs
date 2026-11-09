//! The host-level allocation check of the federation plan
//! (docs/design/federation-plan.md, "Conflicts with other peerings").
//!
//! Allocation keeps the peerings of one environment apart on paper. This check
//! looks at hosts: it maps every declared peering's edge and core groups to the
//! machines the peering inventory names (by address, so two inventory names for
//! one machine are one host), and finds any value two peerings would bind on
//! the same machine: a WireGuard interface, a tunnel UDP port, an overlapping
//! link subnet, or a republisher metrics port. It catches what a per-environment
//! allocation cannot see: two environments, or two hand-named groups, landing
//! on one machine.
//!
//! The edge router's partner-facing port is not checked: `goat federate apply`
//! binds each edge router to its own link address, so the port is shared by
//! design, and the link subnets (which are checked) keep those addresses apart.
//!
//! Other peerings are read as they are declared (their group's vars, then the
//! role defaults, never this run's environment variables) and their link
//! subnets from their own agreements, when found.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::plan::{inventory_hosts, resolve, Readiness, Resource};
use crate::config;

/// The root of every peering's working directory unless `--work` says otherwise.
const DEFAULT_WORK_ROOT: &str = "/var/lib/goat-federation";

/// One value one peering binds on one machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    /// The machine, by address (`ansible_host`, else the inventory name).
    pub host: String,
    pub peering: String,
    pub resource: String,
    pub value: String,
}

/// Two or more peerings binding the same value (or overlapping subnets) on
/// one machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub host: String,
    pub resource: String,
    /// Each peering with the value it binds, sorted.
    pub claims: Vec<(String, String)>,
}

/// (resource kind in the plan, on the core host, role, role variable).
const CHECKED: &[(&str, bool, &str, &str)] = &[
    (
        "wireguard interface",
        false,
        "goat-peering-wg",
        "peering_wg_interface",
    ),
    (
        "udp port (tunnel)",
        false,
        "goat-peering-wg",
        "peering_wg_listen_port",
    ),
    (
        "tcp port (republisher metrics)",
        true,
        "goat-fed-republisher",
        "goat_fed_republisher_metrics_port",
    ),
];

fn parse_v4(s: &str) -> Option<(u32, u32)> {
    let (addr, len) = s.split_once('/')?;
    let len: u32 = len.parse().ok().filter(|l| *l <= 32)?;
    let mut n = 0u32;
    let parts: Vec<&str> = addr.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    for p in parts {
        n = (n << 8) | p.parse::<u8>().ok()? as u32;
    }
    let mask = if len == 0 { 0 } else { u32::MAX << (32 - len) };
    Some((n & mask, (n & mask) | !mask))
}

/// Whether two IPv4 subnets share an address. Anything unparsable compares
/// as text, so two identical strings still conflict.
pub fn overlaps(a: &str, b: &str) -> bool {
    match (parse_v4(a), parse_v4(b)) {
        (Some((a0, a1)), Some((b0, b1))) => a0 <= b1 && b0 <= a1,
        _ => a == b,
    }
}

/// The link subnet a peering's agreement fixed, from its working directory.
fn link_subnet(work: &Path) -> Option<String> {
    let text = std::fs::read_to_string(work.join("agreement.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v["transport"]["link_subnet"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Where another peering's working directory is: beside ours when ours sits
/// under a common folder and it is there, else the default root.
fn work_of_other(our_work: &Path, with: &str) -> std::path::PathBuf {
    let beside = our_work.parent().map(|p| p.join(with));
    match beside {
        Some(b) if b.join("agreement.json").is_file() => b,
        _ => Path::new(DEFAULT_WORK_ROOT).join(with),
    }
}

/// Every claim on every machine: ours from the plan's resolved resources and
/// our agreement, every other declared peering's from its declaration.
pub fn claims(
    repo: &Path,
    peerings: &[config::Peering],
    ours: &config::Peering,
    our_edge_group: &str,
    our_work: &Path,
    our_resources: &[Resource],
) -> Vec<Claim> {
    let mut out = Vec::new();
    for p in peerings {
        let mine = p.with == ours.with && p.environment == ours.environment;
        let edge_group = if mine {
            our_edge_group.to_string()
        } else {
            format!("{}_edge", p.with)
        };
        let core_group = format!("{}_core", p.with);
        let edge_hosts = inventory_hosts(repo, &edge_group);
        let core_hosts = inventory_hosts(repo, &core_group);
        for (kind, on_core, role, key) in CHECKED {
            let value = if mine {
                our_resources
                    .iter()
                    .find(|r| r.kind == *kind)
                    .and_then(|r| r.name.clone())
            } else {
                let g = if *on_core {
                    core_group.as_str()
                } else {
                    edge_group.as_str()
                };
                let parent = if *on_core {
                    "peering_cores"
                } else {
                    "peering_edges"
                };
                resolve(repo, kind, None, &[g, parent, "all"], role, key).value
            };
            let Some(value) = value else { continue };
            for (_, addr) in if *on_core { &core_hosts } else { &edge_hosts } {
                out.push(Claim {
                    host: addr.clone(),
                    peering: p.with.clone(),
                    resource: kind.to_string(),
                    value: value.clone(),
                });
            }
        }
        let work = if mine {
            our_work.to_path_buf()
        } else {
            work_of_other(our_work, &p.with)
        };
        if let Some(subnet) = link_subnet(&work) {
            for (_, addr) in &edge_hosts {
                out.push(Claim {
                    host: addr.clone(),
                    peering: p.with.clone(),
                    resource: "link subnet".into(),
                    value: subnet.clone(),
                });
            }
        }
    }
    out
}

/// The values two or more peerings would bind on one machine. A peering never
/// conflicts with itself (the same peering declared on two environments that
/// share a machine still binds one interface).
pub fn conflicts(claims: &[Claim]) -> Vec<Conflict> {
    let mut by: BTreeMap<(String, String), Vec<&Claim>> = BTreeMap::new();
    for c in claims {
        by.entry((c.host.clone(), c.resource.clone()))
            .or_default()
            .push(c);
    }
    let mut out = Vec::new();
    for ((host, resource), cs) in by {
        let mut hit: Vec<(String, String)> = Vec::new();
        for (i, a) in cs.iter().enumerate() {
            for b in &cs[i + 1..] {
                let same = if resource == "link subnet" {
                    overlaps(&a.value, &b.value)
                } else {
                    a.value == b.value
                };
                if same && a.peering != b.peering {
                    for c in [a, b] {
                        let pair = (c.peering.clone(), c.value.clone());
                        if !hit.contains(&pair) {
                            hit.push(pair);
                        }
                    }
                }
            }
        }
        if !hit.is_empty() {
            hit.sort();
            out.push(Conflict {
                host,
                resource,
                claims: hit,
            });
        }
    }
    out
}

/// The readiness row for the conflicts that involve this peering: FAIL with
/// GB-203 naming each, PASS when none, SKIP when the inventory names no host
/// for this peering (nothing was compared, which is not a pass).
pub fn readiness(with: &str, claims: &[Claim], found: &[Conflict]) -> Readiness {
    let ours: Vec<&Conflict> = found
        .iter()
        .filter(|c| c.claims.iter().any(|(p, _)| p == with))
        .collect();
    let row = |outcome: &str, code: Option<&str>, detail: String| Readiness {
        check: "host-allocation".into(),
        outcome: outcome.into(),
        code: code.map(String::from),
        detail,
    };
    if !claims.iter().any(|c| c.peering == with) {
        return row(
            "SKIP",
            None,
            "the peering inventory names no host for this peering, so no host was compared".into(),
        );
    }
    if ours.is_empty() {
        return row("PASS", None, "no other declared peering binds the same interface, tunnel port, link subnet or metrics port on these hosts".into());
    }
    let each: Vec<String> = ours
        .iter()
        .map(|c| {
            let who: Vec<String> = c.claims.iter().map(|(p, v)| format!("{p} {v}")).collect();
            format!("{} on {}: {}", c.resource, c.host, who.join(" and "))
        })
        .collect();
    row("FAIL", Some("GB-203"), each.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(with: &str, env: &str) -> config::Peering {
        config::Peering {
            with: with.into(),
            environment: env.into(),
            site: "alpha".into(),
            edge_class: "edge".into(),
        }
    }

    fn res(kind: &str, v: &str) -> Resource {
        Resource {
            kind: kind.into(),
            name: Some(v.into()),
            on: "x".into(),
            source: "test".into(),
        }
    }

    /// A repository with one machine behind two inventory names: the edge of
    /// `velmara` on one environment, the edge of `meadow` on another.
    fn shared_host_repo() -> tempfile::TempDir {
        let r = tempfile::tempdir().unwrap();
        let inv = r.path().join("ops/ansible/inventory");
        std::fs::create_dir_all(&inv).unwrap();
        std::fs::write(
            inv.join("peering-edges.ini"),
            "[velmara_edge]\nedge-v ansible_host=192.0.2.10\n[meadow_edge]\nedge-m ansible_host=192.0.2.10\n[velmara_core]\ncore-v ansible_host=192.0.2.20\n[meadow_core]\ncore-m ansible_host=192.0.2.21\n",
        )
        .unwrap();
        let gv = r.path().join("ops/ansible/group_vars");
        std::fs::create_dir_all(gv.join("meadow_edge")).unwrap();
        std::fs::write(
            gv.join("meadow_edge/peering.yml"),
            "peering_wg_interface: wg-fed0\npeering_wg_listen_port: 51825\n",
        )
        .unwrap();
        r
    }

    #[test]
    fn subnets_overlap_by_address_range() {
        assert!(overlaps("169.254.10.0/31", "169.254.10.1/32"));
        assert!(overlaps("169.254.10.0/30", "169.254.10.2/31"));
        assert!(!overlaps("169.254.10.0/31", "169.254.10.2/31"));
        assert!(overlaps("weird", "weird") && !overlaps("weird", "other"));
    }

    /// Two peerings of different environments on one machine (two inventory
    /// names, one address) that both bind wg-fed0: a conflict on that host,
    /// FAIL GB-203 for each of them. Different tunnel ports do not conflict.
    #[test]
    fn two_peerings_on_one_machine_with_one_interface_conflict() {
        let r = shared_host_repo();
        let w = tempfile::tempdir().unwrap();
        let ours = p("velmara", "zagros");
        let all = vec![ours.clone(), p("meadow", "pasture")];
        let mine = vec![
            res("wireguard interface", "wg-fed0"),
            res("udp port (tunnel)", "51823"),
            res("tcp port (republisher metrics)", "9310"),
        ];
        let cs = claims(
            r.path(),
            &all,
            &ours,
            "velmara_edge",
            &w.path().join("velmara"),
            &mine,
        );
        let found = conflicts(&cs);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].host, "192.0.2.10");
        assert_eq!(found[0].resource, "wireguard interface");
        let row = readiness("velmara", &cs, &found);
        assert_eq!(
            (row.outcome.as_str(), row.code.as_deref()),
            ("FAIL", Some("GB-203"))
        );
        assert!(
            row.detail.contains("meadow wg-fed0 and velmara wg-fed0"),
            "{}",
            row.detail
        );
        // The metrics port is on two different core machines: no conflict.
        assert!(found
            .iter()
            .all(|c| c.resource != "tcp port (republisher metrics)"));
    }

    /// Link subnets come from each peering's own agreement: overlapping ones
    /// on one machine conflict, and another peering's agreement is found
    /// beside ours.
    #[test]
    fn overlapping_link_subnets_on_one_machine_conflict() {
        let r = shared_host_repo();
        let root = tempfile::tempdir().unwrap();
        for (who, subnet) in [
            ("velmara", "169.254.10.0/31"),
            ("meadow", "169.254.10.0/31"),
        ] {
            std::fs::create_dir_all(root.path().join(who)).unwrap();
            std::fs::write(
                root.path().join(who).join("agreement.json"),
                format!(r#"{{"transport":{{"link_subnet":"{subnet}"}}}}"#),
            )
            .unwrap();
        }
        let ours = p("velmara", "zagros");
        let all = vec![ours.clone(), p("meadow", "pasture")];
        let mine = vec![
            res("wireguard interface", "wg-fed1"),
            res("udp port (tunnel)", "51823"),
        ];
        let found = conflicts(&claims(
            r.path(),
            &all,
            &ours,
            "velmara_edge",
            &root.path().join("velmara"),
            &mine,
        ));
        assert_eq!(
            found
                .iter()
                .map(|c| c.resource.as_str())
                .collect::<Vec<_>>(),
            ["link subnet"]
        );
    }

    /// With distinct values everywhere there is nothing to report, and a
    /// peering with no inventory host is SKIP, never PASS.
    #[test]
    fn distinct_values_pass_and_no_host_is_skip() {
        let r = shared_host_repo();
        let w = tempfile::tempdir().unwrap();
        let ours = p("velmara", "zagros");
        let all = vec![ours.clone(), p("meadow", "pasture")];
        let mine = vec![
            res("wireguard interface", "wg-fed1"),
            res("udp port (tunnel)", "51823"),
        ];
        let cs = claims(
            r.path(),
            &all,
            &ours,
            "velmara_edge",
            &w.path().join("velmara"),
            &mine,
        );
        assert_eq!(readiness("velmara", &cs, &conflicts(&cs)).outcome, "PASS");
        let cs = claims(
            r.path(),
            &all,
            &ours,
            "nowhere_edge",
            &w.path().join("velmara"),
            &[],
        );
        assert_eq!(readiness("velmara", &cs, &conflicts(&cs)).outcome, "SKIP");
    }
}
