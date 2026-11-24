#!/usr/bin/env python3
"""A joiner's Postgres re-initialisation pauses the stack's watcher first.

site-bootstrap.yml refuses a joiner whose existing cluster encoding differs
from the founder's, and tells the operator to stop the stack, remove the
pgdata-a volume and re-run. While the stack is stopped, mgmt-host-compose-watch
would start the old postgres-a again within its poll interval. So both refusal
texts tell the operator to write the watcher's pause file first, with the reason
postgres-reinit. And once the re-run's cluster has the founder's encoding, the
play clears that pause, and only a pause it gave the reason for.

Reads the play and runs the clear task's own command against a temporary file.
"""
from pathlib import Path
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/site-bootstrap.yml"
PAUSE = "/run/mgmt-host-compose-watch.paused"


def plays():
    return yaml.safe_load(PLAY.read_text())


def play(word):
    return next(p for p in plays() if word in p.get("name", ""))


def refusals():
    """Every encoding refusal that tells the operator to remove the volume."""
    out = []
    for p in plays():
        for t in p.get("tasks", []) or []:
            for b in [t] + list(t.get("block", []) or []):
                a = b.get("ansible.builtin.assert")
                if a and "pgdata-a volume" in a.get("fail_msg", ""):
                    out.append((b["name"], " ".join(a["fail_msg"].split())))
    return out


class RefusalTextPausesTheWatcher(unittest.TestCase):
    def test_both_refusals_name_the_pause_before_the_stop(self):
        found = refusals()
        self.assertEqual(len(found), 2, found)
        for name, msg in found:
            self.assertIn(PAUSE, msg, name)
            self.assertIn("postgres-reinit", msg, name)
            self.assertLess(msg.index(PAUSE), msg.index("stop the stack"), name)


class ThePlayClearsItsOwnPause(unittest.TestCase):
    def clear_task(self):
        tasks = play("encoding matches the founder (after the stack is up)")["tasks"]
        names = [t["name"] for t in tasks]
        clear = [i for i, t in enumerate(tasks) if PAUSE in str(t.get("ansible.builtin.shell", ""))]
        self.assertEqual(len(clear), 1, names)
        check = names.index("encoding | this joiner's cluster encoding is the founder's")
        self.assertGreater(clear[0], check, "the pause is cleared only after the encoding check passes")
        return tasks[clear[0]]

    def run_clear(self, content):
        cmd = self.clear_task()["ansible.builtin.shell"]
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "paused"
            if content is not None:
                f.write_text(content)
            out = subprocess.run(["bash", "-c", cmd.replace(PAUSE, str(f))], capture_output=True, text=True)
            self.assertEqual(out.returncode, 0, out.stderr)
            return f.exists(), out.stdout

    def test_a_postgres_reinit_pause_is_cleared(self):
        exists, out = self.run_clear("postgres-reinit\n")
        self.assertFalse(exists)
        self.assertIn("cleared", out)

    def test_a_pause_for_another_reason_is_kept(self):
        exists, out = self.run_clear("router maintenance\n")
        self.assertTrue(exists)
        self.assertNotIn("cleared", out)

    def test_no_pause_is_no_change(self):
        exists, out = self.run_clear(None)
        self.assertFalse(exists)
        self.assertNotIn("cleared", out)


if __name__ == "__main__":
    unittest.main()
