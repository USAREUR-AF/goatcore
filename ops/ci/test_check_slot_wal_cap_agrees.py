"""check-slot-wal-cap-agrees.py: the slot WAL cap is stated once, and agrees.

Each case builds a small repository in a temporary directory with the image's
postgresql.conf, the streaming-replication role's defaults and the Spock
README's section, changes one thing, and runs the check on it.

Run: python3 -m pytest -q ops/ci/test_check_slot_wal_cap_agrees.py
"""
from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "check-slot-wal-cap-agrees.py")
_spec = importlib.util.spec_from_file_location("slotcap", SCRIPT)
slotcap = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(slotcap)

CONF = "listen_addresses = '*'\nmax_slot_wal_keep_size = 1GB\nwal_level = logical\n"
ROLE = '# The cap on a pair\'s primary.\npostgres_max_slot_wal_keep_size: "6GB"\n'
TABLE = (
    "| Where | Value | Which Postgres |\n"
    "|---|---|---|\n"
    "| This image's `conf/postgresql.conf` | {image} | every site |\n"
    "| The `postgres-streaming-replication` role, variable `postgres_max_slot_wal_keep_size` | {role} | a pair's primary |\n"
)
README = (
    "# Spock\n\nIntro.\n\n## The slot WAL cap\n\n"
    "`max_slot_wal_keep_size` is set in two places:\n\n{table}\n"
    "6 GB is about 18 hours at 330 MB an hour; a 1.9 GB spike was measured.\n\n"
    "## Next\n\nMore.\n"
)


