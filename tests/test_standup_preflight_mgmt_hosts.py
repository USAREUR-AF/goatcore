#!/usr/bin/env python3
"""A management-stack group missing from [mgmt_hosts:children] fails the
config preflight, for the site checked and for the founder it joins: Spock
peer discovery walks groups['mgmt_hosts'] and would see neither."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("preflight", ROOT / "ops/scripts/standup-config-preflight.py")
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)


class MgmtHostsMembership(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self._tmp.name)
        self.gv = self.repo / "ops/ansible/group_vars"
        self.inv = self.repo / "ops/ansible/inventory/mgmt-hosts.ini"
        self.inv.parent.mkdir(parents=True)
        for g, body in (("zagros_hosts", "mgmt_stack_profile: full\n"),
                        ("zagros_b_hosts", "has_mgmt_api: true\njoin_founder_site: zagros\n"),
                        ("zagros_c_hosts", "mgmt_stack_profile: postgres-spock-only\n")):
            (self.gv / g).mkdir(parents=True)
            (self.gv / g / "vars.yml").write_text(body)
        self._saved = (preflight.REPO, preflight.GV, list(preflight.FAILS))
        preflight.REPO, preflight.GV = self.repo, self.gv
        preflight.FAILS.clear()

    def tearDown(self):
        preflight.REPO, preflight.GV = self._saved[:2]
        preflight.FAILS[:] = self._saved[2]
        self._tmp.cleanup()

    def check(self, group):
        preflight.FAILS.clear()
        preflight.check_mgmt_hosts_membership(group, preflight.load_group(group)[0])
        return list(preflight.FAILS)

    def test_both_listed_passes(self):
        self.inv.write_text("[mgmt_hosts:children]\nzagros_hosts\nzagros_b_hosts  # joiner\n\n[zagros_hosts]\n")
        self.assertEqual(self.check("zagros_b_hosts"), [])

    def test_a_founder_missing_from_mgmt_hosts_fails_the_joiner_with_the_fix(self):
        self.inv.write_text("[mgmt_hosts:children]\nzagros_b_hosts\n")
        fails = self.check("zagros_b_hosts")
        self.assertEqual(len(fails), 1, fails)
        self.assertIn("zagros_hosts runs the management stack", fails[0])
        self.assertIn("Add the line `zagros_hosts` under [mgmt_hosts:children]", fails[0])

    def test_no_section_fails_closed(self):
        self.inv.write_text("[zagros_hosts]\n")
        self.assertIn("has no [mgmt_hosts:children]", self.check("zagros_hosts")[0])
        self.inv.unlink()
        self.assertIn("has no [mgmt_hosts:children]", self.check("zagros_hosts")[0])

    def test_a_group_without_the_management_stack_is_not_required(self):
        self.inv.write_text("[mgmt_hosts:children]\nzagros_hosts\n")
        self.assertEqual(self.check("zagros_c_hosts"), [])


if __name__ == "__main__":
    unittest.main()
