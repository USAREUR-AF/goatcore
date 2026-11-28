#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""The scoped Alertmanager render refuses to drop a receiver the host runs.

Covers ops/ansible/roles/obs-host/files/alertmanager-receiver-guard.py and the
wiring of ops/ansible/obs-alertmanager-render.yml: the guard runs before the
write, and both plays write the file through the same task.
"""
import json
import subprocess
import sys
import unittest
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parents[2]
ROLE = REPO / "ops/ansible/roles/obs-host"
GUARD = ROLE / "files/alertmanager-receiver-guard.py"
PLAY = REPO / "ops/ansible/obs-alertmanager-render.yml"

FULL = """
receivers:
  - name: deadman
    webhook_configs:
      - url: "http://192.0.2.10:9099/obs-1"
        http_config:
          authorization:
            type: Bearer
            credentials: "synthetic-token"
  - name: critical-fanout
    pagerduty_configs:
      - routing_key: "synthetic-key"
    webhook_configs:
      - url: "https://ntfy.example.net/topic"
        http_config:
          basic_auth:
            username: am
            password: "synthetic-password"
  - name: noop
    webhook_configs:
      - url: "http://localhost:9999/noop"
"""


def run(live: str, new: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(GUARD)],
        input=json.dumps({"live": live, "new": new}),
        text=True,
        capture_output=True,
    )


def without(text: str, mutate) -> str:
    doc = yaml.safe_load(text)
    mutate({r["name"]: r for r in doc["receivers"]}, doc)
    return yaml.safe_dump(doc)


class GuardTests(unittest.TestCase):
    def test_same_config_passes(self):
        self.assertEqual(run(FULL, FULL).returncode, 0)

    def test_first_install_passes(self):
        self.assertEqual(run("", FULL).returncode, 0)

    def test_a_changed_address_passes(self):
        new = FULL.replace("192.0.2.10", "192.0.2.77")
        self.assertEqual(run(FULL, new).returncode, 0)

    def test_an_added_receiver_passes(self):
        live = without(FULL, lambda by, doc: by["critical-fanout"].pop("pagerduty_configs"))
        self.assertEqual(run(live, FULL).returncode, 0)

    def test_a_lost_heartbeat_token_is_refused(self):
        new = without(FULL, lambda by, doc: by["deadman"]["webhook_configs"][0].pop("http_config"))
        res = run(FULL, new)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the credential of deadman/webhook_configs#0", res.stdout)

    def test_a_lost_paging_receiver_is_refused(self):
        new = without(FULL, lambda by, doc: by["critical-fanout"].pop("pagerduty_configs"))
        res = run(FULL, new)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("critical-fanout/pagerduty_configs#0", res.stdout)

    def test_an_emptied_paging_key_is_refused(self):
        new = FULL.replace('routing_key: "synthetic-key"', 'routing_key: ""')
        self.assertEqual(run(FULL, new).returncode, 1)

    def test_a_lost_receiver_is_refused(self):
        def drop(by, doc):
            doc["receivers"] = [r for r in doc["receivers"] if r["name"] != "deadman"]
        self.assertEqual(run(FULL, without(FULL, drop)).returncode, 1)

    def test_output_carries_no_address_and_no_credential(self):
        new = without(FULL, lambda by, doc: by["critical-fanout"].pop("webhook_configs"))
        out = run(FULL, new).stdout
        for secret in ("synthetic-token", "synthetic-key", "synthetic-password", "192.0.2.10", "ntfy.example.net"):
            self.assertNotIn(secret, out)

    def test_unreadable_input_is_not_a_pass(self):
        self.assertEqual(run("receivers: [", FULL).returncode, 2)


class WiringTests(unittest.TestCase):
    def test_the_guard_runs_before_the_write(self):
        play = yaml.safe_load(PLAY.read_text())
        steps = [t["ansible.builtin.include_role"]["tasks_from"]
                 for t in play[0]["tasks"] if "ansible.builtin.include_role" in t]
        self.assertEqual(steps, ["alertmanager-guard", "alertmanager-render"])

    def test_the_roles_defaults_stay_below_the_sites_settings(self):
        # vars_files and play vars rank above group_vars. Loading the role's
        # defaults that way replaced every receiver a site declares with the
        # role's empty default, and the render dropped them all.
        play = yaml.safe_load(PLAY.read_text())[0]
        self.assertNotIn("vars_files", play)
        self.assertNotIn("vars", play)
        for task in play["tasks"]:
            self.assertNotIn("ansible.builtin.include_vars", task)
            self.assertNotIn("ansible.builtin.import_tasks", task)

    def test_site_settings_reach_the_render(self):
        # The same precedence, run: a group_vars value must win over the role's
        # default when the guard's tasks run as the play runs them.
        import shutil, subprocess, tempfile
        if not shutil.which("ansible-playbook"):
            self.skipTest("ansible-playbook not installed")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "group_vars/obs").mkdir(parents=True)
            (root / "group_vars/obs/vars.yml").write_text(
                'deadman_webhook_url: "http://192.0.2.10:9099/{{ inventory_hostname }}"\n'
                'deadman_webhook_token: "synthetic-token"\n'
                'pagerduty_integration_key: "synthetic-key"\n')
            (root / "hosts.ini").write_text("[obs]\nobs-1 ansible_connection=local\n")
            (root / "play.yml").write_text(
                "- hosts: obs\n  gather_facts: false\n  tasks:\n"
                "    - ansible.builtin.include_role: {name: obs-host, tasks_from: alertmanager-guard, public: true, allow_duplicates: true}\n"
                "      vars: {obs_compose_dir: '%s', rollback_ledger_dir: '%s/ledger', rollback_ledger_owner: '{{ lookup(\"env\", \"USER\") }}', rollback_ledger_group: '{{ lookup(\"pipe\", \"id -gn\") }}'}\n"
                "    - ansible.builtin.copy:\n        content: \"{{ lookup('ansible.builtin.template', '%s/templates/alertmanager.yml.j2') }}\"\n        dest: '%s/out.yml'\n        mode: '0600'\n"
                % (tmp, tmp, ROLE, tmp))
            env = dict(__import__("os").environ, ANSIBLE_ROLES_PATH=str(ROLE.parent), ANSIBLE_LOCALHOST_WARNING="False")
            res = subprocess.run(["ansible-playbook", "-i", str(root / "hosts.ini"), str(root / "play.yml")],
                                 cwd=root, env=env, text=True, capture_output=True)
            self.assertEqual(res.returncode, 0, res.stdout[-1500:])
            out = yaml.safe_load((root / "out.yml").read_text())
            names = {r["name"]: r for r in out["receivers"]}
            self.assertIn("192.0.2.10:9099/obs-1", names["deadman"]["webhook_configs"][0]["url"])
            self.assertTrue(names["critical-fanout"]["pagerduty_configs"])

    def test_one_task_writes_the_file(self):
        writers = [p.name for p in (ROLE / "tasks").glob("*.yml")
                   if "src: alertmanager.yml.j2" in p.read_text()]
        self.assertEqual(writers, ["alertmanager-render.yml"])
        self.assertIn("import_tasks: alertmanager-render.yml", (ROLE / "tasks/main.yml").read_text())

    def test_the_guard_refuses_an_empty_heartbeat_token(self):
        guard = yaml.safe_load((ROLE / "tasks/alertmanager-guard.yml").read_text())
        first = guard[0]["ansible.builtin.assert"]
        self.assertIn("deadman_webhook_token", " ".join(first["that"]))
        self.assertIn("X-ANSIBLE-006", first["fail_msg"])

    def test_the_guard_changes_nothing_and_logs_no_config(self):
        guard = yaml.safe_load((ROLE / "tasks/alertmanager-guard.yml").read_text())
        for task in guard:
            if "ansible.builtin.slurp" in task or "ansible.builtin.command" in task:
                self.assertTrue(task.get("no_log"), task["name"])


if __name__ == "__main__":
    unittest.main()
