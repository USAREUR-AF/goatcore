#!/usr/bin/env python3
"""netbird-client-pin holds the package even when dpkg's lock is busy at that
moment, as it is on a fresh host while apt-daily or unattended-upgrades runs.

Runs the role's own hold task, taken unchanged from
ops/ansible/roles/netbird-client-pin/tasks/main.yml, against localhost with a
stand-in dpkg on PATH that refuses the first --set-selections the way dpkg does
when another process holds the lock: it exits before reading its input, so the
module fails with a broken pipe.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
ROLE_TASKS = ROOT / "ops/ansible/roles/netbird-client-pin/tasks/main.yml"

DPKG = """#!/bin/sh
# A stand-in for dpkg: the package is installed and not held; the first
# --set-selections finds the lock taken (before reading stdin), later ones hold.
state="$(dirname "$0")/../state"
case "$1" in
  --get-selections) if [ -f "$state/held" ]; then echo "netbird hold"; else echo "netbird install"; fi ;;
  --set-selections)
    n=$(cat "$state/tries" 2>/dev/null || echo 0); n=$((n + 1)); echo "$n" > "$state/tries"
    if [ "$n" -le "${LOCKED_TRIES:-1}" ]; then
      echo "dpkg: error: dpkg frontend lock was locked by another process" >&2; exit 2
    fi
    cat > "$state/selections"; touch "$state/held" ;;
  *) echo "unexpected: $*" >&2; exit 9 ;;
esac
"""


def hold_task():
    """The role's own hold task, unchanged."""
    def walk(tasks):
        for t in tasks:
            yield t
            yield from walk(t.get("block", []))
    found = [t for t in walk(yaml.safe_load(ROLE_TASKS.read_text())) if str(t.get("name", "")).startswith("Hold netbird at")]
    assert len(found) == 1, "the role's hold task is where this test reads it"
    return found[0]


def require_ansible_playbook(case):
    """Skip without ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1 (CI sets
    it), where its absence fails the test instead of skipping it quietly."""
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class HoldWhileLocked(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        (self.dir / "bin").mkdir()
        (self.dir / "state").mkdir()
        (self.dir / "bin/dpkg").write_text(DPKG)
        (self.dir / "bin/dpkg").chmod(0o755)

    def tearDown(self):
        self._tmp.cleanup()

    def run_hold(self, locked_tries):
        play = [{
            "hosts": "node-a", "gather_facts": False, "become": False,
            "vars": {"netbird_version": "0.75.1"},
            "environment": {"PATH": f"{self.dir / 'bin'}:/usr/bin:/bin", "LOCKED_TRIES": str(locked_tries)},
            "tasks": [hold_task()],
        }]
        pb = self.dir / "play.yml"
        pb.write_text(yaml.safe_dump(play, sort_keys=False))
        inv = self.dir / "inv.ini"
        inv.write_text("node-a ansible_connection=local ansible_python_interpreter='{{ ansible_playbook_python }}'\n")
        env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_RETRY_FILES_ENABLED="0")
        return subprocess.run(["ansible-playbook", "-i", str(inv), str(pb)], capture_output=True, text=True, env=env, cwd=self.dir, timeout=300)

    def test_a_busy_lock_is_waited_out_and_the_package_is_held(self):
        r = self.run_hold(locked_tries=1)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual((self.dir / "state/selections").read_text().split(), ["netbird", "hold"])
        self.assertEqual((self.dir / "state/tries").read_text().strip(), "2")

    def test_a_lock_that_never_frees_still_fails_the_play(self):
        r = self.run_hold(locked_tries=1000)
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertFalse((self.dir / "state/held").exists())
        self.assertEqual((self.dir / "state/tries").read_text().strip(), "13", "one try and twelve retries")


if __name__ == "__main__":
    unittest.main()
