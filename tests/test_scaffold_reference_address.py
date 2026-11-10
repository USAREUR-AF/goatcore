#!/usr/bin/env python3
"""The standup scaffold never carries a reference goatnet's public address across.

zagros-dev's enrolment-name record carried zagros's five relay IPs from its scaffold
until #2119: a bare IPv4 names no environment, so the name check let it through.
"""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("preflight", ROOT / "ops/scripts/standup-config-preflight.py")
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)


class ReferenceAddress(unittest.TestCase):
    def test_public_addresses_are_refused(self):
        for val in ('"9.9.9.9"', "relay: 1.1.1.1:33073", "rels://8.8.8.8"):
            self.assertTrue(preflight._names_reference_address(val), val)

    def test_mesh_private_and_templated_values_are_kept(self):
        for val in ('"100.127.0.148"', "10.0.11.50", "192.168.1.172", "127.0.0.1:9193",
                    '"{{ mesh_ip_a }}"', "netbird.zagros-dev.example", "1.2.3", ""):
            self.assertFalse(preflight._names_reference_address(val), val)


if __name__ == "__main__":
    unittest.main()
