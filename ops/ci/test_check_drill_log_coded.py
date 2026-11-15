"""The drill-log check (gate row C5): a refusal or a failed step with no GB-nnn code fails it."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("check_drill_log_coded", HERE / "check-drill-log-coded.py")
check = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check)


def findings(text, name="drill.log"):
    with tempfile.TemporaryDirectory() as d:
        p = Path(d) / name
        p.write_text(text)
        return check.uncoded([p])


class TextLogs(unittest.TestCase):
    def test_a_coded_refusal_passes(self):
        self.assertEqual(findings("goatbuilder: GB-008: federate is a mutation: pass --yes\n"), [])

    def test_an_uncoded_refusal_fails_and_names_its_line(self):
        got = findings("ok\ngoatbuilder: grow takes a goatlite config: this one has no lite: block\n")
        self.assertEqual(len(got), 1)
        self.assertEqual(got[0][1], 2)
        self.assertIn("grow takes a goatlite config", got[0][2])

    def test_a_failed_step_coded_on_its_interpreted_row_passes(self):
        log = ("!! Phase 8 (\"mint\") FAILED — exit 2.\n"
               "interpreted: GB-191: the partner's package is not well formed\n")
        self.assertEqual(findings(log), [])

    def test_a_failed_step_left_uncatalogued_fails(self):
        log = "teardown of goat-zenoh FAILED — exit 1\ncause: uncatalogued\nnext line\n"
        self.assertEqual(len(findings(log)), 1)

    def test_a_clap_usage_error_is_held_too(self):
        self.assertEqual(len(findings("error: unexpected argument '--frob' found\n")), 1)
        self.assertEqual(findings("error: unexpected argument '--frob' found (GB-242)\n"), [])


class JsonReports(unittest.TestCase):
    def test_a_failed_row_without_a_code_fails(self):
        doc = {"verb": "federate", "exit": 2, "checks": [
            {"name": "phase", "ok": True, "detail": "=== Phase 1 ==="},
            {"name": "failed", "ok": False, "detail": "phase 1 exited 2", "fix": "read the log"},
        ]}
        self.assertEqual(len(findings(json.dumps(doc), "report.json")), 1)

    def test_a_failed_row_with_an_interpreted_code_passes(self):
        doc = {"verb": "federate", "exit": 2, "checks": [
            {"name": "failed", "ok": False, "detail": "phase 1 exited 2"},
            {"name": "interpreted", "ok": False, "detail": "GB-149: the peering is not declared"},
        ]}
        self.assertEqual(findings(json.dumps(doc), "report.json"), [])

    def test_a_verdict_row_is_not_a_refusal(self):
        doc = {"verb": "status", "exit": 3, "checks": [
            {"name": "host", "ok": False, "detail": "pending"},
        ]}
        self.assertEqual(findings(json.dumps(doc), "report.json"), [])


class Command(unittest.TestCase):
    def test_exit_codes(self):
        with tempfile.TemporaryDirectory() as d:
            good = Path(d) / "good.log"
            good.write_text("goatbuilder: GB-008: needs --yes\n")
            bad = Path(d) / "bad.log"
            bad.write_text("goatbuilder: something went wrong\n")
            self.assertEqual(check.main([str(good)]), 0)
            self.assertEqual(check.main([str(bad)]), 1)
            self.assertEqual(check.main([str(Path(d) / "missing.log")]), 2)


if __name__ == "__main__":
    unittest.main()
