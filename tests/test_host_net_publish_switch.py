#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_host_net_publish_switch.py: ops/ansible/host-net-publish-deploy.yml with
# host_net_publish_enabled off ends at once on the host. It changes nothing,
# writes no deploy-ledger row, and does not fail, so a phase that runs it on
# every core machine of a joining site leaves a site that does not publish as it
# is. With the switch on and an input missing, the role still refuses.
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
    return subprocess.run(
        ["ansible-playbook", "-i", inv, os.path.join(ANSIBLE, "host-net-publish-deploy.yml"),
         "-e", "target=zagros_hosts", "-e", "ansible_become=false",
         "-e", "rollback_ledger_dir=" + os.path.join(tmp, "ledger")] + list(extra),
        cwd=ANSIBLE, env=env, capture_output=True, text=True, timeout=300)


class Switch(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def test_off_ends_at_once_and_changes_nothing(self):
        r = run(self.tmp, "-e", "host_net_publish_enabled=false")
        out = r.stdout + r.stderr
        self.assertEqual(r.returncode, 0, out)
        self.assertRegex(out, r"localhost\s*:\s*ok=\d+\s+changed=0\s+unreachable=0\s+failed=0", out)
        self.assertNotIn("require inputs", out, "nothing after the switch runs")
        # A hand run says why nothing happened, at the default verbosity.
        self.assertIn("host-net-publish: host_net_publish_enabled is off for this host, so nothing was changed. Set it in group_vars to publish.", out)
        for task, result in outcomes(out):
            if "rollback-ledger" in task:
                self.assertEqual(result, "skipping", "%s ran with the switch off" % task)
        self.assertFalse(os.path.exists(os.path.join(self.tmp, "ledger")), "no ledger is written")

    def test_on_runs_the_ledger_and_the_role(self):
        # With the switch on the ledger dependency runs (here it stops at the
        # ledger directory: the test runs without root), so the gate is not
        # always closed.
        r = run(self.tmp, "-e", "host_net_publish_enabled=true")
        got = dict(outcomes(r.stdout + r.stderr))
        self.assertEqual(got.get("rollback-ledger : rollback-ledger | require a component name (fail-closed per ADR 1060)"), "ok", r.stdout)


def outcomes(out):
    """(task, its first result word) for each TASK block of ansible's output."""
    rows, task = [], None
    for line in out.splitlines():
        if line.startswith("TASK ["):
            task = line[len("TASK ["):line.rfind("]")]
        elif task and line.split(":")[0] in ("ok", "changed", "skipping", "fatal", "failed"):
            rows.append((task, line.split(":")[0]))
            task = None
    return rows


if __name__ == "__main__":
    unittest.main()
