# SPDX-License-Identifier: Apache-2.0
"""Exercise the real shell login helper against controlled HTTP responses."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SOURCE = (ROOT / "ops/postgres/spock/tests/partition-harness/run.sh").read_text()
HELPER = SOURCE[SOURCE.index("kratos_login()"):SOURCE.index("kname()")]

CURL = """
curl() {
  if [[ "$*" == *"/self-service/login/api"* ]]; then
    printf '%s' '{"id":"fixture-flow"}'
    return
  fi
  local output=""
  while [[ $# -gt 0 ]]; do
    if [[ "$1" == "-o" ]]; then output="$2"; shift; fi
    shift
  done
  printf '%s' "$FIXTURE_RESPONSE" > "$output"
  printf '%s' "$FIXTURE_STATUS"
}
"""


@unittest.skipUnless(shutil.which("jq"), "jq is required by the partition harness")
class LoginHelperTests(unittest.TestCase):
    def run_response(self, status, body):
        with tempfile.TemporaryDirectory() as scratch:
            env = dict(os.environ, SCRATCH=scratch, FIXTURE_STATUS=str(status),
                       FIXTURE_RESPONSE=json.dumps(body))
            return subprocess.run(
                ["bash", "-c", "set -euo pipefail\nKRATOS_PUBLIC_a=54433\n"
                 + CURL + HELPER + '\nkratos_login a fixture@example.org "Fixture!Password"'],
                env=env, capture_output=True, text=True,
            )

    def test_success_returns_identity_without_token(self):
        result = self.run_response(200, {
            "session": {"active": True, "identity": {"id": "fixture-identity"}},
            "session_token": "never-print-this",
        })
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "fixture-identity")
        self.assertNotIn("never-print-this", result.stdout + result.stderr)

    def test_precise_wrong_password_rejection(self):
        result = self.run_response(400, {"ui": {"messages": [{"id": 4000006}]}})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "invalid_credentials")

    def test_other_bad_request_is_not_wrong_password(self):
        result = self.run_response(400, {"ui": {"messages": [{"id": 4000002}]}})
        self.assertNotEqual(result.returncode, 0)

    def test_outage_cannot_count_as_rejection(self):
        result = self.run_response(503, {"error": "unavailable"})
        self.assertNotEqual(result.returncode, 0)

    def test_missing_session_cannot_count_as_login(self):
        self.assertNotEqual(self.run_response(200, {}).returncode, 0)

    def test_inactive_session_cannot_count_as_login(self):
        result = self.run_response(200, {
            "session": {"active": False, "identity": {"id": "fixture-identity"}},
        })
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
