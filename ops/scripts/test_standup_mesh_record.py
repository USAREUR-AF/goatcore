#!/usr/bin/env python3
"""Tests for standup-mesh-record.py (standup phase 8, second step).

Run: python3 ops/scripts/test_standup_mesh_record.py
"""
import importlib.util
import json
import stat
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("smr", HERE / "standup-mesh-record.py")
smr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smr)

GV = """site_id: demo
site_enrollment_dns: "netbird.demo.example"
mesh_ip_a: "100.127.0.41"   # old
"""
INV = """[other_hosts]
mgmt-demo-01 ansible_host=10.0.0.9

[demo_hosts]
mgmt-demo-01 ansible_host=TBD_DEMO_A_MESH_IP ansible_connection=ssh
"""
MANIFEST = """site_id: demo
out_dir: {out}
leaves:
  - name: router
    cn: goat-zenoh-router
    sans:
      - goat-zenoh-router
      - 100.127.0.41        # mesh IP
      - TBD_DEMO_MESH_IP_B
      - 127.0.0.1
  - name: portal
    sans:
      - 100.127.0.41
"""
MINT = """#!/bin/sh
# stands in for mint-site-certs.sh: records its arguments, writes the pair
echo "$@" > "{log}"
touch "{out}/router-cert.pem" "{out}/router-key.pem"
"""


class Pure(unittest.TestCase):
    def test_set_gv_replaces_then_appends(self):
        t, prev = smr.set_gv(GV, "mesh_ip_a", "100.127.0.50", "n")
        self.assertEqual(prev, "100.127.0.41")
        self.assertIn('mesh_ip_a: "100.127.0.50"   # n', t)
        self.assertNotIn("100.127.0.41", t)
        t, prev = smr.set_gv(t, "mesh_ip_b", "100.127.0.54", "n")
        self.assertEqual(prev, "")
        self.assertTrue(t.endswith('mesh_ip_b: "100.127.0.54"   # n\n'))

    def test_inventory_only_touches_the_site_section(self):
        t, st = smr.set_mesh_inventory(INV, "demo_hosts", "mgmt-demo-01", "100.127.0.50")
        self.assertEqual(st, "set")
        self.assertIn("mgmt-demo-01 ansible_host=10.0.0.9", t)
        self.assertIn("mgmt-demo-01 ansible_host=100.127.0.50 ansible_connection=ssh", t)
        self.assertEqual(smr.set_mesh_inventory(t, "demo_hosts", "mgmt-demo-01", "100.127.0.50")[1], "same")
        self.assertEqual(smr.set_mesh_inventory(t, "demo_hosts", "mgmt-demo-02", "100.127.0.51")[1], "absent")

    def test_only_the_router_leaf_moves(self):
        t, n = smr.replace_router_sans(MANIFEST, {"100.127.0.41": "100.127.0.50", "": "100.127.0.63"})
        self.assertEqual(n, 1)
        self.assertIn("      - 100.127.0.50        # mesh IP", t)
        self.assertIn("  - name: portal\n    sans:\n      - 100.127.0.41", t)


