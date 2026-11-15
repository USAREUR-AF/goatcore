#!/usr/bin/env python3
"""ops/scripts/standup-timings-close.py: a run of goat-standup.sh resumed after the
operator step closes that step's open row in the timings file, as goatbuilder
standup does (close_operator_stop in crates/goatbuilder/src/standup.rs)."""
import subprocess
import tempfile
import unittest
from pathlib import Path

CLOSE = Path(__file__).resolve().parents[1] / "ops/scripts/standup-timings-close.py"
OPEN = "20270101T102100Z\t11\tfirst login\t-\t-\toperator\n"
BEFORE = "20270101T100010Z\t10\tenrol\t10\t0\tok\n"


def close(text, frm, now="20270101T102215Z"):
    d = tempfile.mkdtemp()
    f = Path(d) / "standup-alpha-timings.tsv"
    if text is not None:
        f.write_text(text)
    r = subprocess.run(["python3", str(CLOSE), str(f), str(frm), "--now", now], capture_output=True, text=True)
    return r, (f.read_text() if f.exists() else None)


class Close(unittest.TestCase):
    def test_an_open_operator_row_is_closed_with_its_wall_time(self):
        r, text = close(BEFORE + OPEN, 12)
        self.assertEqual((r.returncode, r.stdout.strip()), (0, "75"), r.stderr)
        self.assertEqual(text.splitlines()[-1], "20270101T102215Z\t11\tfirst login\t75\t0\toperator")

    def test_it_is_closed_once(self):
        r, text = close(BEFORE + OPEN + "20270101T102215Z\t11\tfirst login\t75\t0\toperator\n", 12)
        self.assertEqual((r.returncode, r.stdout), (0, ""), r.stderr)
        self.assertEqual(len(text.splitlines()), 3)

    def test_a_run_from_the_operator_phase_or_before_closes_nothing(self):
        for frm in (1, 11):
            r, text = close(BEFORE + OPEN, frm)
            self.assertEqual((r.returncode, r.stdout, text), (0, "", BEFORE + OPEN), frm)

    def test_no_open_row_and_no_file_change_nothing(self):
        r, text = close(BEFORE, 12)
        self.assertEqual((r.returncode, r.stdout, text), (0, "", BEFORE))
        r, text = close(None, 12)
        self.assertEqual((r.returncode, r.stdout, text), (0, "", None))

    def test_an_open_stop_on_purpose_is_not_the_operator_step(self):
        stopped = "20270101T102100Z\t12\tmint the bot\t-\t-\tstopped\n"
        r, text = close(BEFORE + stopped, 13)
        self.assertEqual((r.returncode, r.stdout, text), (0, "", BEFORE + stopped))


if __name__ == "__main__":
    unittest.main()
