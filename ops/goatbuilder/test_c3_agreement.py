"""c3-agreement.py, gate row C3's comparison of status, diff and verify.

The success path is also run by the Rust test status_agreement_tests in
crates/goatbuilder/src/inspect.rs. These cases hold the verdicts an operator
reads: agreement, a disagreement, a component verify was never run for, and a
verify report that read no machine at all (its transport failed), which is not
a measurement and must not be reported as a verify that was not run.

Run: python3 -m pytest -q ops/goatbuilder/test_c3_agreement.py
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "c3-agreement.py")


def rows(*pairs):
    return {"checks": [{"name": n, "detail": d} for n, d in pairs]}


STATUS = rows(
    ("zagros/zagros_hosts/inventory", "2 host(s) must answer"),
    ("zagros/core-a/portal", "converged on portal:v1.3.2@sha256:9f8e00000000 (row deploy 2026-11-01T00:00:00Z)"),
    ("zagros/core-b/portal", "pending: record names portal:v1.3.1@sha256:1d2c00000000, declared portal:v1.3.2"),
)
DIFF = rows(
    ("zagros/transport", "read via direct (mgmt-hosts.ini)"),
    ("zagros/core-a/portal", "same: portal:v1.3.2@sha256:9f8e00000000"),
    ("zagros/core-b/portal", "DRIFT: running portal:v1.3.1@sha256:1d2c00000000, declared portal:v1.3.2"),
)
VERIFY = rows(
    ("zagros/preflight", "prep: stateful"),
    ("zagros/transport", "groups [zagros_hosts] via direct (mgmt-hosts.ini)"),
    ("zagros/core-a", "converged on sha256:9f8e00000000"),
    ("zagros/core-b", "NOT converged — ledger tail: core-b | CHANGED"),
    ("zagros/portal", "portal on zagros: NOT converged everywhere (see above)"),
)
# A verify whose transport failed before any host was read: only its own rows.
VERIFY_READ_NOTHING = rows(
    ("zagros/preflight", "prep: stateful"),
    ("zagros/transport", "groups [zagros_hosts] via direct (mgmt-hosts.ini)"),
    ("zagros/zagros_hosts/inventory", "inventory read FAILED — ansible-inventory exited 1"),
    ("zagros/portal", "portal on zagros: NOT converged everywhere (see above)"),
)


class C3Agreement(unittest.TestCase):
    def run_c3(self, status, diff, verifies):
        with tempfile.TemporaryDirectory() as d:
            def put(name, doc):
                p = os.path.join(d, name)
                with open(p, "w") as fh:
                    json.dump(doc, fh)
                return p

            args = [sys.executable, SCRIPT, "--status", put("status.json", status), "--diff", put("diff.json", diff)]
            for comp, doc in verifies:
                args += ["--verify", f"{comp}={put(f'verify-{comp}.json', doc)}"]
            p = subprocess.run(args, capture_output=True, text=True)
            return p.returncode, p.stdout + p.stderr

    def test_agreement_exits_0_and_counts_machines(self):
        rc, out = self.run_c3(STATUS, DIFF, [("portal", VERIFY)])
        self.assertEqual(rc, 0, out)
        self.assertIn("c3: transport: read via direct (mgmt-hosts.ini)", out)
        self.assertIn("agree on 2 machine(s), 1 component(s): 1 healthy, 1 drifted, 0 unreachable; reached 2 of 2", out)

    def test_a_disagreement_exits_3_and_names_the_machine(self):
        flipped = json.loads(json.dumps(STATUS))
        flipped["checks"][2]["detail"] = "converged on portal:v1.3.2@sha256:9f8e00000000"
        rc, out = self.run_c3(flipped, DIFF, [("portal", VERIFY)])
        self.assertEqual(rc, 3, out)
        self.assertIn("c3: DISAGREE core-b/portal: status healthy, verify drifted, diff drifted", out)

    def test_a_component_verify_was_not_run_for_exits_3(self):
        both = rows(*[(c["name"], c["detail"]) for c in STATUS["checks"]],
                    ("zagros/core-a/herd", "converged on herd:v0.2.1@sha256:4b5a00000000"))
        rc, out = self.run_c3(both, DIFF, [("portal", VERIFY)])
        self.assertEqual(rc, 3, out)
        self.assertIn("verify was not run for: herd", out)

    def test_a_verify_that_read_no_machine_is_not_a_measurement(self):
        rc, out = self.run_c3(STATUS, DIFF, [("portal", VERIFY_READ_NOTHING)])
        self.assertEqual(rc, 2, out)
        self.assertNotIn("was not run", out, "verify was run; it read no machine")
        self.assertIn("verify-portal.json", out)
        self.assertIn("read no machine", out)


if __name__ == "__main__":
    unittest.main()
