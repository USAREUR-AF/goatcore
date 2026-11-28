#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_relay_health_host_guards.py: `goatbuilder deploy` runs a component's play
# on every member group of a goatnet that lists it. The two relay-health plays
# therefore guard the host themselves:
#   - relay-health-cloud-deploy.yml ends a host that is in no <site>_relay_health
#     group (a mgmt host), with one line, and runs the role on the box that is;
#   - relay-health-deploy.yml ends a host that is in a <site>_relay_health group
#     (the cloud box), with one line, and goes on to its checks on a mgmt host.
#
# Each play runs for real against localhost (connection local, no become), once
# with localhost in zagros_hosts and once in zagros_relay_health. Skips without
# ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1.

import os
import re
import shutil
import subprocess
import tempfile
import unittest

REPO = os.path.normpath(os.path.join(os.path.dirname(__file__), ".."))
ANSIBLE = os.path.join(REPO, "ops", "ansible")
CLOUD_LINE = "the cloud dashboard was not deployed here"
MESH_LINE = "the mesh dashboard was not deployed here"


def require_ansible_playbook(case):
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


def run(tmp, play, group, *extra):
    inv = os.path.join(tmp, "inv.ini")
    with open(inv, "w") as f:
        f.write("[%s]\nlocalhost ansible_connection=local\n" % group)
    env = dict(os.environ, ANSIBLE_ROLES_PATH=os.path.join(ANSIBLE, "roles"),
               ANSIBLE_LOCAL_TEMP=os.path.join(tmp, "ansible-tmp"), ANSIBLE_NOCOLOR="1",
               ANSIBLE_RETRY_FILES_ENABLED="0", ANSIBLE_CACHE_PLUGIN="memory")
    r = subprocess.run(
        ["ansible-playbook", "-i", inv, os.path.join(ANSIBLE, play), "-e", "target=" + group,
         "-e", "ansible_become=false", "-e", "rollback_ledger_dir=" + os.path.join(tmp, "ledger")] + list(extra),
        cwd=ANSIBLE, env=env, capture_output=True, text=True, timeout=300)
    return r, r.stdout + r.stderr


def tasks(out):
    return [l[len("TASK ["):l.rfind("]")] for l in out.splitlines() if l.startswith("TASK [")]


def ended_quietly(case, r, out, line):
    case.assertEqual(r.returncode, 0, out)
    case.assertRegex(out, r"localhost\s*:\s*ok=\d+\s+changed=0\s+unreachable=0\s+failed=0", out)
    case.assertIn(line, out)
    case.assertFalse([t for t in tasks(out) if t.startswith(("relay-health :", "rollback-ledger :")) or "Hydra" in t],
                     "nothing after the guard runs: %s" % tasks(out))


class Guards(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def test_cloud_play_ends_a_mgmt_host(self):
        r, out = run(self.tmp, "relay-health-cloud-deploy.yml", "zagros_hosts")
        ended_quietly(self, r, out, CLOUD_LINE)

    def test_cloud_play_runs_the_role_on_the_relay_health_box(self):
        _, out = run(self.tmp, "relay-health-cloud-deploy.yml", "zagros_relay_health")
        self.assertNotIn(CLOUD_LINE, out)
        self.assertTrue([t for t in tasks(out) if t.startswith(("relay-health :", "rollback-ledger :"))], out)

    def test_cloud_play_hands_its_pinned_image_to_the_role(self):
        # The relay-health-cloud pins row governs its own var (a var belongs to
        # one component); the play hands it to the role's usual one.
        image = "example.org/oauth2-proxy@sha256:" + "a" * 64
        _, out = run(self.tmp, "relay-health-cloud-deploy.yml", "zagros_relay_health",
                     "-e", "relay_health_cloud_oauth2_proxy_image=" + image, "-v")
        self.assertRegex(out, r'"relay_health_oauth2_proxy_image":\s*"%s"' % re.escape(image), out)

    def test_mesh_play_ends_the_relay_health_box(self):
        r, out = run(self.tmp, "relay-health-deploy.yml", "zagros_relay_health")
        ended_quietly(self, r, out, MESH_LINE)

    def test_mesh_play_goes_on_to_its_checks_on_a_mgmt_host(self):
        _, out = run(self.tmp, "relay-health-deploy.yml", "zagros_hosts")
        self.assertNotIn(MESH_LINE, out)
        self.assertIn("Verify Ory Hydra admin reachable on loopback", tasks(out), out)


if __name__ == "__main__":
    unittest.main()
