#!/usr/bin/env python3
"""The zenoh door's client CA is installed on a host whose router has not run yet.

On a new site the public-certificate play runs before the router's play, which is
what writes the CA bundle the door copies. The play's own three door-CA tasks run
here, unchanged, against localhost: with the router's bundle present the door takes
it; without it the door takes the goatnet's CA root from the operator machine
instead of failing on a missing file.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/traefik-acme.yml"
MARK = "zenoh-door client-CA"


def door_tasks():
    tasks = []
    for play in yaml.safe_load(PLAY.read_text()):
        for t in play.get("tasks") or []:
            if MARK in str(t.get("name", "")):
                tasks.append(t)
    return tasks


def require_ansible_playbook(case):
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class DoorCaBeforeRouter(unittest.TestCase):
    def install(self, router_bundle):
        require_ansible_playbook(self)
        tasks = door_tasks()
        self.assertEqual(len(tasks), 4, "directory, stat, copy from the host, copy from the operator machine")
        for t in tasks:
            t.pop("owner", None)
            for mod in ("ansible.builtin.copy", "ansible.builtin.file"):
                if mod in t:
                    t[mod].pop("owner", None); t[mod].pop("group", None)
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            (d / "root.pem").write_text("GOATNET ROOT\n")
            if router_bundle:
                (d / "bundle.pem").write_text("ROUTER BUNDLE\n")
            play = [{"hosts": "localhost", "connection": "local", "gather_facts": False,
                     "vars": {"zenoh_door_ingress_enabled": True, "goat_host_etc_dir": str(d / "etc"),
                              "zenoh_door_client_ca_src": str(d / "bundle.pem"),
                              "zenoh_door_client_ca_root": str(d / "root.pem"),
                              "ansible_python_interpreter": sys.executable},
                     "tasks": tasks}]
            (d / "play.yml").write_text(yaml.safe_dump(play))
            r = subprocess.run(["ansible-playbook", "-i", "localhost,", str(d / "play.yml")],
                               capture_output=True, text=True, timeout=300)
            self.assertEqual(r.returncode, 0, r.stdout[-2000:] + r.stderr[-2000:])
            return (d / "etc/tls/zenoh-door-client-ca.pem").read_text()

    def test_the_router_bundle_is_used_when_it_is_there(self):
        self.assertEqual(self.install(router_bundle=True), "ROUTER BUNDLE\n")

    def test_a_new_site_takes_the_goatnet_root(self):
        self.assertEqual(self.install(router_bundle=False), "GOATNET ROOT\n")


if __name__ == "__main__":
    unittest.main()
