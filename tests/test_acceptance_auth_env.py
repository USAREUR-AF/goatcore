#!/usr/bin/env python3
"""A row's `auth:` names the environment variable run-acceptance.py reads its bearer
token from. Secret stores export upper-case names per site (EFDI_PORTAL_OPERATOR_API_KEY),
so a literal like `portal_operator_key` is a variable no store sets: the portal rows read
UNKNOWN ("needs $portal_operator_key") on a run made under with-secrets. Every auth must
come from the value set (generate-values.py), as `{{ nb_api_token_env }}` does.

Run: python3 -m unittest tests/test_acceptance_auth_env.py -v
"""
import re
import unittest
from pathlib import Path

ROWS = Path(__file__).resolve().parents[1] / "ops/validation/acceptance-tests.yaml"


class AuthEnv(unittest.TestCase):
    def test_every_auth_is_a_derived_value(self):
        auths = re.findall(r"\bauth:\s*(\"[^\"]*\"|[^,}\s]+)", ROWS.read_text())
        self.assertGreaterEqual(len(auths), 4, "expected the mgmt API and portal rows; test miswired")
        literal = [a for a in auths if not re.fullmatch(r'"\{\{ \w+ \}\}"', a)]
        self.assertEqual(literal, [], "auth names an env var no secret store exports")


if __name__ == "__main__":
    unittest.main()
