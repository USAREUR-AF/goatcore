#!/usr/bin/env python3
"""Before a node enrols it has no mesh address, so join-node's phases 3 and 7
run the config preflight with --before-enrolment: the mesh-address placeholder
is a warning there, and every other TBD_ sentinel still fails. Phase 12 runs it
without the flag, so the placeholder fails once the node has enrolled."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("preflight", ROOT / "ops/scripts/standup-config-preflight.py")
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)

RAW = "mesh_ip_a: TBD_GAMMA_MESH_IP_A\n"


class BeforeEnrolment(unittest.TestCase):
    def setUp(self):
        preflight.FAILS.clear()
        preflight.WARNS.clear()

    def tearDown(self):
        preflight.FAILS.clear()
        preflight.WARNS.clear()

    def test_the_mesh_placeholder_fails_after_enrolment(self):
        preflight.check_tbd_sentinels(RAW)
        self.assertEqual(preflight.FAILS, ["TBD sentinel still present: TBD_GAMMA_MESH_IP_A"])

    def test_the_mesh_placeholder_is_a_warning_before_enrolment(self):
        preflight.check_tbd_sentinels(RAW, before_enrolment=True)
        self.assertEqual(preflight.FAILS, [])
        self.assertEqual(len(preflight.WARNS), 1)
        self.assertIn("TBD_GAMMA_MESH_IP_A", preflight.WARNS[0])

    def test_every_other_sentinel_fails_before_enrolment_too(self):
        preflight.check_tbd_sentinels(RAW + "pgbackrest_cipher_ref: TBD_BKP_KEY\nsan: TBD_ZAGROS_MESH_IP_A_HOST\n", before_enrolment=True)
        self.assertEqual(preflight.FAILS, ["TBD sentinel still present: TBD_BKP_KEY"])

    def test_a_name_that_only_mentions_mesh_fails(self):
        preflight.check_tbd_sentinels("x: TBD_MESH_KEY\n", before_enrolment=True)
        self.assertEqual(preflight.FAILS, ["TBD sentinel still present: TBD_MESH_KEY"])


if __name__ == "__main__":
    unittest.main()
