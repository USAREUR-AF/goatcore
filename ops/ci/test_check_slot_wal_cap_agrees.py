"""check-slot-wal-cap-agrees.py: the slot WAL cap is stated once, and agrees.

Run: python3 -m pytest -q ops/ci/test_check_slot_wal_cap_agrees.py
"""
from __future__ import annotations

import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "check-slot-wal-cap-agrees.py")


class SlotWalCapAgrees(unittest.TestCase):
    def run_check(self, *args):
        p = subprocess.run([sys.executable, SCRIPT, *args], capture_output=True, text=True)
        return p.returncode, p.stdout + p.stderr

    def test_this_repository_states_the_cap_once(self):
        rc, out = self.run_check()
        self.assertEqual(rc, 0, out)

    def test_each_planted_violation_is_refused(self):
        rc, out = self.run_check("--self-test")
        self.assertIn("each of 3 planted violations is refused", out)
        self.assertEqual(rc, 0, out)


if __name__ == "__main__":
    unittest.main()
