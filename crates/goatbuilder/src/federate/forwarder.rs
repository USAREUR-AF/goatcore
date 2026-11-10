//! The peering forwarder, applied by the driver from the same `peerings:`
//! declaration (ADR 1218, docs/design/peering-forwarder.md).
//!
//! A peering whose declaration carries `forwarder: { endpoint: <host> }` is
//! relayed by a forwarder: the partner dials `<endpoint>:<public_port>`, the
//! forwarder relays that port down a carrier the edge dials, and the edge's
//! WireGuard listens on `wg_listen_port`. Allocation (federate/alloc.rs) gives
//! the peering its public port, carrier interface, carrier ports and carrier
//! subnet. This module renders:
//!
//! - `peering_forwarder_peerings` for the forwarder's group, one entry per
//!   peering it relays, from every such peering's values (a second peering's
//!   port mapping is a second entry: public port -> edge inner listen port);
//! - the edge's carrier variables, appended to the peering's edge values;
//! - the forwarder's inventory group, when the inventory is rendered;
//!
//! and splices the forwarder into phase 2 of `goatbuilder federate <with>`:
//! prepare the edge (it makes its carrier key), copy the carrier public keys to
//! the machine running the driver, apply the forwarder, copy again, prepare the
//! edge again (the carrier comes up). The two carrier public keys reach the
//! plays through `lookup('file', ...)` in the rendered group_vars, so no
//! re-render is needed between the steps.
//!
//! A peering without `forwarder:` is untouched: no file, no variable, phase 2
//! as before.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;

use super::alloc::{self, AllocContext, PeeringValues};
use super::peering_files::{header, q, Rendered};
use super::{Options, Phase};
use crate::config::{self, Config};

/// The forwarder group a declaration names, or the play's own group.
pub const DEFAULT_GROUP: &str = "peering_forwarders";
/// The file the edge's carrier public key is copied to, in the peering's cert_dir.
pub const EDGE_KEY_FILE: &str = "edge-carrier.pub";
/// The file the forwarder's carrier public key is copied to, in the peering's cert_dir.
pub const FORWARDER_KEY_FILE: &str = "forwarder-carrier.pub";

fn group_of(f: &config::PeeringForwarder) -> &str {
    f.group.as_deref().unwrap_or(DEFAULT_GROUP)
}

/// The two addresses of a carrier /31 or /30: (forwarder side, edge side), the
/// forwarder role's and the edge role's default order.
pub fn carrier_pair(subnet: &str) -> Option<(String, String, u32)> {
    let (net, size) = alloc::parse_subnet(subnet)?;
    let (a, b) = if size == 2 {
        (net, net + 1)
    } else {
        (net + 1, net + 2)
    };
    let ip = |n: u32| std::net::Ipv4Addr::from(n).to_string();
    Some((ip(a), ip(b), 32 - size.trailing_zeros()))
}

fn lookup(path: &str) -> String {
    format!("{{{{ lookup('ansible.builtin.file', '{path}', errors='ignore') | default('', true) | trim }}}}")
}

