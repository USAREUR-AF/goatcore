#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_oci_substrate_admit_audit_password.py: the admit-audit play of
# ops/ansible/oci-substrate-deploy.yml takes admit_audit_password from the
# deploy environment only. With the variable unset, the value the play renders
# stops at the admit-audit role's own "Require admit_audit_password" check, with
# that check's message; with it set, the check passes.
#
# Ansible renders the play's own vars expression and runs the role's own check
# task, both read from the files, against localhost. Skips without
# ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1.

import os
import shutil
import subprocess
import tempfile
import unittest

import yaml

REPO = os.path.normpath(os.path.join(os.path.dirname(__file__), ".."))
ANSIBLE = os.path.join(REPO, "ops", "ansible")
PLAY = os.path.join(ANSIBLE, "oci-substrate-deploy.yml")
ROLE_TASKS = os.path.join(ANSIBLE, "roles", "admit-audit", "tasks", "deploy.yml")
CHECK = "Require admit_audit_password"


def require_ansible_playbook(case):
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


def play_password_expression():
    plays = yaml.safe_load(open(PLAY, encoding="utf-8"))
    found = [p["vars"]["admit_audit_password"] for p in plays if "admit_audit_password" in (p.get("vars") or {})]
    assert len(found) == 1, "one play of oci-substrate-deploy.yml sets admit_audit_password"
    return found[0]


def role_check_task():
    tasks = yaml.safe_load(open(ROLE_TASKS, encoding="utf-8"))
    found = [t for t in tasks if t.get("name") == CHECK]
    assert len(found) == 1, "the admit-audit role has one task named %r" % CHECK
    return found[0]


def run(tmp, env_value):
    play = [{
        "name": "render the admit-audit play's password and run the role's check",
        "hosts": "localhost",
        "connection": "local",
        "gather_facts": False,
        "vars": {"admit_audit_password": play_password_expression()},
        "tasks": [role_check_task()],
    }]
    path = os.path.join(tmp, "play.yml")
    with open(path, "w", encoding="utf-8") as f:
        yaml.safe_dump(play, f)
    env = {k: v for k, v in os.environ.items() if k != "ADMIT_AUDIT_PASS"}
    if env_value is not None:
        env["ADMIT_AUDIT_PASS"] = env_value
    env.update(ANSIBLE_LOCAL_TEMP=os.path.join(tmp, "ansible-tmp"), ANSIBLE_NOCOLOR="1",
               ANSIBLE_RETRY_FILES_ENABLED="0")
    return subprocess.run(["ansible-playbook", "-i", "localhost,", path],
                          cwd=tmp, env=env, capture_output=True, text=True, timeout=300)


class AdmitAuditPassword(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def test_unset_stops_at_the_roles_check(self):
        r = run(self.tmp, None)
        out = r.stdout + r.stderr
        self.assertNotEqual(r.returncode, 0, out)
        self.assertIn("Pass -e admit_audit_password=", out)

    def test_set_passes_the_check(self):
        r = run(self.tmp, "a-synthetic-value-for-the-test")
        out = r.stdout + r.stderr
        self.assertEqual(r.returncode, 0, out)


if __name__ == "__main__":
    unittest.main()
