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


if __name__ == "__main__":
    unittest.main()
