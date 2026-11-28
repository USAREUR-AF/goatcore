#!/usr/bin/env python3
"""consistency-console deploys on a checkout whose release ladder has no live rung.

ops/release/ladder.yaml ships with `rungs: []`, and `graduate.py readiness`
refuses it ("ladder has no live rungs"), so every console deploy from such a
checkout failed. The role renders the readiness document only when a live rung
exists, by the same rule graduate.py uses (status == live), and otherwise
removes any stale document. The role's own expression is run here by
ansible-playbook against several ladders."""
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
TASKS = ROOT / "ops/ansible/roles/consistency-console/tasks/main.yml"
CHECK = "consistency-console | does this checkout declare a live release rung"
REMOVE = "consistency-console | no live release rung, so no readiness document"
GATED = (
    "consistency-console | a controller-side seat for the rendered readiness document",
    "consistency-console | render the release-readiness document from this checkout",
    "consistency-console | place the release-readiness document",
    "consistency-console | drop the controller-side seat",
)


def tasks():
    return {t["name"]: t for t in yaml.safe_load(TASKS.read_text()) if isinstance(t, dict) and "name" in t}


class ReadinessNeedsALiveRung(unittest.TestCase):

    def test_every_readiness_task_waits_for_a_live_rung(self):
        ts = tasks()
        self.assertIn(CHECK, ts)
        for name in GATED:
            with self.subTest(name):
                self.assertEqual(ts[name].get("when"), "gcc_ladder_has_live_rung | bool")
        self.assertEqual(ts[REMOVE].get("when"), "not (gcc_ladder_has_live_rung | bool)")

    def test_the_check_runs_before_the_readiness_tasks(self):
        names = [t.get("name") for t in yaml.safe_load(TASKS.read_text()) if isinstance(t, dict)]
        self.assertLess(names.index(CHECK), min(names.index(n) for n in GATED))

    def live(self, ladder):
        if not shutil.which("ansible-playbook"):
            self.skipTest("ansible-playbook is required")
        d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, d)
        (d / "release").mkdir()
        (d / "release" / "ladder.yaml").write_text(yaml.safe_dump(ladder))
        (d / "ansible").mkdir()
        check = dict(tasks()[CHECK])
        check.pop("run_once", None)
        play = [{"hosts": "localhost", "gather_facts": False, "tasks": [
            check,
            {"ansible.builtin.debug": {"msg": "LIVE={{ gcc_ladder_has_live_rung | bool }}"}},
        ]}]
        (d / "ansible" / "play.yml").write_text(yaml.safe_dump(play))
        r = subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local", str(d / "ansible" / "play.yml")],
                           capture_output=True, text=True, cwd=d, timeout=120)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        return "LIVE=True" in r.stdout

    def test_the_shipped_ladder_has_no_live_rung(self):
        self.assertFalse(self.live(yaml.safe_load((ROOT / "ops/release/ladder.yaml").read_text())))

    def test_only_retired_rungs_is_no_live_rung(self):
        self.assertFalse(self.live({"rungs": [{"name": "r0", "status": "retired"}, {"name": "r1"}]}))

    def test_a_live_rung_is_seen(self):
        self.assertTrue(self.live({"rungs": [{"name": "r0", "status": "retired"}, {"name": "r1", "status": "live"}]}))


if __name__ == "__main__":
    unittest.main()
