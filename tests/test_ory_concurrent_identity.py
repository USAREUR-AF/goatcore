# SPDX-License-Identifier: Apache-2.0
"""Regression coverage for the F3 measurement contract; no database required."""
import importlib.util
import io
import json
from pathlib import Path
from threading import Barrier
import unittest
from unittest.mock import patch
from urllib.error import HTTPError, URLError

SOURCE = Path(__file__).resolve().parents[1] / "ops/ansible/roles/ory/tests/soak/concurrent_identity.py"
spec = importlib.util.spec_from_file_location("concurrent_identity", SOURCE)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)
IDA = "00000000-0000-4000-8000-000000000001"
IDB = "00000000-0000-4000-8000-000000000002"


def response(status, identity=None, error=None):
    return {"http": status, "identity_id": identity, "error": error}


class ConcurrentIdentityTests(unittest.TestCase):
    def test_real_create_status_and_conflict_pass_both_orders(self):
        for a, b in [(response(201, IDA), response(409)),
                     (response(409), response(201, IDB))]:
            with self.subTest(a=a):
                result = probe.classify(a, b)
                self.assertEqual(result["f3_ok_count"], 1)
                self.assertEqual(result["f3_app_layer_caught"], 1)

    def test_both_accepted_retains_distinct_ids_and_fails(self):
        result = probe.classify(response(201, IDA), response(201, IDB))
        self.assertEqual(result["f3_outcome"], "both_accepted")
        self.assertEqual(result["f3_ok_count"], 2)
        self.assertEqual(result["f3_identity_id_a"], IDA)
        self.assertEqual(result["f3_identity_id_b"], IDB)
        self.assertEqual(result["f3_app_layer_caught"], 0)

    def test_outages_and_invalid_success_cannot_pass_as_conflicts(self):
        for other in [response(0, error="transport"), response(500),
                      response(400), response(200), response(201),
                      response(201, error="invalid_identity_response")]:
            with self.subTest(other=other):
                result = probe.classify(response(201, IDA), other)
                self.assertEqual(result["f3_outcome"], "inconclusive")
                self.assertEqual(result["f3_app_layer_caught"], 0)

    def test_no_success_is_not_a_uniqueness_pass(self):
        result = probe.classify(response(409), response(409))
        self.assertEqual(result["f3_app_layer_caught"], 0)
        self.assertEqual(result["f3_outcome"], "inconclusive")

    def test_old_200_classifier_rejects_valid_201_409_control(self):
        # This is the old driver's exact counting rule, demonstrating why
        # historical f3_app_layer_caught=0 is not evidence of two creates.
        self.assertEqual("201409".count("200"), 0)
        self.assertEqual(probe.classify(response(201, IDA), response(409))
                         ["f3_app_layer_caught"], 1)

    def test_submissions_overlap_and_keep_separate_credentials(self):
        both_started = Barrier(2)
        bodies = {}
        def create(url, body):
            bodies[url] = body
            both_started.wait(timeout=2)
            return response(201, IDA if url == "a" else IDB)
        result = probe.probe("a", "b", "fixture@example.org", create=create)
        self.assertEqual(result["f3_outcome"], "both_accepted")
        self.assertEqual(bodies["a"]["traits"]["email"], bodies["b"]["traits"]["email"])
        self.assertNotEqual(bodies["a"]["credentials"], bodies["b"]["credentials"])
        self.assertNotIn("password", json.dumps(result))

    def test_transport_error_is_structured(self):
        with patch.object(probe, "urlopen", side_effect=URLError("sensitive detail")):
            result = probe.create_identity("http://fixture", {})
        self.assertEqual(result, response(0, error="transport"))

    def test_http_conflict_does_not_export_error_body(self):
        exc = HTTPError("http://fixture", 409, "Conflict", {}, io.BytesIO(b"private"))
        with patch.object(probe, "urlopen", side_effect=exc):
            self.assertEqual(probe.create_identity("http://fixture", {}), response(409))

    def test_create_parses_identity_but_drops_other_fields(self):
        class Reply(io.BytesIO):
            status = 201
        raw = json.dumps({"id": IDA, "credentials": {"password": "secret"}}).encode()
        with patch.object(probe, "urlopen", return_value=Reply(raw)):
            self.assertEqual(probe.create_identity("http://fixture", {}), response(201, IDA))

    def test_malformed_created_response_is_inconclusive(self):
        class Reply(io.BytesIO):
            status = 201
        for raw in [b"{}", b"not json", b'{"id":"bad"}', b'{"id":null}', b'[]']:
            with self.subTest(raw=raw), patch.object(probe, "urlopen", return_value=Reply(raw)):
                result = probe.create_identity("http://fixture", {})
                self.assertEqual(result["error"], "invalid_identity_response")


if __name__ == "__main__":
    unittest.main()
