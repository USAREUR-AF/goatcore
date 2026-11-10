#!/usr/bin/env python3
"""A goatlite render declares the goatnet's ACL, because ADR 1185 refuses a
goatnet without one.

Nothing wrote it, so phase 1's gate failed with "'<name>' has no entry in
ops/ansible/netbird_acl_matrix.yml", and phase 12's group bootstrap would have
failed on the taxonomy the same way. Both machines running the tutorial hit
this independently (G-18).
"""
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("goatlite_render", ROOT / "ops/scripts/goatlite-render.py")
render = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = render
spec.loader.exec_module(render)

FACTS = {"name": "fixture", "name_us": "fixture", "upper": "FIXTURE", "zone": "fixture.lan"}


class Acl(unittest.TestCase):
    def test_the_intents_block_satisfies_the_adr_1185_gate(self):
        entry = yaml.safe_load(render.acl_intents_block(FACTS))["fixture"]
        self.assertEqual(entry["cutover"]["default_policy_action"], "delete")
        self.assertTrue(entry["rules"], "an empty rule list means no reach at all after cutover")
        self.assertEqual(entry["site_prefix"], "FIXTURE")

    def test_every_rule_names_only_groups_the_taxonomy_declares(self):
        tax = yaml.safe_load(render.acl_taxonomy_block(FACTS))["fixture"]
        declared = {g["suffix"] for g in tax["groups"]}
        entry = yaml.safe_load(render.acl_intents_block(FACTS))["fixture"]
        for rule in entry["rules"]:
            for group in rule["sources"] + rule["destinations"]:
                self.assertIn(group, declared, f"rule {rule['name']!r} names {group}, "
                              "which the bootstrap would never create")

    def test_the_administrator_keeps_any_port_reach_to_the_box(self):
        # The cutover deletes the Default policy; a rule set that locks the
        # only administrator out of the box is the known mistake the intents
        # file warns about.
        entry = yaml.safe_load(render.acl_intents_block(FACTS))["fixture"]
        self.assertTrue(any(r["protocol"] == "all" and "PLATFORM-ADMIN" in r["sources"]
                            and "INFRA" in r["destinations"] for r in entry["rules"]))


    def test_the_render_writes_assertions_because_a_silent_scope_is_not_a_pass(self):
        # `goat acl test <name>` fails on a scope with no assertions, and the
        # acceptance row goat-acl-declaration-holds reads that verb. The render
        # writes the assertions with the rules so the row proves something.
        rows = yaml.safe_load(render.acl_tests_block(FACTS))["fixture"]
        self.assertEqual(len(rows), 6)
        self.assertTrue(any(r.get("deny") for r in rows),
                        "at least one isolation assertion, or the posture is only existence checks")

    def test_every_assertion_names_only_groups_the_taxonomy_declares(self):
        tax = yaml.safe_load(render.acl_taxonomy_block(FACTS))["fixture"]
        declared = {g["suffix"] for g in tax["groups"]}
        for row in yaml.safe_load(render.acl_tests_block(FACTS))["fixture"]:
            src = row["src"] if isinstance(row["src"], list) else [row["src"]]
            targets = [t.split(":")[0] for t in row.get("accept", []) + row.get("deny", [])]
            for group in src + targets:
                self.assertIn(group, declared, f"assertion {row['name']!r} names {group}, "
                              "which the bootstrap would never create")

    def test_the_service_assertions_check_the_port_set_the_rules_grant(self):
        sets = yaml.safe_load((ROOT / "ops/warrant/port-sets.yaml").read_text())["port_sets"]
        substrate = next(e for e in sets if e["name"] == "substrate")
        rows = {r["proto"]: r for r in yaml.safe_load(render.acl_tests_block(FACTS))["fixture"]
                if r["name"].startswith("every participant")}
        for member in substrate["members"]:
            self.assertEqual(rows[member["protocol"]]["accept"],
                             [f"INFRA:{p}" for p in member["ports"]])


