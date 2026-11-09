"""peering-backout.yml touches one peering of two that share an edge, a core and a
forwarder, and nothing of the other.

Two checks run ansible-playbook on localhost only (connection local, no privilege):
  * the names each peering's back-out derives are disjoint: no path, unit, interface,
    nftables table, container or stack of one is a name of the other;
  * a survey (the dry run: peering_backout_apply off) of peering xac, with peering xab
    declared on the same edge, core and forwarder, reports rows that name xac's
    resources only; its core plan removes xac's ACL ids and keeps xab admitted.
It also checks that the role defaults the back-out repeats (it includes the roles'
absent tasks by path, so their defaults are not loaded) equal the roles' own.

Skipped, and saying so, when ansible-playbook is not on PATH.
"""
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

import yaml

HERE = Path(__file__).resolve().parent
ANSIBLE = HERE.parents[1]
REPO = HERE.parents[3]

spec = importlib.util.spec_from_file_location("tbc", REPO / "ops/federation/standup/test_backout_core.py")
tbc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tbc)

PEERINGS = {
    "xab": {"wg": "wg-fed0", "carrier": "wg-carry0", "link": "169.254.77", "spire_out": 18443,
            "fwd_if": "wg-carry0", "fwd_ports": (51824, 51825), "carrier_ips": ("169.254.30.0", "169.254.30.1")},
    "xac": {"wg": "wg-fed1", "carrier": "wg-carry1", "link": "169.254.78", "spire_out": 18444,
            "fwd_if": "wg-carry1", "fwd_ports": (51826, 51827), "carrier_ips": ("169.254.31.0", "169.254.31.1")},
}