class Repo:
    def __init__(self, conf=CONF, role=ROLE, image="1 GB", role_value="6 GB", extra=None):
        self.d = tempfile.TemporaryDirectory()
        self.write(slotcap.IMAGE_CONF, conf)
        self.write(slotcap.ROLE_DEFAULTS, role)
        self.write(slotcap.README, README.format(table=TABLE.format(image=image, role=role_value)))
        for rel, text in (extra or {}).items():
            self.write(rel, text)

    def write(self, rel, text):
        p = os.path.join(self.d.name, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf-8") as fh:
            fh.write(text)

    def check(self):
        return slotcap.check(self.d.name)


class Values(unittest.TestCase):
    def test_an_agreeing_repository_passes(self):
        self.assertEqual(Repo().check(), [])

    def test_a_readme_that_swaps_the_two_values_is_refused(self):
        bad = Repo(image="6 GB", role_value="1 GB").check()
        self.assertTrue(any("image" in b and "6GB" in b.replace(" ", "") for b in bad), bad)
        self.assertTrue(any("role" in b for b in bad), bad)

    def test_the_role_value_moving_is_refused(self):
        bad = Repo(role='postgres_max_slot_wal_keep_size: "8GB"\n').check()
        self.assertTrue(any("role" in b and "8" in b for b in bad), bad)

    def test_the_last_image_line_is_the_one_postgres_uses(self):
        bad = Repo(conf=CONF + "max_slot_wal_keep_size = 2GB\n").check()
        self.assertTrue(any("image" in b and "2" in b for b in bad), bad)

    def test_an_included_file_that_sets_the_cap_is_read(self):
        conf = CONF + "include 'extra.conf'\n"
        extra = {os.path.join(os.path.dirname(slotcap.IMAGE_CONF), "extra.conf"): "max_slot_wal_keep_size = 3GB\n"}
        bad = Repo(conf=conf, extra=extra).check()
        self.assertTrue(any("image" in b and "3" in b for b in bad), bad)

    def test_a_command_line_that_sets_the_cap_is_refused(self):
        compose = {"ops/compose/x/docker-compose.yml": "command: postgres -c max_slot_wal_keep_size=4GB\n"}
        bad = Repo(extra=compose).check()
        self.assertTrue(any("ops/compose/x/docker-compose.yml:1" in b and "command line" in b for b in bad), bad)

    def test_the_same_size_in_other_units_agrees(self):
        self.assertEqual(Repo(image="1024 MB").check(), [])
        self.assertEqual(Repo(conf="max_slot_wal_keep_size = 1024MB\n").check(), [])

    def test_sizes_are_read_with_decimals_and_every_postgres_unit(self):
        self.assertEqual(slotcap.size_bytes("1.5 GB"), 1610612736)
        self.assertEqual(slotcap.size_bytes("1TB"), 1099511627776)
        self.assertEqual(slotcap.size_bytes("512kB"), 524288)
        self.assertEqual(slotcap.size_bytes("1GiB"), 1073741824)
        self.assertEqual(slotcap.size_bytes("2048"), 2147483648, "a bare number is megabytes")

    def test_other_sizes_in_the_section_prose_are_allowed(self):
        self.assertEqual(Repo().check(), [], "the section mentions 6 GB, 330 MB and 1.9 GB in prose")

    def test_a_missing_table_row_is_refused(self):
        r = Repo()
        r.write(slotcap.README, "## The slot WAL cap\n\nNo table.\n")
        bad = r.check()
        self.assertTrue(any("no table row" in b for b in bad), bad)


class Mentions(unittest.TestCase):
    def test_a_size_beside_the_setting_anywhere_is_refused(self):
        bad = Repo(extra={"docs/x.md": "The cap max_slot_wal_keep_size is 1 GB.\n"}).check()
        self.assertTrue(any(b.startswith("docs/x.md:1") for b in bad), bad)

    def test_a_size_on_the_next_line_is_refused(self):
        bad = Repo(extra={"docs/x.md": "the setting `max_slot_wal_keep_size`\n(default 1 GiB).\n"}).check()
        self.assertTrue(any(b.startswith("docs/x.md:1") for b in bad), bad)

    def test_a_byte_literal_beside_the_setting_is_refused(self):
        bad = Repo(extra={"rules/a.yml": "expr: retained > 0.8 * 1073741824 # max_slot_wal_keep_size\n"}).check()
        self.assertTrue(any(b.startswith("rules/a.yml:1") for b in bad), bad)

    def test_the_unlimited_sentinel_is_not_a_size(self):
        extra = {"rules/a.yml": "expr: spock_config_max_slot_wal_keep_size_bytes >= 9223372036854775807\n"}
        self.assertEqual(Repo(extra=extra).check(), [])

    def test_a_mention_without_a_size_passes(self):
        self.assertEqual(Repo(extra={"docs/x.md": "Read max_slot_wal_keep_size on the site.\n"}).check(), [])

    def test_the_allow_list_exempts_a_file_with_its_reason(self):
        extra = {
            "docs/drill.md": "Set `max_slot_wal_keep_size = 1GB` for the drill.\n",
            slotcap.ALLOW: "docs/drill.md  a drill sets a small cap on purpose\n",
        }
        self.assertEqual(Repo(extra=extra).check(), [])

    def test_an_allow_line_without_a_reason_is_refused(self):
        extra = {"docs/drill.md": "Set `max_slot_wal_keep_size = 1GB`.\n", slotcap.ALLOW: "docs/drill.md\n"}
        bad = Repo(extra=extra).check()
        self.assertTrue(any(slotcap.ALLOW in b and "reason" in b for b in bad), bad)


class ThisRepository(unittest.TestCase):
    def run_check(self, *args):
        p = subprocess.run([sys.executable, SCRIPT, *args], capture_output=True, text=True)
        return p.returncode, p.stdout + p.stderr

    def test_this_repository_states_the_cap_once(self):
        rc, out = self.run_check()
        self.assertEqual(rc, 0, out)

    def test_each_planted_violation_is_refused(self):
        rc, out = self.run_check("--self-test")
        self.assertIn("planted violations is refused", out)
        self.assertEqual(rc, 0, out)


if __name__ == "__main__":
    unittest.main()
