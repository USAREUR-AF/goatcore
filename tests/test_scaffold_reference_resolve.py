#!/usr/bin/env python3
"""The parity reference is resolved one way, and an unusable one says what to do.

A fresh checkout ships `reference: ''`. --scaffold-plan is the first command a
new site runs, so its refusal must name the setting and the way out, not report
an empty name as missing.
"""
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("preflight", ROOT / "ops/scripts/standup-config-preflight.py")
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)


class ResolveReference(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.gv = Path(self._tmp.name)
        (self.gv / "alpha_hosts").mkdir()
        (self.gv / "alpha_hosts" / "vars.yml").write_text("site_id: alpha\n")
        self._saved = (preflight.REPO, preflight.GV)
        preflight.REPO, preflight.GV = self.gv, self.gv

    def tearDown(self):
        preflight.REPO, preflight.GV = self._saved
        self._tmp.cleanup()

    def test_declared_reference_resolves(self):
        self.assertEqual(preflight.resolve_reference({"reference": "alpha_hosts"}), ("alpha_hosts", None))

    def test_override_wins_over_an_empty_manifest(self):
        self.assertEqual(preflight.resolve_reference({"reference": ""}, "alpha_hosts"), ("alpha_hosts", None))

    def test_empty_reference_names_the_setting_and_the_way_out(self):
        ref, why = preflight.resolve_reference({"reference": ""})
        self.assertIsNone(ref)
        for needle in ("no reference goatnet is declared", ".groupvars-parity.yaml", "--scaffold-from", "goatlite"):
            self.assertIn(needle, why)

    def test_unknown_reference_names_the_missing_directory(self):
        ref, why = preflight.resolve_reference({"reference": "bravo_hosts"})
        self.assertIsNone(ref)
        self.assertIn("'bravo_hosts' has no directory", why)

    def test_scaffold_plan_refuses_with_the_reason_and_accepts_an_override(self):
        (self.gv / ".groupvars-parity.yaml").write_text("reference: ''\nexclusions:\n")
        with self.assertRaises(SystemExit) as refused:
            preflight.scaffold_plan("charlie_hosts")
        self.assertIn("no reference goatnet is declared", str(refused.exception.code))
        # With a reference, the plan runs and returns 1: vars.yml is owed.
        self.assertEqual(preflight.scaffold_plan("charlie_hosts", "alpha_hosts"), 1)


if __name__ == "__main__":
    unittest.main()
