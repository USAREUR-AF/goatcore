#!/usr/bin/env python3
"""The spiffe-helper sidecar image is governed by the pin table.

fabric-health's compose template falls back to the placeholder
`TODO_PIN_spiffe_helper_image`, and id-admin-console and onboard-runner reuse
`fabric_health_spiffe_helper_image` as "the digest-pinned" helper. No row set it,
so a goatnet could only hand-pin it in group_vars, which generate-pins.py --check
refuses, and without one fabric-health pulled the placeholder and failed. With
the row, every goatnet gets the helper digest-pinned in its generated pins.yml."""
import sys
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "ops/consistency/python"))
from goat_consistency import resolve  # noqa: E402


class SpiffeHelperRow(unittest.TestCase):
    def test_row_is_declared_and_digest_pinned(self):
        comps = yaml.safe_load((ROOT / "ops/pins/fleet-pins.yaml").read_text())["components"]
        self.assertIn("spiffe-helper", comps)
        spec = comps["spiffe-helper"]
        self.assertEqual(spec["var"], "fabric_health_spiffe_helper_image")
        self.assertRegex(spec["fleet"], r"^ghcr\.io/spiffe/spiffe-helper:[0-9.]+@sha256:[0-9a-f]{64}$")

    def test_var_is_governed(self):
        comps = yaml.safe_load((ROOT / "ops/pins/fleet-pins.yaml").read_text())["components"]
        self.assertIn("fabric_health_spiffe_helper_image", resolve.governed_vars(comps))


if __name__ == "__main__":
    unittest.main()
