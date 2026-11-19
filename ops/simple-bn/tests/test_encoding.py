# SPDX-License-Identifier: Apache-2.0
"""Encoding checks that need only the standard library: python3 -m unittest discover ops/simple-bn/tests"""
import os
import random
import socket
import time
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


class Guard(unittest.TestCase):
    def test_plain_needs_notional(self):
        self.assertIsNone(s.guard("notional", "none"))
        self.assertIsNone(s.guard("notional", "wireguard"))
        self.assertIn("not built", s.guard("fabric", "wireguard"))
        self.assertIsNotNone(s.guard("fabric", "none"))
        self.assertIsNotNone(s.guard("notional", "rot13"))

    def test_cli_refuses(self):
        import subprocess
        r = subprocess.run([sys.executable, os.path.join(os.path.dirname(__file__), "..", "simple_bn_sender.py"),
                            "--dest", "127.0.0.1:9", "--label", "t", "--input", "fabric", "--status", "/tmp/x.json"],
                           capture_output=True, text=True, timeout=20)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("refusing to start", r.stderr)


class Tcp(unittest.TestCase):
    """TCP: whole ASTERIX blocks arrive on one connection per feed, and the feed reconnects after the receiver returns."""

    def _server(self, port=0):
        srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind(("127.0.0.1", port)); srv.listen(8); srv.settimeout(10)
        return srv

    def _read_blocks(self, conn, n):
        buf, cats = b"", []
        conn.settimeout(10)
        while len(cats) < n:
            chunk = conn.recv(65536)
            if not chunk:
                break
            buf += chunk
            while len(buf) >= 3:
                ln = struct.unpack(">H", buf[1:3])[0]
                if len(buf) < ln:
                    break
                cats.append(buf[0]); buf = buf[ln:]
        return cats

    def test_blocks_and_reconnect(self):
        srv = self._server(); port = srv.getsockname()[1]
        prof = s.load_profiles(None)
        f = s.Feed("counterfire", 0, ("127.0.0.1", port), random.Random(3), prof, transport="tcp")
        f.scan()
        conn, _ = srv.accept()
        cats = self._read_blocks(conn, 1)
        self.assertEqual(cats[0], 34)
        self.assertTrue(set(cats) <= {34, 48})
        self.assertEqual(f.status(time.time())["transport"], "tcp")
        conn.close(); srv.close()
        for _ in range(5):            # the receiver is gone: sends fail, the feed backs off and drops
            f.scan(); time.sleep(0.05)
        st = f.status(time.time())
        self.assertIn(st["state"], ("reconnecting", "receiver-refusing"))   # a refused connect is the receiver refusing
        self.assertGreater(st["totals"]["dropped"], 0)
        srv = self._server(port)       # the receiver returns on the same port
        f.next_retry = 0
        f.scan()
        conn, _ = srv.accept()
        self.assertEqual(self._read_blocks(conn, 1)[0], 34)
        self.assertEqual(f.status(time.time())["state"], "sending")
        conn.close(); srv.close(); f.sock.close()


class FixedSourcePort(unittest.TestCase):
    def test_udp_feed_sends_from_its_declared_port(self):
        rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); rx.bind(("127.0.0.1", 0)); rx.settimeout(5)
        probe = socket.socket(); probe.bind(("127.0.0.1", 0)); port = probe.getsockname()[1]; probe.close()
        prof = s.load_profiles(None)
        prof["fixed"] = dict(prof["counterfire"], src_port=port)
        f = s.Feed("fixed", 0, rx.getsockname(), random.Random(1), prof)
        self.assertEqual(f.src_port, port)
        f.scan()
        _, addr = rx.recvfrom(65535)
        self.assertEqual(addr[1], port)
        f.sock.close(); rx.close()


class FeedbackChannel(unittest.TestCase):
    def test_parse(self):
        self.assertEqual(s.parse_feedback(b'{"type":"ack","sic":42,"received":10,"window_s":60,"x":1}'),
                         {"type": "ack", "sic": 42, "received": 10, "window_s": 60})
        self.assertIsNone(s.parse_feedback(b'{"type":"ack","received":10}'))      # an ack names its SIC
        self.assertIsNone(s.parse_feedback(b'{"type":"rm -rf"}'))
        self.assertIsNone(s.parse_feedback(b"not json"))
        self.assertIsNone(s.parse_feedback(b"x" * 5000))
        m = s.parse_feedback(b'{"type":"status","receiving":true,"sics":[42,"a",53],"note":"' + b"n" * 500 + b'"}')
        self.assertEqual(m["sics"], [42, 53]); self.assertEqual(len(m["note"]), 200)

    def test_only_the_receiver_is_heard(self):
        fb = s.Feedback(["127.0.0.1"])
        self.assertIsNotNone(fb.take(b'{"type":"hello","system":"crc"}', ("127.0.0.1", 9), "port:1"))
        self.assertIsNone(fb.take(b'{"type":"hello","system":"spoof"}', ("10.9.9.9", 9), "port:1"))
        st = fb.status()
        self.assertEqual(st["counts"], {"accepted": 1, "rejected": 1})
        self.assertEqual(st["hello"]["msg"]["system"], "crc")

    def test_reply_to_feed_source_port_is_heard(self):
        rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); rx.bind(("127.0.0.1", 0)); rx.settimeout(5)
        f = s.Feed("counterfire", 0, rx.getsockname(), random.Random(1), s.load_profiles(None))
        fb = s.Feedback(["127.0.0.1"])
        import threading
        threading.Thread(target=fb.listen, args=(f.sock, "reply"), daemon=True).start()
        f.scan()
        _, addr = rx.recvfrom(65535)
        rx.sendto(b'{"type":"ack","sic":2,"received":3,"window_s":60}', addr)
        for _ in range(50):
            if fb.status()["acks"]: break
            time.sleep(0.05)
        self.assertEqual(fb.status()["acks"]["2"]["received"], 3)
        rx.close()
