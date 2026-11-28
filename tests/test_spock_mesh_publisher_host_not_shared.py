#!/usr/bin/env python3
"""spock-mesh-peering refuses a publisher host this site shares with a peer.

Each peer subscribes to this site at this site's publisher host, which comes
from signal_uri when spock_mesh_publisher_host is unset. A joining site whose
signal_uri names its founder's enrolment name advertised the founder's own
host, and the founder's sub_create then pointed at itself and failed, censored,
after the joiner's own subscription already existed (measured in a VM founder
run, join-node phase 13).

The role's own tasks run here unchanged: the one that derives the local
publisher host, and the refusal, with a peer list as the role builds it.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
TASKS = ROOT / "ops/ansible/roles/spock-mesh-peering/tasks/main.yml"
NAMES = ("spock-mesh | derive local Spock node name from site shape",
         "spock-mesh | refuse a publisher host this site shares with a peer")


def require_ansible_playbook(case):
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class PublisherHostNotShared(unittest.TestCase):
    def run_tasks(self, hostvars):
        require_ansible_playbook(self)
        tasks = [t for t in yaml.safe_load(TASKS.read_text()) if t.get("name") in NAMES]
        self.assertEqual([t["name"] for t in tasks], list(NAMES), "the role derives the host, then refuses a shared one")
        peers = [{"site_id": "zagros", "publisher_host": "netbird.zagros.example.org", "node_name": "zagros"}]
        play = [{"hosts": "here", "gather_facts": False,
                 "vars": dict({"site_id": "beta", "spock_mesh_target_peers": peers}, **hostvars),
                 "tasks": tasks}]
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "inv.ini").write_text("[mgmt_hosts]\nhere ansible_connection=local ansible_host=192.0.2.20\n")
            (Path(d) / "play.yml").write_text(yaml.safe_dump(play))
            env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_RETRY_FILES_ENABLED="0")
            return subprocess.run(["ansible-playbook", "-i", str(Path(d) / "inv.ini"), str(Path(d) / "play.yml")],
                                  capture_output=True, text=True, env=env, timeout=120)

    def test_a_joiner_whose_signal_uri_names_its_founder_is_refused(self):
        r = self.run_tasks({"signal_uri": "netbird.zagros.example.org:443"})
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("GB-441 beta's Spock publisher host", r.stdout)
        self.assertIn("is also the publisher host of zagros", r.stdout)
        self.assertIn("spock_mesh_publisher_host", r.stdout)

    def test_its_own_publisher_host_passes(self):
        r = self.run_tasks({"signal_uri": "netbird.zagros.example.org:443", "spock_mesh_publisher_host": "100.127.0.3"})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        r = self.run_tasks({"signal_uri": "netbird.beta.example.org:443"})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