class AclInATree(unittest.TestCase):
    """render_acl and remove_acl against a copy of this checkout's ACL files,
    through the real generator, as `goatlite-render.py` runs them."""

    FILES = ("ops/warrant/port-sets.yaml", "ops/warrant/rule-intents/netbird-acl.yaml",
             "ops/warrant/generate.py", "ops/ansible/netbird_acl_taxonomy.yml",
             "ops/ansible/netbird_acl_matrix.yml")

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        for rel in self.FILES:
            (self.root / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / rel, self.root / rel)
        # `goat acl test` finds a checkout by this manifest.
        (self.root / "crates/goat-cli").mkdir(parents=True)
        (self.root / "crates/goat-cli/Cargo.toml").write_text('[package]\nname = "goat-cli"\n')
        self.saved = render.REPO
        render.REPO = self.root
        # Compare against the generator's own output, not whatever matrix is
        # committed: the render regenerates it either way.
        subprocess.run([sys.executable, str(self.root / "ops/warrant/generate.py")],
                       check=True, capture_output=True)
        self.original = {rel: (self.root / rel).read_text() for rel in self.FILES}

    def tearDown(self):
        render.REPO = self.saved
        self.tmp.cleanup()

    def matrix(self):
        return yaml.safe_load((self.root / "ops/ansible/netbird_acl_matrix.yml").read_text())

    def test_the_generated_matrix_carries_the_assertions_and_remove_restores_the_tree(self):
        render.render_acl(FACTS, render.Report(), check=False)
        doc = self.matrix()
        self.assertEqual(len(doc["netbird_acl_tests"]["fixture"]), 6)
        self.assertIn("fixture", doc["netbird_acl_matrix"], "the rules stay under the matrix")
        # A second render changes nothing.
        before = {rel: (self.root / rel).read_text() for rel in self.FILES}
        render.render_acl(FACTS, render.Report(), check=False)
        self.assertEqual(before, {rel: (self.root / rel).read_text() for rel in self.FILES})
        render.remove_acl(FACTS, render.Report(), check=False)
        for rel in self.FILES:
            self.assertEqual((self.root / rel).read_text(), self.original[rel], rel)

    def test_a_second_goatlite_puts_its_rules_under_the_matrix_and_its_assertions_under_the_tests(self):
        other = dict(FACTS, name="second", name_us="second", upper="SECOND")
        render.render_acl(FACTS, render.Report(), check=False)
        render.render_acl(other, render.Report(), check=False)
        doc = self.matrix()
        for name in ("fixture", "second"):
            self.assertIn(name, doc["netbird_acl_matrix"])
            self.assertEqual(len(doc["netbird_acl_tests"][name]), 6)
        render.remove_acl(FACTS, render.Report(), check=False)
        doc = self.matrix()
        self.assertNotIn("fixture", doc["netbird_acl_tests"])
        self.assertEqual(len(doc["netbird_acl_tests"]["second"]), 6, "the other goatnet's assertions stay")
        render.remove_acl(other, render.Report(), check=False)
        for rel in self.FILES:
            self.assertEqual((self.root / rel).read_text(), self.original[rel], rel)

    @unittest.skipUnless(os.environ.get("GOAT_BIN"), "set GOAT_BIN to a built goat to run the evaluator")
    def test_goat_acl_test_holds_the_rendered_assertions(self):
        render.render_acl(FACTS, render.Report(), check=False)
        r = subprocess.run([os.environ["GOAT_BIN"], "acl", "test", "fixture", "--verbose"],
                           cwd=self.root, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        # The evaluator adds its own flat-mesh check to the six rendered rows.
        self.assertRegex(r.stdout, r"fixture: all \d+ assertions hold")
        for row in yaml.safe_load(render.acl_tests_block(FACTS))["fixture"]:
            self.assertIn(f"] {row['name']}", r.stdout)
        render.remove_acl(FACTS, render.Report(), check=False)
        render.render_acl(dict(FACTS, name="bare", name_us="bare", upper="BARE"), render.Report(), check=False)
        # Without the render's assertions the same verb must refuse, not pass.
        intents = self.root / "ops/warrant/rule-intents/netbird-acl.yaml"
        render.marker_remove(intents, "bare", render.Report(), False, tag="acl-tests")
        render.drop_empty_acl_tests_key(intents, render.Report(), False)
        render.regenerate_acl_matrix(render.Report(), False)
        r = subprocess.run([os.environ["GOAT_BIN"], "acl", "test", "bare"],
                           cwd=self.root, capture_output=True, text=True)
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("declares no ACL assertions", r.stderr)

if __name__ == "__main__":
    unittest.main()
