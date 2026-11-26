"""Every pinned component a goatlite cannot deploy is declared absent for it.

goatlite-render.py writes an `absent: true` deviation for each component in
LITE_ABSENT. A component left out of that list stays pinned for the goatlite, so
`goatbuilder diff`, `status` and the assay row deploy-ledgers-match-main look for
its ledger on the box. When no standup phase runs its deploy play, there never is
one, and a finished standup reads as not converged: a goatlite founder on
hardware read so for peer-health, pinned after the list was written.

The plays a standup can run are the ones goat-standup.sh names (the script
avenue, kept at parity with `goatbuilder standup` by the parity gates), with
every play they import. A component that declares a deploy play no standup
phase reaches, has a fleet row, and is not opt-in, must be in LITE_ABSENT.
This is an over-approximation of a goatlite's phases (it counts plays only an
internet or multi-host site runs), so it misses a component deployed only on
those; it never fails a component a goatlite does deploy.

Run: python3 -m pytest tests/test_goatlite_absent_components.py
"""
import ast
from pathlib import Path
import re
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
ANSIBLE = ROOT / "ops/ansible"
RENDER = ROOT / "ops/scripts/goatlite-render.py"
STANDUP = ROOT / "ops/scripts/goat-standup.sh"
PINS = ROOT / "ops/pins/fleet-pins.yaml"


def lite_absent():
    """LITE_ABSENT as goatlite-render.py declares it, read without running it."""
    for node in ast.parse(RENDER.read_text()).body:
        if isinstance(node, ast.Assign) and any(getattr(t, "id", "") == "LITE_ABSENT" for t in node.targets):
            return set(ast.literal_eval(node.value))
    raise AssertionError("goatlite-render.py declares no LITE_ABSENT")


def standup_plays():
    """Every play goat-standup.sh names, and every play those import."""
    stack = re.findall(r"([A-Za-z0-9_.-]+\.yml)\b", STANDUP.read_text())
    seen = set()
    while stack:
        name = stack.pop()
        if name in seen or not (ANSIBLE / name).is_file():
            continue
        seen.add(name)
        stack += re.findall(r"import_playbook:\s*[\"']?([A-Za-z0-9_.-]+\.yml)", (ANSIBLE / name).read_text())
    return seen


class GoatliteAbsentComponents(unittest.TestCase):

    def test_the_reader_sees_the_plays_it_must(self):
        plays = standup_plays()
        # site-bootstrap.yml imports mgmt-host.yml, which deploys netbird-mgmt.
        for p in ("site-bootstrap.yml", "mgmt-host.yml", "zenoh-deploy.yml", "ory-deploy.yml"):
            self.assertIn(p, plays)
        self.assertIn("fabric-health", lite_absent())

    def test_a_component_no_standup_phase_deploys_is_absent_on_a_goatlite(self):
        plays, absent = standup_plays(), lite_absent()
        missing = []
        for name, spec in yaml.safe_load(PINS.read_text())["components"].items():
            play = spec.get("deploy_playbook")
            if not play or name in absent or spec.get("fleet") is None or spec.get("opt_in"):
                continue
            if play not in plays:
                missing.append(f"{name} ({play})")
        self.assertEqual(missing, [], "pinned, deployed by no standup phase, and not in goatlite-render.py's LITE_ABSENT")

    def test_every_absent_name_is_a_pinned_component(self):
        components = set(yaml.safe_load(PINS.read_text())["components"])
        self.assertEqual(sorted(lite_absent() - components), [])


if __name__ == "__main__":
    unittest.main()