class Run(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        r = Path(self.tmp.name)
        self.repo, self.out = r / "repo", r / "leaves"
        self.out.mkdir()
        (self.repo / "ops/ansible/group_vars/demo_hosts").mkdir(parents=True)
        (self.repo / "ops/ansible/inventory").mkdir(parents=True)
        (self.repo / "ops/enrollment/manifests").mkdir(parents=True)
        (self.repo / "ops/scripts").mkdir(parents=True)
        (self.repo / "ops/ansible/group_vars/demo_hosts/vars.yml").write_text(GV)
        (self.repo / "ops/ansible/inventory/mgmt-hosts-mesh.ini").write_text(INV)
        (self.repo / "ops/enrollment/manifests/demo-certs.yaml").write_text(MANIFEST.format(out=self.out))
        self.log = r / "mint.log"
        mint = self.repo / "ops/scripts/mint-site-certs.sh"
        mint.write_text(MINT.format(log=self.log, out=self.out))
        mint.chmod(mint.stat().st_mode | stat.S_IEXEC)
        self.ips = r / "ips.json"
        self.ips.write_text(json.dumps({"mgmt-demo-01": "100.127.0.50", "mgmt-demo-02": "100.127.0.51"}))
        smr.site_hosts = lambda repo, group: ["mgmt-demo-01", "mgmt-demo-02"]

    def tearDown(self):
        self.tmp.cleanup()

    def run_it(self, *extra):
        return smr.main(["demo", "--ips", str(self.ips), "--repo", str(self.repo), *extra])

    def test_records_everything_and_remints_once(self):
        self.assertEqual(self.run_it(), 0)
        gv = (self.repo / "ops/ansible/group_vars/demo_hosts/vars.yml").read_text()
        self.assertIn('mesh_ip_a: "100.127.0.50"', gv)
        self.assertIn('mesh_ip_b: "100.127.0.51"', gv)
        self.assertIn("ansible_host=100.127.0.50", (self.repo / "ops/ansible/inventory/mgmt-hosts-mesh.ini").read_text())
        self.assertIn("- 100.127.0.50        # mesh IP", (self.repo / "ops/enrollment/manifests/demo-certs.yaml").read_text())
        self.assertEqual(self.log.read_text().split(), ["demo", "--only", "router"])
        self.log.unlink()
        self.assertEqual(self.run_it(), 0)
        self.assertFalse(self.log.exists(), "a second run with the same IPs mints nothing")

    def test_a_non_mesh_address_is_refused(self):
        self.ips.write_text(json.dumps({"mgmt-demo-01": "192.168.1.5"}))
        self.assertEqual(self.run_it(), 1)

    def test_an_ip_file_naming_no_site_host_is_refused(self):
        self.ips.write_text(json.dumps({"elsewhere-01": "100.127.0.50"}))
        self.assertEqual(self.run_it(), 1)

    def test_no_mint_leaves_the_certificate(self):
        self.assertEqual(self.run_it("--no-mint"), 0)
        self.assertFalse(self.log.exists())

    def test_a_second_node_joining_keeps_the_first_hosts_address(self):
        # join-node records only the node it enrolled: the second host of the
        # pair takes mesh_ip_b, and mesh_ip_a stays the first host's.
        self.ips.write_text(json.dumps({"mgmt-demo-02": "100.127.0.51"}))
        self.assertEqual(self.run_it("--no-mint"), 0)
        gv = (self.repo / "ops/ansible/group_vars/demo_hosts/vars.yml").read_text()
        self.assertIn('mesh_ip_a: "100.127.0.41"', gv)
        self.assertIn('mesh_ip_b: "100.127.0.51"', gv)

    def test_a_node_of_another_class_sets_only_its_inventory_line(self):
        (self.repo / "ops/ansible/inventory/mgmt-hosts-mesh.ini").write_text(INV + "\n[demo_obs_hosts]\nobs-demo-01 ansible_host=TBD_DEMO_OBS_MESH_IP\n")
        smr.site_hosts = lambda repo, group: ["obs-demo-01"] if group == "demo_obs_hosts" else ["mgmt-demo-01", "mgmt-demo-02"]
        self.ips.write_text(json.dumps({"obs-demo-01": "100.127.0.60"}))
        self.assertEqual(self.run_it("--group", "demo_obs_hosts"), 0)
        self.assertEqual((self.repo / "ops/ansible/group_vars/demo_hosts/vars.yml").read_text(), GV)
        self.assertIn("obs-demo-01 ansible_host=100.127.0.60", (self.repo / "ops/ansible/inventory/mgmt-hosts-mesh.ini").read_text())
        self.assertFalse(self.log.exists(), "nothing is minted for another class")

    def test_a_node_with_no_mesh_line_gains_one_only_when_asked(self):
        # join-node reaches the node at its mesh address from phase 12, so it
        # asks for the line; without --add-missing nothing is added, as before.
        inv = self.repo / "ops/ansible/inventory/mgmt-hosts-mesh.ini"
        inv.write_text(INV)
        smr.site_hosts = lambda repo, group: ["bg-demo-1"] if group == "demo_breakglass_hosts" else ["mgmt-demo-01", "mgmt-demo-02"]
        self.ips.write_text(json.dumps({"bg-demo-1": "100.127.0.70"}))
        self.assertEqual(self.run_it("--group", "demo_breakglass_hosts", "--no-mint"), 0)
        self.assertEqual(inv.read_text(), INV)
        self.assertEqual(self.run_it("--group", "demo_breakglass_hosts", "--add-missing", "--no-mint"), 0)
        text = inv.read_text()
        self.assertTrue(text.startswith(INV), text)
        self.assertIn("[demo_breakglass_hosts]\nbg-demo-1 ansible_host=100.127.0.70\n", text)
        # A second run finds the line and changes nothing.
        self.assertEqual(self.run_it("--group", "demo_breakglass_hosts", "--add-missing", "--no-mint"), 0)
        self.assertEqual(inv.read_text(), text)
        # With no mesh inventory at all, it is made.
        inv.unlink()
        self.assertEqual(self.run_it("--group", "demo_breakglass_hosts", "--add-missing", "--no-mint"), 0)
        self.assertEqual(inv.read_text(), "[demo_breakglass_hosts]\nbg-demo-1 ansible_host=100.127.0.70\n")

    def test_a_missing_line_goes_at_the_end_of_its_groups_section(self):
        text = "[a_hosts]\nx ansible_host=100.127.0.1\n\n[b_hosts]\ny ansible_host=100.127.0.2\n\n[c_hosts]\nz\n"
        out = smr.add_mesh_line(text, "b_hosts", "w", "100.127.0.3")
        self.assertEqual(out, "[a_hosts]\nx ansible_host=100.127.0.1\n\n[b_hosts]\ny ansible_host=100.127.0.2\nw ansible_host=100.127.0.3\n\n[c_hosts]\nz\n")
        self.assertEqual(smr.add_mesh_line("[a_hosts]\nx", "b_hosts", "w", "100.127.0.3"), "[a_hosts]\nx\n\n[b_hosts]\nw ansible_host=100.127.0.3\n")
        self.assertEqual(smr.add_mesh_line("", "b_hosts", "w", "100.127.0.3"), "[b_hosts]\nw ansible_host=100.127.0.3\n")

    def test_no_mint_with_no_manifest_is_not_a_refusal(self):
        (self.repo / "ops/enrollment/manifests/demo-certs.yaml").unlink()
        self.assertEqual(self.run_it("--no-mint"), 0)
        self.assertEqual(self.run_it(), 1)


if __name__ == "__main__":
    unittest.main(verbosity=1)
