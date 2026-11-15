#!/usr/bin/env python3
"""goatlite-render.py --hand-over (gate row G1, docs/design/goatlite-growth.md §6).

`goatbuilder grow` runs it after it writes the full configuration. It takes the
generated header off every whole file the render wrote, so the renderer treats
each as the learner's from then on; a file already the learner's is left as it is.
"""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("goatlite_render", ROOT / "ops/scripts/goatlite-render.py")
render = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = render
spec.loader.exec_module(render)

FACTS = {"name": "fixture", "group": "fixture_hosts", "zone": "fixture.lan"}


class HandOver(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.repo = Path(self.tmp.name)
        self.saved = render.REPO
        render.REPO = self.repo
        self.addCleanup(setattr, render, "REPO", self.saved)
        gv = self.repo / "ops/ansible/group_vars/fixture_hosts"
        gv.mkdir(parents=True)
        self.generated = gv / "vars.yml"
        self.generated.write_text(render.HEADER + "\nsite_name: fixture\n", encoding="utf-8")
        self.zone = self.repo / "ops/dns/fixture/fixture.lan.hosts"
        self.zone.parent.mkdir(parents=True)
        self.zone.write_text(render.HEADER + "\n192.0.2.50 fixture.lan\n", encoding="utf-8")
        self.owned = gv / "spire.yml"
        self.owned.write_text("# mine now\nspire_x: 1\n", encoding="utf-8")

    def statuses(self, rep):
        return {Path(p).relative_to(self.repo).as_posix() if isinstance(p, Path) else p: s for p, s in rep.rows}

    def test_the_header_comes_off_every_generated_whole_file(self):
        rep = render.hand_over(FACTS, check=False)
        self.assertEqual(self.generated.read_text(encoding="utf-8"), "site_name: fixture\n")
        self.assertEqual(self.zone.read_text(encoding="utf-8"), "192.0.2.50 fixture.lan\n")
        got = self.statuses(rep)
        self.assertTrue(got["ops/ansible/group_vars/fixture_hosts/vars.yml"].startswith("handed over"), got)
        self.assertTrue(got["ops/dns/fixture/fixture.lan.hosts"].startswith("handed over"), got)

    def test_a_file_already_the_learners_is_left_alone(self):
        rep = render.hand_over(FACTS, check=False)
        self.assertEqual(self.owned.read_text(encoding="utf-8"), "# mine now\nspire_x: 1\n")
        self.assertTrue(self.statuses(rep)["ops/ansible/group_vars/fixture_hosts/spire.yml"].startswith("already yours"))

    def test_a_render_after_the_hand_over_keeps_the_file(self):
        render.hand_over(FACTS, check=False)
        rep = render.Report()
        render.whole_file(self.generated, render.HEADER + "\nsite_name: other\n", rep, check=False)
        self.assertEqual(self.generated.read_text(encoding="utf-8"), "site_name: fixture\n")

    def test_check_writes_nothing(self):
        rep = render.hand_over(FACTS, check=True)
        self.assertTrue(self.generated.read_text(encoding="utf-8").startswith(render.HEADER + "\n"))
        self.assertTrue(rep.changed)
        self.assertTrue(self.statuses(rep)["ops/ansible/group_vars/fixture_hosts/vars.yml"].startswith("would hand over"))

    def test_a_second_hand_over_changes_nothing(self):
        render.hand_over(FACTS, check=False)
        rep = render.hand_over(FACTS, check=True)
        self.assertFalse(rep.changed, rep.rows)


if __name__ == "__main__":
    unittest.main()
