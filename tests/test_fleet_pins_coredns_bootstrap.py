#!/usr/bin/env python3
"""The mesh CoreDNS and mgmt-host-bootstrap images are governed by the pin table.

Neither had a fleet-pins.yaml row, so a goatnet could pin them only by hand in
its group_vars, which generate-pins.py --check refuses for any `*_image` var
outside pins.yml, and the lite sidecar cannot carry a component definition.
With the rows, every listed goatnet gets both images, digest-pinned, from its
generated pins.yml, and a hand pin is refused as shadowing."""
import os
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
    "mesh-coredns": ("mesh_coredns_image", "coredns/coredns:1.15.0@sha256:"),
    "mgmt-host-bootstrap": ("mgmt_host_bootstrap_image", "ghcr.io/{{ ghcr_org }}/mgmt-host-bootstrap:latest@sha256:"),
}


def table():
    return yaml.safe_load((ROOT / "ops/pins/fleet-pins.yaml").read_text())


class Rows(unittest.TestCase):
    def test_each_row_is_declared_and_digest_pinned(self):
        comps = table()["components"]
        for cname, (var, prefix) in ROWS.items():
            self.assertIn(cname, comps)
            spec = comps[cname]
            self.assertEqual(spec["var"], var)
            self.assertTrue(spec["fleet"].startswith(prefix), spec["fleet"])
            resolve.check_ref(cname, "fleet", spec["fleet"], allow_mutable=False)
            for key in ("owns_state", "rollback", "image_workflow", "ledger_component"):
                self.assertIn(key, spec, (cname, key))

    def test_the_rows_name_what_the_deploy_reads(self):
        comps = table()["components"]
        role = (ROOT / "ops/ansible/roles/mesh-coredns/meta/main.yml").read_text()
        self.assertIn("rollback_component: %s" % comps["mesh-coredns"]["ledger_component"], role)
        self.assertIn("{{ mgmt_host_bootstrap_image }}", (ROOT / "ops/ansible/mgmt-host.yml").read_text())
        wf = comps["mgmt-host-bootstrap"]["image_workflow"]
        self.assertTrue((ROOT / wf).is_file(), wf)


class Render(unittest.TestCase):
    """generate-pins.py on a scratch tree with one lite goatnet: both vars land in
    its pins.yml, and the same var hand-pinned in vars.yml is refused."""

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
        gv = self.tmp / "ops/ansible/group_vars/demo_hosts"
        gv.mkdir(parents=True)
        (gv / "vars.yml").write_text("site_id: demo\n")
        # A component with no fleet value needs a deviation from every goatnet.
        nulls = {c: {"deviations": {"demo": {"absent": True, "reason": "fixture"}}}
                 for c, spec in table()["components"].items() if spec.get("fleet") is None}
        (self.tmp / "ops/pins/lite-pins.yaml").write_text(yaml.safe_dump(
            {"goatnets": {"demo": {"members": ["demo_hosts"]}}, "components": nulls}))
        (self.tmp / "ops/pins/unmigrated-pins.txt").write_text("# baseline: 0\n")

    def gen(self, *args):
        return subprocess.run([sys.executable, "-B", str(self.tmp / "ops/pins/generate-pins.py"), *args],
                              capture_output=True, text=True, cwd=self.tmp, timeout=120)

    def test_pins_yml_carries_both(self):
        r = self.gen()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        pins = yaml.safe_load((self.tmp / "ops/ansible/group_vars/demo_hosts/pins.yml").read_text())
        for cname, (var, prefix) in ROWS.items():
            self.assertTrue(str(pins.get(var, "")).startswith(prefix), (var, pins.get(var)))

    def test_a_hand_pin_is_refused_as_governed(self):
        self.assertEqual(self.gen().returncode, 0)
        gv = self.tmp / "ops/ansible/group_vars/demo_hosts/vars.yml"
        gv.write_text(gv.read_text() + 'mgmt_host_bootstrap_image: "x@sha256:%s"\n' % ("0" * 64))
        r = self.gen("--check")
        self.assertIn("defines mgmt_host_bootstrap_image (demo_hosts), which is GOVERNED", r.stdout)


if __name__ == "__main__":
    unittest.main()
