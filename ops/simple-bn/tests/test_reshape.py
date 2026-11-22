# SPDX-License-Identifier: Apache-2.0
"""Reshape: real CAT062 re-encoded into the shapes a receiver displays. python3 -m unittest discover ops/simple-bn/tests"""
import math
import os
import socket
import sys
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import simple_bn_sender as s  # noqa: E402

# Two real-shaped CAT062 data blocks (one record each): I062/010, 070, 105, 185, 040, 080, 130.
TRACK = bytes.fromhex("3e001f9b0d1026cb5be3e4007dde7700506aab004cfff00218a90101000080")
FUSED = bytes.fromhex("3e001f9b0d1026cc5be500007e198b00507c73ffff00010155a101010000b2")
SITE = "44.24,28.27,30"          # a made-up radar position near the tracks


def items_of(rec):
    bits, i = s._fspec_at(rec, 0)
    return {k + 1 for k, on in enumerate(bits) if on}


class Parse(unittest.TestCase):
    def test_records(self):
        r = s.cat062_records(TRACK)
        self.assertEqual(len(r), 1)
        self.assertEqual(sorted(r[0]), [1, 4, 5, 7, 12, 13, 18])
        self.assertEqual(r[0][1], bytes([38, 203]))

    def test_refuses_other_categories(self):
        with self.assertRaises(ValueError):
            s.cat062_records(s.block(48, [b"\x80\x01\x02"]))


class Radar(unittest.TestCase):
    def setUp(self):
        os.environ["TEST_RADAR_POSITION"] = SITE
        self.r = s.Reshape({"to": "radar", "position_env": "TEST_RADAR_POSITION"}, 38, 203)

    def test_cat048_from_the_radar(self):
        out = self.r.convert(TRACK, time.time())
        self.assertEqual(out[0], 48)
        rec = out[3:]
        frns = items_of(rec)
        self.assertTrue({1, 2, 3, 4, 11, 12, 13, 14, 19} <= frns)
        o = len(s.fspec(sorted(frns)))
        self.assertEqual(rec[o:o + 2], bytes([38, 203]))                  # declared SAC/SIC stamped
        self.assertEqual(rec[o + 5] & 0x10, 0)                            # I048/020 SIM clear
        rng = int.from_bytes(rec[o + 6:o + 8], "big") / 256 * s.NM_M
        az = int.from_bytes(rec[o + 8:o + 10], "big") * 360 / 65536
        lat, lon = 44.2508, 28.2715                                       # the track's I062/105, rounded
        n = math.radians(lat - 44.24) * 6371000.0
        e = math.radians(lon - 28.27) * 6371000.0 * math.cos(math.radians(44.24))
        self.assertAlmostEqual(rng, math.hypot(e, n), delta=30)
        self.assertAlmostEqual(az, math.degrees(math.atan2(e, n)) % 360, delta=1)

    def test_drops_the_zero_position_status_record(self):
        rec = s.cat062_records(TRACK)[0]
        rec[5] = bytes(8)                                                  # lat 0, lon 0
        zero = s.block(62, [s.record(rec)])
        self.assertIsNone(self.r.convert(zero, time.time()))
        self.assertEqual(self.r.dropped, 1)
        self.assertEqual(self.r.convert(s.block(62, [s.record(rec), s.cat062_records(TRACK)[0] and s.record(s.cat062_records(TRACK)[0])]), time.time())[0], 48)

    def test_height_from_flight_level_when_no_geometric_altitude(self):
        rec = s.cat062_records(TRACK)[0]
        del rec[18]                                                       # no I062/130
        rec[17] = (40).to_bytes(2, "big", signed=True)                    # I062/136: FL 10 = 1000 ft
        out = self.r.convert(s.block(62, [s.record(rec)]), time.time())
        r48 = out[3:]
        self.assertIn(19, items_of(r48))                                  # I048/110 present
        self.assertEqual(int.from_bytes(r48[-2:], "big") * 25, 1000)

    def test_drops_beyond_max_range(self):
        r = s.Reshape({"to": "radar", "position_env": "TEST_RADAR_POSITION", "max_range_nm": 0.5}, 38, 203)
        self.assertIsNone(r.convert(TRACK, time.time()))                  # the track is about 0.6 NM out

    def test_status_carries_the_position(self):
        st = self.r.status(time.time())
        self.assertEqual(st[0], 34)
        frns = items_of(st[3:])
        self.assertIn(11, frns)                                           # I034/120

    def test_position_comes_from_the_environment_only(self):
        os.environ.pop("TEST_RADAR_POSITION")
        with self.assertRaises(ValueError):
            s.Reshape({"to": "radar", "position_env": "TEST_RADAR_POSITION"}, 38, 203)
        with self.assertRaises(ValueError):
            s.Reshape({"to": "radar", "position_env": "TEST_RADAR_POSITION"}, None, 203)


class System(unittest.TestCase):
    def test_adds_service_and_cartesian(self):
        os.environ["TEST_REFERENCE"] = "44.25,28.27"
        r = s.Reshape({"to": "system", "reference_env": "TEST_REFERENCE"}, 38, 204)
        out = r.convert(FUSED, time.time())
        rec = s.cat062_records(out)[0]
        self.assertEqual(rec[1], bytes([38, 204]))
        self.assertEqual(rec[3], bytes([1]))                              # I062/015
        self.assertIn(6, rec)                                             # I062/100
        self.assertEqual(rec[13], bytes([0xa1, 0x00]))                   # two octets, SIM clear, empty extents dropped
        self.assertEqual(rec[5], s.cat062_records(FUSED)[0][5])          # WGS-84 kept as it came


class Batch(unittest.TestCase):
    def test_one_picture_per_period_split_under_the_mtu(self):
        os.environ["TEST_REFERENCE"] = "44.25,28.27"
        r = s.Reshape({"to": "system", "reference_env": "TEST_REFERENCE", "batch_s": 1, "max_datagram": 200}, 38, 204)
        recs = [x for _ in range(20) for x in r.records(FUSED, time.time())]
        blocks = r.blocks(recs)
        self.assertGreater(len(blocks), 1)
        self.assertTrue(all(len(b) <= 200 for b in blocks))
        self.assertEqual(sum(len(s.cat062_records(b)) for b in blocks), 20)


class RelayReshapes(unittest.TestCase):
    def test_relay_sends_cat048_and_cat034(self):
        os.environ["TEST_RADAR_POSITION"] = SITE
        p = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        p.bind(("127.0.0.1", 0))
        port = p.getsockname()[1]
        p.close()
        rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        rx.bind(("127.0.0.1", 0))
        rx.settimeout(2)
        prof = {"r": {"title": "R", "src_port": port, "expect_sac": 38, "expect_sic": 201, "cats": [62],
                      "reshape": {"to": "radar", "position_env": "TEST_RADAR_POSITION", "status_period_s": 0.5}}}
        f = s.RelayFeed("r", rx.getsockname(), prof, "127.0.0.1", ["127.0.0.1"])
        threading.Thread(target=f.run, daemon=True).start()
        tx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        tx.sendto(TRACK, ("127.0.0.1", port))
        cats = set()
        end = time.time() + 2
        while time.time() < end and cats != {34, 48}:
            try:
                cats.add(rx.recvfrom(65535)[0][0])
            except socket.timeout:
                break
        tx.close()
        rx.close()
        self.assertEqual(cats, {34, 48})
        self.assertEqual(f.status(time.time())["reshape"], {"to": "radar", "sac": 38, "sic": 201, "position_set": True, "not_tracks_dropped": 0})


if __name__ == "__main__":
    unittest.main()
