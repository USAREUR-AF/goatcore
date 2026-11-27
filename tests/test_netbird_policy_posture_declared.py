#!/usr/bin/env python3
"""The offline ACL gate (standup phase 1) reads both files the ACL play reads.

The play that applies a goatnet's ACL at phase 12 refuses a site with no stanza
in the group taxonomy. The phase-1 gate read only the rule matrix, so a goatnet
with rules and no groups passed phase 1 and stopped at phase 12.
"""
import contextlib
import importlib.util
import io
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("posture", ROOT / "ops/scripts/netbird-policy-posture.py")
posture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(posture)

MATRIX = {"netbird_acl_matrix": {"zagros": {"rules": [{"name": "infra to all"}], "cutover": {"default_policy_action": "delete"}}}}


class DeclaredGate(unittest.TestCase):
    def run_gate(self, taxonomy):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = posture.check_declared("zagros", MATRIX, taxonomy)
        return rc, out.getvalue()

    def test_rules_and_groups_declared_passes(self):
        rc, out = self.run_gate({"netbird_acl_taxonomy": {"zagros": {"site_prefix": "ZAGROS", "groups": []}}})
        self.assertEqual(rc, 0, out)

    def test_rules_without_a_taxonomy_stanza_is_refused(self):
        rc, out = self.run_gate({"netbird_acl_taxonomy": {"_default": {}}})
        self.assertEqual(rc, 1)
        self.assertIn("no stanza", out)


if __name__ == "__main__":
    unittest.main()
