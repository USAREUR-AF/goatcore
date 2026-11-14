#!/usr/bin/env python3
"""The end state grades federation-proxy, signal federation's dual-path proxy.

`signal-federation-transport.yml` starts a `federation-proxy` container next to
signal on a site that declares `signal_federation_proxy_enabled: true`, and
`mgmt-host.yml` keeps its compose profile on for the same flag. A read of the
running containers on two joined sites of a standing goatnet (2026-11-13) found
it on both, but the catalog had no row for it, so the model graded it neither
way. This test holds the row: present where the flag is true, absent with the
flag as its reason where it is not, and graded on its container.
"""
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("endstate", ROOT / "ops/goatbuilder/endstate/endstate.py")
ENDSTATE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ENDSTATE)


def model(vars_text):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        gv = root / "ops/ansible/group_vars/zagros_b_hosts"
        gv.mkdir(parents=True)
        (gv / "vars.yml").write_text(vars_text)
        return ENDSTATE.model("zagros-b", root)


JOINER = "goatnet_founder: false\njoin_founder_site: zagros\n"


class FederationProxy(unittest.TestCase):
    def test_the_catalog_has_a_row_for_it(self):
        row = ENDSTATE.catalog(ROOT)["services"].get("federation-proxy")
        self.assertIsNotNone(row, "federation-proxy has no row in ops/goatbuilder/endstate/catalog.yaml")
        self.assertEqual(row["present"], {"var": "signal_federation_proxy_enabled", "default": False})
        self.assertEqual(row["play"], "signal-federation-transport.yml")
        self.assertEqual(row["containers"], ["federation-proxy"])

    def test_a_site_that_turns_it_on_runs_it(self):
        s = model(JOINER + "signal_federation_proxy_enabled: true\n")["services"]["federation-proxy"]
        self.assertIs(s["present"], True)
        self.assertEqual(s["containers"], ["federation-proxy"])

    def test_its_play_is_the_signal_federation_transport_play(self):
        row = ENDSTATE.catalog(ROOT)["services"]["federation-proxy"]
        self.assertEqual(ENDSTATE.deploy_play(ROOT, "federation-proxy", row, {}), "ops/ansible/signal-federation-transport.yml")

    def test_a_site_that_does_not_set_it_reads_absent_with_its_flag(self):
        s = model(JOINER)["services"]["federation-proxy"]
        self.assertIs(s["present"], False)
        self.assertIn("signal_federation_proxy_enabled", s["reason"])

    def test_a_site_that_turns_it_off_reads_its_flag(self):
        s = model(JOINER + "signal_federation_proxy_enabled: false\n")["services"]["federation-proxy"]
        self.assertIs(s["present"], False)
        self.assertEqual(s["reason"], "`signal_federation_proxy_enabled` is false in group_vars")

    def test_the_grade_reads_it_present_or_absent_never_ungraded(self):
        for flag, status in (("true", "PASS"), ("false", "N/A")):
            m = model(JOINER + f"signal_federation_proxy_enabled: {flag}\n")
            g = ENDSTATE.grade(m, {"hosts": {}}, None)
            row = next((r for r in g["services"] if r["service"] == "federation-proxy"), None)
            self.assertIsNotNone(row, "the grade has no federation-proxy row")
            self.assertEqual(row["presence"]["status"], status, row)


if __name__ == "__main__":
    sys.exit(unittest.main())