/// The edge's carrier variables, for the edge group's values file. Empty when
/// no forwarder relays this peering.
pub fn edge_vars(p: &config::Peering, v: &PeeringValues) -> String {
    let (Some(f), Some(ci), Some(cl), Some(cp), Some(cs)) = (
        &p.forwarder,
        &v.carrier_interface,
        v.carrier_listen_port,
        v.carrier_port,
        &v.carrier_subnet,
    ) else {
        return String::new();
    };
    let Some((fwd_ip, edge_ip, len)) = carrier_pair(cs) else {
        return String::new();
    };
    let mut s = format!(
        "\n# Behind the peering forwarder {}: the partner dials {}:{}, and this edge\n\
         # dials the forwarder's carrier port (docs/design/peering-forwarder.md).\n",
        f.endpoint,
        f.endpoint,
        v.public_port.unwrap_or_default()
    );
    s.push_str("peering_wg_via_forwarder: true\n");
    s.push_str(&format!("peering_wg_carrier_interface: {}\n", q(ci)));
    s.push_str(&format!("peering_wg_carrier_listen_port: {cl}\n"));
    s.push_str(&format!("peering_wg_carrier_local_ip: {}\n", q(&edge_ip)));
    s.push_str(&format!("peering_wg_carrier_remote_ip: {}\n", q(&fwd_ip)));
    s.push_str(&format!("peering_wg_carrier_prefix_len: {len}\n"));
    s.push_str(&format!(
        "peering_wg_forwarder_endpoint: {}\n",
        q(&format!("{}:{cp}", f.endpoint))
    ));
    s.push_str(
        "# Copied from the forwarder by peering-carrier-keys.yml in phase 2; empty until then.\n",
    );
    s.push_str(&format!(
        "peering_wg_forwarder_carrier_public_key: \"{}\"\n",
        lookup(&format!("{}/{FORWARDER_KEY_FILE}", v.cert_dir))
    ));
    s.push_str("# Where peering-carrier-keys.yml copies this edge's carrier public key.\n");
    s.push_str(&format!(
        "peering_carrier_edge_key_out: {}\n",
        q(&format!("{}/{EDGE_KEY_FILE}", v.cert_dir))
    ));
    s
}

/// The forwarder's inventory name: one per endpoint.
fn host_name(endpoint: &str) -> String {
    format!("forwarder-{}", endpoint.replace('.', "-"))
}

/// One forwarder: its endpoint, its inventory address, the peerings it relays.
type Relayed<'a> = (
    String,
    String,
    Vec<(&'a config::Peering, &'a PeeringValues)>,
);

/// Who uses which forwarder: group -> its forwarder.
fn by_group<'a>(
    cfg: &'a Config,
    values: &'a [PeeringValues],
) -> Result<BTreeMap<String, Relayed<'a>>> {
    let mut out: BTreeMap<String, Relayed<'a>> = BTreeMap::new();
    for (p, v) in cfg.peerings.iter().zip(values.iter()) {
        let Some(f) = &p.forwarder else { continue };
        let g = group_of(f).to_string();
        let ah = f.ansible_host.clone().unwrap_or_else(|| f.endpoint.clone());
        let e = out
            .entry(g.clone())
            .or_insert_with(|| (f.endpoint.clone(), ah.clone(), Vec::new()));
        if e.0 != f.endpoint {
            return Err(goat_errors::coded(
                "GB-162",
                format!(
                    "the forwarder group '{g}' holds two forwarders, {} and {} (GB-162): its group_vars would give each of them every peering. Name a group per forwarder with `forwarder: group:`",
                    e.0, f.endpoint
                ),
            )
            .into());
        }
        // The forwarder role's own rule for an entry's name.
        let name_ok = (2..=16).contains(&p.with.len())
            && p.with
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !name_ok {
            return Err(goat_errors::coded(
                "GB-161",
                format!(
                    "peering '{}' cannot be relayed by a forwarder (GB-161): a forwarder names each peering by the partner id, 2-16 characters of a-z 0-9 -. Declare the partner by its sovereign id",
                    p.with
                ),
            )
            .into());
        }
        e.2.push((p, v));
    }
    // One forwarder's peerings must share nothing on it. Allocation keeps them
    // apart within one environment; two environments behind one forwarder are
    // checked here.
    for (g, (_, _, list)) in &out {
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        for (_, v) in list {
            let who = format!("{}/{}", v.environment, v.peering);
            let mut keys = vec![
                format!(
                    "interface {}",
                    v.carrier_interface.clone().unwrap_or_default()
                ),
                format!("UDP port {}", v.public_port.unwrap_or_default()),
                format!("UDP port {}", v.carrier_port.unwrap_or_default()),
            ];
            if let Some((a, b, _)) = v.carrier_subnet.as_deref().and_then(carrier_pair) {
                keys.push(format!("carrier address {a}"));
                keys.push(format!("carrier address {b}"));
            }
            for k in keys {
                if let Some(other) = seen.insert(k.clone(), who.clone()) {
                    return Err(goat_errors::coded(
                        "GB-160",
                        format!(
                            "peerings {other} and {who} both use {k} on the forwarder group '{g}' (GB-160). Pin a different value under one peering's values: (public_port, carrier_port, carrier_interface or carrier_subnet)"
                        ),
                    )
                    .into());
                }
            }
        }
    }
    Ok(out)
}

