#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_names_collector_switch.py: ops/ansible/names-collector-deploy.yml with
# names_collector_enabled false (its default) says why in one line at the default
# verbosity and changes nothing, so a phase that runs it on every core machine
# of a site leaves one that does not collect as it is. With the switch on, the
# role runs (here it stops at its first check, the leaf certificate it needs).
#
# The play runs for real against localhost (connection local, no become). Skips
# without ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1.

import os
import shutil
import subprocess
import tempfile
import unittest

REPO = os.path.normpath(os.path.join(os.path.dirname(__file__), ".."))
ANSIBLE = os.path.join(REPO, "ops", "ansible")
SAYS = ("names-collector: names_collector_enabled is false for this host's group, so nothing "
        "was changed. Set it in group_vars to run it.")


def require_ansible_playbook(case):
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


def run(tmp, *extra):
    inv = os.path.join(tmp, "inv.ini")
    with open(inv, "w") as f:
        f.write("[zagros_hosts]\nlocalhost ansible_connection=local\n")
    env = dict(os.environ, ANSIBLE_ROLES_PATH=os.path.join(ANSIBLE, "roles"),
               ANSIBLE_LOCAL_TEMP=os.path.join(tmp, "ansible-tmp"), ANSIBLE_NOCOLOR="1",
               ANSIBLE_RETRY_FILES_ENABLED="0")
    r = subprocess.run(
        ["ansible-playbook", "-i", inv, os.path.join(ANSIBLE, "names-collector-deploy.yml"),
         "-e", "target=zagros_hosts", "-e", "ansible_become=false"] + list(extra),
        cwd=ANSIBLE, env=env, capture_output=True, text=True, timeout=300)
    return r.returncode, " ".join((r.stdout + r.stderr).split())


class Switch(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def test_off_says_why_and_changes_nothing(self):
        rc, out = run(self.tmp)
        self.assertEqual(rc, 0, out)
        self.assertIn(SAYS, out)
        self.assertRegex(out, r"localhost : ok=\d+ changed=0 unreachable=0 failed=0", out)
        self.assertNotIn("pre-flight leaf cert present", out)

    def test_on_runs_the_role(self):
        rc, out = run(self.tmp, "-e", "names_collector_enabled=true")
        self.assertNotIn(SAYS, out)
        self.assertIn("pre-flight leaf cert present", out)


if __name__ == "__main__":
    unittest.main()
