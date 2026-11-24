# SPDX-License-Identifier: Apache-2.0
"""Feed initialization contracts without network sockets or worker threads."""
import os
import random
import sys
import time
from types import SimpleNamespace
import unittest.mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import simple_bn_sender as s  # noqa: E402


class RecordingSocket:
    def __init__(self, kind, port):
        self.kind = kind
        self.bound = ("127.0.0.1", port)
        self.remote = None
        self.sent = []
        self.options = []
        self.timeout = None
        self.closed = False

    def setsockopt(self, *args):
        self.options.append(args)

    def bind(self, address):
        self.bound = address

    def connect(self, address):
        self.remote = address

    def getsockname(self):
        return self.bound

    def settimeout(self, seconds):
        self.timeout = seconds

    def send(self, data):
        self.sent.append(data)
        return len(data)

    def sendall(self, data):
        self.sent.append(data)

    def sendto(self, data, address):
        self.remote = address
        self.sent.append(data)
        return len(data)

    def close(self):
        self.closed = True


class Initialization(unittest.TestCase):
    def setUp(self):
        self.sockets = []
        self.dest = ("192.0.2.10", 10009)
        self.intake = "100.127.0.10"
        self.profiles = {"a": {"title": "Notional A", "sic": 42, "period": 1.0, "max_nm": 16,
                               "mix": {"uas": 1}, "clutter": 0, "salvo": None,
                               "site": {"east_m": 0, "north_m": 0}}}

        def make_socket(family, kind):
            self.assertEqual(family, s.socket.AF_INET)
            sock = RecordingSocket(kind, 47000 + len(self.sockets))
            self.sockets.append(sock)
            return sock

        factory = unittest.mock.patch.object(s.socket, "socket", side_effect=make_socket)
        route = unittest.mock.patch.object(s, "local_addr_for", return_value="127.0.0.1")
        factory.start()
        self.addCleanup(factory.stop)
        route.start()
        self.addCleanup(route.stop)

    def frame(self, sic=42):
        return s.block(34, [s.cat034(7, sic, time.time(), 1)])

    def test_generated_feed_sends_service_and_target_blocks(self):
        for transport in ("udp", "tcp"):
            with self.subTest(transport=transport):
                feed = s.Feed("a", 0, self.dest, random.Random(11), self.profiles, transport)
                feed.scan()
                self.assertEqual([data[0] for data in feed.sock.sent], [34, 48])
                for data in feed.sock.sent:
                    self.assertEqual(s.source_ids(data), [(data[0], 0, 42)])
                status = feed.status(time.time())
                self.assertEqual(status["data_class"], "synthetic")
                self.assertEqual(status["totals"]["datagrams"], 2)
                self.assertEqual(status["live_targets_by_kind"], {"uas": 1})
                self.assertEqual(status["transport"], transport)

    def test_scene_retains_the_notional_random_draws(self):
        reference = s.Feed("a", 0, self.dest, random.Random(11), self.profiles)
        scene = SimpleNamespace(frame=s.scn.Frame(10.0, 20.0))
        feed = s.SceneRadarFeed("a", 0, self.dest, random.Random(11), self.profiles, scene, object())
        self.assertEqual(feed.rnd.getstate(), reference.rnd.getstate())
        self.assertEqual(feed.next_tn, reference.next_tn)
        self.assertEqual(feed.targets, [])
        self.assertEqual(len(reference.targets), 1)

    def test_received_profiles_need_no_notional_fields(self):
        feeds = [s.RelayFeed("r", None, {"r": {"src_port": 40001}}, self.intake, [], "tcp"),
                 s.FabricFeed("f", self.dest, {"f": {"key": "release/test/f"}}, "tcp")]
        for feed in feeds:
            with self.subTest(feed=feed.name):
                status = feed.status(time.time())
                self.assertEqual(status["data_class"], "real")
                self.assertEqual(status["state"], "waiting-for-input")
                self.assertFalse(status["connected"])
                for name in ("rnd", "targets", "next_tn", "next_salvo"):
                    self.assertFalse(hasattr(feed, name), name)

    def test_relay_udp_keeps_its_declared_source_port(self):
        for route in (self.intake, "127.0.0.1"):
            with self.subTest(route=route), unittest.mock.patch.object(s, "local_addr_for", return_value=route):
                feed = s.RelayFeed("r", self.dest, {"r": {"src_port": 40001}}, self.intake, [self.intake])
                data = self.frame()
                feed.send(data)
                self.assertEqual(feed.isock.bound, (self.intake, 40001))
                self.assertEqual(feed.sock.bound, (route, 40001))
                self.assertEqual(feed.sock.sent, [data])
                self.assertEqual(feed.sock.remote, self.dest)
                self.assertEqual(feed.status(time.time())["src_port"], 40001)
                if route == self.intake:
                    self.assertIs(feed.sock, feed.isock)
                else:
                    self.assertIsNot(feed.sock, feed.isock)

    def test_fabric_preserves_validation_and_payload(self):
        feed = s.FabricFeed("f", self.dest, {"f": {"key": "release/test/f", "sic": 42}})
        data = self.frame()
        feed.on_payload(self.frame(sic=99))
        feed.on_payload(data)
        self.assertEqual(feed.sock.sent, [data])
        status = feed.status(time.time())
        self.assertEqual(status["data_class"], "real")
        self.assertEqual(status["totals"]["received"], 2)
        self.assertEqual(status["totals"]["rejected_format"], 1)
        self.assertEqual(status["totals"]["datagrams"], 1)
        self.assertEqual(status["fabric"]["key"], "release/test/f")

    def test_received_tcp_connects_lazily_from_the_fixed_port(self):
        feeds = [s.RelayFeed("r", self.dest, {"r": {"src_port": 40001}}, self.intake, [self.intake], "tcp"),
                 s.FabricFeed("f", self.dest, {"f": {"key": "release/test/f", "src_port": 40001}}, "tcp")]
        for feed in feeds:
            with self.subTest(feed=feed.name):
                self.assertIsNone(feed.sock)
                data = self.frame()
                feed.send(data)
                self.assertEqual(feed.sock.kind, s.socket.SOCK_STREAM)
                self.assertEqual(feed.sock.bound, ("127.0.0.1", 40001))
                self.assertEqual(feed.sock.sent, [data])
                self.assertEqual(feed.status(time.time())["ack"]["mode"], "tcp")

    def test_invalid_profiles_refuse_before_opening_sockets(self):
        with self.assertRaisesRegex(ValueError, "needs a src_port"):
            s.RelayFeed("r", self.dest, {"r": {}}, self.intake, [self.intake])
        with self.assertRaisesRegex(ValueError, "not under release/"):
            s.FabricFeed("f", self.dest, {"f": {"key": "0011/private"}})
        self.assertEqual(self.sockets, [])

    def test_received_feed_counters_are_independent(self):
        relay = s.RelayFeed("r", None, {"r": {"src_port": 40001}}, self.intake, [])
        fabric = s.FabricFeed("f", self.dest, {"f": {"key": "release/test/f"}})
        fabric.on_payload(self.frame())
        self.assertEqual(fabric.status(time.time())["totals"]["received"], 1)
        self.assertEqual(relay.status(time.time())["totals"]["received"], 0)
        self.assertIsNot(relay.lock, fabric.lock)


if __name__ == "__main__":
    unittest.main()
