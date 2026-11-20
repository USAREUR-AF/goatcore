# SPDX-License-Identifier: Apache-2.0
"""Encoding checks that need only the standard library: python3 -m unittest discover ops/simple-bn/tests"""
import os
import random
import struct
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import simple_bn_sender as s  # noqa: E402


class Encoding(unittest.TestCase):
    def test_fspec(self):
        self.assertEqual(s.fspec([1]), b"\x80")
        self.assertEqual(s.fspec([1, 7]), b"\x82")
        self.assertEqual(s.fspec([1, 8]), b"\x81\x80")          # FX set on the first octet
        self.assertEqual(s.fspec([1, 19]), b"\x81\x01\x08")      # I048/110 is FRN 19

    def test_block_length(self):
        b = s.block(48, [b"\x80\x00\x01", b"\x80\x00\x02"])
        self.assertEqual(b[0], 48)
        self.assertEqual(struct.unpack(">H", b[1:3])[0], len(b))

    def test_sim_bit_always_set(self):
        rnd = random.Random(1)
        for kind in ("uas", "rotary", "fixed-wing", "projectile", "clutter"):
            t = s.Target(kind, 7, rnd, 16)
            rec = s.cat048(0, 1, 0.0, t, with_track=kind != "clutter")
            fs_len = 1
            while rec[fs_len - 1] & 1:
                fs_len += 1
            i020 = rec[fs_len + 2 + 3]                            # after I048/010 (2) and I048/140 (3)
            self.assertTrue(i020 & 0x10, kind)

    def test_polar_matches_cartesian(self):
        t = s.Target("uas", 1, random.Random(2), 16)
        rng, az = t.polar()
        import math
        self.assertAlmostEqual(rng * s.NM_M, math.hypot(t.x, t.y), places=3)
        self.assertAlmostEqual(az, math.degrees(math.atan2(t.x, t.y)) % 360, places=6)

    def test_profiles_override(self):
        import json, tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump({"site-a": {"base": "counterfire", "sic": 99, "title": "A"}}, f)
        p = s.load_profiles(f.name)
        self.assertEqual(p["site-a"]["sic"], 99)
        self.assertEqual(p["site-a"]["salvo"], s.PROFILES["counterfire"]["salvo"])
        self.assertEqual(s.load_profiles("none"), s.PROFILES)


if __name__ == "__main__":
    unittest.main()