/// The forwarder group_vars files, one per forwarder group.
pub fn render(cfg: &Config, values: &[PeeringValues]) -> Result<Vec<Rendered>> {
    let mut out = Vec::new();
    for (g, (endpoint, _, list)) in by_group(cfg, values)? {
        let mut s = header(&format!("the peerings the forwarder {endpoint} relays."));
        s.push_str(
            "# One entry per peering: the partner dials peering_port, the forwarder relays it\n\
             # down the carrier to the edge's WireGuard on edge_peering_port. Each edge's\n\
             # carrier public key is copied by peering-carrier-keys.yml in phase 2, from every\n\
             # relayed edge. An entry whose edge has no carrier key yet (that peering's phase 2\n\
             # has not run) is left out until it has one, so one peering's run never waits on\n\
             # another's.\n\n\
             peering_forwarder_enabled: true\n\
             peering_forwarder_peerings: \"{{ goatbuilder_forwarder_peerings | rejectattr('edge_carrier_public_key', 'equalto', '') | list }}\"\n\
             goatbuilder_forwarder_peerings:\n",
        );
        let mut keys_out = String::new();
        for (_, v) in &list {
            let (Some(ci), Some(pp), Some(cp), Some(cs)) = (
                &v.carrier_interface,
                v.public_port,
                v.carrier_port,
                &v.carrier_subnet,
            ) else {
                continue;
            };
            let Some((fwd_ip, edge_ip, len)) = carrier_pair(cs) else {
                continue;
            };
            s.push_str(&format!("  - name: {}\n", q(&v.peering)));
            s.push_str(&format!("    interface: {}\n", q(ci)));
            s.push_str(&format!("    peering_port: {pp}\n"));
            s.push_str(&format!("    edge_peering_port: {}\n", v.wg_listen_port));
            s.push_str(&format!("    carrier_port: {cp}\n"));
            s.push_str(&format!("    carrier_local_ip: {}\n", q(&fwd_ip)));
            s.push_str(&format!("    carrier_remote_ip: {}\n", q(&edge_ip)));
            s.push_str(&format!("    carrier_prefix_len: {len}\n"));
            s.push_str(&format!(
                "    edge_carrier_public_key: \"{}\"\n",
                lookup(&format!("{}/{EDGE_KEY_FILE}", v.cert_dir))
            ));
            keys_out.push_str(&format!(
                "  {}: {}\n",
                ci,
                q(&format!("{}/{FORWARDER_KEY_FILE}", v.cert_dir))
            ));
        }
        s.push_str(
            "\n# Where peering-carrier-keys.yml copies each carrier's forwarder public key.\n",
        );
        s.push_str("peering_carrier_forwarder_key_out:\n");
        s.push_str(&keys_out);
        out.push(Rendered {
            path: format!("ops/ansible/group_vars/{g}/peering-forwarder.yml"),
            body: s,
        });
    }
    Ok(out)
}

/// The forwarder groups of the rendered inventory. Empty when no peering is
/// relayed.
pub fn inventory(cfg: &Config, values: &[PeeringValues]) -> Result<String> {
    let groups = by_group(cfg, values)?;
    if groups.is_empty() {
        return Ok(String::new());
    }
    let mut s = String::from(
        "\n# The peering forwarders: in no goatnet group, holding no peering material.\n",
    );
    let custom: Vec<&String> = groups
        .keys()
        .filter(|g| g.as_str() != DEFAULT_GROUP)
        .collect();
    if !custom.is_empty() {
        s.push_str(&format!("[{DEFAULT_GROUP}:children]\n"));
        for g in &custom {
            s.push_str(&format!("{g}\n"));
        }
    }
    for (g, (endpoint, ah, _)) in &groups {
        s.push_str(&format!(
            "\n[{g}]\n{} ansible_host={ah}\n",
            host_name(endpoint)
        ));
    }
    Ok(s)
}

