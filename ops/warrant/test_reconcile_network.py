#!/usr/bin/env python3
"""Tests for `reconcile.py --network` — mesh-change-control.md §5.

Offline: the live side is ops/warrant/testdata/network-live.json, the shape
`{"nameservers": [...], "routes": [...]}` that --live-json reads.

Usage: python3 ops/warrant/test_reconcile_network.py
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import reconcile  # noqa: E402

LIVE = HERE / "testdata" / "network-live.json"
SCRIPT = HERE / "reconcile.py"


def declaration(ns=(), routes=(), scope="zagros") -> str:
    def lst(xs):
        return "[" + ", ".join(xs) + "]"
    return f"version: 1\nscope: {scope}\nnameserver_groups: {lst(ns)}\nroutes: {lst(routes)}\n"


class ClassifyTest(unittest.TestCase):
    def setUp(self):
        self.live = json.loads(LIVE.read_text())

    def test_everything_undeclared_against_empty(self):
        b = reconcile.classify_network({}, self.live["nameservers"], self.live["routes"])
        self.assertEqual(len(b["nameserver_groups"]["undeclared"]), 2)
        self.assertEqual(len(b["routes"]["undeclared"]), 2)
        self.assertTrue(reconcile.network_drift(b))

    def test_declared_and_missing(self):
        b = reconcile.classify_network(
            {"nameserver_groups": ["mesh-dns", "gone"], "routes": ["site-lan"]},
            self.live["nameservers"], self.live["routes"],
        )
        self.assertEqual([r["name"] for r in b["nameserver_groups"]["declared"]], ["mesh-dns"])
        self.assertEqual([r["name"] for r in b["nameserver_groups"]["undeclared"]], ["console-made"])
        self.assertEqual(b["nameserver_groups"]["missing"], ["gone"])
        self.assertEqual([r["network_id"] for r in b["routes"]["declared"]], ["site-lan"])
        self.assertEqual([r["network_id"] for r in b["routes"]["undeclared"]], ["hand-route"])
        self.assertFalse(b["routes"]["undeclared"][0]["enabled"])

    def test_all_declared_is_clean(self):
        b = reconcile.classify_network(
            {"nameserver_groups": ["mesh-dns", "console-made"], "routes": ["site-lan", "hand-route"]},
            self.live["nameservers"], self.live["routes"],
        )
        self.assertFalse(reconcile.network_drift(b))


class CliTest(unittest.TestCase):
    def run_cli(self, decl: str | None, *extra, live=LIVE):
        with tempfile.TemporaryDirectory() as tmp:
            args = [sys.executable, str(SCRIPT), "--network", "--scope", "zagros"]
            if decl is not None:
                f = Path(tmp) / "zagros.yaml"
                f.write_text(decl)
                args += ["--network-file", str(f)]
            if live is not None:
                args += ["--live-json", str(live)]
            return subprocess.run(args + list(extra), capture_output=True, text=True)

    def test_drift_exits_1(self):
        r = self.run_cli(declaration())
        self.assertEqual(r.returncode, 1, r.stderr)
        self.assertIn("undeclared  console-made", r.stdout)

    def test_clean_exits_0(self):
        r = self.run_cli(declaration(["mesh-dns", "console-made"], ["site-lan", "hand-route"]))
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_missing_exits_1(self):
        r = self.run_cli(declaration(["mesh-dns", "console-made", "extra"], ["site-lan", "hand-route"]))
        self.assertEqual(r.returncode, 1)
        self.assertIn("missing     extra", r.stdout)

    def test_json(self):
        r = self.run_cli(declaration(["mesh-dns"]), "--json")
        self.assertEqual(r.returncode, 1)
        doc = json.loads(r.stdout)
        self.assertEqual(doc["scope"], "zagros")
        self.assertEqual(len(doc["buckets"]["routes"]["undeclared"]), 2)

    def test_invalid_declaration_exits_2(self):
        r = self.run_cli("version: 2\nscope: zagros\n")
        self.assertEqual(r.returncode, 2)

    def test_unreadable_live_exits_2(self):
        with tempfile.TemporaryDirectory() as tmp:
            bad = Path(tmp) / "live.json"
            bad.write_text('{"nameservers": []}')
            r = self.run_cli(declaration(), live=bad)
        self.assertEqual(r.returncode, 2)

    def test_no_source_exits_2(self):
        r = subprocess.run([sys.executable, str(SCRIPT), "--network", "--scope", "zagros"],
                           capture_output=True, text=True, env={"PATH": "/usr/bin:/bin"})
        self.assertEqual(r.returncode, 2)

    @unittest.skipUnless((Path(__file__).parent / "network" / "zagros.yaml").exists(), "no declared goatnet in this tree")
    def test_repo_declaration_is_read(self):
        r = subprocess.run([sys.executable, str(SCRIPT), "--network", "--scope", "zagros",
                            "--live-json", str(LIVE)], capture_output=True, text=True)
        self.assertEqual(r.returncode, 1, r.stderr)  # the file is empty; everything live is undeclared


if __name__ == "__main__":
    unittest.main(verbosity=1)
