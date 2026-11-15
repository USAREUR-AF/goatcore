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
    def test_a_code_after_the_next_error_line_does_not_count_for_this_one(self):
        # Each error's window ends at the next refusal, usage error or failed row, so
        # a later error's code never covers an earlier uncoded one.
        log = ("goatbuilder: grow takes a goatlite config: this one has no lite: block\n"
               "goatbuilder: federate is a mutation: pass --yes without a terminal (C10)\n"
               "  (GB-008 — goatbuilder explain GB-008)\n")
        got = findings(log)
        self.assertEqual([g[1] for g in got], [1], got)

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

    def test_a_probe_the_run_went_past_is_not_an_error(self):
        log = ("  $ ssh host 'podman ps'\n"
               "  FAILED: exit 1 in 0.1s; log ~/.goat/state/runs/r/logs/009-3.log\n"
               "  $ ssh host 'docker ps'\n"
               "  ok in 0.1s\n")
        self.assertEqual(findings(log), [])

    def test_a_probe_at_the_end_of_a_phase_is_not_an_error(self):
        log = ("  $ ssh host 'stat -c %u -- /var/log/state.json'\n"
               "  FAILED: exit 1 in 0.1s; log ~/.goat/state/runs/r/logs/020-3.log\n"
               "init — our half, read from the running system\n"
               "[✓] declaration — read\n")
        self.assertEqual(findings(log), [])

    def test_a_phase_that_stopped_the_run_without_a_code_is_an_error(self):
        log = ("  $ ansible-playbook site.yml\n"
               "  FAILED: exit 2 in 12s; log ~/.goat/state/runs/r/logs/004-4.log\n"
               "[✗] failed — !! Phase 4 (\"lint\") FAILED — exit 2. No later phase has run\n"
               "run record: ~/.goat/state/runs/r/run.json\n")
        got = findings(log)
        self.assertEqual(len(got), 1)
        self.assertEqual(got[0][1], 3)

    def test_a_wrapped_coloured_failed_row_is_read_as_one(self):
        row = ("  \x1b[31m✗\x1b[0m failed  !! Phase 14 (\"acceptance\")\n"
               "      FAILED: exit 3. No later phase has run; the peering is left partway\n"
               "      through phase 14. exit codes are specific: 20 composition\n"
               "      refused, 21 lint refused.\n")
        got = findings(row)
        self.assertEqual([n for _, n, _ in got], [1])
        self.assertEqual(findings(row + "  ✗ interpreted  GB-251: a required acceptance check was blocked\n"), [])

    def test_a_wave_that_failed_with_its_code_passes(self):
        log = ("!! Phase 12 FAILED: portal. Steps in that wave that passed stay done\n"
               "interpreted: GB-221: the play could not reach the host\n")
        self.assertEqual(findings(log), [])

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
