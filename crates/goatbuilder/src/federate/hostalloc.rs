//! The host-level allocation check of the federation plan
//! (docs/design/federation-plan.md, "Conflicts with other peerings").
//!
//! Allocation keeps the peerings of one environment apart on paper. This check
//! looks at machines: it maps every declared peering's edge and core groups to
//! the machines the peering inventory names (by address, so two inventory
//! names for one address are one machine), and finds what two peerings would
//! share on one machine that they must not:
//!
//! - one inventory name in two peerings' groups: Ansible merges both groups'
//!   variables onto that host, so one peering's values overwrite the other's;
//! - a WireGuard interface or tunnel UDP port (WireGuard peerings only; an
//!   ipsec-fips peering binds neither);
//! - behind a forwarder, the carrier interface or carrier UDP port;
//! - overlapping link subnets;
//! - a republisher metrics port on the core.
//!
//! It catches what a per-environment allocation cannot see: two environments,
//! or two hand-named groups, landing on one machine.
//!
//! The edge router's partner-facing port is not compared: `goat federate
//! apply` binds each edge router to its own link address (an agreement without
//! a transport stops at phase 11), so the port is shared by design, and the
//! link subnets keep those addresses apart.
//!
//! Other peerings are read as they are declared (their group's vars, then the
//! role defaults, never this run's environment variables), and their transport
//! and link subnet from their own agreements when found. A value that cannot be
//! resolved on a machine we share is "not verified", never a pass.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::plan::{inventory_hosts, resolve, Readiness, Resource};
use crate::config;

/// The root of every peering's working directory unless `--work` says otherwise.
const DEFAULT_WORK_ROOT: &str = "/var/lib/goat-federation";

/// One value one peering binds on one machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    /// The machine, by address.
    pub host: String,
    /// `<with>/<environment>`: the same partner on two environments is two peerings.
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

/// What the check compared: the claims, what it could not resolve on a
/// machine this peering shares, and whether this peering has any host at all.
#[derive(Debug, Clone, Default)]
pub struct Comparison {
    pub claims: Vec<Claim>,
    pub unverified: Vec<String>,
    pub our_hosts: bool,
}

/// The label of a peering in claims and conflicts.
pub fn label(p: &config::Peering) -> String {
    format!("{}/{}", p.with, p.environment)
}

/// (resource kind, on the core host, role, role variable, WireGuard only,
/// behind a forwarder only).
const CHECKED: &[(&str, bool, &str, &str, bool, bool)] = &[
    (
        "wireguard interface",
        false,
        "goat-peering-wg",
        "peering_wg_interface",
        true,
        false,
    ),
    (
        "udp port (tunnel)",
        false,
        "goat-peering-wg",
        "peering_wg_listen_port",
        true,
        false,
    ),
    (
        "carrier interface",
        false,
        "goat-peering-wg",
        "peering_wg_carrier_interface",
        true,
        true,
    ),
    (
        "udp port (carrier)",
        false,
        "goat-peering-wg",
        "peering_wg_carrier_listen_port",
        true,
        true,
    ),
    (
        "tcp port (republisher metrics)",
        true,
        "goat-fed-republisher",
        "goat_fed_republisher_metrics_port",
        false,
        false,
    ),
];

fn parse_net(s: &str) -> Option<(u128, u128, bool)> {
    let (addr, len) = s.split_once('/')?;
    let len: u32 = len.parse().ok()?;
    let (n, bits, v6) = match addr.parse::<std::net::IpAddr>().ok()? {
        std::net::IpAddr::V4(a) => (u32::from(a) as u128, 32, false),
        std::net::IpAddr::V6(a) => (u128::from(a), 128, true),
    };
    if len > bits {
        return None;
    }
    let host_bits = bits - len;
    let host_mask = if host_bits == 128 {
        u128::MAX
    } else {
        (1u128 << host_bits) - 1
    };
    let lo = n & !host_mask;
    Some((lo, lo | host_mask, v6))
}

/// Whether two subnets (IPv4 or IPv6) share an address. Anything unparsable
/// compares as text, so two identical strings still conflict.
pub fn overlaps(a: &str, b: &str) -> bool {
    match (parse_net(a), parse_net(b)) {
        (Some((a0, a1, a6)), Some((b0, b1, b6))) => a6 == b6 && a0 <= b1 && b0 <= a1,
        _ => a == b,
    }
}

