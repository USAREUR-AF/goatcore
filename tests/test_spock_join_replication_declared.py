#!/usr/bin/env python3
"""spock-join-replication-declared.yml: a joining node and its founder declare replication.

join-node phase 13 at a joiner wires Spock on both sides, and the roles that do
it run only on a host whose own variables declare participates_in_spock, an
inter_site_spock_topology other than none and ory_spock_enabled (router_role A).
A host without them is a no-op, so a goatlite founder, which declares none of
them, would get no Ory replication node and the joiner nothing to subscribe to.
The play refuses that (GB-417) before phase 13 changes anything, naming each
missing value and the group_vars it belongs in. It reads variables only.

The play runs here through a real ansible-playbook over an inventory of two
local hosts, with group_vars beside it.
"""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/spock-join-replication-declared.yml"

ALL = {"participates_in_spock": True, "inter_site_spock_topology": "explicit",
       "inter_site_spock_peers": ["zagros"], "ory_spock_enabled": True}
# What a goatlite founder renders (ops/scripts/goatlite-render.py): replication off.
GOATLITE = {"goatnet_founder": True, "participates_in_spock": False}


class ReplicationDeclared(unittest.TestCase):
    def test_the_play_touches_no_host(self):
        doc = yaml.safe_load(PLAY.read_text())
        self.assertEqual(len(doc), 1)
        self.assertIs(doc[0]["gather_facts"], False)
        self.assertNotIn("become", doc[0])
        modules = {k for task in doc[0]["tasks"] for k in task if k.startswith("ansible.builtin.")}
        self.assertEqual(modules, {"ansible.builtin.set_fact", "ansible.builtin.fail", "ansible.builtin.debug"})

    @unittest.skipUnless(shutil.which("ansible-playbook") or os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1",
                         "ansible-playbook is not installed (CI sets GOAT_TEST_REQUIRE_ANSIBLE=1)")
    def test_a_node_or_founder_that_declares_no_replication_is_refused(self):
        def run(founder, joiner, founder_hosts="founder", extra_founder=None):
            with tempfile.TemporaryDirectory() as t:
                t = Path(t)
                (t / "hosts.ini").write_text(
                    "[gamma_hosts]\nnode-x ansible_connection=local\n"
                    f"[zagros_hosts]\n{founder_hosts}\n")
                for group, values in (("zagros_hosts", founder), ("gamma_hosts", joiner)):
                    (t / "group_vars" / group).mkdir(parents=True)
                    (t / "group_vars" / group / "vars.yml").write_text(yaml.safe_dump(values))
                if extra_founder:
                    # A second file in the group's vars directory: Ansible reads it too.
                    (t / "group_vars/zagros_hosts/zz-growth.yml").write_text(extra_founder)
                env = {k: v for k, v in os.environ.items() if not k.startswith("ANSIBLE_")}
                env.update({"ANSIBLE_CONFIG": str(t / "none.cfg"), "ANSIBLE_LOCAL_TEMP": str(t / "tmp"),
                            "ANSIBLE_HOME": str(t / "home"), "ANSIBLE_NOCOLOR": "1"})
                r = subprocess.run(["ansible-playbook", "-i", str(t / "hosts.ini"), str(PLAY),
                                    "-e", "join_check_node=node-x", "-e", "join_check_site=gamma", "-e", "join_check_founder_group=zagros_hosts"],
                                   cwd=t, env=env, capture_output=True, text=True, timeout=300)
                return r.returncode, r.stdout + r.stderr

        rc, out = run(ALL, ALL)
        self.assertEqual(rc, 0, out)
        self.assertIn("OK: ", out)

        # A goatlite founder: every value missing, named with where it goes.
        rc, out = run(GOATLITE, ALL)
        self.assertEqual(rc, 2, out)
        self.assertIn("GB-417", out)
        self.assertIn("founder (ops/ansible/group_vars/zagros_hosts/) lacks participates_in_spock: true, "
                      "an inter_site_spock_topology other than none, ory_spock_enabled: true", out)
        self.assertNotIn("node-x (", out)

        # Growth declared everything but the Ory databases.
        rc, out = run({k: v for k, v in ALL.items() if k != "ory_spock_enabled"}, ALL)
        self.assertEqual(rc, 2, out)
        self.assertIn("founder (ops/ansible/group_vars/zagros_hosts/) lacks ory_spock_enabled: true.", out)

        # The node's own side is judged too.
        rc, out = run(ALL, dict(ALL, inter_site_spock_topology="none"))
        self.assertEqual(rc, 2, out)
        self.assertIn("node-x (ops/ansible/group_vars/gamma_hosts/) lacks an inter_site_spock_topology other than none", out)

        # Judged as the roles judge them: Ansible's bool words, a templated
        # value, and every file of the group's vars directory.
        rc, out = run(dict(ALL, participates_in_spock="yes", ory_spock_enabled="{{ 1 == 1 }}"), ALL)
        self.assertEqual(rc, 0, out)
        rc, out = run(GOATLITE, ALL, extra_founder=yaml.safe_dump(ALL))
        self.assertEqual(rc, 0, out)

        # A B standby does not wire: its values are not asked, and a founder
        # with no A host in the run is refused.
        rc, out = run(ALL, ALL, founder_hosts="founder\nfounder-b router_role=B")
        self.assertEqual(rc, 0, out)
        rc, out = run(ALL, ALL, founder_hosts="founder-b router_role=B")
        self.assertEqual(rc, 2, out)
        self.assertIn("no host of zagros_hosts with router_role A", out)


if __name__ == "__main__":
    unittest.main()
