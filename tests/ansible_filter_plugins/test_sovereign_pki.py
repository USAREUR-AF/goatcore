"""test_sovereign_pki.py: the icam-mtls-proxy role's filters render what the registry
library renders, and refuse what must not deploy. Needs PyYAML.
Run directly: python3 tests/ansible_filter_plugins/test_sovereign_pki.py
"""
import importlib.util
import unittest
from pathlib import Path

FILTERS = Path(__file__).resolve().parents[2] / "ops" / "ansible" / "filter_plugins"
SPEC = importlib.util.spec_from_file_location("sovereign_pki", FILTERS / "sovereign_pki.py")
mod = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mod)
REGISTRY = FILTERS.parents[1] / "identity" / "sovereign-pkis.yaml"


class FilterTest(unittest.TestCase):
    def setUp(self):
        self.reg = mod._sp.load(REGISTRY)

    def test_select_returns_live_profiles_in_order(self):
        chosen = mod.sovereign_pki_select(self.reg, ["us-dod"])
        self.assertEqual([p["id"] for p in chosen], ["us-dod"])

    def test_select_refuses_a_sketch_profile(self):
        with self.assertRaises(ValueError):
            mod.sovereign_pki_select(self.reg, ["us-dod", "nato-npki"])

    def test_select_refuses_an_unknown_id_and_an_empty_list(self):
        with self.assertRaises(KeyError):
            mod.sovereign_pki_select(self.reg, ["no-such-pki"])
        with self.assertRaises(ValueError):
            mod.sovereign_pki_select(self.reg, [])

    def test_maps_are_the_library_rendering(self):
        chosen = mod.sovereign_pki_select(self.reg, ["us-dod"])
        self.assertEqual(mod.sovereign_pki_nginx_maps(chosen), mod._sp.nginx_maps(chosen))
        self.assertIn("js_set $pki_san_upn pki.sanUpn;", mod.sovereign_pki_nginx_js_set(chosen))
        self.assertEqual(mod.sovereign_pki_nginx_profiles_js(chosen), "")

    def test_filter_module_exposes_all_four(self):
        names = set(mod.FilterModule().filters())
        self.assertEqual(names, {"sovereign_pki_select", "sovereign_pki_nginx_maps",
                                 "sovereign_pki_nginx_js_set", "sovereign_pki_nginx_profiles_js"})


if __name__ == "__main__":
    unittest.main()
