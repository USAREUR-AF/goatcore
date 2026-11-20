# SPDX-License-Identifier: Apache-2.0
"""Shared scene, radar models, tracker and CAT062: python3 -m unittest discover ops/simple-bn/tests"""
import json
import math
import os
import random
import socket
import struct
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import scene as scn  # noqa: E402
import simple_bn_sender as s  # noqa: E402

SENDER = os.path.join(os.path.dirname(__file__), "..", "simple_bn_sender.py")
PROFILES = {
    "a": {"base": "hemispheric-cuas", "title": "A", "sic": 42, "src_port": 41042, "site": {"east_m": 0, "north_m": 0}},
    "b": {"base": "counterfire", "title": "B", "sic": 53, "src_port": 41053, "site": {"east_m": -4000, "north_m": 0}},
    "c": {"base": "surveillance-3d", "title": "C", "sic": 30, "src_port": 41030, "site": {"east_m": 0, "north_m": 3000}},
    "f": {"kind": "fused", "title": "Fused", "sic": 62, "src_port": 41062},
}


def items(rec):
    """FRN set of one record and the offset where its items start."""
    frns, i = set(), 0
    while True:
        b = rec[i]
        for bit in range(7):
            if b & (0x80 >> bit):
                frns.add(i * 7 + bit + 1)
        i += 1
        if not b & 1:
            return frns, i


class Cat062(unittest.TestCase):
    def test_record_fields(self):
        frame = scn.Frame(10.0, 20.0)
        trk = {"number": 77, "x": 1000.0, "y": -2000.0, "vx": 50.0, "vy": -25.0, "alt": 300.0, "mode3a": 0o1234,
               "mono": False, "kind": "fixed-wing"}
        rec = scn.cat062_track(0, 62, 3600.0, trk, frame, tod=s.tod, record=s.record)
        frns, o = items(rec)
        self.assertEqual(frns, {1, 3, 4, 5, 6, 7, 9, 12, 13, 18})
        self.assertEqual(rec[o:o + 2], bytes([0, 62]))                     # I062/010
        o += 2 + 1 + 3                                                     # 015, 070
        lat, lon = struct.unpack(">ii", rec[o:o + 8])                     # I062/105
        elat, elon = frame.to_wgs84(1000.0, -2000.0)
        self.assertAlmostEqual(lat * 180 / (1 << 25), elat, places=5)
        self.assertAlmostEqual(lon * 180 / (1 << 25), elon, places=5)
        o += 8
        x = int.from_bytes(rec[o:o + 3], "big", signed=True) / 2
        y = int.from_bytes(rec[o + 3:o + 6], "big", signed=True) / 2      # I062/100
        self.assertEqual((x, y), (1000.0, -2000.0))
        o += 6
        self.assertEqual(struct.unpack(">hh", rec[o:o + 4]), (200, -100))  # I062/185, 0.25 m/s
        o += 4
        self.assertEqual(struct.unpack(">H", rec[o:o + 2])[0], 0o1234)    # I062/060
        o += 2
        self.assertEqual(struct.unpack(">H", rec[o:o + 2])[0], 77)        # I062/040
        o += 2
        self.assertEqual(rec[o:o + 2], bytes([0x11, 0x80]))               # I062/080: multisensor, SRC 3D radar, SIM

    def test_i034_120(self):
        b = scn.i034_120(55, 10.0, 20.0)
        self.assertEqual(struct.unpack(">h", b[:2])[0], 55)
        self.assertAlmostEqual(int.from_bytes(b[2:5], "big", signed=True) * 180 / (1 << 23), 10.0, places=4)


