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


def jload(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def leaves(v):
    if isinstance(v, dict):
        for x in v.values():
            yield from leaves(x)
    elif isinstance(v, list):
        for x in v:
            yield from leaves(x)
    elif isinstance(v, str) and v:
        yield v


def run(argv, tmp, path_prefix=None):
    env = dict(os.environ, ANSIBLE_CONFIG=str(ANSIBLE / "ansible.cfg"), ANSIBLE_NOCOLOR="1",
               ANSIBLE_FACT_CACHING="memory", ANSIBLE_LOCAL_TEMP=f"{tmp}/ansible-tmp",
               ANSIBLE_REMOTE_TEMP=f"{tmp}/ansible-tmp")
    if path_prefix:
        env["PATH"] = path_prefix + os.pathsep + env["PATH"]
    return subprocess.run(argv, cwd=ANSIBLE, env=env, capture_output=True, text=True, check=False)


# Measured, nftables 1.1.6: a missing table or chain is exit 1, then nft's own
# not-found line, then the command it could not run.
NFT_NOT_FOUND = ('[ "$1" = -a ] && shift\n'
                 'echo "Error: No such file or directory" >&2; echo "$*" >&2; exit 1')
# The tools every survey test puts on PATH unless it gives its own, so a survey does
# not depend on what this machine's ip, nft and systemctl say without privilege.
NETWORK_NOT_FOUND = {
    "ip": ('case "$*" in\n'
           '  "-o link show") exit 0 ;;\n'
           '  *) eval "name=\\${$#}"; echo "Device \\"$name\\" does not exist." >&2; exit 1 ;;\n'
           'esac'),
    "nft": NFT_NOT_FOUND,
    "systemctl": 'printf "LoadState=not-found\\nActiveState=inactive\\nUnitFileState=\\n"',
}


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
            a = jload(f"{out}/{role}-xab.json")
            b = jload(f"{out}/{role}-xac.json")
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
        # nft as this user cannot read any table; fakes that say each table, chain and
        # link does not exist let the forwarder row carry its note about the other entries.
        bindir = f"{self.tmp}/fake-net"
        os.makedirs(bindir, exist_ok=True)
        for name, body in NETWORK_NOT_FOUND.items():
            with open(f"{bindir}/{name}", "w", encoding="utf-8") as f:
                f.write("#!/bin/sh\n" + body + "\n")
            os.chmod(f"{bindir}/{name}", 0o755)
        r = run(["ansible-playbook", "-i", self.inv, "--limit", "xac_edge:xac_core:peering_forwarders",
                 str(ANSIBLE / "peering-backout.yml"),
                 "-e", "peering_backout_id=xac", "-e", f"fed_work={work}", "-e", "fed_edge_group=xac_edge",
                 "-e", "ansible_become=false",
                 "-e", f"peering_backout_core_seed={fx.seed}", "-e", f"peering_backout_core_org={fx.org}",
                 "-e", f"peering_backout_core_config={fx.live}"], self.tmp, path_prefix=bindir)
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
        self.assertEqual(Path(fx.org).read_text(), fx.vendor + tbc.fragment("xaa", "xab") + tbc.fragment("xaa", "xac"))

    def survey_with_tools(self, tools):
        """A survey of xac with fake tools on PATH: {name: shell body}; ip, nft and
        systemctl answer with their own not-found answers unless the test gives its own."""
        bindir = f"{self.tmp}/fake-bin"
        os.makedirs(bindir, exist_ok=True)
        for name, body in {**NETWORK_NOT_FOUND, **tools}.items():
            path = f"{bindir}/{name}"
            with open(path, "w", encoding="utf-8") as f:
                f.write("#!/bin/sh\n" + body + "\n")
            os.chmod(path, 0o700)
        fx = tbc.Fixture(tbc.fragment("xaa", "xab"), tbc.fragment("xaa", "xac"))
        work = f"{self.tmp}/work/xac"
        r = run(["ansible-playbook", "-i", self.inv, "--limit", "xac_edge:xac_core:peering_forwarders",
                 str(ANSIBLE / "peering-backout.yml"),
                 "-e", "peering_backout_id=xac", "-e", f"fed_work={work}", "-e", "fed_edge_group=xac_edge",
                 "-e", "ansible_become=false",
                 "-e", f"peering_backout_core_seed={fx.seed}", "-e", f"peering_backout_core_org={fx.org}",
                 "-e", f"peering_backout_core_config={fx.live}"], self.tmp, path_prefix=bindir)
        self.assertEqual(r.returncode, 0, r.stdout[-6000:] + r.stderr[-2000:])
        rows = {}
        for host in ("edge-xac", "core-xac", "fwd"):
            for row in jload(f"{work}/backout/survey/{host}.json")["rows"]:
                rows[host + ": " + row["resource"]] = row
        return rows

    PROBES = ("edge-xac: container goat-zenoh-edge-xac", "core-xac: container goat-fed-republisher-xac",
              "fwd: carrier wg-carry1", "edge-xac: federation spiffe://xac.example.org",
              "edge-xac: bundle spiffe://xac.example.org")

    def test_a_probe_that_fails_for_another_reason_is_unknown_not_absent(self):
        denied = 'echo "permission denied while trying to connect to the docker daemon socket" >&2; exit 1'
        rows = self.survey_with_tools({
            "docker": denied,
            "wg": 'echo "Unable to access interface: Operation not permitted" >&2; exit 1'})
        for resource in self.PROBES:
            with self.subTest(resource=resource):
                row = rows[resource]
                self.assertEqual(row["state"], "unknown", row)
                self.assertEqual(row["action"], "none", row)
                self.assertIn("rc 1", row["note"], row)
        self.assertIn("permission denied", rows["edge-xac: container goat-zenoh-edge-xac"]["note"])
        self.assertIn("Operation not permitted", rows["fwd: carrier wg-carry1"]["note"])

    def test_a_missing_tool_is_unknown(self):
        rows = self.survey_with_tools({"docker": "exit 127", "wg": "exit 127"})
        for resource in self.PROBES:
            with self.subTest(resource=resource):
                self.assertEqual(rows[resource]["state"], "unknown", rows[resource])

    def test_each_tools_own_not_found_is_absent(self):
        rows = self.survey_with_tools({
            "docker": ('case "$*" in\n'
                       '  *"federation show"*) echo "rpc error: code = NotFound desc = federation'
                       ' relationship does not exist" >&2; exit 1 ;;\n'
                       '  *"bundle list"*) echo "rpc error: code = NotFound desc = bundle not found" >&2; exit 1 ;;\n'
                       '  *) eval "name=\\${$#}"; echo "Error response from daemon: No such container: $name" >&2; exit 1 ;;\n'
                       'esac'),
            "wg": 'echo "Unable to access interface: No such device" >&2; exit 1'})
        for resource in self.PROBES:
            with self.subTest(resource=resource):
                self.assertEqual(rows[resource]["state"], "absent", rows[resource])
                self.assertEqual(rows[resource]["action"], "none", rows[resource])

    TUNNEL_PROBES = ("edge-xac: interface wg-fed1", "edge-xac: nft table inet goat_peering_wg_fed1",
                     "edge-xac: carrier wg-carry1", "edge-xac: interface ipsec-xaa-xac",
                     "edge-xac: nft clamp rules for ipsec-xaa-xac",
                     "fwd: nft rules for wg-carry1 in the forwarder table")

    UNITS = ("edge-xac: unit goat-peering-wg@wg-fed1.service", "edge-xac: unit goat-peering-link@xaa-xac.service")

    def test_a_unit_whose_bus_cannot_be_reached_is_unknown(self):
        rows = self.survey_with_tools({
            "systemctl": 'echo "Failed to connect to system scope bus via local transport: No such file or directory" >&2; exit 1'})
        for resource in self.UNITS:
            with self.subTest(resource=resource):
                self.assertEqual(rows[resource]["state"], "unknown", rows[resource])
                self.assertEqual(rows[resource]["action"], "none", rows[resource])
                self.assertIn("rc 1", rows[resource]["note"], rows[resource])

    def test_a_unit_that_is_not_loaded_is_absent_and_a_running_one_present(self):
        rows = self.survey_with_tools({})
        for resource in self.UNITS:
            with self.subTest(resource=resource):
                self.assertEqual(rows[resource]["state"], "absent", rows[resource])
        rows = self.survey_with_tools({"systemctl": 'printf "LoadState=loaded\\nActiveState=active\\nUnitFileState=enabled\\n"'})
        self.assertEqual(rows["edge-xac: unit goat-peering-link@xaa-xac.service"]["state"], "present")

    def test_the_survey_shows_this_links_clamp_rules_and_only_them(self):
        clamp = ('table inet goat_peering {', 'chain clamp {',
                 'oifname "ipsec-xaa-xac" tcp flags syn tcp option maxseg size set rt mtu # handle 7',
                 'iifname "ipsec-xaa-xac" tcp flags syn tcp option maxseg size set rt mtu # handle 8',
                 'oifname "ipsec-xaa-xab" tcp flags syn tcp option maxseg size set rt mtu # handle 9', '}', '}')
        rows = self.survey_with_tools({
            "docker": 'eval "name=\\${$#}"; echo "Error response from daemon: No such container: $name" >&2; exit 1',
            "wg": 'echo "Unable to access interface: No such device" >&2; exit 1',
            "nft": ('case "$*" in\n'
                    '  *"chain inet goat_peering clamp"*) cat <<\'EOF\'\n' + "\n".join(clamp) + '\nEOF\n ;;\n'
                    '  *) ' + NFT_NOT_FOUND.replace("\n", "; ") + ' ;;\n'
                    'esac')})
        row = rows["edge-xac: nft clamp rules for ipsec-xaa-xac"]
        self.assertEqual(row["state"], "present", row)
        self.assertEqual(row["action"], "remove", row)
        self.assertIn("handles 7, 8", row["note"], row)

    def test_tunnel_and_table_probes_that_fail_otherwise_are_unknown(self):
        rows = self.survey_with_tools({
            "docker": 'echo "Error response from daemon: No such container: x" >&2; exit 1',
            "wg": 'echo "Unable to access interface: Operation not permitted" >&2; exit 1',
            "nft": 'echo "Operation not permitted (you must be root)" >&2; exit 1',
            "ip": ('case "$*" in *"link show "*) echo "RTNETLINK answers: Operation not permitted" >&2; exit 2 ;; '
                   '*) exit 0 ;; esac')})
        for resource in self.TUNNEL_PROBES:
            with self.subTest(resource=resource):
                self.assertIn(resource, rows, sorted(rows))
                self.assertEqual(rows[resource]["state"], "unknown", rows[resource])
                self.assertEqual(rows[resource]["action"], "none", rows[resource])
                self.assertRegex(rows[resource]["note"], r"rc \d+: ", rows[resource])

    def test_a_table_whose_sharing_cannot_be_checked_is_unknown(self):
        # The table answers, but the list of links that says whether another link
        # shares its name could not be read: removing it could take another's.
        rows = self.survey_with_tools({
            "docker": 'eval "name=\\${$#}"; echo "Error response from daemon: No such container: $name" >&2; exit 1',
            "wg": 'echo "Unable to access interface: No such device" >&2; exit 1',
            "nft": 'printf "table inet x {\\n}\\n"',
            "ip": 'echo "RTNETLINK answers: Operation not permitted" >&2; exit 2'})
        row = rows["edge-xac: nft table inet goat_peering_wg_fed1"]
        self.assertEqual(row["state"], "unknown", row)
        self.assertEqual(row["action"], "none", row)
        self.assertIn("rc 2", row["note"], row)

    def test_tunnel_and_table_probes_read_each_tools_not_found(self):
        rows = self.survey_with_tools({
            "docker": 'eval "name=\\${$#}"; echo "Error response from daemon: No such container: $name" >&2; exit 1',
            "wg": 'echo "Unable to access interface: No such device" >&2; exit 1',
            "nft": NFT_NOT_FOUND,
            "ip": ('case "$*" in *"link show "*) eval "name=\\${$#}"; '
                   'echo "Device \\"$name\\" does not exist." >&2; exit 1 ;; *) exit 0 ;; esac')})
        for resource in self.TUNNEL_PROBES:
            with self.subTest(resource=resource):
                self.assertIn(resource, rows, sorted(rows))
                self.assertEqual(rows[resource]["state"], "absent", rows[resource])
                self.assertEqual(rows[resource]["action"], "none", rows[resource])

    def test_a_probe_that_answers_is_present(self):
        rows = self.survey_with_tools({
            "docker": ('case "$*" in\n'
                       '  *"bundle list"*) echo "-----BEGIN CERTIFICATE-----" ;;\n'
                       '  *) echo "/x running" ;;\n'
                       'esac'),
            "wg": 'echo "Unable to access interface: No such device" >&2; exit 1'})
        # The carrier is left absent here: an interface that answers on this one host
        # is also the edge's carrier, which the WireGuard back-out rightly refuses to
        # claim. ProbeState covers a carrier that answers.
        for resource in self.PROBES[:2] + self.PROBES[3:]:
            with self.subTest(resource=resource):
                self.assertEqual(rows[resource]["state"], "present", rows[resource])
                self.assertEqual(rows[resource]["action"], "remove", rows[resource])
    def test_a_core_backed_out_by_the_driver_is_one_row_and_nothing_runs_on_it(self):
        # FED_CORE_*: admission placed the core's fragment from the driver, so the
        # driver backs it out. The play reports that in one row and reads nothing on
        # the core host, even with core paths that do not exist.
        work = f"{self.tmp}/work/xac"
        missing = f"{self.tmp}/no-such-core"
        r = run(["ansible-playbook", "-i", self.inv, "--limit", "xac_core",
                 str(ANSIBLE / "peering-backout.yml"),
                 "-e", "peering_backout_id=xac", "-e", f"fed_work={work}", "-e", "fed_edge_group=xac_edge",
                 "-e", "ansible_become=false", "-e", "peering_backout_core_by_driver=true",
                 "-e", f"peering_backout_core_seed={missing}/seed", "-e", f"peering_backout_core_org={missing}/org",
                 "-e", f"peering_backout_core_config={missing}/live"], self.tmp)
        self.assertEqual(r.returncode, 0, r.stdout[-6000:] + r.stderr[-2000:])
        rows = jload(f"{work}/backout/survey/core-xac.json")["rows"]
        core = [row for row in rows if row["component"] == "core admission"]
        self.assertEqual(core, [{"component": "core admission", "resource": "the driver (FED_CORE_*)",
                                 "state": "present", "action": "kept",
                                 "note": "surveyed and removed by the driver, the way admission placed it"}])
        # Every core-admission task but the one that writes that row was skipped.
        ran = []
        for section in r.stdout.split("\nTASK [")[1:]:
            name, _, body = section.partition("]")
            if name.startswith("core | absent |") and re.search(r"^(ok|changed|fatal|failed): \[core-xac", body, re.M):
                ran.append(name)
        self.assertEqual(ran, ["core | absent | the core is backed out by the driver"], r.stdout[-4000:])
        self.assertFalse(os.path.exists(f"{work}/backout/core/core-xac"))

    def test_by_the_driver_a_role_declared_peering_is_still_reported(self):
        # A core that also declares the partner in zenoh_sovereign_peerings keeps
        # admitting it after the driver backs out the splice: the row that says so
        # (GB-227) must survive by_driver, read from the core host's declarations
        # without touching the host.
        cv = f"{self.tmp}/inventory/group_vars/xac_core.yml"
        with open(cv, encoding="utf-8") as f:
            values = yaml.safe_load(f)
        values["zenoh_sovereign_peerings"] = [{"sovereign": "xac", "cn": "xac-peer", "release_key_exprs": []},
                                              {"sovereign": "xab", "cn": "xab-peer", "release_key_exprs": []}]
        dump(cv, values)
        work = f"{self.tmp}/work/xac"
        r = run(["ansible-playbook", "-i", self.inv, "--limit", "xac_core",
                 str(ANSIBLE / "peering-backout.yml"),
                 "-e", "peering_backout_id=xac", "-e", f"fed_work={work}", "-e", "fed_edge_group=xac_edge",
                 "-e", "ansible_become=false", "-e", "peering_backout_core_by_driver=true"], self.tmp)
        self.assertEqual(r.returncode, 0, r.stdout[-6000:] + r.stderr[-2000:])
        core = [row for row in jload(f"{work}/backout/survey/core-xac.json")["rows"]
                if row["component"] == "core admission"]
        self.assertEqual(core, [
            {"component": "core admission", "resource": "the driver (FED_CORE_*)", "state": "present",
             "action": "kept", "note": "surveyed and removed by the driver, the way admission placed it"},
            {"component": "core admission", "resource": "zenoh_sovereign_peerings[sovereign=xac]",
             "state": "present", "action": "kept",
             "note": "configuration: remove the entry by review and run the core play (a router restart)"}])
        ran = []
        for section in r.stdout.split("\nTASK [")[1:]:
            name, _, body = section.partition("]")
            if name.startswith("core | absent |") and re.search(r"^(ok|changed|fatal|failed): \[core-xac", body, re.M):
                ran.append(name)
        self.assertEqual(ran, ["core | absent | the core is backed out by the driver"], r.stdout[-4000:])

    def test_a_survey_without_the_rendered_values_or_agreement_still_runs(self):
        # A peering backed out before phase 9: no agreement, no applied values.
        shutil.rmtree(f"{self.tmp}/work/xac")
        os.makedirs(f"{self.tmp}/work/xac")
        r = run(["ansible-playbook", "-i", self.inv, "--limit", "xac_edge",
                 str(ANSIBLE / "peering-backout.yml"),
                 "-e", "peering_backout_id=xac", "-e", f"fed_work={self.tmp}/work/xac",
                 "-e", "fed_edge_group=xac_edge", "-e", "ansible_become=false"], self.tmp)
        self.assertEqual(r.returncode, 0, r.stdout[-6000:] + r.stderr[-2000:])
        rec = jload(f"{self.tmp}/work/xac/backout/survey/edge-xac.json")
        spire = [r for r in rec["rows"] if r["component"] == "spire" and r["resource"].startswith("federation")]
        self.assertEqual(spire[0]["state"], "unknown")


FILTER = ANSIBLE / "filter_plugins/backout_probe.py"


class ProbeState(unittest.TestCase):
    """The rule every back-out probe follows: present on rc 0, absent only on the
    tool's own not-found line for the probed name, unknown with the rc and the
    error otherwise."""

    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("backout_probe", FILTER)
        cls.mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.mod)

    def state(self, kind, rc, stderr="", stdout="", name="goat-zenoh-edge-xac"):
        return self.mod.backout_probe_state({"rc": rc, "stdout": stdout, "stderr": stderr}, kind, name)

    def test_each_tools_own_not_found_line_is_absent(self):
        for kind, rc, err, name in (
                # Measured: docker 29 and podman 5.7 against a container that does not exist.
                ("docker", 1, "\nError response from daemon: No such container: goat-zenoh-edge-xac", "goat-zenoh-edge-xac"),
                ("docker", 1, "Error: No such object: goat-zenoh-edge-xac", "goat-zenoh-edge-xac"),
                ("docker", 125, 'Error: no such container "goat-zenoh-edge-xac"', "goat-zenoh-edge-xac"),
                ("wg", 1, "Unable to access interface: No such device", "wg-carry1"),
                ("spire", 1, "rpc error: code = NotFound desc = federation relationship does not exist", "xac.example.org"),
                ("spire", 1, "Error: rpc error: code = NotFound desc = bundle not found", "xac.example.org"),
                ("spire-bundle", 1, "rpc error: code = NotFound desc = bundle not found", "xac.example.org")):
            with self.subTest(kind=kind, err=err):
                self.assertEqual(self.state(kind, rc, err, name=name)["state"], "absent")

    def test_a_not_found_that_is_not_the_tools_own_line_is_unknown(self):
        for kind, rc, err, name in (
                # Another error that mentions the words, or another name.
                ("docker", 1, "Error response from daemon: error while inspecting: No such object: layer sha256:ab",
                 "goat-zenoh-edge-xac"),
                ("docker", 1, "Error response from daemon: No such container: goat-zenoh-edge-xab", "goat-zenoh-edge-xac"),
                ("docker", 1, "Error response from daemon: No such container: goat-zenoh-edge-xac-old",
                 "goat-zenoh-edge-xac"),
                ("docker", 125, "Error response from daemon: No such container: goat-zenoh-edge-xac",
                 "goat-zenoh-edge-xac"),
                ("docker", 1, 'Error: no such container "goat-zenoh-edge-xac"', "goat-zenoh-edge-xac"),
                ("docker", 2, "Error: No such container: goat-zenoh-edge-xac", "goat-zenoh-edge-xac"),
                ("wg", 1, "Unable to access interface: No such device or address", "wg-carry1"),
                ("wg", 1, "Impossible d'acceder a l'interface : Aucun peripherique de ce type", "wg-carry1"),
                ("spire", 1, "rpc error: code = Unavailable desc = connection error: code = NotFound", "xac.example.org"),
                ("spire", 1, "dial failed\nrpc error: code = NotFound desc = x", "xac.example.org")):
            with self.subTest(kind=kind, err=err, name=name):
                got = self.state(kind, rc, err, name=name)
                self.assertEqual(got["state"], "unknown", got)
                self.assertIn("rc %d" % rc, got["note"])

    def test_anything_else_is_unknown_with_the_rc_and_first_error_line(self):
        for kind, rc, err in (("docker", 1, "permission denied while trying to connect\nmore"),
                              ("docker", 127, ""), ("docker", 255, "ssh: connect to host"),
                              ("wg", 1, "Unable to access interface: Operation not permitted"),
                              ("spire", 1, "Error: No such container: spire-server"),
                              ("spire", 126, "OCI runtime exec failed")):
            with self.subTest(kind=kind, rc=rc, err=err):
                got = self.state(kind, rc, err)
                self.assertEqual(got["state"], "unknown")
                self.assertIn("rc %d" % rc, got["note"])
                self.assertNotIn("\n", got["note"])

    def test_a_unit_is_read_by_its_load_state(self):
        # Measured: systemd 259, `systemctl show -p LoadState -p ActiveState -p
        # UnitFileState <unit>`. A unit that does not exist is LoadState=not-found
        # with exit 0; a bus that cannot be reached is exit 1 and an error line.
        def unit(rc, out="", err=""):
            return self.state("systemd", rc, err, stdout=out, name="goat-peering-wg@wg-fed1.service")
        missing = unit(0, "LoadState=not-found\nActiveState=inactive\nUnitFileState=\n")
        self.assertEqual(missing["state"], "absent", missing)
        bus = unit(1, err="Failed to connect to system scope bus via local transport: No such file or directory")
        self.assertEqual(bus["state"], "unknown", bus)
        self.assertIn("rc 1", bus["note"])
        self.assertEqual(unit(0, "LoadState=loaded\nActiveState=active\nUnitFileState=enabled\n")["state"], "present")
        self.assertEqual(unit(0, "LoadState=loaded\nActiveState=inactive\nUnitFileState=enabled\n")["state"], "present")
        self.assertEqual(unit(0, "LoadState=loaded\nActiveState=activating\nUnitFileState=disabled\n")["state"], "present")
        # Running from a unit file that was since deleted: still there.
        self.assertEqual(unit(0, "LoadState=not-found\nActiveState=active\nUnitFileState=\n")["state"], "present")
        # A template instance that was never started loads from its template but is
        # neither running nor enabled: not this peering's link (as before).
        self.assertEqual(unit(0, "LoadState=loaded\nActiveState=inactive\nUnitFileState=disabled\n")["state"], "absent")
        for out in ("LoadState=masked\nActiveState=inactive\nUnitFileState=masked\n",
                    "LoadState=error\nActiveState=inactive\n", "ActiveState=inactive\n", ""):
            with self.subTest(out=out):
                self.assertEqual(unit(0, out)["state"], "unknown", out)

    def test_a_probe_that_did_not_run_is_unknown(self):
        self.assertEqual(self.mod.backout_probe_state({"skipped": True}, "docker", "x")["state"], "unknown")
        self.assertEqual(self.mod.backout_probe_state(None, "docker", "x")["state"], "unknown")

    def test_nft_and_ip_not_found_lines(self):
        # Measured in a user and network namespace: nft 1.1 and iproute2.
        nft_missing = "Error: No such file or directory\nlist table inet goat_x\n                ^^^^^^"
        self.assertEqual(self.state("nft", 1, nft_missing, name="goat_x")["state"], "absent")
        chain_missing = "Error: No such file or directory\nlist chain inet goat_x clamp\n                   ^^^^^"
        self.assertEqual(self.state("nft", 1, chain_missing, name="goat_x")["state"], "absent")
        self.assertEqual(self.state("ip-link", 1, 'Device "wg-fed1" does not exist.', name="wg-fed1")["state"],
                         "absent")
        for kind, rc, err, name in (
                ("nft", 1, "Operation not permitted (you must be root)\nnetlink: Error: cache initialization failed",
                 "goat_x"),
                ("nft", 1, "Error: Could not process rule: No such file or directory", "goat_x"),
                # nft writes the command it could not run on the next line: absent
                # only when that command names the probed table.
                ("nft", 1, "Error: No such file or directory\nlist table inet goat_y", "goat_x"),
                ("nft", 1, "Error: No such file or directory\nlist table inet goat_x_old", "goat_x"),
                ("nft", 1, "Error: No such file or directory", "goat_x"),
                ("nft", 127, "", "goat_x"),
                ("ip-link", 1, 'Device "wg-fed2" does not exist.', "wg-fed1"),
                ("ip-link", 255, "RTNETLINK answers: Operation not permitted", "wg-fed1")):
            with self.subTest(kind=kind, err=err):
                self.assertEqual(self.state(kind, rc, err, name=name)["state"], "unknown")

    def test_a_spire_bundle_list_reads_its_answer(self):
        self.assertEqual(self.state("spire-bundle", 0, stdout="-----BEGIN CERTIFICATE-----")["state"], "present")
        # A bundle holding only JWT authorities lists no certificate: it is there,
        # so it is not absent; the probe cannot show it, so it is unknown.
        jwt_only = self.state("spire-bundle", 0, stdout="")
        self.assertEqual(jwt_only["state"], "unknown")
        self.assertIn("bundle has no X.509 authority", jwt_only["note"])
        self.assertEqual(self.state("spire-bundle", 1, "permission denied")["state"], "unknown")


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

    def test_the_forwarder_entry_defaults_match_the_role(self):
        text = (ANSIBLE / "roles/goat-peering-forwarder/tasks/absent.yml").read_text()
        m = re.search(r"set role_defaults = (\{.*?\}) -%\}", text, re.S)
        self.assertIsNotNone(m)
        repeated = yaml.safe_load(re.sub(r"\s+", " ", m.group(1)))
        role = yaml.safe_load((ANSIBLE / "roles/goat-peering-forwarder/defaults/main.yml").read_text())
        self.assertEqual(repeated, role["peering_forwarder_peering_defaults"])


if __name__ == "__main__":
    unittest.main()
