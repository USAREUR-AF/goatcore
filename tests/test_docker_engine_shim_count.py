#!/usr/bin/env python3
"""The docker-engine role's guard on moving containerd's root counts running
container shims. It runs its count through /bin/bash, as the role does; on a
machine with no containerd-shim the count must be 0. With `pgrep -f` the shell
running the command matched itself, so a fresh host counted 1 or more and the
role refused to move containerd's root."""
from pathlib import Path
import platform
import subprocess
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
TASKS = ROOT / "ops/ansible/roles/docker-engine/tasks/main.yml"


def shim_count_cmd():
    for t in yaml.safe_load(TASKS.read_text()):
        if t.get("name") == "Count running container shims":
            return t["ansible.builtin.shell"]["cmd"]
    raise AssertionError("the role counts running container shims")


class ShimCount(unittest.TestCase):
    def test_the_count_does_not_match_its_own_shell(self):
        if platform.system() != "Linux":
            self.skipTest("procps pgrep -c (Linux)")
        if subprocess.run(["pgrep", "-c", "containerd-shim"], capture_output=True, text=True).stdout.strip() not in ("", "0"):
            self.skipTest("a containerd-shim is running on this machine")
        out = subprocess.run(["/bin/bash", "-c", shim_count_cmd()], capture_output=True, text=True).stdout.strip()
        self.assertEqual(out, "0")


if __name__ == "__main__":
    unittest.main()
