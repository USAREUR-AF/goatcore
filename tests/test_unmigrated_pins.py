#!/usr/bin/env python3
"""ops/pins/unmigrated-pins.txt holds the shrink-only ratchet `generate-pins.py
--check` and `goatbuilder pins` both read. Without it every site's --check fails
on "declares no `# baseline: <n>` line", whatever the site's pins are."""
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "ops/consistency/python"))

from goat_consistency import resolve  # noqa: E402
from goat_consistency.paths import ALLOWLIST  # noqa: E402


class UnmigratedPins(unittest.TestCase):
    def test_the_file_is_shipped(self):
        self.assertTrue(Path(ALLOWLIST).is_file(), ALLOWLIST)

    def test_it_declares_a_baseline(self):
        self.assertIsNotNone(resolve.load_baseline(), "no `# baseline: <n>` line")

    def test_the_baseline_holds_its_rows_exactly(self):
        rows, count = resolve.load_allowlist()
        self.assertEqual(count, resolve.load_baseline(),
                         "the ratchet sits at its row count: more rows fail --check, "
                         "fewer leave room to regress unseen")
        for row in rows:
            member, _, var = row.partition(":")
            self.assertTrue(member and var, row)

    def test_the_hermetic_overlay_keeps_the_baseline(self):
        # ops/e2e/overlay.py keeps the file's `#` lines and drops its rows.
        header = [l for l in Path(ALLOWLIST).read_text().splitlines() if l.startswith("#")]
        self.assertTrue(any(l.startswith("# baseline:") for l in header))


if __name__ == "__main__":
    unittest.main()
