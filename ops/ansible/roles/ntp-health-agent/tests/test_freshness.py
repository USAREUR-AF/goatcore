#!/usr/bin/env python3
"""Offline shell simulation and task-shape checks for artifact freshness.

These tests load the production task YAML and execute its shell bodies, but
they do not run ansible-core. The role-local syntax-check covers Ansible
parsing when ansible-playbook is available.
"""

import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

import yaml


ROLE_DIR = Path(__file__).resolve().parents[1]
TASKS = yaml.safe_load((ROLE_DIR / "tasks/main.yml").read_text())


def task_named(name):
    return next(task for task in TASKS if task.get("name") == name)


FRESHNESS_TASK = task_named(
    "Check whether the local ntp-health-agent artifact is current"
)
BUILD_TASK = task_named(
    "Rebuild ntp-health-agent from source when the artifact is stale"
)


class FreshnessTaskTest(unittest.TestCase):
    def simulate_tasks(
        self, *, binary_exists, source_mtime, binary_mtime=200, make_exit=0
    ):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            playbook_dir = root / "ops/ansible"
            source_dir = root / "ops/time-hygiene"
            binary = root / "bin/ntp-health-agent"
            fake_bin = root / "fake-bin"
            make_calls = root / "make.calls"

            playbook_dir.mkdir(parents=True)
            source_dir.mkdir(parents=True)
            binary.parent.mkdir(parents=True)
            fake_bin.mkdir()

            source = source_dir / "main.go"
            source.write_text("package main\n")
            os.utime(source, (source_mtime, source_mtime))
            if binary_exists:
                binary.write_bytes(b"test binary")
                os.utime(binary, (binary_mtime, binary_mtime))

            fake_make = fake_bin / "make"
            fake_make.write_text(
                "#!/bin/sh\n"
                'printf \'%s\\n\' "$*" >> "$MAKE_CALLS"\n'
                'exit "$MAKE_EXIT"\n'
            )
            fake_make.chmod(0o755)

            env = os.environ.copy()
            env["PATH"] = f"{fake_bin}:{env['PATH']}"
            env["MAKE_CALLS"] = str(make_calls)
            env["MAKE_EXIT"] = str(make_exit)

            freshness_script = FRESHNESS_TASK["ansible.builtin.shell"]
            freshness_script = freshness_script.replace(
                "{{ playbook_dir }}", str(playbook_dir)
            ).replace("{{ ntp_health_agent_binary_src }}", str(binary))
            freshness = subprocess.run(
                [FRESHNESS_TASK["args"]["executable"], "-c", freshness_script],
                check=False,
                capture_output=True,
                text=True,
                env=env,
            )

            build = None
            if "REBUILD" in freshness.stdout:
                build_dir = BUILD_TASK["ansible.builtin.command"]["chdir"].replace(
                    "{{ playbook_dir }}", str(playbook_dir)
                )
                build = subprocess.run(
                    shlex.split(BUILD_TASK["ansible.builtin.command"]["cmd"]),
                    cwd=build_dir,
                    check=False,
                    capture_output=True,
                    text=True,
                    env=env,
                )

            calls = make_calls.read_text().splitlines() if make_calls.exists() else []
            return freshness, build, calls

    def test_current_artifact_does_not_build(self):
        freshness, build, calls = self.simulate_tasks(
            binary_exists=True, source_mtime=100
        )

        self.assertEqual(freshness.returncode, 0)
        self.assertEqual(freshness.stdout.strip(), "CURRENT")
        self.assertIsNone(build)
        self.assertEqual(calls, [])

    def test_stale_artifact_builds_exactly_once(self):
        freshness, build, calls = self.simulate_tasks(
            binary_exists=True, source_mtime=300
        )

        self.assertEqual(freshness.returncode, 0)
        self.assertEqual(freshness.stdout.strip(), "REBUILD")
        self.assertIsNotNone(build)
        self.assertEqual(build.returncode, 0)
        self.assertEqual(calls, ["build"])

    def test_simulated_failed_build_returns_nonzero(self):
        freshness, build, calls = self.simulate_tasks(
            binary_exists=False, source_mtime=100, make_exit=23
        )

        self.assertEqual(freshness.returncode, 0)
        self.assertEqual(freshness.stdout.strip(), "REBUILD")
        self.assertIsNotNone(build)
        self.assertEqual(build.returncode, 23)
        self.assertEqual(calls, ["build"])

    def test_pipefail_task_explicitly_uses_bash(self):
        self.assertIn("set -euo pipefail", FRESHNESS_TASK["ansible.builtin.shell"])
        self.assertEqual(FRESHNESS_TASK["args"]["executable"], "/bin/bash")

    def test_real_tasks_preserve_ansible_control_flow_and_failure_defaults(self):
        self.assertEqual(
            FRESHNESS_TASK["register"], "ntp_health_agent_freshness"
        )
        self.assertIs(FRESHNESS_TASK["changed_when"], False)
        self.assertEqual(FRESHNESS_TASK["delegate_to"], "localhost")
        self.assertIs(FRESHNESS_TASK["become"], False)
        self.assertIs(FRESHNESS_TASK["run_once"], True)

        self.assertEqual(
            BUILD_TASK["when"],
            "'REBUILD' in ntp_health_agent_freshness.stdout",
        )
        self.assertEqual(
            BUILD_TASK["ansible.builtin.command"],
            {
                "cmd": "make build",
                "chdir": "{{ playbook_dir }}/../../ops/time-hygiene",
            },
        )
        self.assertEqual(BUILD_TASK["delegate_to"], "localhost")
        self.assertIs(BUILD_TASK["become"], False)
        self.assertIs(BUILD_TASK["run_once"], True)
        self.assertNotIn("failed_when", BUILD_TASK)
        self.assertNotIn("ignore_errors", BUILD_TASK)
        self.assertNotIn("block", BUILD_TASK)
        self.assertNotIn("rescue", BUILD_TASK)


if __name__ == "__main__":
    unittest.main()
