#!/usr/bin/env python3
"""node-exporter probes the address it was told to bind. With node_exporter_listen_addr set
(the mesh address the role recommends), nothing listens on localhost, so a probe that
hard-codes localhost fails a healthy exporter. Every probe in the role must use the bind address.

Run: python3 -m unittest tests/test_node_exporter_probe_addr.py -v
"""
import re
import unittest
from pathlib import Path

TASKS = Path(__file__).resolve().parents[1] / "ops/ansible/roles/node-exporter/tasks/main.yml"


class ProbeAddr(unittest.TestCase):
    def test_no_probe_hardcodes_localhost(self):
        text = TASKS.read_text()
        probes = re.findall(r"https?://[^\n]*?:\{\{ node_exporter_port \}\}", text)
        self.assertGreaterEqual(len(probes), 3, "expected the role's three probes; test miswired")
        bad = [p for p in probes if "node_exporter_listen_addr" not in p]
        self.assertEqual(bad, [], "probes that ignore node_exporter_listen_addr")


if __name__ == "__main__":
    unittest.main()
