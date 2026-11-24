# SPDX-License-Identifier: Apache-2.0
"""A simulated scene in the real feeds' shape: same items, same datagram form; only SIM and SAC differ."""
import os
import random
import socket
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import scene as scn  # noqa: E402
import simple_bn_sender as s  # noqa: E402

RADAR = [1, 2, 3, 4, 11, 12, 13, 14, 19]      # what reshape to radar sends (track number, position, velocity, status, height)
PROFILES = dict(s.PROFILES)
for _n, _sic, _port, _e in (("r1", 201, 0, 0), ("r2", 202, 0, 40000)):
    PROFILES[_n] = dict(s.PROFILES["surveillance-3d"], title=_n, sic=_sic, shape="real", area=_n,
                        site={"east_m": _e, "north_m": 0}, mix={"uas": 4, "fixed-wing": 2})
PROFILES["f"] = {"kind": "fused", "title": "f", "sic": 204, "shape": "real"}
PROFILES["b"] = {"kind": "blue", "title": "b", "sic": 205, "site_m": [40000, 0]}


def frns(rec):
    bits, _ = s._fspec_at(rec, 0)
    return [k + 1 for k, on in enumerate(bits) if on]


class Shape(unittest.TestCase):
    def setUp(self):
        self.rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.rx.bind(("127.0.0.1", 0))
        self.other = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.other.bind(("127.0.0.1", 0))
        self.other.settimeout(1)
        self.dest = self.rx.getsockname()
        self.addCleanup(self.rx.close)
        self.addCleanup(self.other.close)
        rnd = random.Random(7)
        self.sc, self.tr = s.build_scene("10,20", ["r1", "r2", "f", "b"], PROFILES, rnd)
        self.out = []

    def feed(self, cls, name, profiles=PROFILES):
        f = cls(name, 0, self.dest, random.Random(3), profiles, self.sc, self.tr)
        self.addCleanup(f.sock.close)
        f.send = self.out.append
        return f

    def test_radar_matches_the_relayed_shape(self):
        f = self.feed(s.SceneRadarFeed, "r1")
        for _ in range(12):
            f.scan()
        plots = [d for d in self.out if d[0] == 48]
        self.assertTrue(plots)
        for d in plots:
            self.assertEqual(frns(d[3:]), RADAR)                        # same items in every plot
            self.assertEqual(int.from_bytes(d[1:3], "big"), len(d))      # one block per datagram
            o = 3 + s._fspec_at(d[3:], 0)[1]
            self.assertEqual(d[o:o + 2], bytes([0, 201]))
            self.assertEqual(d[o + 5], 0x30)                             # I048/020: primary (as real, 0x20) + SIM
        status = [d for d in self.out if d[0] == 34]
        self.assertEqual(len(status), 1)                                 # one per 4 s, not one per scan

    def test_areas_keep_far_radars_apart(self):
        east = [t[2] for t in self.sc.snapshot()]
        self.assertTrue(any(e > 20000 for e in east) and any(e < 20000 for e in east))
        ids = [t[0] for t in self.sc.snapshot()]
        self.assertEqual(len(ids), len(set(ids)))

    def test_system_tracks(self):
        r = self.feed(s.SceneRadarFeed, "r1")
        for _ in range(8):
            r.scan()
        self.out.clear()
        self.feed(s.FusedFeed, "f").scan()
        recs = [x for d in self.out for x in s.cat062_records(d)]
        self.assertTrue(recs)
        for x in recs:
            self.assertEqual(set(x), {1, 3, 4, 5, 6, 7, 12, 13, 18})     # no Mode 3/A
            self.assertEqual(x[13].hex(), "a180")                        # real: a100; SIM is the one difference

    def test_blue(self):
        self.feed(s.BlueFeed, "b").scan()
        (x,) = s.cat062_records(self.out[0])
        self.assertEqual(set(x), {1, 3, 4, 5, 6, 7, 12, 13})             # no height
        self.assertEqual((x[1], x[12], x[13].hex()), (bytes([0, 205]), b"\x00\x01", "8180"))

    def test_dest_port(self):
        port = self.other.getsockname()[1]
        prof = dict(PROFILES, b=dict(PROFILES["b"], dest_port=port))
        f = s.BlueFeed("b", 0, self.dest, random.Random(3), prof, self.sc, self.tr)
        self.addCleanup(f.sock.close)
        f.scan()
        self.assertEqual(self.other.recvfrom(2048)[0][0], 62)
        self.assertEqual(f.status(0)["dest_port"], port)

    def test_unnamed_area_is_unchanged(self):
        prof = {n: {k: v for k, v in p.items() if k != "area"} for n, p in PROFILES.items()}
        sc, _ = s.build_scene("10,20", ["r1", "r2"], prof, random.Random(1))
        self.assertIsInstance(sc, scn.Scene)


if __name__ == "__main__":
    unittest.main()
