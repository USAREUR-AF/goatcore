"""A goatlite's router gives the assay probe identity fabric read.

A goatlite runs the router in per-uuid-slot-pool mode, where the universal read
names its non-slot readers: zenoh_universal_read_cns, plus the seeded vendors.
The render declared the assay probe identity (assay_probe_enabled) but did not
list its CN there, so the acceptance row fabric-put-subscribe-roundtrip, run as
the probe, published and heard nothing back: found on a goatlite founder
standup on hardware and again on a VM founder.

The rule is ops/ci/check-assay-probe-identity.py's (universal_read_error); this
test applies it to the group_vars a goatlite renders, on a LAN and an internet
goatlite.

Run: python3 -m pytest tests/test_goatlite_assay_probe_read.py
"""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    return mod


render = load("goatlite_render", ROOT / "ops/scripts/goatlite-render.py")
probe = load("check_assay_probe_identity", ROOT / "ops/ci/check-assay-probe-identity.py")


def vars_text(**lite):
    d = Path(tempfile.mkdtemp())
    pub = d / "key.pub"
    pub.write_text("placeholder: load_config only checks that the file exists\n")
    cfg = d / "goatlite.yaml"
    cfg.write_text(yaml.safe_dump({"schema": "goatbuilder/v1", "preset": "lite",
                                   "lite": {"name": "fixture", "host": "192.0.2.50", "ssh_user": "ubuntu",
                                            "admin": "admin@example.org", "ssh_pubkey": str(pub), **lite}}))
    return render.vars_yml(render.facts(render.load_config(cfg, "fixture")))


class AssayProbeRead(unittest.TestCase):

    def check(self, text):
        gv = yaml.safe_load(text)
        self.assertIs(gv.get("assay_probe_enabled"), True)
        self.assertEqual(gv.get("zenoh_identity_model"), "per-uuid-slot-pool")
        self.assertIn(probe.CN, gv.get("zenoh_universal_read_cns") or [])
        self.assertIsNone(probe.universal_read_error(text))

    def test_a_lan_goatlite_lists_the_probe_as_a_universal_reader(self):
        self.check(vars_text())

    def test_an_internet_goatlite_lists_the_probe_as_a_universal_reader(self):
        self.check(vars_text(internet={"zone": "fixture.example.org",
                                       "relays": [{"name": "a", "region": "us-east-2"}]}))

    def test_the_rule_refuses_a_slot_pool_that_leaves_the_probe_out(self):
        text = "zenoh_identity_model: per-uuid-slot-pool\nzenoh_universal_read_cns:\n  - other-trial-peer\n"
        self.assertIn("does not list", probe.universal_read_error(text) or "")


if __name__ == "__main__":
    unittest.main()
