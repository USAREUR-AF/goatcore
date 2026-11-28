# SPDX-License-Identifier: Apache-2.0
"""Egress relay (--input relay): python3 -m unittest discover ops/simple-bn/tests"""
import os
import random
import socket
import subprocess
import sys
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import simple_bn_sender as s  # noqa: E402

SENDER = os.path.join(os.path.dirname(__file__), "..", "simple_bn_sender.py")


def free_port():
    p = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    p.bind(("127.0.0.1", 0))
    port = p.getsockname()[1]
    p.close()
    return port


def sample(sac=7, sic=42, cat=48):
    """A real-shaped datagram: what the ADR 1251 adapter publishes as raw bytes."""
    t = time.time()
    if cat == 34:
        return s.block(34, [s.cat034(sac, sic, t, 1, rot_s=4.0)])
    tg = s.Target("uas", 1, random.Random(1), 10)
    return s.block(48, [s.cat048(sac, sic, t, tg, with_track=True) for _ in range(3)])


class Check(unittest.TestCase):
    def test_accepts_whole_blocks(self):
        self.assertIsNone(s.asterix_check(sample(), {34, 48}))
        self.assertIsNone(s.asterix_check(sample(cat=34), {34, 48}, sac=7, sic=42))
        self.assertIsNone(s.asterix_check(sample() + sample(cat=34), {34, 48}, sic=42))

    def test_refuses(self):
        good = sample()
        self.assertEqual(s.asterix_check(b"", {48}), "empty")
        self.assertEqual(s.asterix_check(good[:-1], {48}), "length")
        self.assertEqual(s.asterix_check(good + b"\x30", {48}), "truncated")
        self.assertEqual(s.asterix_check(good, {34}), "category")
        self.assertEqual(s.asterix_check(good, {48}, sic=53), "source-id")
        self.assertEqual(s.asterix_check(good, {48}, sac=8), "source-id")
        self.assertIsNotNone(s.asterix_check(b"hello world", {48}))


class Guard(unittest.TestCase):
    def test_relay_needs_encryption_to_forward(self):
        self.assertIsNotNone(s.guard("relay", "none"))
        self.assertIsNone(s.guard("relay", "none", holding=True))
        self.assertIsNone(s.guard("relay", "wireguard"))
        self.assertIsNotNone(s.guard("notional", "none", holding=True))

    def test_cli_refuses_plain_forwarding_and_missing_senders(self):
        for extra in (["--dest", "127.0.0.1:9"],                                   # real data, no encryption
                      ["--dest", "127.0.0.1:9", "--encryption", "receiver-transport",  # forwarding, no sender
                       "--relay-intake", "127.0.0.1"],
                      ["--dest", "none", "--relay-from", "127.0.0.1"],             # no intake address
                      ["--dest", "127.0.0.1:9", "--encryption", "wireguard",       # WireGuard, no interface
                       "--relay-intake", "127.0.0.1", "--relay-from", "127.0.0.1"]):
            r = subprocess.run([sys.executable, SENDER, "--label", "t", "--input", "relay", "--feeds", "radar-a",
                                "--status", "/tmp/simple-bn-relay-test.json"] + extra,
                               capture_output=True, text=True, timeout=20)
            self.assertNotEqual(r.returncode, 0, extra)
            self.assertIn("refusing to start", r.stderr, extra)


