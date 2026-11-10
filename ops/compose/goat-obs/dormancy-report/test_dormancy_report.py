#!/usr/bin/env python3
"""Tests for dormancy-report's device-expiry count (ADR 1216).

A NETWORK, CONTROL or EDGE-SERVER peer with a NetBird login or inactivity
expiration turned on drops off the mesh when the timer runs out. The fixture
fixtures/peers.json is shaped like NetBird's GET /api/peers answer and holds
one expirable peer of each class, a clean switch, a NETWORK-JUMP box and an
engineer laptop; the last two must never be counted.

Run: python3 -m unittest discover -s ops/compose/goat-obs/dormancy-report
"""
import importlib.util
import io
import json
import os
import pathlib
import tempfile
import unittest
import unittest.mock

HERE = pathlib.Path(__file__).parent
_spec = importlib.util.spec_from_file_location("dormancy_report", HERE / "dormancy_report.py")
dr = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(dr)

PEERS = json.loads((HERE / "fixtures" / "peers.json").read_text())


class DeviceClassTest(unittest.TestCase):
    def test_suffix_match(self):
        self.assertEqual(dr.device_class({"groups": [{"name": "SITE_ONE-NETWORK"}]}), "NETWORK")
        self.assertEqual(dr.device_class({"groups": [{"name": "EDGE-SERVER"}]}), "EDGE-SERVER")
        self.assertEqual(dr.device_class({"groups": [{"name": "zagros-control"}]}), "CONTROL")

    def test_network_jump_and_people_are_not_device_classes(self):
        self.assertIsNone(dr.device_class({"groups": [{"name": "SITE_ONE-NETWORK-JUMP"}]}))
        self.assertIsNone(dr.device_class({"groups": [{"name": "SITE_ONE-NET-ENGINEER"}]}))
        self.assertIsNone(dr.device_class({"groups": []}))
        self.assertIsNone(dr.device_class({}))


class ExpirableTest(unittest.TestCase):
    def test_fixture_rows(self):
        self.assertEqual(
            sorted(dr.expirable_device_peers(PEERS)),
            [
                ("c2-console-a", "CONTROL", "inactivity"),
                ("c2-console-a", "CONTROL", "login"),
                ("edge-server-a", "EDGE-SERVER", "inactivity"),
                ("switch-a", "NETWORK", "login"),
            ],
        )

    def test_counts_cover_every_class_and_kind(self):
        counts = dr.expiry_counts(dr.expirable_device_peers(PEERS))
        self.assertEqual(len(counts), 6)
        self.assertEqual(counts[("NETWORK", "login")], 1)
        self.assertEqual(counts[("NETWORK", "inactivity")], 0)
        self.assertEqual(counts[("CONTROL", "login")], 1)
        self.assertEqual(counts[("CONTROL", "inactivity")], 1)
        self.assertEqual(counts[("EDGE-SERVER", "login")], 0)
        self.assertEqual(counts[("EDGE-SERVER", "inactivity")], 1)

    def test_missing_field_is_not_expirable(self):
        peer = {"name": "x", "groups": [{"name": "NETWORK"}]}
        self.assertEqual(dr.expirable_device_peers([peer]), [])


class RenderTest(unittest.TestCase):
    def test_series_written_with_zeros(self):
        body = dr.render_prom("zagros", 1, {}, {}, 0, 1, dr.expiry_counts([]))
        self.assertIn(
            'goat_device_peer_expiry_enabled{env="zagros",class="EDGE-SERVER",kind="login"} 0', body)
        self.assertEqual(body.count("goat_device_peer_expiry_enabled{"), 6)

    def test_not_written_when_peer_list_unread(self):
        body = dr.render_prom("zagros", 0, {}, {}, 0, 0)
        self.assertNotIn("goat_device_peer_expiry_enabled{", body)


class MainTest(unittest.TestCase):
    """End to end: main() against the fixture, no network."""

    def test_main_writes_counts_and_names(self):
        out = io.StringIO()
        env = {"NETBIRD_MGMT_URL": "http://mgmt.invalid", "NETBIRD_API_TOKEN": "t",
               "GOAT_ENV": "site-one"}
        with tempfile.TemporaryDirectory() as d, \
                unittest.mock.patch.object(dr, "fetch_peers", return_value=PEERS), \
                unittest.mock.patch.object(dr, "fetch_identities", return_value=[]), \
                unittest.mock.patch.dict(os.environ, env), \
                unittest.mock.patch("sys.argv", ["dormancy_report.py", d]), \
                unittest.mock.patch("sys.stdout", out):
            self.assertEqual(dr.main(), 0)
            body = pathlib.Path(d, "goat_dormancy_report.prom").read_text()
        printed = out.getvalue()
        self.assertIn(
            'goat_device_peer_expiry_enabled{env="site-one",class="NETWORK",kind="login"} 1', body)
        self.assertIn(
            'goat_device_peer_expiry_enabled{env="site-one",class="CONTROL",kind="inactivity"} 1', body)
        self.assertIn("EXPIRABLE-DEVICE switch-a class=NETWORK expiry=login", printed)
        self.assertNotIn("jump-box-a", printed)
        self.assertNotIn("engineer-laptop-a", printed)
        # Names never reach the metrics file.
        self.assertNotIn("switch-a", body)


if __name__ == "__main__":
    unittest.main()
