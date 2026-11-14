#!/usr/bin/env python3
"""The end-state model words why a service is absent at a site, and names a gap as a gap.

`goatbuilder join-node` prints, for a joining site, each gap the model names
and each service the site turns off (docs/design/goatbuilder.md, `join-node`
part (d)); the grade reads the same fields, so the plan and the grade cannot
disagree. A service the site turns off reads its flag. Every site is declared to
run every service (ADR 1165, gate group I): an admin surface a joining site
does not run yet is a gap, "declared, not yet built at this site", with the
gate rows that close it, never an absence by design. The same surface turned
off at a founder reads its flag only."""
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("endstate", ROOT / "ops/goatbuilder/endstate/endstate.py")
ENDSTATE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ENDSTATE)

ADMIN_GAP = {"row": "I1, I4", "why": "an admin surface cannot be turned on at a second site before its store is at the multi-writer schema and its write class is switched on there"}
ADMIN_GAP_TEXT = "declared, not yet built at this site: " + ADMIN_GAP["why"] + " (gate row I1, I4)"


def model(vars_text):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        gv = root / "ops/ansible/group_vars/zagros_b_hosts"
        gv.mkdir(parents=True)
        (gv / "vars.yml").write_text(vars_text)
        return ENDSTATE.model("zagros-b", root)


JOINER = "goatnet_founder: false\njoin_founder_site: zagros\nisitup_enabled: false\nid_admin_console_enabled: false\n"


class AbsentReason(unittest.TestCase):
    def test_a_flag_off_reads_its_flag(self):
        s = model(JOINER)["services"]["isitup"]
        self.assertIs(s["present"], False)
        self.assertEqual(s["reason"], "`isitup_enabled` is false in group_vars")

    def test_an_admin_surface_at_a_joiner_is_a_named_gap(self):
        m = model(JOINER)["services"]
        for svc in ("id-admin-console", "netbird-dashboard"):
            self.assertIs(m[svc]["present"], True, svc)
            self.assertEqual(m[svc]["gap"], ADMIN_GAP, svc)
            self.assertIsNone(m[svc]["reason"], svc)

    def test_the_grade_reads_a_gap_as_a_gap_never_na(self):
        g = ENDSTATE.grade(model(JOINER), {"hosts": {}}, None)
        row = next(r for r in g["services"] if r["service"] == "id-admin-console")
        self.assertEqual(row["presence"]["status"], "GAP")
        self.assertEqual(row["presence"]["why"], ADMIN_GAP_TEXT)
        self.assertNotIn("active", row, "a gap has nothing built to check")

    def test_a_founder_that_turns_an_admin_surface_off_reads_the_flag_only(self):
        s = model("isitup_enabled: false\nid_admin_console_enabled: false\n")["services"]["id-admin-console"]
        self.assertIs(s["present"], False)
        self.assertEqual(s["reason"], "`id_admin_console_enabled` is false in group_vars")

    def test_a_present_service_has_no_reason(self):
        s = model(JOINER.replace("isitup_enabled: false", "isitup_enabled: true"))["services"]["isitup"]
        self.assertIs(s["present"], True)
        self.assertIsNone(s["reason"])

    def test_the_grade_reads_the_same_reason(self):
        m = model(JOINER)
        g = ENDSTATE.grade(m, {"hosts": {}}, None)
        row = next(r for r in g["services"] if r["service"] == "isitup")
        self.assertEqual(row["presence"]["status"], "N/A")
        self.assertEqual(row["presence"]["why"], "declared absent: `isitup_enabled` is false in group_vars")


if __name__ == "__main__":
    sys.exit(unittest.main())
