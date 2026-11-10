#!/usr/bin/env python3
"""One sovereign's own declarations, written into that sovereign's clone.

    overlay.py <clone> <us> <them> <subnet-octet> <image>
               [--core-addr A] [--spire-listen A] [--local] [--republisher-image I]
               [--also THEM:INTERFACE:PORT:SPIRE_OUT_PORT:METRICS_PORT]...

Each party has its own repository, as two organisations would. Only synthetic names
and addresses: this run reaches no goatnet.

  --core-addr     where the core router and the SPIRE server listen
                  (default 10.77.<octet>.10, the container core host)
  --spire-listen  where the outbound bundle-endpoint forward listens
                  (default 10.77.<octet>.20, the container edge host)
  --local         ansible reaches the edge and the core on this machine with a local
                  connection: the live run, where one host carries both
  --republisher-image
                  the outbound republisher's image on the core host: a digest-pinned
                  reference, or the `sha256:<id>` of an image loaded there
  --also          one more peering on the SAME edge host and the SAME core host, with
                  the values two peerings cannot share there: its WireGuard interface
                  and listen port, its SPIRE outbound forward port and its republisher's
                  metrics port. Repeatable. `them` keeps the role defaults.

ONE INVENTORY NAME PER PEERING FOR EACH MACHINE. Every peering's edge and core are
listed as `<us>-edge-<them>` and `<us>-core-<them>`, all with the same ansible_host.
Ansible merges the group_vars of every group an inventory host is in, the group that
sorts last winning a conflict, so one name in two peering groups makes each peering's
run read the other's interface, ports and certificate paths. Two names for one machine
keep each peering's variables its own.
"""
import argparse
from pathlib import Path

import yaml

ap = argparse.ArgumentParser()
for name in ("clone", "us", "them", "octet", "image"):
    ap.add_argument(name)
ap.add_argument("--core-addr")
ap.add_argument("--spire-listen")
ap.add_argument("--local", action="store_true")
ap.add_argument("--republisher-image", default="")
ap.add_argument("--also", action="append", default=[])
a = ap.parse_args()

clone, us = Path(a.clone), a.us
core_ip = a.core_addr or f"10.77.{a.octet}.10"
spire_listen = a.spire_listen or f"10.77.{a.octet}.20"
conn = "ansible_connection=local" if a.local else "ansible_user=goat"
core_host = "127.0.0.1" if a.local else core_ip

# (them, per-peering overrides). The first peering keeps every role default.
peerings = [(a.them, {})]
for spec in a.also:
    them2, iface, port, spire_out, metrics = spec.split(":")
    peerings.append((them2, {"iface": iface, "port": int(port),
                             "spire_out": int(spire_out), "metrics": int(metrics)}))
names = [p for p, _ in peerings]
assert len(set(names)) == len(names), f"a partner named twice: {names}"

inv = ["[peering_edges:children]", *[f"{t}_edge" for t in names], ""]
for t in names:
    inv += [f"[{t}_edge]", f"{us}-edge-{t} ansible_host=127.0.0.1 {conn}", ""]
inv += ["# The core host each peering's edge dials: that peering's outbound republisher runs",
        "# there (phase 12). One machine, one inventory name per peering.",
        "[peering_cores:children]", *[f"{t}_core" for t in names], ""]
for t in names:
    inv += [f"[{t}_core]", f"{us}-core-{t} ansible_host={core_host} {conn}", ""]
(clone / "ops/ansible/inventory/peering-edges.ini").write_text("\n".join(inv))

