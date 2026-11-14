"""affiliation_kind.py: the onboard play's kind comes from gen-roster.py's derive_kind,
and mints the same kind the play's inline table did before it moved there.

Needs PyYAML (gen-roster.py imports it), as every Ansible controller has.
Run directly: `python3 tests/ansible_filter_plugins/test_affiliation_kind.py`.
"""

import importlib.util
import unittest
from pathlib import Path

FILTERS = Path(__file__).resolve().parents[2] / "ops" / "ansible" / "filter_plugins"
SPEC = importlib.util.spec_from_file_location("affiliation_kind", FILTERS / "affiliation_kind.py")
affiliation_kind = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(affiliation_kind)
derive = affiliation_kind.affiliation_derive_kind

# The table operator-onboard.yml carried inline (_aff_kind_by_sector) until it
# called the filter: every other sector minted no kind.
PLAY_TABLE = {"commercial": "vendor", "civil-society": "civil", "ngo": "civil", "media": "civil", "academic": "civil"}
STATE_FAMILY = ("state", "military", "law-enforcement", "intelligence", "civil-government")


class PlayBehaviourTest(unittest.TestCase):
    def test_every_sector_the_play_derived_derives_the_same_kind(self):
        for sector, kind in PLAY_TABLE.items():
            with self.subTest(sector=sector):
                self.assertEqual(derive(sector), kind)
                self.assertEqual(derive(f"  {sector.upper()} "), kind)

    def test_a_state_sector_with_no_sovereign_mints_no_kind(self):
        for sector in STATE_FAMILY:
            with self.subTest(sector=sector):
                self.assertEqual(derive(sector), "")

    def test_an_absent_or_unknown_sector_mints_no_kind(self):
        for sector in ("", None, "banana", "None"):
            with self.subTest(sector=sector):
                self.assertEqual(derive(sector), "")


class SharedTableTest(unittest.TestCase):
    def test_it_is_gen_roster_derive_kind_not_a_copy(self):
        gen_roster = affiliation_kind._derive_kind()
        self.assertEqual(Path(gen_roster.__code__.co_filename).resolve(), affiliation_kind.GEN_ROSTER)
        self.assertEqual(derive("state", "USA", "USA"), "organic")
        self.assertEqual(derive("state", "EST", "USA"), "partner")

    def test_the_filter_is_registered_under_the_name_the_play_uses(self):
        play = (FILTERS.parent / "operator-onboard.yml").read_text()
        self.assertIn("| affiliation_derive_kind", play)
        self.assertIs(affiliation_kind.FilterModule().filters()["affiliation_derive_kind"], derive)


if __name__ == "__main__":
    unittest.main()
