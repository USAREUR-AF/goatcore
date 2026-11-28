"""standup-relay-health-record.py writes the box's mesh address where it is read:
the box's inventory line, the site's group, and the obs group where it exists."""
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "ops/scripts/standup-relay-health-record.py"


class Record(unittest.TestCase):
    def setUp(self):
        self.d = pathlib.Path(tempfile.mkdtemp())
        (self.d / "ops/ansible/inventory").mkdir(parents=True)
        (self.d / "ops/ansible/group_vars/x_hosts").mkdir(parents=True)
        (self.d / "ops/ansible/group_vars/x_hosts/vars.yml").write_text("site_id: x\n")
        (self.d / "ips.json").write_text(json.dumps({"box": "100.127.0.9"}))

    def run_it(self):
        env = dict(os.environ, GOAT_STANDUP_STATE_DIR=str(self.d / "state"))
        return subprocess.run([sys.executable, str(SCRIPT), "x", "--ips", str(self.d / "ips.json"), "--repo", str(self.d)],
                              capture_output=True, text=True, env=env)

    def test_no_box_declared_is_not_a_refusal(self):
        r = self.run_it()
        self.assertEqual(r.returncode, 0)
        self.assertIn("declares no relay-health box", r.stdout)

    def test_the_address_lands_in_all_three_places(self):
        inv = self.d / "ops/ansible/inventory/x-relay-health.ini"
        inv.write_text("[x_relay_health]\nbox ansible_host=i-0abc mesh_ip_a=TBD_RELAY_HEALTH_MESH_IP\n")
        obs = self.d / "ops/ansible/group_vars/x_obs_hosts"
        obs.mkdir()
        (obs / "vars.yml").write_text("relay_health_mesh_ip: TBD_RELAY_HEALTH_MESH_IP\n")
        r = self.run_it()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("mesh_ip_a=100.127.0.9", inv.read_text())
        self.assertIn('relay_health_mesh_ip: "100.127.0.9"', (self.d / "ops/ansible/group_vars/x_hosts/vars.yml").read_text())
        self.assertIn('relay_health_mesh_ip: "100.127.0.9"', (obs / "vars.yml").read_text())
        self.assertEqual(self.run_it().returncode, 0)   # idempotent

    def test_an_address_off_the_mesh_range_is_refused(self):
        (self.d / "ops/ansible/inventory/x-relay-health.ini").write_text("[x_relay_health]\nbox ansible_host=i-0abc\n")
        (self.d / "ips.json").write_text(json.dumps({"box": "10.0.0.5"}))
        self.assertEqual(self.run_it().returncode, 1)


if __name__ == "__main__":
    unittest.main()
