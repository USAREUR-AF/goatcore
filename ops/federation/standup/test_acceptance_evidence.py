"""The evidence bundle and the verdict rule of a federation acceptance run."""
import base64
import importlib.util
import io
import json
import os
from pathlib import Path
import contextlib
import tempfile
import unittest

_spec = importlib.util.spec_from_file_location("acceptance_evidence", Path(__file__).with_name("acceptance_evidence.py"))
ev = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ev)

T = "2026-11-09T00:00:00Z"


def check(name, verdict, reason="why", required=True, direction="inbound"):
    return ev.make_check(name, direction, verdict, reason, T, ended=T, required=required)


class Verdict(unittest.TestCase):
    def test_all_required_pass_is_pass(self):
        self.assertEqual(ev.overall([check("a", "PASS"), check("b", "PASS")])[:2], ("PASS", 0))

    def test_any_fail_is_fail_even_beside_blocked(self):
        v = ev.overall([check("a", "PASS"), check("b", "BLOCKED"), check("c", "FAIL")])
        self.assertEqual(v[:2], ("FAIL", 1))
        self.assertIn("c", v[2])

    def test_a_failed_optional_check_still_fails(self):
        self.assertEqual(ev.overall([check("a", "PASS"), check("b", "FAIL", required=False)])[:2], ("FAIL", 1))

    def test_blocked_or_skipped_required_check_is_incomplete(self):
        for v in ("BLOCKED", "SKIP"):
            with self.subTest(v=v):
                verdict = ev.overall([check("a", "PASS"), check("b", v)])
                self.assertEqual(verdict[:2], ("INCOMPLETE", 3))
                self.assertIn("b", verdict[2])

    def test_optional_skip_does_not_block(self):
        self.assertEqual(ev.overall([check("a", "PASS"), check("b", "SKIP", required=False)])[:2], ("PASS", 0))

    def test_nothing_required_proves_nothing(self):
        self.assertEqual(ev.overall([])[:2], ("INCOMPLETE", 3))
        self.assertEqual(ev.overall([check("a", "NOT APPLICABLE", required=False)])[:2], ("INCOMPLETE", 3))

    def test_not_applicable_beside_passes_is_pass(self):
        self.assertEqual(ev.overall([check("a", "PASS"), check("b", "NOT APPLICABLE", required=False)])[:2], ("PASS", 0))


class Records(unittest.TestCase):
    def test_a_required_check_cannot_be_not_applicable(self):
        with self.assertRaises(ev.Malformed):
            check("a", "NOT APPLICABLE")

    def test_anything_but_pass_needs_a_reason(self):
        for v in ("FAIL", "BLOCKED", "SKIP"):
            with self.subTest(v=v), self.assertRaises(ev.Malformed):
                check("a", v, reason=" ")
        self.assertEqual(check("a", "PASS", reason="")["verdict"], "PASS")

    def test_unknown_words_are_refused(self):
        with self.assertRaises(ev.Malformed):
            check("a", "pass")
        with self.assertRaises(ev.Malformed):
            check("a", "PASS", direction="sideways")
        with self.assertRaises(ev.Malformed):
            check("a b", "PASS")

    def test_observations_round_trip_through_the_shell_encoding(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "obs")
            with open(p, "w") as f:
                for k, v in (("trace", "line one\nline two"), ("received", "")):
                    f.write("%s\t%s\n" % (k, base64.b64encode(v.encode()).decode()))
            self.assertEqual(ev.read_obs(p), {"trace": "line one\nline two", "received": ""})


class Bundle(unittest.TestCase):
    def run_cli(self, *argv):
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
            rc = ev.main(list(argv))
        return rc, out.getvalue()

    def test_record_then_finalize_writes_the_bundle(self):
        with tempfile.TemporaryDirectory() as d:
            ag = os.path.join(d, "agreement.json")
            with open(ag, "w") as f:
                json.dump({"spec": "federation-agreement/v1", "expires": "2026-12-01"}, f)
            checks = os.path.join(d, "checks.jsonl")
            self.assertEqual(self.run_cli("record", checks, "--name", "delivery/inbound/release/all", "--direction",
                                          "inbound", "--tier", "positive", "--verdict", "PASS", "--started", T)[0], 0)
            self.assertEqual(self.run_cli("record", checks, "--name", "spire", "--direction", "local", "--verdict",
                                          "SKIP", "--reason", "not configured", "--started", T)[0], 0)
            out = os.path.join(d, "evidence", "acceptance")
            rc, text = self.run_cli("finalize", checks, "--out", out, "--agreement", ag, "--us", "xaa", "--them", "xab",
                                    "--run-id", "acc-1", "--started", T, "--inputs", "theirs_rest=1", "spire=0")
            self.assertEqual(rc, 3)
            self.assertIn("acceptance-counts: pass=1 fail=0 skip=1 as=xaa blocked=0 not_applicable=0", text)
            self.assertIn("ACCEPTANCE FAILED", text)
            b = json.load(open(os.path.join(out, "result.json")))
            self.assertEqual(b["schema"], "goat-federation-acceptance/v1")
            self.assertEqual((b["verdict"], b["exit"]), ("INCOMPLETE", 3))
            self.assertEqual(b["inputs"], {"theirs_rest": True, "spire": False})
            self.assertTrue(b["agreement"]["sha256"].startswith("sha256:"))
            self.assertEqual([c["name"] for c in b["checks"]], ["delivery/inbound/release/all", "spire"])
            for c in b["checks"]:
                for field in ("name", "direction", "verdict", "reason", "started_at", "ended_at", "observations", "required"):
                    self.assertIn(field, c)
            self.assertTrue(os.path.exists(os.path.join(out, "history", "acc-1.json")))
            self.assertIn("| SKIP | local | `spire` |", open(os.path.join(out, "summary.md")).read())

    def test_a_malformed_record_is_refused_and_not_written(self):
        with tempfile.TemporaryDirectory() as d:
            checks = os.path.join(d, "checks.jsonl")
            rc, _ = self.run_cli("record", checks, "--name", "a", "--direction", "inbound", "--verdict", "FAIL",
                                 "--started", T)
            self.assertEqual(rc, 2)
            self.assertFalse(os.path.exists(checks) and open(checks).read().strip())

    def test_an_unwritable_bundle_exits_2(self):
        with tempfile.TemporaryDirectory() as d:
            blocker = os.path.join(d, "file")
            open(blocker, "w").close()
            rc, _ = self.run_cli("finalize", os.path.join(d, "none.jsonl"), "--out", os.path.join(blocker, "x"),
                                 "--agreement", blocker, "--us", "xaa", "--them", "xab", "--run-id", "r", "--started", T)
            self.assertEqual(rc, 2)


if __name__ == "__main__":
    unittest.main()