for them, o in peerings:
    work = f"/var/lib/goat-federation/{them}"
    # The republisher's deployment half. The agreement half comes from `goat federate
    # apply`; our CA is the one phase 8 signs with, as the edge reads it.
    cg = clone / "ops/ansible/group_vars" / f"{them}_core"
    cg.mkdir(parents=True, exist_ok=True)
    core_vars = {
        "goat_fed_republisher_image": a.republisher_image,
        "goat_fed_republisher_core_endpoint": f"tls/{core_ip}:7447",
        "goat_fed_republisher_core_ca_src": f"{work}/pki/our-ca.pem",
    }
    if o:
        core_vars["goat_fed_republisher_metrics_port"] = o["metrics"]
    (cg / "republisher.yml").write_text(yaml.safe_dump(core_vars, sort_keys=True))

    gv = clone / "ops/ansible/group_vars" / f"{them}_edge"
    gv.mkdir(parents=True, exist_ok=True)
    edge_vars = {
        "goat_zenoh_edge_image": a.image,
        "goat_zenoh_edge_trust_mode": "fragment",
        "goat_zenoh_edge_partner_ca_path": f"{work}/pki/partner-ca.pem",
        "goat_zenoh_edge_our_ca_path": f"{work}/pki/our-ca.pem",
        "goat_zenoh_edge_outward_cert": f"{work}/pki/peering-outward.pem",
        "goat_zenoh_edge_outward_key": f"{work}/pki/peering-outward-key.pem",
        "goat_zenoh_edge_inward_cert": f"{work}/pki/peering-inward.pem",
        "goat_zenoh_edge_inward_key": f"{work}/pki/peering-inward-key.pem",
        "goat_zenoh_edge_core_cn": f"core-{us}",
        "goat_zenoh_edge_core_endpoints": [
            f"tls/{core_ip}:7447#root_ca_certificate_file={work}/pki/our-ca.pem"
        ],
        # The same port for every peering: `goat federate apply` binds each edge router
        # to its own link address, so two edges on one host do not collide on it.
        "goat_zenoh_edge_listen_port": 7448,
        # A refusal is silent to the sender, so the receiving edge's trace is the record.
        "goat_zenoh_edge_acl_trace": True,
        "peering_wg_enabled": True,
        "peering_wg_fips_waived_reason": "federation e2e on stock Ubuntu, DBB-WVR-001; no FIPS claim",
        "peering_wg_extra_ports": [8443],
        "peering_spire_enabled": True,
        "peering_spire_server_host": f"{us}-core-{them}",
        "peering_spire_server_addr": core_ip,
        "peering_spire_outbound_listen": spire_listen,
    }
    if o:
        edge_vars.update({
            "peering_wg_interface": o["iface"],
            "peering_wg_listen_port": o["port"],
            "peering_spire_outbound_port": o["spire_out"],
        })
    (gv / "deployment.yml").write_text(yaml.safe_dump(edge_vars, sort_keys=True))

# This sovereign's goatbuilder config, written whole: one environment, its one founder
# site with the core and the edge class, and every peering. Nothing is taken from the
# checkout's own config, which a public checkout does not carry (it is a deployment's
# file): the run is the same from any clone.
cfg_path = clone / "ops/goatbuilder/goatbuilder.yaml"
cfg_path.parent.mkdir(parents=True, exist_ok=True)
cfg = {
    "schema": "goatbuilder/v1",
    "preset": "full",
    "repo": {"source": f"example-{us}/goat", "image_namespace": f"ghcr.io/example-{us}"},
    "components": {
        "goat-zenoh-edge": {"declared_in": f"ops/ansible/group_vars/{a.them}_edge/deployment.yml", "var": "goat_zenoh_edge_image"},
    },
    "environments": [{
        "name": us,
        "converger": "push-ansible",
        "sites": [{"name": "home", "role": "founder", "classes": {"core": {}, "edge": {}}}],
    }],
    "promotion": {"path": [{"name": us, "kind": "rung", "environment": us}]},
    "peerings": [{"with": t, "environment": us, "site": "home", "edge_class": "edge"} for t in names],
    # The run templates no credential: nothing to resolve, nothing to read from a file.
    "secrets": {"store": "env"},
}
cfg_path.write_text(
    f"# {us}: this sovereign's synthetic goatbuilder config for the federation e2e\n"
    "# (ops/federation-e2e/overlay.py). Written whole on every run.\n"
    + yaml.safe_dump(cfg, sort_keys=False)
)
