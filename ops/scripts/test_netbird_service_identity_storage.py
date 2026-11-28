"""Credential transport regressions; all commands and tokens are synthetic."""
import importlib.util
import json
import pathlib
import subprocess
import unittest
from unittest.mock import patch

path = pathlib.Path(__file__).with_name("netbird-service-identities.py")
spec = importlib.util.spec_from_file_location("service_identities", path)
identities = importlib.util.module_from_spec(spec)
spec.loader.exec_module(identities)


class StorageTests(unittest.TestCase):
    def test_create_and_edit_send_document_only_on_stdin(self):
        token = "synthetic-sensitive-value"
        registry = {"onepassword": {"account": "fixture", "vault": "test"}}
        identity = {"onepassword": {"item": "sample", "field": "service-token"}}
        for exists in (False, True):
            with self.subTest(exists=exists):
                calls = []

                def run(argv, **kwargs):
                    calls.append((argv, kwargs))
                    if len(calls) == 1:
                        document = {"id": "fixture-id", "fields": []}
                        return subprocess.CompletedProcess(argv, 0 if exists else 1, json.dumps(document), "")
                    self.assertNotIn(token, " ".join(argv))
                    self.assertFalse(any("--template" in arg for arg in argv))
                    self.assertEqual(json.loads(kwargs["input"])["fields"][-1]["value"], token)
                    if not exists:
                        self.assertEqual(argv[-1], "-")
                    return subprocess.CompletedProcess(argv, 1, "", "rejected " + token)

                with patch.object(identities.subprocess, "run", side_effect=run):
                    ok, status = identities.store_in_1password(registry, identity, "sample", token)
                self.assertFalse(ok)
                self.assertNotIn(token, status)
                self.assertIn("op exited 1", status)
                self.assertEqual(len(calls), 2)


class ReferencesOnly(unittest.TestCase):
    """Under the 1password posture a minted token is on disk only as a reference
    (ADR 1186); the value goes to 1Password and is read back from there."""

    def test_a_stored_token_the_api_rejects_is_replaced_and_a_live_one_is_not(self):
        spec = {"onepassword": {"vault": "v", "account": "a"}}
        ident = {"onepassword": {"item": "i", "field": "f"}}
        doc = {"id": "x", "fields": [{"label": "f", "value": "old-token"}]}
        for is_dead, want_ok in ((True, True), (False, False)):
            with self.subTest(dead=is_dead):
                sent = []
                def run(argv, **kwargs):
                    if "get" in argv:
                        return subprocess.CompletedProcess(argv, 0, json.dumps(doc), "")
                    sent.append(kwargs.get("input"))
                    return subprocess.CompletedProcess(argv, 0, "", "")
                with patch.object(identities.subprocess, "run", side_effect=run):
                    ok, note = identities.store_in_1password(spec, ident, "zagros", "new-token", dead=lambda t: is_dead)
                self.assertEqual(ok, want_ok, note)
                self.assertEqual(len(sent), 1 if is_dead else 0)
                self.assertNotIn("new-token", note)
                self.assertNotIn("old-token", note)

    def test_the_same_reference_already_declared_is_not_written_twice(self):
        import tempfile
        d = pathlib.Path(tempfile.mkdtemp())
        (d / "36-site.refs").write_text('export T="op://v/i/f"\n')
        identities.append_reference(d / "30-site-services.refs", "T", "op://v/i/f", "k", "u")
        self.assertFalse((d / "30-site-services.refs").exists())
        with self.assertRaises(RuntimeError):
            identities.append_reference(d / "30-site-services.refs", "T", "op://v/other/f", "k", "u")
    def test_a_cli_that_waits_on_its_prompt_is_a_named_refusal_not_a_hang(self):
        spec = {"onepassword": {"vault": "v", "account": "a"}}
        ident = {"onepassword": {"item": "i", "field": "f"}}
        def run(argv, **kwargs):
            self.assertIn("timeout", kwargs)
            raise subprocess.TimeoutExpired(argv, kwargs["timeout"])
        with patch.object(identities.subprocess, "run", side_effect=run):
            ok, note = identities.store_in_1password(spec, ident, "zagros", "token-value")
        self.assertFalse(ok)
        self.assertIn("did not answer", note)
        self.assertIn("approve", note)
        self.assertNotIn("token-value", note)

    def test_only_a_reference_is_ever_written(self):
        import os, stat, tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory() as d:
            refs = Path(d) / ".envrc.local.d" / "30-site-services.refs"
            identities.append_reference(refs, "SAMPLE_PORTAL_TOKEN", "op://vault/sample-portal/token", "portal-mint", "svc-portal")
            text = refs.read_text()
            self.assertIn('export SAMPLE_PORTAL_TOKEN="op://vault/sample-portal/token"', text)
            self.assertEqual(stat.S_IMODE(os.stat(refs).st_mode), 0o600)
            with self.assertRaises(RuntimeError):
                identities.append_reference(refs, "SAMPLE_OTHER_TOKEN", "nbp_not_a_reference_0000000000000000", "x", "y")
            self.assertNotIn("nbp_", refs.read_text())
            with self.assertRaises(RuntimeError):
                identities.append_reference(refs, "SAMPLE_PORTAL_TOKEN", "op://vault/other/token", "portal-mint", "svc-portal")

    def test_the_reference_names_the_registry_item(self):
        spec = {"onepassword": {"vault": "vault"}}
        ident = {"onepassword": {"item": "{goatnet}-portal", "field": "token"}}
        self.assertEqual(identities.item_reference(spec, ident, "sample"), "op://vault/sample-portal/token")
        self.assertIsNone(identities.item_reference(spec, {}, "sample"))

    def test_a_reference_is_read_back_from_the_store_not_from_disk(self):
        import tempfile
        from pathlib import Path
        from unittest import mock
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "30-site-services.refs").write_text('export SAMPLE_PORTAL_TOKEN="op://vault/sample-portal/token"\nexport OTHER=plain\n')
            done = mock.Mock(returncode=0, stdout="resolved-value")
            with mock.patch.object(identities.subprocess, "run", return_value=done) as run:
                got = identities.reference_values(["SAMPLE_PORTAL_TOKEN", "OTHER"], Path(d), "acct")
            self.assertEqual(got, {"SAMPLE_PORTAL_TOKEN": "resolved-value"})
            self.assertEqual(run.call_args[0][0], ["op", "--account", "acct", "read", "--no-newline", "op://vault/sample-portal/token"])

    def test_the_1password_mint_never_takes_the_plaintext_path(self):
        from pathlib import Path
        src = Path(identities.__file__).read_text()
        branch = src[src.index('if store == "1password":\n                    # References only'):src.index("if store in p.VALUE_STORES:")]
        self.assertNotIn("append_export", branch)
        self.assertIn("continue", branch)


if __name__ == "__main__":
    unittest.main()
