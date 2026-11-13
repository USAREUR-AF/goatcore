#!/usr/bin/env python3
"""The end-state model words why a service is absent at a site.

`goatbuilder join-node` prints, for a joining site, every catalog service the
model marks absent, with this reason (docs/design/goatbuilder.md, `join-node`
part (d)); the grade's N/A row reads the same field, so the plan and the grade
cannot disagree. A service the site turns off reads its flag. An admin surface
at a joiner reads as single-homed at the founder under ADR 1084 §D3, with the
declaration that keeps it off. The same surface at a founder reads its flag
only: the single-homing is not a rule of the machine class."""
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("endstate", ROOT / "ops/goatbuilder/endstate/endstate.py")
ENDSTATE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ENDSTATE)

D3 = "single-homed at the founder (ADR 1084 §D3); every site once ADR 1165 lifts it for this class"


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

    def test_an_admin_surface_at_a_joiner_reads_d3_and_its_declaration(self):
        m = model(JOINER)["services"]
        self.assertEqual(m["id-admin-console"]["reason"], f"{D3} (declared by: `id_admin_console_enabled` is false in group_vars)")
        self.assertEqual(
            m["netbird-dashboard"]["reason"],
            f"{D3} (declared by: founder-only service; this site joins a goatnet (goatnet_founder: false))",
        )

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
