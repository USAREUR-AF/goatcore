#!/usr/bin/env python3
"""The ory role provisions the bootstrap administrators at a founder site only.

A joining site (goatnet_founder: false) gets its Kratos identities from its
founder by replication. If its own ory-deploy run (join-node phase 13) also
created them, the joiner would hold a second identity per administrator, with
its own password, beside the replicated one.

The role's bootstrap-admin tasks live in tasks/bootstrap-admin.yml, and
tasks/main.yml includes that file only when `goatnet_founder | default(true) |
bool` is true: the way site-bootstrap.yml reads the same key. The second test
runs the include as the role writes it through a real ansible-playbook, so the
boolean is read by Ansible itself: false, False, "false", no and 0 skip it;
unset, true and yes run it.
"""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
TASKS = ROOT / "ops/ansible/roles/ory/tasks"
GATE = "goatnet_founder | default(true) | bool"


def main_tasks():
    return yaml.safe_load((TASKS / "main.yml").read_text())


def include_task():
    """The main.yml task that includes bootstrap-admin.yml, or None."""
    for t in main_tasks():
        for key in ("ansible.builtin.include_tasks", "include_tasks"):
            if key in t and str(t[key]).endswith("bootstrap-admin.yml"):
                return t
    return None


class BootstrapAdminIsFounderOnly(unittest.TestCase):
    def test_main_holds_no_bootstrap_admin_task_and_includes_them_behind_the_founder_gate(self):
        names = [t.get("name", "") for t in main_tasks()]
        self.assertFalse([n for n in names if n.startswith("bootstrap-admin |")],
                         "tasks/main.yml still runs bootstrap-admin tasks itself")
        inc = include_task()
        self.assertIsNotNone(inc, "tasks/main.yml does not include tasks/bootstrap-admin.yml")
        when = inc.get("when")
        when = when if isinstance(when, list) else [when]
        self.assertIn(GATE, when, f"the include is not gated on `{GATE}`")
        moved = yaml.safe_load((TASKS / "bootstrap-admin.yml").read_text())
        self.assertTrue(moved and all(t.get("name", "").startswith("bootstrap-admin |") for t in moved),
                        "tasks/bootstrap-admin.yml holds a task that is not a bootstrap-admin task")

    @unittest.skipUnless(shutil.which("ansible-playbook") or os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1",
                         "ansible-playbook is not installed (CI sets GOAT_TEST_REQUIRE_ANSIBLE=1)")
    def test_ansible_reads_the_gate_as_site_bootstrap_does(self):
        inc = include_task()
        self.assertIsNotNone(inc, "tasks/main.yml does not include tasks/bootstrap-admin.yml")
        cases = [  # (the vars.yml line, or None for unset; whether the tasks run)
            (None, True), ("goatnet_founder: true", True), ("goatnet_founder: yes", True),
            ("goatnet_founder: false", False), ("goatnet_founder: False", False),
            ('goatnet_founder: "false"', False), ("goatnet_founder: no", False), ("goatnet_founder: 0", False),
        ]
        env = {k: v for k, v in os.environ.items() if not k.startswith("ANSIBLE_")}
        for line, runs in cases:
            with self.subTest(vars=line), tempfile.TemporaryDirectory() as d:
                d = Path(d)
                # The include as main.yml writes it, pointed at a stand-in that records it ran.
                task = dict(inc)
                key = "ansible.builtin.include_tasks" if "ansible.builtin.include_tasks" in task else "include_tasks"
                task[key] = str(d / "stand-in.yml")
                (d / "stand-in.yml").write_text(yaml.safe_dump([{
                    "name": "record that the bootstrap-admin tasks ran",
                    "ansible.builtin.copy": {"dest": str(d / "ran"), "content": "ran\n", "mode": "0600"}}]))
                (d / "vars.yml").write_text((line or "{}") + "\n")
                (d / "play.yml").write_text(yaml.safe_dump([{
                    "hosts": "localhost", "gather_facts": False, "become": False,
                    "vars_files": [str(d / "vars.yml")],
                    "vars": {"site_admins": [{"email": "admin@example.org", "name": "Admin"}]},
                    "tasks": [task]}]))
                env.update({"ANSIBLE_BECOME": "False", "ANSIBLE_CONFIG": str(d / "none.cfg"),
                            "ANSIBLE_LOCAL_TEMP": str(d / "tmp"), "ANSIBLE_HOME": str(d / "home"),
                            "ANSIBLE_NOCOLOR": "1"})
                r = subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local", str(d / "play.yml")],
                                   cwd=d, env=env, capture_output=True, text=True, timeout=300)
                self.assertEqual(r.returncode, 0, (r.stdout + r.stderr)[-3000:])
                self.assertEqual((d / "ran").exists(), runs,
                                 f"with {line or 'goatnet_founder unset'} the bootstrap-admin tasks "
                                 f"{'did not run' if runs else 'ran'}")


if __name__ == "__main__":
    unittest.main()