/// Phase 2 for a peering a forwarder relays: prepare, keys, forwarder, keys,
/// prepare. Every other peering's plan is left exactly as it was.
pub(super) fn splice(
    phases: &mut [Phase],
    cfg: &Config,
    cfg_dir: &Path,
    o: &Options,
    edge_group: &str,
) -> Result<()> {
    // Only a relayed entry changes phase 2; ambiguity matters only then.
    let relayed = cfg.peerings.iter().any(|p| {
        p.with == o.with
            && o.env.map(|e| p.environment == e).unwrap_or(true)
            && p.forwarder.is_some()
    });
    if !relayed {
        return Ok(());
    }
    let Some(p) = super::declaration::peering(cfg, o.with, o.env)? else {
        return Ok(());
    };
    let Some(f) = &p.forwarder else { return Ok(()) };
    let ctx = AllocContext::load(cfg_dir)?;
    // The forwarder steps read the group_vars `goatbuilder federate allocate`
    // rendered from the record; without the record they would not exist.
    if !ctx
        .record
        .peerings
        .contains_key(&format!("{}/{}", p.environment, p.with))
    {
        return Err(goat_errors::coded(
            "GB-163",
            format!(
                "peering '{}' on '{}' is relayed by the forwarder {}, and {} does not hold it yet (GB-163): phase 2 runs the forwarder from the files `goatbuilder federate allocate` renders. Run `goatbuilder federate allocate`, review and commit what it writes, then run this again",
                p.with,
                p.environment,
                f.endpoint,
                alloc::RECORD_FILE
            ),
        )
        .into());
    }
    // Allocation refuses for every peering's problem; the values are this one's.
    let v = alloc::allocate_in(cfg, Some(&p.environment), &p.with, &ctx)?;
    let g = group_of(f);
    let Some(ph) = phases.iter_mut().find(|ph| ph.n == 2) else {
        return Ok(());
    };
    let limit = format!("--limit '{edge_group}'");
    let prepare = " peering-prepare.yml";
    if !(ph.cmd.contains(&limit) && ph.cmd.ends_with(prepare)) {
        return Err(anyhow::anyhow!(
            "phase 2 is no longer one peering-prepare.yml run on --limit '{edge_group}', so the forwarder steps cannot be placed around it"
        ));
    }
    let with = |lim: &str, play: &str| {
        ph.cmd
            .replace(&limit, &format!("--limit '{lim}'"))
            .replacen(prepare, &format!(" {play}"), 1)
    };
    // Every relayed edge's key, not only this peering's: the forwarder play
    // applies every peering it relays.
    let keys = with(&format!("peering_edges:{g}"), "peering-carrier-keys.yml");
    let forward = with(g, "peering-forwarder.yml");
    let first = ph.cmd.clone();
    // Each step in its own subshell: each begins with a `cd`, which may be relative.
    // The second keys step must leave the forwarder's carrier key on this
    // machine; without it the edge's carrier would come up with no peer.
    let fwd_key = format!("{}/{FORWARDER_KEY_FILE}", v.cert_dir);
    let check = format!(
        "test -s '{fwd_key}' || {{ echo \"    the forwarder's carrier public key did not reach {fwd_key}: check the forwarder play and peering-carrier-keys.yml on {g}\"; false; }}"
    );
    ph.cmd = format!("({first}) && ({keys}) && ({forward}) && ({keys} && {check}) && ({first})");
    ph.label = format!(
        "{}, behind the forwarder {} (public port {}, carrier {})",
        ph.label,
        f.endpoint,
        v.public_port.unwrap_or_default(),
        v.carrier_interface.clone().unwrap_or_default()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(peerings: serde_json::Value) -> Config {
        serde_json::from_value(json!({
            "schema": "goatbuilder/v1",
            "preset": "full",
            "repo": {"source": "example-org/example-repo", "image_namespace": "ghcr.io/example-org"},
            "components": {},
            "environments": [
                {"name": "dev", "converger": "push-ansible", "sites": [{"name": "alpha", "role": "founder", "classes": {"core": {}, "edge": {}}}]},
                {"name": "prod", "converger": "push-ansible", "sites": [{"name": "alpha", "role": "founder", "classes": {"core": {}, "edge": {}}}]}
            ],
            "promotion": {"path": [{"name": "dev", "kind": "rung", "environment": "dev"}]},
            "peerings": peerings,
        }))
        .unwrap()
    }

    fn p(with: &str, fwd: bool) -> serde_json::Value {
        let mut v = json!({"with": with, "environment": "dev", "site": "alpha", "edge_class": "edge",
            "hosts": {"edge": "192.0.2.20", "core": "192.0.2.10"}});
        if fwd {
            v["forwarder"] = json!({"endpoint": "203.0.113.7"});
        }
        v
    }

    fn values(c: &Config) -> Vec<PeeringValues> {
        alloc::allocate_all(c, &AllocContext::default()).unwrap()
    }

    /// A partner on two environments with no forwarder: phase 2 is left as it
    /// was, with no --env demanded.
    #[test]
    fn splice_demands_env_only_when_a_forwarder_is_declared() {
        let mut prod = p("xab", false);
        prod["environment"] = json!("prod");
        let c = cfg(json!([p("xab", false), prod]));
        let dir = tempfile::tempdir().unwrap();
        let o = Options {
            with: "xab",
            env: None,
            r#as: None,
            edge_group: None,
            work: None,
            mock: None,
            from: 1,
            to: 99,
            dry_run: true,
            table: false,
        };
        let mut phases = vec![Phase {
            n: 2,
            kind: "A",
            label: "prepare".into(),
            cmd: "x peering-prepare.yml".into(),
        }];
        splice(&mut phases, &c, dir.path(), &o, "xab_edge").unwrap();
        assert_eq!(phases[0].cmd, "x peering-prepare.yml");
    }

    #[test]
    fn carrier_pairs_put_the_forwarder_first() {
        assert_eq!(
            carrier_pair("169.254.30.0/31"),
            Some(("169.254.30.0".into(), "169.254.30.1".into(), 31))
        );
        assert_eq!(
            carrier_pair("169.254.30.4/30"),
            Some(("169.254.30.5".into(), "169.254.30.6".into(), 30))
        );
    }

    /// Two peerings behind one forwarder: two entries, each mapping its own
    /// public port to its own edge listen port, down its own carrier.
    #[test]
    fn a_second_peering_is_a_second_port_mapping() {
        let c = cfg(json!([p("xab", true), p("xac", true), p("xad", false)]));
        let vs = values(&c);
        let fs = render(&c, &vs).unwrap();
        assert_eq!(fs.len(), 1);
        assert_eq!(
            fs[0].path,
            "ops/ansible/group_vars/peering_forwarders/peering-forwarder.yml"
        );
        let b = &fs[0].body;
        assert!(b.contains("peering_forwarder_enabled: true"), "{b}");
        for frag in [
            "  - name: \"xab\"\n    interface: \"wg-carry0\"\n    peering_port: 51820\n    edge_peering_port: 51823\n    carrier_port: 51821\n    carrier_local_ip: \"169.254.30.0\"\n    carrier_remote_ip: \"169.254.30.1\"\n    carrier_prefix_len: 31\n",
            "  - name: \"xac\"\n    interface: \"wg-carry1\"\n    peering_port: 51822\n    edge_peering_port: 51825\n    carrier_port: 51823\n    carrier_local_ip: \"169.254.30.2\"\n    carrier_remote_ip: \"169.254.30.3\"\n",
            "edge_carrier_public_key: \"{{ lookup('ansible.builtin.file', '/var/lib/goat-federation/xac/pki/edge-carrier.pub', errors='ignore') | default('', true) | trim }}\"",
            "peering_carrier_forwarder_key_out:\n  wg-carry0: \"/var/lib/goat-federation/xab/pki/forwarder-carrier.pub\"\n  wg-carry1: \"/var/lib/goat-federation/xac/pki/forwarder-carrier.pub\"\n",
            "peering_forwarder_peerings: \"{{ goatbuilder_forwarder_peerings | rejectattr('edge_carrier_public_key', 'equalto', '') | list }}\"\ngoatbuilder_forwarder_peerings:\n  - name: \"xab\"",
        ] {
            assert!(b.contains(frag), "{frag}\n---\n{b}");
        }
        assert!(
            !b.contains("xad"),
            "a peering with no forwarder is not relayed: {b}"
        );
        // The rendered entries parse as YAML the role reads.
        let y: serde_yaml::Value = serde_yaml::from_str(b).unwrap();
        assert_eq!(
            y["goatbuilder_forwarder_peerings"]
                .as_sequence()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn the_edge_gets_its_carrier_variables_and_only_when_relayed() {
        let c = cfg(json!([p("xab", true), p("xac", true), p("xad", false)]));
        let vs = values(&c);
        let e = edge_vars(&c.peerings[1], &vs[1]);
        for frag in [
            "peering_wg_via_forwarder: true",
            "peering_wg_carrier_interface: \"wg-carry1\"",
            "peering_wg_carrier_listen_port: 51824",
            "peering_wg_carrier_local_ip: \"169.254.30.3\"",
            "peering_wg_carrier_remote_ip: \"169.254.30.2\"",
            "peering_wg_forwarder_endpoint: \"203.0.113.7:51823\"",
            "/var/lib/goat-federation/xac/pki/forwarder-carrier.pub",
            "peering_carrier_edge_key_out: \"/var/lib/goat-federation/xac/pki/edge-carrier.pub\"",
        ] {
            assert!(e.contains(frag), "{frag}\n---\n{e}");
        }
        assert_eq!(edge_vars(&c.peerings[2], &vs[2]), "");
    }

    #[test]
    fn the_inventory_lists_the_forwarder_in_its_own_group() {
        let c = cfg(json!([p("xab", true)]));
        let inv = inventory(&c, &values(&c)).unwrap();
        assert!(
            inv.contains("[peering_forwarders]\nforwarder-203-0-113-7 ansible_host=203.0.113.7"),
            "{inv}"
        );
        let mut x = p("xab", true);
        x["forwarder"] =
            json!({"endpoint": "203.0.113.7", "group": "fwd_west", "ansible_host": "198.51.100.4"});
        let c = cfg(json!([x]));
        let inv = inventory(&c, &values(&c)).unwrap();
        assert!(
            inv.contains("[peering_forwarders:children]\nfwd_west\n")
                && inv.contains("[fwd_west]\nforwarder-203-0-113-7 ansible_host=198.51.100.4"),
            "{inv}"
        );
        assert_eq!(
            inventory(
                &cfg(json!([p("xab", false)])),
                &values(&cfg(json!([p("xab", false)])))
            )
            .unwrap(),
            ""
        );
    }

    fn code(e: anyhow::Error) -> String {
        e.downcast_ref::<goat_errors::Coded>()
            .map(|c| c.code.clone())
            .unwrap_or_default()
    }

    /// Two environments behind one forwarder each start at index 0: refused,
    /// naming the pins that separate them.
    #[test]
    fn two_environments_behind_one_forwarder_must_not_share_a_port() {
        // Allocation keeps one forwarder's values apart across environments.
        let mut b = p("xac", true);
        b["environment"] = json!("prod");
        let c = cfg(json!([p("xab", true), b]));
        let vs = values(&c);
        assert!(render(&c, &vs).is_ok());
        // GB-160 is the backstop for values that collide anyway (a record
        // edited by hand).
        let mut bad = vs.clone();
        bad[1].public_port = bad[0].public_port;
        assert_eq!(code(render(&c, &bad).unwrap_err()), "GB-160");
    }

    #[test]
    fn one_group_two_forwarders_and_bad_names_are_refused() {
        let mut b = p("xac", true);
        b["forwarder"] = json!({"endpoint": "203.0.113.8"});
        let c = cfg(json!([p("xab", true), b]));
        assert_eq!(code(render(&c, &values(&c)).unwrap_err()), "GB-162");
        let c = cfg(json!([p("x_ab", true)]));
        assert_eq!(code(render(&c, &values(&c)).unwrap_err()), "GB-161");
    }

    /// Phase 2 of a relayed peering: prepare, keys, forwarder, keys, prepare,
    /// each on its own --limit. A peering without a forwarder keeps its phase 2.
    #[test]
    fn phase_2_runs_the_forwarder_between_two_prepares() {
        let base = "cd '/r/ops/ansible' && x -i inventory/peering-edges.ini --limit 'xab_edge' -e fed_edge_group='xab_edge' peering-prepare.yml";
        let mk = || {
            vec![Phase {
                n: 2,
                kind: "A",
                label: "prepare".into(),
                cmd: base.into(),
            }]
        };
        let dir = tempfile::tempdir().unwrap();
        let o = |with: &'static str| Options {
            with,
            env: None,
            r#as: None,
            edge_group: None,
            work: None,
            mock: None,
            from: 1,
            to: 99,
            dry_run: true,
            table: false,
        };
        let c = cfg(json!([p("xab", true), p("xac", false)]));
        // Not allocated yet: refused, because phase 2 would read files that do not exist.
        let e = splice(&mut mk(), &c, dir.path(), &o("xab"), "xab_edge").unwrap_err();
        assert_eq!(
            e.downcast_ref::<goat_errors::Coded>().unwrap().code,
            "GB-163"
        );
        let rec = alloc::record_of(&values(&c), &alloc::Record::default());
        std::fs::write(
            dir.path().join(alloc::RECORD_FILE),
            alloc::record_text(&rec),
        )
        .unwrap();
        let mut phases = mk();
        splice(&mut phases, &c, dir.path(), &o("xab"), "xab_edge").unwrap();
        let cmd = phases[0].cmd.clone();
        assert!(
            cmd.starts_with("(cd ") && cmd.ends_with(" peering-prepare.yml)"),
            "{cmd}"
        );
        let parts: Vec<&str> = cmd
            .trim_start_matches('(')
            .trim_end_matches(')')
            .split(") && (")
            .collect();
        assert_eq!(parts.len(), 5, "{}", phases[0].cmd);
        assert!(
            parts[0]
                .ends_with("--limit 'xab_edge' -e fed_edge_group='xab_edge' peering-prepare.yml"),
            "{}",
            parts[0]
        );
        assert!(
            parts[1].contains("--limit 'peering_edges:peering_forwarders'")
                && parts[1].ends_with(" peering-carrier-keys.yml"),
            "{}",
            parts[1]
        );
        assert!(
            parts[2].contains("--limit 'peering_forwarders'")
                && parts[2].ends_with(" peering-forwarder.yml"),
            "{}",
            parts[2]
        );
        assert!(parts[1].starts_with(parts[3].split(" && ").next().unwrap()));
        // After the second keys step, phase 2 fails unless the forwarder's
        // carrier key reached the machine running the driver.
        assert!(
            parts[3].contains("test -s '/var/lib/goat-federation/xab/pki/forwarder-carrier.pub'"),
            "{}",
            parts[3]
        );
        assert!(parts[4].ends_with(" peering-prepare.yml"));
        assert!(
            phases[0].label.contains(
                "behind the forwarder 203.0.113.7 (public port 51820, carrier wg-carry0)"
            ),
            "{}",
            phases[0].label
        );
        let mut untouched = mk();
        splice(&mut untouched, &c, dir.path(), &o("xac"), "xac_edge").unwrap();
        assert_eq!(untouched[0].cmd, base);
    }
}