class Relay(unittest.TestCase):
    def setUp(self):
        self.port = free_port()
        self.prof = {"radar-a": {"title": "Radar A", "sac": 7, "sic": 42, "src_port": self.port}}
        self.rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.rx.bind(("127.0.0.1", 0))
        self.rx.settimeout(2)
        self.tx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)

    def tearDown(self):
        self.rx.close()
        self.tx.close()

    def _relay(self, dest, allowed=("127.0.0.1",)):
        f = s.RelayFeed("radar-a", dest, self.prof, "127.0.0.1", allowed)
        threading.Thread(target=f.run, daemon=True).start()
        return f

    def test_forwards_unchanged_from_the_feed_port(self):
        f = self._relay(self.rx.getsockname())
        for d in (sample(), sample(cat=34)):
            self.tx.sendto(d, ("127.0.0.1", self.port))
            got, addr = self.rx.recvfrom(65535)
            self.assertEqual(got, d)
            self.assertEqual(addr[1], self.port)
        st = f.status(time.time())
        self.assertEqual(st["totals"]["datagrams"], 2)
        self.assertEqual(st["totals"]["received"], 2)
        self.assertTrue(st["relay"]["forwarding"])
        self.assertEqual(st["state"], "sending")

    def test_drops_what_is_not_declared(self):
        f = self._relay(self.rx.getsockname())
        self.tx.sendto(b"not asterix", ("127.0.0.1", self.port))
        self.tx.sendto(sample(sic=99), ("127.0.0.1", self.port))
        with self.assertRaises(socket.timeout):
            self.rx.recvfrom(65535)
        st = f.status(time.time())
        self.assertEqual(st["totals"]["rejected_format"], 2)
        self.assertEqual(st["totals"]["datagrams"], 0)

    def test_only_declared_senders(self):
        f = self._relay(self.rx.getsockname(), allowed=("192.0.2.1",))
        self.tx.sendto(sample(), ("127.0.0.1", self.port))
        with self.assertRaises(socket.timeout):
            self.rx.recvfrom(65535)
        self.assertEqual(f.status(time.time())["totals"]["rejected_source"], 1)

    def test_holding_sends_nothing(self):
        f = self._relay(None)
        self.tx.sendto(sample(), ("127.0.0.1", self.port))
        deadline = time.time() + 2
        while f.status(time.time())["totals"]["held"] < 1 and time.time() < deadline:
            time.sleep(0.05)
        st = f.status(time.time())
        self.assertEqual(st["totals"]["held"], 1)
        self.assertEqual(st["state"], "holding")
        self.assertFalse(st["relay"]["forwarding"])
        self.assertIsNone(f.sock)

    def test_identity_shows_what_arrived(self):
        f = self._relay(None)
        self.tx.sendto(sample(sac=7, sic=42), ("127.0.0.1", self.port))
        self.tx.sendto(sample(sac=0, sic=155), ("127.0.0.1", self.port))
        deadline = time.time() + 2
        while len(f.status(time.time())["identity"]["observed"]) < 2 and time.time() < deadline:
            time.sleep(0.05)
        idn = f.status(time.time())["identity"]
        self.assertEqual(idn["data_class"], "real")
        self.assertEqual(idn["declared"], {"sac": 7, "sic": 42})
        self.assertEqual({(o["sac"], o["sic"]) for o in idn["observed"]}, {(7, 42), (0, 155)})
        self.assertEqual(idn["check"], "mismatch")
        self.assertEqual(f.status(time.time())["totals"]["held"], 1)      # the wrong one never passes

    def test_modifications_unchanged_relay(self):
        f = self._relay(self.rx.getsockname())
        self.tx.sendto(sample(), ("127.0.0.1", self.port))
        self.rx.recvfrom(65535)
        m = f.status(time.time())["modifications"]
        self.assertEqual(m["source"], "relayed")
        self.assertEqual([(o["sac"], o["sic"]) for o in m["input"]], [(7, 42)])
        self.assertEqual([(o["category"], o["sac"], o["sic"]) for o in m["output"]], [(48, 7, 42)])
        self.assertEqual(m["changes"], [])
        self.assertTrue(m["unchanged"])

    def test_modifications_names_an_identity_rewrite(self):
        f = s.RelayFeed("radar-a", None, self.prof, "127.0.0.1", ("127.0.0.1",))
        f.observed = {(9, 17): {"count": 5, "last": 1.0, "cats": {62}}}
        f.sent_ids = {(48, 60, 7): {"count": 4, "last": 2.0}, (34, 60, 7): {"count": 2, "last": 2.0}}
        m = f.modifications()
        ident = [c for c in m["changes"] if c["kind"] == "identity"]
        self.assertEqual(ident, [{"kind": "identity", "from": {"sac": 9, "sic": 17}, "to": {"sac": 60, "sic": 7},
                                  "why": "the identity agreed with the receiver is stamped on every record"}])
        self.assertFalse(m["unchanged"])

    def test_identity_synthetic_must_be_sac_0(self):
        self.assertEqual(s.identity_doc("synthetic", 0, 1, {})["check"], "unseen")
        self.assertEqual(s.identity_doc("synthetic", 0, 1, {(0, 1): {"count": 3, "last": 1.0, "cats": {48}}})["check"], "ok")
        self.assertEqual(s.identity_doc("synthetic", 38, 1, {})["check"], "mismatch")

    def test_waiting_until_input(self):
        f = s.RelayFeed("radar-a", None, self.prof, "127.0.0.1", ["127.0.0.1"])
        self.assertEqual(f.status(time.time())["state"], "waiting-for-input")


class CliHolding(unittest.TestCase):
    def test_runs_and_writes_status(self):
        import json
        import tempfile
        port = free_port()
        d = tempfile.mkdtemp()
        prof = os.path.join(d, "p.json")
        with open(prof, "w") as fh:
            json.dump({"radar-a": {"title": "Radar A", "sic": 42, "src_port": port}}, fh)
        st = os.path.join(d, "status.json")
        p = subprocess.Popen([sys.executable, SENDER, "--label", "t", "--input", "relay", "--dest", "none",
                              "--relay-intake", "127.0.0.1", "--relay-from", "127.0.0.1", "--feeds", "radar-a",
                              "--profiles", prof, "--status", st, "--duration", "6"],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        time.sleep(1)
        tx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        tx.sendto(sample(), ("127.0.0.1", port))
        tx.close()
        p.wait(timeout=20)
        self.assertEqual(p.returncode, 0, p.stderr.read())
        doc = json.load(open(st))
        self.assertEqual(doc["input"], "relay")
        self.assertFalse(doc["receiver"]["declared"])
        self.assertEqual(doc["feeds"][0]["state"], "holding")
        self.assertEqual(doc["feeds"][0]["totals"]["held"], 1)


if __name__ == "__main__":
    unittest.main()
