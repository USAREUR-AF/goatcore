#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_dashboard_fence_switch.py: ops/ansible/dashboard-fence-deploy.yml with
# dashboard_oauth2_proxy_enabled off ends the everyday fence at once on the
# host. It changes nothing and does not fail, so a phase that runs the play
# after dashboard-deploy.yml leaves a site without a fence as it is. With the
# switch on, the play goes on to its guards. The admin fence keeps its own
# switch (netbird_dashboard_split_enabled).
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
OFF_LINE = ("dashboard-fence: dashboard_oauth2_proxy_enabled is off for this host, so the "
            "everyday fence was not deployed. Set it in group_vars to fence the dashboard.")


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
        ["ansible-playbook", "-i", inv, os.path.join(ANSIBLE, "dashboard-fence-deploy.yml"),
         "-e", "target=zagros_hosts", "-e", "ansible_become=false",
         "-e", "site_id=zagros", "-e", "site_dns_zone=example.org"] + list(extra),
        cwd=ANSIBLE, env=env, capture_output=True, text=True, timeout=300)


def outcomes(out):
    """(task, its first result word) for each TASK block of ansible's output."""
    rows, task = [], None
    for line in out.splitlines():
        if line.startswith("TASK ["):
            task = line[len("TASK ["):line.rfind("]")]
        elif task and line.split(":")[0] in ("ok", "changed", "skipping", "fatal", "failed", "included"):
            rows.append((task, line.split(":")[0]))
            task = None
    return rows


class Switch(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def test_off_ends_at_once_and_changes_nothing(self):
        r = run(self.tmp, "-e", "dashboard_oauth2_proxy_enabled=false",
                "-e", "netbird_dashboard_split_enabled=false")
        out = r.stdout + r.stderr
        self.assertEqual(r.returncode, 0, out)
        self.assertRegex(out, r"localhost\s*:\s*ok=\d+\s+changed=0\s+unreachable=0\s+failed=0", out)
        # A hand run says why nothing happened, at the default verbosity.
        self.assertIn(OFF_LINE, out)
        for task, _ in outcomes(out):
            self.assertNotIn("Guard", task, "nothing after the switch runs: %s" % task)
            self.assertNotIn("Verify", task, "nothing after the switch runs: %s" % task)

    def test_on_goes_on_to_the_guards(self):
        # With the switch on the play goes past the switch to its guards (here
        # the first check of a running Hydra fails: the test has none), so the
        # gate is not always closed.
        r = run(self.tmp, "-e", "dashboard_oauth2_proxy_enabled=true",
                "-e", "netbird_dashboard_split_enabled=false")
        out = r.stdout + r.stderr
        self.assertNotIn(OFF_LINE, out)
        got = dict(outcomes(out))
        self.assertIn("Guard — admin surfaces are single-homed (ADR 1084 §D3)", got, out)


if __name__ == "__main__":
    unittest.main()
