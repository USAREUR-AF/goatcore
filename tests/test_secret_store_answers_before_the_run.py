"""A stand-up asks the secret store for one bounded read before it touches a
host (GB-392), so an unattended run does not stop mid-way on a prompt nobody
sees."""
import importlib.util
import os
import pathlib
import subprocess
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("posture", ROOT / "ops/secrets/posture.py")
posture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(posture)


def env(**extra):
    base = {"PATH": os.environ.get("PATH", ""), "GOAT_SECRET_STORE": "1password", "OP_ACCOUNT": "example.1password.com"}
    base.update(extra)
    return mock.patch.dict(os.environ, base, clear=True)


class Answers(unittest.TestCase):
    def test_one_bounded_read_of_a_reference_with_no_standard_input(self):
        calls = []
        def run(cmd, **kw):
            calls.append((cmd, kw))
            return subprocess.CompletedProcess(cmd, 0, "the-value", "")
        with env(X_PG_PASS="op://vault/item/field"):
            ok, text = posture.answers(run=run)
        self.assertTrue(ok)
        self.assertEqual(len(calls), 1)
        cmd, kw = calls[0]
        self.assertEqual(cmd[-3:], ["read", "--no-newline", "op://vault/item/field"])
        self.assertEqual(kw["stdin"], subprocess.DEVNULL)
        self.assertGreater(kw["timeout"], 0)
        self.assertNotIn("the-value", text)

    def test_a_store_that_waits_on_a_prompt_is_a_named_refusal(self):
        def run(cmd, **kw):
            raise subprocess.TimeoutExpired(cmd, kw["timeout"])
        with env(X_PG_PASS="op://vault/item/field"):
            ok, text = posture.answers(run=run)
        self.assertFalse(ok)
        self.assertIn("did not answer", text)
        self.assertIn("unlock", text)

    def test_a_refused_read_is_a_named_refusal(self):
        with env(X_PG_PASS="op://vault/item/field"):
            ok, text = posture.answers(run=lambda cmd, **kw: subprocess.CompletedProcess(cmd, 1, "", "not signed in"))
        self.assertFalse(ok)
        self.assertIn("op exited 1", text)

    def test_no_reference_yet_asks_for_the_vault_list(self):
        seen = []
        with env():
            with mock.patch.object(posture, "REPO", "/nonexistent"):
                ok, _ = posture.answers("/nonexistent", run=lambda cmd, **kw: seen.append(cmd) or subprocess.CompletedProcess(cmd, 0, "[]", ""))
        self.assertTrue(ok)
        self.assertEqual(seen[0][-4:], ["vault", "list", "--format", "json"])

    def test_other_stores_and_an_opt_out_are_not_asked(self):
        boom = mock.Mock(side_effect=AssertionError("the store must not be called"))
        with env(GOAT_SECRET_STORE="env"):
            self.assertEqual(posture.answers(run=boom), (True, "not asked"))
        with env(GOAT_STORE_PROBE="0"):
            self.assertEqual(posture.answers(run=boom), (True, "not asked"))

    def test_the_gate_names_the_code(self):
        src = (ROOT / "ops/scripts/standup-required-inputs.py").read_text()
        self.assertIn("GB-392", src)
        self.assertIn("(a.from_phase or 0) <= 1", src)


if __name__ == "__main__":
    unittest.main()
