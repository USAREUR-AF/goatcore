#!/usr/bin/env python3
"""fabric-health-deploy.yml stops a meter left on a node that is not its home.

The play's single-home guard ends the play on every node whose router_role is
not admin_write_role, so a meter installed there earlier (before the guard, or
before the home moved) kept running: a second writer to the site key, or a
container that restarts forever. The play now runs `docker compose down` there
before the guard, and only there. The play's own pre-task runs here against a
stub docker."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/fabric-health-deploy.yml"
NAME = "A node that is not the roll-up's home stops a meter left from an earlier deploy"
GUARD = "Guard — one roll-up per site, on the router host (doctrine P13)"


def pre_tasks():
    return yaml.safe_load(PLAY.read_text())[0]["pre_tasks"]


class LeftoverMeter(unittest.TestCase):

    def test_the_cleanup_runs_before_the_guard(self):
        names = [t["name"] for t in pre_tasks()]
        self.assertLess(names.index(NAME), names.index(GUARD))

    def run_cleanup(self, hostvars, installed=True):
        if not shutil.which("ansible-playbook"):
            self.skipTest("ansible-playbook is required")
        d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, d)
        (d / "bin").mkdir()
        (d / "bin" / "docker").write_text('#!/bin/sh\necho "$PWD $*" >> "%s"\n' % (d / "calls"))
        (d / "bin" / "docker").chmod(0o755)
        compose = d / "fabric-health"
        compose.mkdir()
        if installed:
            (compose / "docker-compose.yml").write_text("services: {}\n")
        task = next(t for t in pre_tasks() if t["name"] == NAME)
        play = [{"hosts": "localhost", "gather_facts": False,
                 "vars": dict(hostvars, fabric_health_compose_dir=str(compose)), "tasks": [task]}]
        (d / "play.yml").write_text(yaml.safe_dump(play))
        env = dict(os.environ, PATH=f"{d / 'bin'}:{os.environ['PATH']}", ANSIBLE_NOCOLOR="1",
                   ANSIBLE_LOCALHOST_WARNING="False")
        env.pop("ANSIBLE_CONFIG", None)
        r = subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local", str(d / "play.yml")],
                           cwd=d, env=env, capture_output=True, text=True, timeout=120)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        calls = (d / "calls").read_text() if (d / "calls").exists() else ""
        return calls, compose

    def test_a_node_that_is_not_the_home_stops_its_meter(self):
        calls, compose = self.run_cleanup({"router_role": "B"})
        self.assertEqual(calls.strip(), f"{compose} compose down")

    def test_the_home_moved_with_admin_write_role(self):
        calls, _ = self.run_cleanup({"router_role": "A", "admin_write_role": "B"})
        self.assertIn("compose down", calls)

    def test_the_home_keeps_its_meter(self):
        calls, _ = self.run_cleanup({"router_role": "A"})
        self.assertEqual(calls, "")

    def test_a_node_with_no_router_role_is_left_to_the_guard(self):
        calls, _ = self.run_cleanup({})
        self.assertEqual(calls, "")

    def test_nothing_installed_nothing_run(self):
        calls, _ = self.run_cleanup({"router_role": "B"}, installed=False)
        self.assertEqual(calls, "")


if __name__ == "__main__":
    unittest.main()
