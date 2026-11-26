#!/usr/bin/env python3
"""mesh CoreDNS and the spiffe-helper sidecar have rows in the pin table.

Without a row, a goatnet can set mesh_coredns_image or
fabric_health_spiffe_helper_image only by hand, and fabric-health's compose
template falls back to the placeholder TODO_PIN_spiffe_helper_image. With the
rows, generate-pins.py writes both, digest-pinned, into every listed goatnet's
pins.yml, and refuses a hand pin of either as shadowing a governed variable."""
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "ops/consistency/python"))
from goat_consistency import resolve  # noqa: E402

ROWS = {
    "mesh-coredns": "mesh_coredns_image",
    "spiffe-helper": "fabric_health_spiffe_helper_image",
}


def components():
    return yaml.safe_load((ROOT / "ops/pins/fleet-pins.yaml").read_text())["components"]


class Rows(unittest.TestCase):
    def test_each_row_names_its_variable_and_pins_a_digest(self):
        comps = components()
        for cname, var in ROWS.items():
            with self.subTest(cname):
                self.assertIn(cname, comps)
                self.assertEqual(comps[cname]["var"], var)
                resolve.check_ref(cname, "fleet", comps[cname]["fleet"], allow_mutable=False)

    def test_both_variables_are_governed(self):
        governed = resolve.governed_vars(components())
        for var in ROWS.values():
            self.assertIn(var, governed)

    def test_mesh_coredns_row_is_the_role_ledger(self):
        meta = (ROOT / "ops/ansible/roles/mesh-coredns/meta/main.yml").read_text()
        self.assertIn("rollback_component: %s" % components()["mesh-coredns"]["ledger_component"], meta)


class Render(unittest.TestCase):
    """generate-pins.py on a scratch copy with one fixture goatnet, zagros."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)
        for d in ("ops/pins", "ops/consistency", "ops/enrollment/schema"):
            if (ROOT / d).exists():
                shutil.copytree(ROOT / d, self.tmp / d, ignore=shutil.ignore_patterns("__pycache__"))
        for f in (ROOT / "ops/ansible/group_vars").glob("all*"):
            dst = self.tmp / "ops/ansible/group_vars" / f.name
            dst.parent.mkdir(parents=True, exist_ok=True)
            (shutil.copytree if f.is_dir() else shutil.copy)(f, dst)
        self.gv = self.tmp / "ops/ansible/group_vars/zagros_hosts"
        self.gv.mkdir(parents=True)
        (self.gv / "vars.yml").write_text("site_id: zagros\n")
        # A component with no fleet value needs a deviation from every goatnet.
        nulls = {c: {"deviations": {"zagros": {"absent": True, "reason": "fixture"}}}
                 for c, spec in components().items() if spec.get("fleet") is None}
        (self.tmp / "ops/pins/lite-pins.yaml").write_text(yaml.safe_dump(
            {"goatnets": {"zagros": {"members": ["zagros_hosts"]}}, "components": nulls}))
        (self.tmp / "ops/pins/unmigrated-pins.txt").write_text("# baseline: 0\n")

    def gen(self, *args):
        return subprocess.run([sys.executable, "-B", str(self.tmp / "ops/pins/generate-pins.py"), *args],
                              capture_output=True, text=True, cwd=self.tmp, timeout=120)

    def test_pins_yml_carries_both_digests(self):
        r = self.gen()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        pins = yaml.safe_load((self.gv / "pins.yml").read_text())
        comps = components()
        for cname, var in ROWS.items():
            self.assertEqual(pins.get(var), comps[cname]["fleet"], var)

    def test_a_hand_pin_of_either_is_refused(self):
        self.assertEqual(self.gen().returncode, 0)
        base = (self.gv / "vars.yml").read_text()
        for var in ROWS.values():
            with self.subTest(var):
                (self.gv / "vars.yml").write_text(base + '%s: "x@sha256:%s"\n' % (var, "0" * 64))
                r = self.gen("--check")
                self.assertNotEqual(r.returncode, 0, r.stdout)
                self.assertIn("defines %s (zagros_hosts), which is GOVERNED" % var, r.stdout)


if __name__ == "__main__":
    unittest.main()