def simulate(seed, seconds=240):
    """Run the scene, radars and tracker on simulated time; mean (visible, tracks, matched, false, error m)."""
    rnd = random.Random(seed)
    prof = s.load_profiles(None)
    for n, p in PROFILES.items():
        base = dict(prof[p["base"]]) if "base" in p else {}
        base.update({k: v for k, v in p.items() if k != "base"})
        prof[n] = base
    sc, tr = s.build_scene("10,20", list(PROFILES), prof, rnd)
    models = []
    for n in ("a", "b", "c"):
        p = prof[n]
        det = list(p["mix"]) + (["projectile"] if p.get("salvo") else [])
        models.append((scn.RadarModel(n, p["site"]["east_m"], p["site"]["north_m"], p["max_nm"] * s.NM_M, det, rnd,
                                      clutter=p["clutter"]), p["period"]))
    t, dt, nxt, stats = 1000.0, 0.5, {}, []
    for step in range(int(seconds / dt)):
        t += dt
        for x in sc.targets:
            x.step(dt, sc.radius_m / s.NM_M)
        sc.targets = [x for x in sc.targets if x.alive]
        if step % 120 == 10:
            az = rnd.uniform(0, 360)
            for _ in range(3):
                pr = sc._new("projectile", x=sc.salvo_origin_m * math.sin(math.radians(az)),
                             y=sc.salvo_origin_m * math.cos(math.radians(az)))
                pr.course = (az + 180) % 360
                sc.targets.append(pr)
        for m, per in models:
            if t >= nxt.get(m.name, 0):
                nxt[m.name] = t + per
                tr.ingest(m.observe(sc.snapshot())[2], t)
        if step > 60 and step % 2 == 0:
            snap, trks = sc.snapshot(), tr.confirmed(t)
            vis = [x for x in snap if any(x[1] in m.detects and math.hypot(x[2] - m.east, x[3] - m.north) <= m.max_m
                                          for m, _ in models)]
            matched, errs, false = set(), [], 0
            for k in trks:
                best = min(vis, key=lambda x: math.hypot(x[2] - k["x"], x[3] - k["y"]), default=None)
                d = math.hypot(best[2] - k["x"], best[3] - k["y"]) if best else 1e9
                if d < 500 and best[0] not in matched:
                    matched.add(best[0])
                    errs.append(d)
                else:
                    false += 1
            stats.append((len(vis), len(trks), len(matched), false, sum(errs) / len(errs) if errs else 0.0))
    return [sum(x[i] for x in stats) / len(stats) for i in range(5)]


class TrackerQuality(unittest.TestCase):
    def test_one_track_per_target(self):
        for seed in (1, 2):
            vis, trks, matched, false, err = simulate(seed)
            self.assertGreater(matched / max(vis, 1e-9), 0.85, (seed, vis, matched))
            self.assertLess(false, 0.5, (seed, false))
            self.assertLess(err, 120, (seed, err))


class EndToEnd(unittest.TestCase):
    def test_four_feeds_from_their_ports(self):
        d = tempfile.mkdtemp()
        prof = os.path.join(d, "p.json")
        ports = {}
        spec = json.loads(json.dumps(PROFILES))
        for n in spec:
            p = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            p.bind(("127.0.0.1", 0))
            ports[n] = spec[n]["src_port"] = p.getsockname()[1]
            p.close()
        with open(prof, "w") as fh:
            json.dump(spec, fh)
        rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        rx.bind(("127.0.0.1", 0))
        rx.settimeout(0.5)
        st = os.path.join(d, "status.json")
        proc = subprocess.Popen([sys.executable, SENDER, "--dest", "%s:%d" % rx.getsockname(), "--label", "t",
                                 "--feeds", "a,b,c,f", "--profiles", prof, "--scene-ref", "10,20",
                                 "--status", st, "--duration", "9"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        seen = set()
        end = time.time() + 12
        while time.time() < end and proc.poll() is None:
            try:
                data, addr = rx.recvfrom(65535)
                seen.add((addr[1], data[0]))
            except socket.timeout:
                pass
        proc.wait(timeout=20)
        rx.close()
        self.assertEqual(proc.returncode, 0, proc.stderr.read())
        for n in ("a", "b", "c"):
            self.assertIn((ports[n], 34), seen)
        self.assertIn((ports["a"], 48), seen)
        self.assertIn((ports["f"], 62), seen)
        doc = json.load(open(st))
        self.assertEqual(doc["receiver"]["format"], "ASTERIX CAT034/CAT048/CAT062")


if __name__ == "__main__":
    unittest.main()