/// A peering's agreement facts this check reads: (transport kind, link subnet).
fn agreement_of(work: &Path) -> Option<(String, Option<String>)> {
    let text = std::fs::read_to_string(work.join("agreement.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let t = &v["transport"];
    Some((
        t["kind"].as_str().unwrap_or_default().to_string(),
        t["link_subnet"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from),
    ))
}

/// Where another peering's working directory is: beside ours when it is
/// there, else the default root. Never our own directory.
fn work_of_other(our_work: &Path, with: &str) -> Option<PathBuf> {
    let beside = our_work.parent().map(|p| p.join(with));
    let candidates = [beside, Some(Path::new(DEFAULT_WORK_ROOT).join(with))];
    candidates
        .into_iter()
        .flatten()
        .find(|c| c.as_path() != our_work && c.join("agreement.json").is_file())
}

struct Side {
    label: String,
    edge: Vec<(String, String)>,
    core: Vec<(String, String)>,
    /// Kind -> value, `None` when it does not resolve.
    values: BTreeMap<&'static str, Option<String>>,
    /// `None` before the agreement is composed (or found).
    agreement: Option<(String, Option<String>)>,
    via_forwarder: bool,
}

impl Side {
    /// Whether this peering binds a WireGuard value: its agreement chose
    /// wireguard, or it has no agreement yet (phase 1 assumes it may).
    fn wireguard(&self) -> bool {
        self.agreement
            .as_ref()
            .map(|(k, _)| k == "wireguard" || k.is_empty())
            .unwrap_or(true)
    }

    fn applies(&self, wg_only: bool, forwarder_only: bool) -> bool {
        (!wg_only || self.wireguard()) && (!forwarder_only || self.via_forwarder)
    }
}

/// Every claim on every machine, ours from the plan's resolved resources (and
/// our edge group for what the plan does not list) and every other declared
/// peering's from its declaration.
pub fn compare(
    repo: &Path,
    peerings: &[config::Peering],
    ours: &config::Peering,
    our_edge_group: &str,
    our_work: &Path,
    our_resources: &[Resource],
) -> Comparison {
    let our_label = label(ours);
    let mut sides = Vec::new();
    for p in peerings {
        let mine = label(p) == our_label;
        let edge_group = if mine {
            our_edge_group.to_string()
        } else {
            format!("{}_edge", p.with)
        };
        let core_group = format!("{}_core", p.with);
        let groups = |g: &str, core: bool| {
            [
                g.to_string(),
                if core {
                    "peering_cores"
                } else {
                    "peering_edges"
                }
                .to_string(),
                "all".to_string(),
            ]
        };
        let mut values = BTreeMap::new();
        for (kind, on_core, role, key, _, _) in CHECKED {
            let g = groups(if *on_core { &core_group } else { &edge_group }, *on_core);
            let gs: Vec<&str> = g.iter().map(String::as_str).collect();
            let listed = our_resources.iter().find(|r| r.kind == *kind);
            let v = match (mine, listed) {
                (true, Some(r)) => r.name.clone(),
                _ => resolve(repo, kind, None, &gs, role, key).value,
            };
            values.insert(*kind, v);
        }
        let eg = groups(&edge_group, false);
        let egs: Vec<&str> = eg.iter().map(String::as_str).collect();
        let via_forwarder = resolve(
            repo,
            "via forwarder",
            None,
            &egs,
            "goat-peering-wg",
            "peering_wg_via_forwarder",
        )
        .value
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
        let work = if mine {
            Some(our_work.to_path_buf())
        } else {
            work_of_other(our_work, &p.with)
        };
        sides.push(Side {
            label: label(p),
            edge: inventory_hosts(repo, &edge_group),
            core: inventory_hosts(repo, &core_group),
            values,
            agreement: work.as_deref().and_then(agreement_of),
            via_forwarder,
        });
    }
    let Some(us) = sides.iter().find(|s| s.label == our_label) else {
        return Comparison::default();
    };
    let our_edge: Vec<&String> = us.edge.iter().map(|(_, a)| a).collect();
    let our_core: Vec<&String> = us.core.iter().map(|(_, a)| a).collect();
    let mut cmp = Comparison {
        our_hosts: !us.edge.is_empty() || !us.core.is_empty(),
        ..Default::default()
    };
    for s in &sides {
        let theirs = s.label != our_label;
        let shares_edge = s.edge.iter().any(|(_, a)| our_edge.contains(&a));
        let shares_core = s.core.iter().any(|(_, a)| our_core.contains(&a));
        for (name, addr) in s.edge.iter().chain(s.core.iter()) {
            cmp.claims.push(Claim {
                host: addr.clone(),
                peering: s.label.clone(),
                resource: "inventory name".into(),
                value: name.clone(),
            });
        }
        for (kind, on_core, _, _, wg_only, fwd_only) in CHECKED {
            if !s.applies(*wg_only, *fwd_only) {
                continue;
            }
            let hosts = if *on_core { &s.core } else { &s.edge };
            match s.values.get(kind).cloned().flatten() {
                Some(v) => {
                    for (_, addr) in hosts {
                        cmp.claims.push(Claim {
                            host: addr.clone(),
                            peering: s.label.clone(),
                            resource: kind.to_string(),
                            value: v.clone(),
                        });
                    }
                }
                None if (!theirs && !hosts.is_empty())
                    || (theirs && if *on_core { shares_core } else { shares_edge }) =>
                {
                    cmp.unverified.push(format!(
                        "{kind} of {} does not resolve (unset or templated)",
                        s.label
                    ));
                }
                None => {}
            }
        }
        match s.agreement.as_ref().map(|(_, subnet)| subnet.clone()) {
            Some(Some(subnet)) => {
                for (_, addr) in &s.edge {
                    cmp.claims.push(Claim {
                        host: addr.clone(),
                        peering: s.label.clone(),
                        resource: "link subnet".into(),
                        value: subnet.clone(),
                    });
                }
            }
            // Ours before phase 6 has nothing to compare yet; another
            // peering on a machine we share, with no agreement found, is
            // not verified.
            _ if theirs && shares_edge && us.agreement.is_some() => {
                cmp.unverified.push(format!("link subnet of {}: no agreement found beside our working directory or under {DEFAULT_WORK_ROOT}", s.label));
            }
            _ => {}
        }
    }
    cmp
}

/// The values two or more peerings would bind on one machine.
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

/// The conflicts this peering is part of. Only these go into its plan, so a
/// conflict between two other peerings never changes this plan's hash.
pub fn involving(found: Vec<Conflict>, our_label: &str) -> Vec<Conflict> {
    found
        .into_iter()
        .filter(|c| c.claims.iter().any(|(p, _)| p == our_label))
        .collect()
}

/// The readiness row: FAIL with GB-203 naming each conflict of ours; SKIP
/// when this peering has no inventory host or a value on a shared machine does
/// not resolve (nothing, or not everything, was compared); else PASS.
pub fn readiness(cmp: &Comparison, ours: &[Conflict]) -> Readiness {
    let row = |outcome: &str, code: Option<&str>, detail: String| Readiness {
        check: "host-allocation".into(),
        outcome: outcome.into(),
        code: code.map(String::from),
        detail,
    };
    if !ours.is_empty() {
        let each: Vec<String> = ours
            .iter()
            .map(|c| {
                let who: Vec<String> = c.claims.iter().map(|(p, v)| format!("{p} {v}")).collect();
                format!("{} on {}: {}", c.resource, c.host, who.join(" and "))
            })
            .collect();
        return row("FAIL", Some("GB-203"), each.join("; "));
    }
    if !cmp.our_hosts {
        return row(
            "SKIP",
            None,
            "the peering inventory names no host for this peering, so no machine was compared"
                .into(),
        );
    }
    if !cmp.unverified.is_empty() {
        return row(
            "SKIP",
            None,
            format!("not verified: {}", cmp.unverified.join("; ")),
        );
    }
    row("PASS", None, "no other declared peering shares an inventory name, interface, tunnel or carrier port, link subnet or metrics port with this peering on its machines".into())
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

    fn mine() -> Vec<Resource> {
        vec![
            res("wireguard interface", "wg-fed1"),
            res("udp port (tunnel)", "51823"),
            res("tcp port (republisher metrics)", "9311"),
        ]
    }

    /// A repository whose inventory is `inv` and whose meadow edge group sets
    /// wg-fed0 on 51825 and metrics 9310.
    fn repo(inv: &str) -> tempfile::TempDir {
        let r = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(r.path().join("ops/ansible/inventory")).unwrap();
        std::fs::write(
            r.path().join("ops/ansible/inventory/peering-edges.ini"),
            inv,
        )
        .unwrap();
        let gv = r.path().join("ops/ansible/group_vars");
        std::fs::create_dir_all(gv.join("meadow_edge")).unwrap();
        std::fs::write(
            gv.join("meadow_edge/peering.yml"),
            "peering_wg_interface: wg-fed0\npeering_wg_listen_port: 51825\n",
        )
        .unwrap();
        std::fs::create_dir_all(gv.join("meadow_core")).unwrap();
        std::fs::write(
            gv.join("meadow_core/peering.yml"),
            "goat_fed_republisher_metrics_port: 9310\n",
        )
        .unwrap();
        r
    }

    const SHARED: &str = "[velmara_edge]\nedge-v ansible_host=192.0.2.10\n[meadow_edge]\nedge-m ansible_host=\"192.0.2.10\"  # same machine\n[velmara_core]\ncore-v ansible_host=192.0.2.20\n[meadow_core]\ncore-m ansible_host=192.0.2.21\n";

    fn run(r: &Path, work: &Path, ours_res: &[Resource]) -> (Comparison, Vec<Conflict>) {
        let ours = p("velmara", "zagros");
        let all = vec![ours.clone(), p("meadow", "pasture")];
        let cmp = compare(r, &all, &ours, "velmara_edge", work, ours_res);
        let found = involving(conflicts(&cmp.claims), &label(&ours));
        (cmp, found)
    }

    #[test]
    fn subnets_overlap_by_address_range() {
        assert!(overlaps("169.254.10.0/31", "169.254.10.1/32"));
        assert!(overlaps("169.254.10.0/30", "169.254.10.2/31"));
        assert!(!overlaps("169.254.10.0/31", "169.254.10.2/31"));
        assert!(overlaps("fd00::/127", "fd00::1/128"));
        assert!(!overlaps("fd00::/127", "fd00::2/127"));
        assert!(
            !overlaps("0.0.0.0/0", "::/0"),
            "IPv4 and IPv6 never overlap"
        );
        assert!(overlaps("weird", "weird") && !overlaps("weird", "other"));
    }

    /// Two peerings of different environments on one machine (two inventory
    /// names, one address, one of them quoted) that both bind wg-fed0.
    #[test]
    fn one_interface_on_one_machine_conflicts() {
        let r = repo(SHARED);
        let w = tempfile::tempdir().unwrap();
        let mut ours = mine();
        ours[0] = res("wireguard interface", "wg-fed0");
        let (cmp, found) = run(r.path(), &w.path().join("velmara"), &ours);
        assert_eq!(
            found
                .iter()
                .map(|c| (c.host.as_str(), c.resource.as_str()))
                .collect::<Vec<_>>(),
            [("192.0.2.10", "wireguard interface")]
        );
        let row = readiness(&cmp, &found);
        assert_eq!(
            (row.outcome.as_str(), row.code.as_deref()),
            ("FAIL", Some("GB-203"))
        );
        assert!(
            row.detail
                .contains("meadow/pasture wg-fed0 and velmara/zagros wg-fed0"),
            "{}",
            row.detail
        );
    }

    /// One inventory name in both peerings' groups: Ansible merges their vars
    /// onto it, so it conflicts even with different values. A bare listing
    /// takes the address the host's own line gave it.
    #[test]
    fn one_inventory_name_in_two_peerings_groups_conflicts() {
        let r = repo("[velmara_edge]\nedge-a ansible_host=192.0.2.10\n[meadow_edge]\nedge-a\n[velmara_core]\ncore-v ansible_host=192.0.2.20\n");
        let w = tempfile::tempdir().unwrap();
        let (cmp, found) = run(r.path(), &w.path().join("velmara"), &mine());
        let kinds: Vec<&str> = found.iter().map(|c| c.resource.as_str()).collect();
        assert_eq!(kinds, ["inventory name"], "{found:?}");
        assert_eq!(found[0].host, "192.0.2.10");
        assert_eq!(readiness(&cmp, &found).outcome, "FAIL");
    }

    /// The same partner on two environments is two peerings: a shared machine
    /// with both on the role default interface conflicts.
    #[test]
    fn the_same_partner_on_two_environments_is_two_peerings() {
        let r = repo("[zagros_velmara_edge]\nedge-z ansible_host=192.0.2.10\n[velmara_edge]\nedge-p ansible_host=192.0.2.10\n");
        let w = tempfile::tempdir().unwrap();
        let ours = p("velmara", "zagros");
        let all = vec![ours.clone(), p("velmara", "pasture")];
        let mut mine = mine();
        mine[0] = res("wireguard interface", "wg-fed0");
        mine[1] = res("udp port (tunnel)", "51820");
        let roles = r.path().join("ops/ansible/roles/goat-peering-wg/defaults");
        std::fs::create_dir_all(&roles).unwrap();
        std::fs::write(
            roles.join("main.yml"),
            "peering_wg_interface: wg-fed0\npeering_wg_listen_port: 51820\n",
        )
        .unwrap();
        let cmp = compare(
            r.path(),
            &all,
            &ours,
            "zagros_velmara_edge",
            &w.path().join("velmara"),
            &mine,
        );
        let found = involving(conflicts(&cmp.claims), &label(&ours));
        let kinds: Vec<&str> = found.iter().map(|c| c.resource.as_str()).collect();
        assert_eq!(
            kinds,
            ["udp port (tunnel)", "wireguard interface"],
            "{found:?}"
        );
    }

    /// An ipsec-fips peering binds no WireGuard value, so it never conflicts
    /// with a WireGuard peering on the role defaults.
    #[test]
    fn an_ipsec_peering_claims_no_wireguard_value() {
        let r = repo(SHARED);
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("meadow")).unwrap();
        std::fs::write(
            root.path().join("meadow/agreement.json"),
            r#"{"transport":{"kind":"ipsec-fips","link_subnet":"169.254.10.2/31"}}"#,
        )
        .unwrap();
        let mut ours = mine();
        ours[0] = res("wireguard interface", "wg-fed0");
        let (cmp, found) = run(r.path(), &root.path().join("velmara"), &ours);
        assert!(found.is_empty(), "{found:?}");
        assert_eq!(readiness(&cmp, &found).outcome, "PASS");
    }

    /// Link subnets come from each peering's own agreement: overlapping ones
    /// on one machine conflict. Another peering on a shared machine whose
    /// agreement is not found is not verified once ours exists.
    #[test]
    fn link_subnets_conflict_and_a_missing_one_is_not_verified() {
        let r = repo(SHARED);
        let root = tempfile::tempdir().unwrap();
        let write = |who: &str, subnet: &str| {
            std::fs::create_dir_all(root.path().join(who)).unwrap();
            std::fs::write(
                root.path().join(who).join("agreement.json"),
                format!(r#"{{"transport":{{"kind":"wireguard","link_subnet":"{subnet}"}}}}"#),
            )
            .unwrap();
        };
        write("velmara", "169.254.10.0/31");
        let (cmp, found) = run(r.path(), &root.path().join("velmara"), &mine());
        assert!(found.is_empty());
        let row = readiness(&cmp, &found);
        assert_eq!(row.outcome, "SKIP", "{}", row.detail);
        assert!(
            row.detail.contains("link subnet of meadow/pasture"),
            "{}",
            row.detail
        );
        write("meadow", "169.254.10.1/32");
        let (_, found) = run(r.path(), &root.path().join("velmara"), &mine());
        assert_eq!(
            found
                .iter()
                .map(|c| c.resource.as_str())
                .collect::<Vec<_>>(),
            ["link subnet"]
        );
    }

    /// Behind a forwarder, the carrier values are compared too.
    #[test]
    fn carrier_values_are_compared_behind_a_forwarder() {
        let r = repo(SHARED);
        let gv = r.path().join("ops/ansible/group_vars");
        std::fs::write(gv.join("meadow_edge/fwd.yml"), "peering_wg_via_forwarder: true\npeering_wg_carrier_interface: wg-carry0\npeering_wg_carrier_listen_port: 51822\n").unwrap();
        std::fs::create_dir_all(gv.join("velmara_edge")).unwrap();
        std::fs::write(gv.join("velmara_edge/fwd.yml"), "peering_wg_via_forwarder: true\npeering_wg_carrier_interface: wg-carry0\npeering_wg_carrier_listen_port: 51824\n").unwrap();
        let w = tempfile::tempdir().unwrap();
        let (_, found) = run(r.path(), &w.path().join("velmara"), &mine());
        assert_eq!(
            found
                .iter()
                .map(|c| c.resource.as_str())
                .collect::<Vec<_>>(),
            ["carrier interface"]
        );
    }

    /// Nothing compared, or a value of ours that does not resolve on our own
    /// host, is SKIP, never PASS; distinct values everywhere is PASS.
    #[test]
    fn pass_only_when_everything_was_compared() {
        let r = repo(SHARED);
        let w = tempfile::tempdir().unwrap();
        let (cmp, found) = run(r.path(), &w.path().join("velmara"), &mine());
        assert_eq!(readiness(&cmp, &found).outcome, "PASS");
        let mut unresolved = mine();
        unresolved[0].name = None;
        let (cmp, found) = run(r.path(), &w.path().join("velmara"), &unresolved);
        assert_eq!(readiness(&cmp, &found).outcome, "SKIP");
        let ours = p("velmara", "zagros");
        let r2 = repo("[meadow_edge]\nedge-m ansible_host=192.0.2.10\n");
        let cmp = compare(
            r2.path(),
            std::slice::from_ref(&ours),
            &ours,
            "velmara_edge",
            &w.path().join("velmara"),
            &mine(),
        );
        assert_eq!(readiness(&cmp, &[]).outcome, "SKIP");
    }

    /// A conflict between two other peerings is not this peering's: it stays
    /// out of this plan.
    #[test]
    fn another_pairs_conflict_is_not_ours() {
        let r = repo("[meadow_edge]\nedge-m ansible_host=192.0.2.30\n[pasture_edge]\nedge-p ansible_host=192.0.2.30\n[velmara_edge]\nedge-v ansible_host=192.0.2.10\n");
        let gv = r.path().join("ops/ansible/group_vars");
        std::fs::create_dir_all(gv.join("pasture_edge")).unwrap();
        std::fs::write(
            gv.join("pasture_edge/p.yml"),
            "peering_wg_interface: wg-fed0\npeering_wg_listen_port: 51825\n",
        )
        .unwrap();
        let w = tempfile::tempdir().unwrap();
        let ours = p("velmara", "zagros");
        let all = vec![
            ours.clone(),
            p("meadow", "pasture"),
            p("pasture", "pasture"),
        ];
        let cmp = compare(
            r.path(),
            &all,
            &ours,
            "velmara_edge",
            &w.path().join("velmara"),
            &mine(),
        );
        assert!(
            !conflicts(&cmp.claims).is_empty(),
            "meadow and pasture do conflict"
        );
        assert!(involving(conflicts(&cmp.claims), &label(&ours)).is_empty());
    }

    /// `[group:children]` is expanded and `[group:vars]` is not read as hosts;
    /// `ansible_host` can come from host_vars.
    #[test]
    fn the_inventory_reads_children_and_host_vars() {
        let r = repo("[velmara_edge:children]\nvelmara_edge_a\n[velmara_edge_a]  # first\nedge-a\n[velmara_edge:vars]\nansible_user=ops\n");
        std::fs::create_dir_all(r.path().join("ops/ansible/host_vars")).unwrap();
        std::fs::write(
            r.path().join("ops/ansible/host_vars/edge-a.yml"),
            "ansible_host: 192.0.2.44\n",
        )
        .unwrap();
        assert_eq!(
            inventory_hosts(r.path(), "velmara_edge"),
            [("edge-a".to_string(), "192.0.2.44".to_string())]
        );
    }
}
