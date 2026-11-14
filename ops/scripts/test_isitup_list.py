#!/usr/bin/env python3
"""Tests for isitup-list.py: the parts that need no ansible and no host.

  - which goatnets a set of changed paths reaches (`changed`);
  - the verdict for a declared list against a page's meta.json (`check`),
    for the three meta.json shapes: current, behind, and written before the
    stamp existed;
  - `apply --dry-run` prints the list-only play and runs nothing.

The render itself (ansible) is covered by
ops/ansible/roles/isitup/tests/list-freshness.sh, which also runs this file.

Run: python3 ops/scripts/test_isitup_list.py
"""
import contextlib
import importlib.util
import io
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("isitup_list", HERE / "isitup-list.py")
il = importlib.util.module_from_spec(spec)
spec.loader.exec_module(il)

KNOWN = ["zagros_hosts", "zagros_prod_hosts"]
DECLARED = {"sha256": "a" * 64, "source": {"goatcore": "1" * 40, "overlay": "2" * 40},
            "rows": [{"id": "portal"}, {"id": "docs"}]}


def meta(ids, stamp=None):
    m = {"rows": {i: {"lifecycle": "live"} for i in ids}}
    if stamp is not None:
        m["rendered"] = stamp
    return m


class ChangedPaths(unittest.TestCase):
    def test_a_goatnets_own_declaration_names_that_goatnet(self):
        for path in ("ops/ansible/group_vars/zagros_hosts/vars.yml",
                     "ops/ansible/group_vars/zagros_hosts/external-service-board.yml"):
            self.assertEqual(il.groups_touched([path], KNOWN), ["zagros_hosts"], path)

    def test_a_group_name_is_matched_whole(self):
        self.assertEqual(il.groups_touched(["ops/ansible/group_vars/zagros_prod_hosts/vars.yml"], KNOWN),
                         ["zagros_prod_hosts"])

    def test_shared_inputs_name_every_goatnet(self):
        for path in ("ops/ansible/group_vars/all.yml",
                     "ops/ansible/roles/isitup/templates/isitup-rows.json.j2",
                     "ops/ansible/roles/isitup/templates/services.env.j2",
                     "goatcore.ref"):
            self.assertEqual(il.groups_touched([path], KNOWN), KNOWN, path)

    def test_unrelated_paths_name_none(self):
        paths = ["docs/design/isitup-status-page.md", "ops/ansible/roles/isitup/files/check.sh",
                 "ops/ansible/group_vars/zagros_obs_hosts/vars.yml", "ops/ansible/roles/isitup/tasks/main.yml"]
        self.assertEqual(il.groups_touched(paths, KNOWN), [])

    def test_a_goatnet_without_the_page_is_not_named(self):
        self.assertEqual(il.groups_touched(["ops/ansible/group_vars/other_hosts/vars.yml"], KNOWN), [])

    def test_names_are_written_either_way(self):
        self.assertEqual(il.group_of("zagros-prod"), "zagros_prod_hosts")
        self.assertEqual(il.group_of("zagros_prod"), "zagros_prod_hosts")
        self.assertEqual(il.name_of("zagros_prod_hosts"), "zagros-prod")


class Verdict(unittest.TestCase):
    def test_a_page_carrying_the_declared_list_is_current(self):
        stamp = {"sha256": "a" * 64, "at": "2026-01-01T00:00:00Z", "source": {"overlay": "2" * 40}}
        code, lines = il.compare("zagros", DECLARED, meta(["portal", "docs"], stamp))
        self.assertEqual(code, 0)
        self.assertIn("zagros: current", lines[0])
        self.assertIn("2" * 12, lines[0])

    def test_a_page_behind_the_declaration_names_the_row_that_should_be_gone(self):
        stamp = {"sha256": "b" * 64, "at": "2026-01-01T00:00:00Z", "source": {"commit": "3" * 40}}
        code, lines = il.compare("zagros", DECLARED, meta(["portal", "docs", "gone"], stamp))
        text = "\n".join(lines)
        self.assertEqual(code, 1)
        self.assertIn("zagros: BEHIND", text)
        self.assertIn("on the page, not declared", text)
        self.assertIn("gone", text)
        self.assertIn("3" * 12, text)

    def test_a_declared_row_missing_from_the_page_is_named(self):
        code, lines = il.compare("zagros", DECLARED, meta(["portal"], {"sha256": "b" * 64}))
        self.assertEqual(code, 1)
        self.assertIn("declared, not on the page: docs", "\n".join(lines))

    def test_same_ids_with_a_changed_field_is_still_behind(self):
        code, lines = il.compare("zagros", DECLARED, meta(["portal", "docs"], {"sha256": "b" * 64}))
        self.assertEqual(code, 1)
        self.assertIn("a field of one or more rows differs", "\n".join(lines))

    def test_a_meta_json_from_before_the_stamp_is_unknown_not_current(self):
        code, lines = il.compare("zagros", DECLARED, meta(["portal", "docs"]))
        self.assertEqual(code, 3)
        self.assertIn("carries no list stamp", lines[0])

    def test_the_declaration_commit_is_the_overlays_in_an_assembled_tree(self):
        self.assertEqual(il.source_commit({"goatcore": "1" * 40, "overlay": "2" * 40}), "2" * 40)
        self.assertEqual(il.source_commit({"commit": "3" * 40}), "3" * 40)
        self.assertEqual(il.source_commit({}), "")
        self.assertEqual(il.source_commit(None), "")


class ApplyDryRun(unittest.TestCase):
    def run_main(self, argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = il.main(argv)
        return code, out.getvalue(), err.getvalue()

    def test_dry_run_prints_the_list_only_play_and_no_secret_variable(self):
        il.transport = lambda group: ("direct", ["-i", "inventory/mgmt-hosts.ini"])
        code, out, _ = self.run_main(["apply", "zagros", "--dry-run"])
        self.assertEqual(code, 0)
        self.assertIn("-e target=zagros_hosts isitup-rows.yml", out)
        self.assertNotIn("isitup-deploy.yml", out)
        self.assertNotIn("SECRET", out.upper())

    def test_verify_only_is_passed_to_the_play(self):
        il.transport = lambda group: ("direct", ["-i", "inventory/mgmt-hosts.ini"])
        code, out, _ = self.run_main(["apply", "zagros-prod", "--verify-only", "--dry-run"])
        self.assertEqual(code, 0)
        self.assertIn("-e target=zagros_prod_hosts -e isitup_rows_verify_only=true isitup-rows.yml", out)

    def test_apply_with_nothing_named_refuses(self):
        code, _, err = self.run_main(["apply"])
        self.assertEqual(code, 2)
        self.assertIn("name a goatnet", err)


if __name__ == "__main__":
    unittest.main()
