# SPDX-License-Identifier: Apache-2.0
"""Fabric input (--input fabric) end to end on loopback: a zenoh publisher puts raw ASTERIX on a
release/ key, the fabric sender forwards it to a relay that forwards to a test receiver.
Skipped when the eclipse-zenoh package is not installed."""
import json
import os
import random
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import simple_bn_sender as s  # noqa: E402

try:
    import zenoh  # noqa: F401
except ImportError:
    zenoh = None

SENDER = os.path.join(os.path.dirname(__file__), "..", "simple_bn_sender.py")


def free_port(kind=socket.SOCK_DGRAM):
    p = socket.socket(socket.AF_INET, kind)
    p.bind(("127.0.0.1", 0))
    port = p.getsockname()[1]
    p.close()
    return port


def frame(sac=7, sic=42):
    tg = s.Target("uas", 1, random.Random(1), 10)
    return s.block(48, [s.cat048(sac, sic, time.time(), tg, with_track=True)])


class Guard(unittest.TestCase):
    def test_only_released_keys(self):
        with self.assertRaises(ValueError):
            s.FabricFeed("a", ("127.0.0.1", 9), {"a": {"key": "0011/stream/asterix/048/raw/v1", "src_port": 40001}})

    def test_cli_needs_a_session_source(self):
        r = subprocess.run([sys.executable, SENDER, "--label", "t", "--input", "fabric", "--relay-dest", "127.0.0.1",
                            "--feeds", "a", "--status", "/tmp/simple-bn-fabric-test.json"],
                           capture_output=True, text=True, timeout=20)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("refusing to start", r.stderr)


class Payload(unittest.TestCase):
    def test_checks_then_sends(self):
        rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        rx.bind(("127.0.0.1", 0))
        rx.settimeout(2)
        f = s.FabricFeed("a", rx.getsockname(), {"a": {"key": "release/test/a", "sic": 42, "src_port": 40001}})
        f.on_payload(frame(sic=99))
        f.on_payload(frame())
        got, _ = rx.recvfrom(65535)
        self.assertEqual(got[0], 48)
        st = f.status(time.time())
        self.assertEqual((st["totals"]["received"], st["totals"]["rejected_format"], st["totals"]["datagrams"]), (2, 1, 1))
        rx.close()


@unittest.skipIf(zenoh is None, "eclipse-zenoh not installed")
class EndToEnd(unittest.TestCase):
    def test_release_key_to_relay_to_receiver(self):
        d = tempfile.mkdtemp()
        zport = free_port(socket.SOCK_STREAM)
        relay_port = free_port()
        # the publisher listens; the fabric sender connects to it (peer mode, no router)
        sub_conf = os.path.join(d, "z.json5")
        with open(sub_conf, "w") as fh:
            json.dump({"mode": "peer", "connect": {"endpoints": [f"tcp/127.0.0.1:{zport}"]},
                       "scouting": {"multicast": {"enabled": False}}}, fh)
        pub_conf = zenoh.Config()
        pub_conf.insert_json5("mode", '"peer"')
        pub_conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{zport}"]))
        pub_conf.insert_json5("scouting/multicast/enabled", "false")
        pub = zenoh.open(pub_conf)
        prof = os.path.join(d, "p.json")
        with open(prof, "w") as fh:
            json.dump({"a": {"title": "Radar A", "key": "release/test/radar-a/asterix/048/raw/v1",
                             "sac": 7, "sic": 42, "src_port": relay_port}}, fh)
        # the relay, forwarding to a test receiver
        rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        rx.bind(("127.0.0.1", 0))
        rx.settimeout(0.5)
        relay = s.RelayFeed("a", rx.getsockname(), {"a": {"sic": 42, "src_port": relay_port}}, "127.0.0.1", ["127.0.0.1"])
        threading.Thread(target=relay.run, daemon=True).start()
        st = os.path.join(d, "status.json")
        p = subprocess.Popen([sys.executable, SENDER, "--label", "relay", "--input", "fabric", "--relay-dest", "127.0.0.1",
                              "--zenoh-config", sub_conf, "--feeds", "a", "--profiles", prof, "--status", st,
                              "--duration", "8"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            want = frame()
            got = None
            deadline = time.time() + 7
            while got is None and time.time() < deadline:
                pub.put("release/test/radar-a/asterix/048/raw/v1", want)
                try:
                    got, addr = rx.recvfrom(65535)
                except socket.timeout:
                    pass
            self.assertEqual(got, want)
            self.assertEqual(addr[1], relay_port)          # left the relay from the feed's fixed port
            pub.put("0011/stream/asterix/048/raw/v1", want)  # not released: never subscribed, never sent
        finally:
            p.wait(timeout=30)
            pub.close()
        self.assertEqual(p.returncode, 0, p.stderr.read())
        doc = json.load(open(st))
        f = doc["feeds"][0]
        self.assertTrue(doc["receiver"]["via_relay"])
        self.assertGreaterEqual(f["totals"]["datagrams"], 1)
        self.assertEqual(f["fabric"]["key"], "release/test/radar-a/asterix/048/raw/v1")
        rx.close()


if __name__ == "__main__":
    unittest.main()