def dump(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        if isinstance(data, str):
            f.write(data)
        else:
            yaml.safe_dump(data, f, sort_keys=True)


def build(tmp):
    """An inventory with one edge host and one core host per peering name (one inventory
    name per peering per shared machine, as the three-party layout has), all localhost."""
    py = sys.executable
    hosts = []
    for p in PEERINGS:
        hosts += [f"edge-{p}", f"core-{p}"]
    inv = ["[all:vars]", "ansible_connection=local", f"ansible_python_interpreter={py}", ""]
    for p in PEERINGS:
        inv += [f"[{p}_edge]", f"edge-{p}", "", f"[{p}_core]", f"core-{p}", ""]
    inv += ["[peering_edges:children]"] + [f"{p}_edge" for p in PEERINGS] + [""]
    inv += ["[peering_cores:children]"] + [f"{p}_core" for p in PEERINGS] + [""]
    inv += ["[peering_forwarders]", "fwd", ""]
    dump(f"{tmp}/inventory/peering-edges.ini", "\n".join(inv) + "\n")
    fwd_entries = []
    for p, v in PEERINGS.items():
        work = f"{tmp}/work/{p}"
        agreement = {"parties": [{"sovereign_id": "xaa", "trust_domain": "spiffe://xaa.example.org"},
                                 {"sovereign_id": p, "trust_domain": f"spiffe://{p}.example.org"}]}
        dump(f"{work}/agreement.json", json.dumps(agreement))
        dump(f"{work}/applied/ops/ansible/group_vars/{p}_edge/federation-{p}-transport.yml", {
            "goat_peering_transport_kind": "wireguard", "goat_peering_our_sovereign": "xaa",
            "goat_peering_partner_sovereign": p, "goat_peering_partner_trust_domain": f"spiffe://{p}.example.org",
            "goat_peering_local_link_ip": v["link"] + ".0", "goat_peering_remote_link_ip": v["link"] + ".1"})
        common = {"peering_id": p, "fed_work": work, "fed_edge_group": f"{p}_edge"}
        dump(f"{tmp}/inventory/group_vars/{p}_edge.yml", {
            **common, "peering_wg_enabled": True, "peering_wg_interface": v["wg"],
            "peering_wg_via_forwarder": True, "peering_wg_carrier_interface": v["carrier"],
            "peering_spire_enabled": True, "peering_spire_server_host": "localhost",
            "peering_spire_outbound_listen": "127.0.0.1", "peering_spire_outbound_port": v["spire_out"],
            "peering_edge_name": f"xaa-{p}", "peering_edge_their_sovereign_id": p})
        dump(f"{tmp}/inventory/group_vars/{p}_core.yml", common)
        fwd_entries.append({"name": p, "interface": v["fwd_if"], "peering_port": v["fwd_ports"][0],
                            "carrier_port": v["fwd_ports"][1], "carrier_local_ip": v["carrier_ips"][0],
                            "carrier_remote_ip": v["carrier_ips"][1],
                            "edge_carrier_public_key": ("A" * 42 + ("B" if p == "xab" else "C") + "=")})
    dump(f"{tmp}/inventory/host_vars/fwd.yml", {
        "peering_forwarder_enabled": True, "peering_forwarder_peerings": fwd_entries,
        "peering_forwarder_public_interface": "eth0"})
    return f"{tmp}/inventory/peering-edges.ini"


def leaves(v):
    if isinstance(v, dict):
        for x in v.values():
            yield from leaves(x)
    elif isinstance(v, list):
        for x in v:
            yield from leaves(x)
    elif isinstance(v, str) and v:
        yield v


def run(argv, tmp):
    env = dict(os.environ, ANSIBLE_CONFIG=str(ANSIBLE / "ansible.cfg"), ANSIBLE_NOCOLOR="1",
               ANSIBLE_FACT_CACHING="memory", ANSIBLE_LOCAL_TEMP=f"{tmp}/ansible-tmp",
               ANSIBLE_REMOTE_TEMP=f"{tmp}/ansible-tmp")
    return subprocess.run(argv, cwd=ANSIBLE, env=env, capture_output=True, text=True, check=False)


@unittest.skipUnless(shutil.which("ansible-playbook"), "ansible-playbook not on PATH")
class TwoPeeringsOneHost(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="peering-backout-")
        self.inv = build(self.tmp)

    def test_the_names_of_two_peerings_are_disjoint(self):
        out = f"{self.tmp}/names"
        os.makedirs(out)
        r = run(["ansible-playbook", "-i", self.inv, "--limit", "edge-xab:edge-xac:core-xab:core-xac",
                 str(HERE / "names.yml"), "-e", f"out={out}"], self.tmp)
        self.assertEqual(r.returncode, 0, r.stdout[-4000:] + r.stderr[-2000:])
        for role in ("edge", "core"):
            a = json.load(open(f"{out}/{role}-xab.json"))
            b = json.load(open(f"{out}/{role}-xac.json"))
            # Where the SPIRE server is reached, and our own sovereign id, are the same for
            # every peering by definition; they are addresses, not resources removed.
            for n in (a, b):
                for k in ("server_host", "container", "bin"):
                    n["spire"].pop(k)
                n.pop("us")
            if role == "core":
                # A core host's back-out reads only the republisher names (and the ACL ids,
                # tested in ops/federation/standup/test_backout_core.py).
                a, b = a["republisher"], b["republisher"]
            shared = set(leaves(a)) & set(leaves(b))
            self.assertEqual(shared, set(), f"{role}: names shared by two peerings")
            if role == "edge":
                self.assertEqual(a["id"], "xab")
                self.assertEqual(b["trust_domain"], "spiffe://xac.example.org")
                self.assertEqual(b["wg"]["nft_table"], "goat_peering_wg_fed1")

    def test_a_survey_of_one_peering_names_nothing_of_the_other(self):
        fx = tbc.Fixture(tbc.fragment("xaa", "xab"), tbc.fragment("xaa", "xac"))
        work = f"{self.tmp}/work/xac"
        r = run(["ansible-playbook", "-i", self.inv, "--limit", "xac_edge:xac_core:peering_forwarders",
                 str(ANSIBLE / "peering-backout.yml"),
                 "-e", "peering_backout_id=xac", "-e", f"fed_work={work}", "-e", "fed_edge_group=xac_edge",
                 "-e", "ansible_become=false",
                 "-e", f"peering_backout_core_seed={fx.seed}", "-e", f"peering_backout_core_org={fx.org}",
                 "-e", f"peering_backout_core_config={fx.live}"], self.tmp)
        self.assertEqual(r.returncode, 0, r.stdout[-6000:] + r.stderr[-2000:])
        records = {}
        for host in ("edge-xac", "core-xac", "fwd"):
            with open(f"{work}/backout/survey/{host}.json", encoding="utf-8") as f:
                records[host] = json.load(f)
        xab = PEERINGS["xab"]
        forbidden = [r"\bxab\b", re.escape(xab["wg"]), re.escape(xab["carrier"]), re.escape(xab["link"]),
                     str(xab["spire_out"]), str(xab["fwd_ports"][0]), str(xab["fwd_ports"][1])]
        for host, rec in records.items():
            self.assertEqual(rec["peering"], "xac")
            for row in rec["rows"]:
                for pat in forbidden:
                    self.assertIsNone(re.search(pat, row["resource"]), f"{host}: {row}")
                # A survey changes nothing: no row says it removed anything.
                self.assertNotIn(row["action"], ("removed", "restarted"), row)
        core = records["core-xac"]["rows"]
        subjects = sorted(r["resource"] for r in core if r["resource"].startswith("subject "))
        self.assertEqual(subjects, ["subject fed-rp-xac", "subject fed-xac"])
        restart = [r for r in core if r["resource"].startswith("router restart")]
        self.assertEqual(len(restart), 1)
        self.assertIn("stays admitted: xab", restart[0]["note"])
        fwd = records["fwd"]["rows"]
        self.assertTrue(any(r["resource"] == "carrier wg-carry1" for r in fwd), fwd)
        self.assertTrue(any("1 other entries" in r.get("note", "") for r in fwd), fwd)
        # The core files were read, never written.
        self.assertEqual(open(fx.org).read(), fx.vendor + tbc.fragment("xaa", "xab") + tbc.fragment("xaa", "xac"))

    def test_a_survey_without_the_rendered_values_or_agreement_still_runs(self):
        # A peering backed out before phase 9: no agreement, no applied values.
        shutil.rmtree(f"{self.tmp}/work/xac")
        os.makedirs(f"{self.tmp}/work/xac")
        r = run(["ansible-playbook", "-i", self.inv, "--limit", "xac_edge",
                 str(ANSIBLE / "peering-backout.yml"),
                 "-e", "peering_backout_id=xac", "-e", f"fed_work={self.tmp}/work/xac",
                 "-e", "fed_edge_group=xac_edge", "-e", "ansible_become=false"], self.tmp)
        self.assertEqual(r.returncode, 0, r.stdout[-6000:] + r.stderr[-2000:])
        rec = json.load(open(f"{self.tmp}/work/xac/backout/survey/edge-xac.json"))
        spire = [r for r in rec["rows"] if r["component"] == "spire" and r["resource"].startswith("federation")]
        self.assertEqual(spire[0]["state"], "unknown")


class RoleDefaults(unittest.TestCase):
    """The literals tasks/peering-backout-names.j2 repeats are the roles' defaults."""

    def test_repeated_defaults_match_the_roles(self):
        names = (ANSIBLE / "tasks/peering-backout-names.j2").read_text()
        pairs = re.findall(r"(peering_[a-z_]+|goat_[a-z_]+) \| default\('([^']*)'\)", names)
        pairs += [(k, int(v)) for k, v in re.findall(r"(peering_[a-z_]+) \| default\((\d+)\)", names)]
        roles = {}
        for d in (ANSIBLE / "roles").glob("*/defaults/main.yml"):
            roles.update({k: v for k, v in (yaml.safe_load(d.read_text()) or {}).items()})
        checked = 0
        for k, v in pairs:
            if k in roles and isinstance(roles[k], (str, int)) and "{{" not in str(roles[k]):
                self.assertEqual(roles[k], v, f"{k}: the back-out repeats {v!r}, the role says {roles[k]!r}")
                checked += 1
        self.assertGreaterEqual(checked, 6)


if __name__ == "__main__":
    unittest.main()
