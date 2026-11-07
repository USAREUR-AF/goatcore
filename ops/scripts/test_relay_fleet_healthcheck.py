#!/usr/bin/env python3
"""Offline regression coverage for the scheduled relay availability guard."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("relay-fleet-healthcheck.py")
spec = importlib.util.spec_from_file_location("relay_health", SCRIPT)
health = importlib.util.module_from_spec(spec)
spec.loader.exec_module(health)


def _has_fleets():
    try:
        return bool(health.FLEETS) and all(
            health.load_fleet(env) for env in health.FLEETS
        )
    except (OSError, ValueError):
        return False


HAS_FLEETS = _has_fleets()
# The first relay inventory this tree carries (a deployment's own), for the
# tests that read the real inventories.
LIVE_ENV = next(iter(health.FLEETS), None)


class FleetTests(unittest.TestCase):
    def inventory(self, text):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "relays.conf").write_text(text)
            with patch.object(
                health, "enroll_default", return_value="netbird.zagros.example"
            ):
                return health.load_fleet("zagros", root)

    def test_fleets_are_the_inventories_present(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "relays.conf").write_text(
                "# header\nfra 192.0.2.1 region instance zagros\n"
            )
            (root / "relays-zagros-dev.conf").write_text("")
            (root / "other.conf").write_text("")
            self.assertEqual(
                health.discover_fleets(root),
                {"zagros": "relays.conf", "zagros-dev": "relays-zagros-dev.conf"},
            )
            with self.assertRaises(ValueError):
                health.load_fleet("zagros-prod", root)

    def test_undeclared_default_needs_enroll_host(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "relays.conf").write_text("fra 192.0.2.1 region instance zagros\n")
            with (
                patch.object(health, "enroll_default", return_value=None),
                self.assertRaises(ValueError),
            ):
                health.load_fleet("zagros", root)

    def test_inventory_default_and_override(self):
        relays = self.inventory(
            "# comment\nfra 192.0.2.1 region instance zagros cap_mbit=1\ndub 192.0.2.2 region instance zagros enroll_host=pool.example.test\n"
        )
        self.assertEqual(
            [r["host"] for r in relays],
            ["netbird.zagros.example", "pool.example.test"],
        )

    def test_bad_inventories(self):
        for text in [
            "",
            "# comment",
            "fra 192.0.2.1",
            "fra bad region instance zagros",
            "fra 192.0.2.1 region instance other",
            "fra 192.0.2.1 region instance zagros malformed",
            "fra 192.0.2.1 region instance zagros enroll_host=bad..test",
            "fra 192.0.2.1 region instance zagros enroll_host=a enroll_host=b",
            "fra 192.0.2.1 region instance zagros\nfra 192.0.2.2 region instance zagros",
        ]:
            with self.subTest(text=text), self.assertRaises(ValueError):
                self.inventory(text)

    @unittest.skipUnless(HAS_FLEETS, "no relay inventories in this tree")
    def test_current_inventories_have_distinct_environment_identity(self):
        fleets = {env: health.load_fleet(env) for env in health.FLEETS}
        self.assertTrue(all(fleets.values()))
        with (
            patch.object(health, "resolve_all", return_value=["192.0.2.99"]),
            patch.object(health, "probe_tcp", return_value="OPEN"),
        ):
            result = health.check_fleets(fleets, 1)
        self.assertTrue(result["healthy"])
        identities = [
            (r["environment"], r["relay"])
            for r in result["checks"]
            if r["tier"] == "relay"
        ]
        self.assertEqual(len(identities), len(set(identities)))
        self.assertEqual(len(identities), sum(map(len, fleets.values())))

    def fleet(self):
        return {
            "zagros": [{"name": "fra", "ip": "192.0.2.1", "host": "pool.example.test"}]
        }

    def test_extra_pool_member_is_probed_and_can_fail(self):
        with (
            patch.object(
                health, "resolve_all", return_value=["192.0.2.1", "192.0.2.99"]
            ),
            patch.object(
                health, "probe_tcp", side_effect=["OPEN", "OPEN", "REFUSED"]
            ) as probe,
        ):
            result = health.check_fleets(self.fleet(), 1)
        self.assertFalse(result["healthy"])
        self.assertEqual(probe.call_count, 3)
        self.assertEqual(
            result["checks"][-1]["target"], "pool.example.test->192.0.2.99"
        )

    def test_dns_failure_and_empty_fleet_never_green(self):
        with (
            patch.object(health, "resolve_all", return_value=[]),
            patch.object(health, "probe_tcp", return_value="OPEN"),
        ):
            result = health.check_fleets(self.fleet(), 1)
        self.assertFalse(result["healthy"])
        self.assertEqual(result["checks"][-1]["result"], "NODNS")
        self.assertFalse(health.check_fleets({}, 1)["healthy"])

    def test_resolver_keeps_every_unique_address(self):
        rows = [
            (None, None, None, None, (ip, 0))
            for ip in ["192.0.2.2", "192.0.2.1", "192.0.2.2"]
        ]
        with patch.object(socket, "getaddrinfo", return_value=rows):
            self.assertEqual(health.resolve_all("pool"), ["192.0.2.1", "192.0.2.2"])
        with patch.object(socket, "getaddrinfo", side_effect=socket.gaierror()):
            self.assertEqual(health.resolve_all("pool"), [])

    def test_tcp_real_listener_and_closed_port(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
            listener.listen()
            with patch.object(health, "PORT", port):
                self.assertEqual(health.probe_tcp("127.0.0.1", 1), "OPEN")
        with patch.object(health, "PORT", port):
            self.assertEqual(health.probe_tcp("127.0.0.1", 1), "REFUSED")

    def test_timeout_reports_filtered(self):
        with patch.object(socket, "create_connection", side_effect=socket.timeout()):
            self.assertEqual(health.probe_tcp("192.0.2.1", 1), "FILTERED")

    def run_main(self, args, status="OPEN"):
        out, err = io.StringIO(), io.StringIO()
        with (
            contextlib.redirect_stdout(out),
            contextlib.redirect_stderr(err),
            patch.object(health, "resolve_all", return_value=["192.0.2.99"]),
            patch.object(health, "probe_tcp", return_value=status),
            patch.dict(os.environ, {"REGIONS": "", "RELAY_HC_TIMEOUT": "1"}),
        ):
            os.environ.pop("REGIONS")
            code = health.main(args)
        return code, out.getvalue(), err.getvalue()

    @unittest.skipUnless(HAS_FLEETS, "no relay inventories in this tree")
    def test_json_exit_status_and_schema(self):
        for status, expected in [("OPEN", 0), ("REFUSED", 1), ("FILTERED", 1)]:
            code, output, _ = self.run_main(["--environment", LIVE_ENV, "--json"], status)
            self.assertEqual(code, expected)
            data = json.loads(output)
            self.assertEqual(data["port"], 33073)
            self.assertEqual(data["healthy"], expected == 0)
            self.assertTrue(data["checks"])

    @unittest.skipUnless(HAS_FLEETS, "no relay inventories in this tree")
    def test_subset_does_not_reduce_pool(self):
        relays = health.load_fleet(LIVE_ENV)
        first = relays[0]["name"]
        code, output, _ = self.run_main(
            ["--environment", LIVE_ENV, "--regions", first, "--json"]
        )
        self.assertEqual(code, 0)
        rows = json.loads(output)["checks"]
        self.assertEqual([r["relay"] for r in rows if r["tier"] == "relay"], [first])
        self.assertEqual(
            rows[-1]["target"], f"{max(r['host'] for r in relays)}->192.0.2.99"
        )

    def test_subset_preserves_other_enrollment_hosts(self):
        fleets = self.fleet()
        fleets["zagros"].append(
            {"name": "dub", "ip": "192.0.2.2", "host": "other.example.test"}
        )
        with (
            patch.object(health, "resolve_all", return_value=["192.0.2.99"]) as resolve,
            patch.object(health, "probe_tcp", return_value="OPEN"),
        ):
            result = health.check_fleets(fleets, 1, {"fra"})
        self.assertTrue(result["healthy"])
        self.assertEqual(
            {call.args[0] for call in resolve.call_args_list},
            {"pool.example.test", "other.example.test"},
        )
        self.assertEqual(len([r for r in result["checks"] if r["tier"] == "relay"]), 1)

    def test_invalid_and_empty_subsets(self):
        for args in [
            ["--regions", ""],
            ["--regions", "invalid"],
            ["--regions", "fra"],
            ["--regions", "$(false)"],
        ]:
            code, output, err = self.run_main(args)
            self.assertEqual(code, 2)
            self.assertEqual(output, "")
            self.assertTrue(err)

    def test_invalid_timeout(self):
        for timeout in ["0", "-1", "nan", "inf", "bad"]:
            with (
                patch.dict(os.environ, {"RELAY_HC_TIMEOUT": timeout}),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                self.assertEqual(health.main([]), 2)

    def test_platform_timeout_overflow_is_usage_error(self):
        with (
            patch.dict(os.environ, {"RELAY_HC_TIMEOUT": "1e100"}),
            patch.object(health, "load_fleet", return_value=self.fleet()["zagros"]),
            patch.dict(health.FLEETS, {"zagros": "relays.conf"}),
            contextlib.redirect_stderr(io.StringIO()) as err,
        ):
            self.assertEqual(health.main(["--environment", "zagros"]), 2)
        self.assertIn("relay-fleet-healthcheck:", err.getvalue())

    def test_wrapper_usage_errors(self):
        for args in [["--regions"], ["--environment", "bad"], ["--unknown"]]:
            result = subprocess.run(
                [str(SCRIPT.with_suffix(".sh")), *args], capture_output=True, text=True
            )
            self.assertEqual(result.returncode, 2)
            self.assertTrue(result.stderr)


if __name__ == "__main__":
    unittest.main()
