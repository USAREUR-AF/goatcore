#!/usr/bin/env python3
"""The forward_origins check reads only the peers this site replicates with.

One inventory can declare several goatnets, all under mgmt_hosts. The role's
own two tasks that build the list of hosts to read run here, unchanged, against
an inventory with this site, a peer it names, a site that names it, and a host
of another goatnet that also takes part in Spock. The other goatnet's host must
not be in the list.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
TASKS = ROOT / "ops/ansible/roles/spock-forward-origins-check/tasks/main.yml"
NAMES = ("spock-forward-origins | start with this host", "spock-forward-origins | add each peer that takes part in Spock")

INVENTORY = """
[mgmt_hosts]
here ansible_connection=local site_id=zagros-a inter_site_spock_topology=explicit participates_in_spock=true inter_site_spock_peers='["zagros-b"]'
named ansible_connection=local site_id=zagros-b participates_in_spock=true inter_site_spock_peers='[]'
names_me ansible_connection=local site_id=zagros-c participates_in_spock=true inter_site_spock_peers='["zagros-a"]'
other_goatnet ansible_connection=local site_id=meadow-a participates_in_spock=true inter_site_spock_peers='["meadow-b"]'
"""


def require_ansible_playbook(case):
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class PeerScope(unittest.TestCase):
    def hosts_read(self, extra=()):
        require_ansible_playbook(self)
        tasks = [t for t in yaml.safe_load(TASKS.read_text()) if t.get("name") in NAMES]
        self.assertEqual(len(tasks), 2, "the role builds its host list in these two tasks")
        tasks.append({"name": "show", "ansible.builtin.debug": {"msg": "READ={{ spock_forward_origins_hosts | join(',') }}"}})
        play = [{"hosts": "here", "gather_facts": False, "vars": {"spock_forward_origins_check_peers": True}, "tasks": tasks}]
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "inv.ini").write_text(INVENTORY)
            (Path(d) / "play.yml").write_text(yaml.safe_dump(play))
            r = subprocess.run(["ansible-playbook", "-i", str(Path(d) / "inv.ini"), str(Path(d) / "play.yml"), *extra],
                               capture_output=True, text=True, timeout=300)
        self.assertEqual(r.returncode, 0, r.stdout[-2000:] + r.stderr[-2000:])
        line = next(l for l in r.stdout.splitlines() if "READ=" in l)
        return sorted(line.split("READ=")[1].strip().strip('"').split(","))

    def test_explicit_topology_reads_only_its_own_peers(self):
        self.assertEqual(self.hosts_read(), ["here", "named", "names_me"])

    def test_full_mesh_reads_every_participant(self):
        self.assertEqual(self.hosts_read(["-e", "inter_site_spock_topology=full_mesh"]),
                         ["here", "named", "names_me", "other_goatnet"])


if __name__ == "__main__":
    unittest.main()
