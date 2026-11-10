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


if __name__ == "__main__":
    unittest.main(verbosity=1)
