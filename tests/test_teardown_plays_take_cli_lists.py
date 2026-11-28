#!/usr/bin/env python3
"""The teardown plays take their lists the way goatbuilder passes them.

`goatbuilder teardown` passes each play its list as one key=value extra var,
`-e 'teardown_units=["rathole-client@*.service"]'`, which Ansible keeps as a
string. Each play runs here against localhost with that exact argument and a
`systemctl`/`docker` stub that reports nothing loaded, so the play reaches its
"nothing to stop" end instead of failing on a string where it loops.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
PLAYS = ROOT / "ops/ansible"


def require_ansible_playbook(case):
    """Skip without ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1 (CI sets
    it), where its absence fails the test instead of skipping it quietly."""
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class TeardownPlaysTakeCliLists(unittest.TestCase):
    def run_play(self, play, *extra):
        require_ansible_playbook(self)
        with tempfile.TemporaryDirectory() as d:
            for tool in ("systemctl", "docker"):
                stub = Path(d) / tool
                stub.write_text("#!/bin/sh\nexit 0\n")
                stub.chmod(0o755)
            env = dict(os.environ, PATH=f"{d}:{os.environ['PATH']}", ANSIBLE_LOCALHOST_WARNING="0",
                       ANSIBLE_INVENTORY_UNPARSED_WARNING="0")
            return subprocess.run(
                ["ansible-playbook", "-i", "localhost,", "-c", "local", play,
                 "-e", "target=localhost", "-e", "ansible_become=false",
                 "-e", f"ansible_python_interpreter={sys.executable}",
                 "-e", "teardown_component=example", *extra],
                cwd=PLAYS, env=env, capture_output=True, text=True, timeout=300)

    def test_systemd_stop_takes_its_units_as_goatbuilder_passes_them(self):
        r = self.run_play("teardown-systemd.yml", "-e", 'teardown_units=["rathole-client@*.service"]')
        self.assertEqual(r.returncode, 0, r.stdout[-2000:] + r.stderr[-2000:])
        self.assertIn("example: nothing to stop (no unit matches rathole-client@*.service)", r.stdout)


if __name__ == "__main__":
    unittest.main()
