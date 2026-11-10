#!/usr/bin/env python3
"""Tests for lifecycle-sweep.py's ledger idempotence — the revoke lane keys
"already done" by peer ID, not by peer name (GH #1305).

Run: python3 ops/scripts/test_lifecycle_sweep_ledger.py
"""
from __future__ import annotations

import contextlib
import copy
import importlib.util
import io
import os
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("lifecycle_sweep", HERE / "lifecycle-sweep.py")
sweep = importlib.util.module_from_spec(spec)
sys.modules["lifecycle_sweep"] = sweep
spec.loader.exec_module(sweep)


def ago(days: int) -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() - 86400 * days))


def dormant(name: str, pid: str) -> dict:
    """An offline peer with no groups: classified as gear, so on the 120-day
    revoke clock; 200 days dark puts it in the revoke lane."""
    return {"name": name, "id": pid, "connected": False, "last_seen": ago(200), "groups": []}


class FakeNetBird:
    """The calls the revoke lane makes, and nothing else."""

    def __init__(self, peers: list[dict]):
        self.peers = peers
        self.deleted: list[str] = []

    def api(self, path, method="GET", body=None, token=""):
        if method == "GET" and path == "/api/peers":
            return copy.deepcopy(self.peers)
        if method == "GET" and path == "/api/setup-keys":
            return []
        if method == "GET" and path == "/api/groups":
            # The sweep refuses to run on an empty group list.
            return [{"id": "g-all", "name": "All", "peers": []}]
        if path.startswith("/api/peers/"):
            pid = path.rsplit("/", 1)[1]
            if method == "GET":
                return copy.deepcopy(next(p for p in self.peers if p["id"] == pid))
            if method == "DELETE":
                self.deleted.append(pid)
                return {}
        raise AssertionError("unexpected API call %s %s" % (method, path))


class RevokeLaneDoneSetTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.ledger = Path(self.tmp.name) / "lifecycle-sweep.log"
        self._ledger_was = sweep.LEDGER
        sweep.LEDGER = str(self.ledger)
        self._holds_was = sweep.HOLD_RECORDS
        sweep.HOLD_RECORDS = str(Path(self.tmp.name) / "hold_records.jsonl")
        self._env = patch.dict(os.environ, {"NB_TOKEN": "test-token"})
        self._env.start()
        # The sweep has no built-in goatnet; the unit's environment names it.
        self._site_was = sweep.SITE
        sweep.SITE = "site-a"

    def tearDown(self):
        self._env.stop()
        sweep.SITE = self._site_was
        sweep.LEDGER = self._ledger_was
        sweep.HOLD_RECORDS = self._holds_was
        self.tmp.cleanup()

    def seed(self, *rows: str) -> None:
        self.ledger.write_text("".join(r + "\n" for r in rows))

    def run_revoke(self, nb: FakeNetBird, *extra: str) -> str:
        out = io.StringIO()
        argv = ["lifecycle-sweep.py", "--apply", "--lane", "revoke", "--actor", "ops", *extra]
        with patch.object(sweep, "api", side_effect=nb.api), \
                patch.object(sys, "argv", argv), \
                contextlib.redirect_stdout(out):
            rc = sweep.main()
        self.assertEqual(rc, 0, out.getvalue())
        return out.getvalue()

    def ledger_rows(self) -> list[list[str]]:
        return [line.split("\t") for line in self.ledger.read_text().splitlines()]

    def test_a_reused_name_is_a_different_peer(self):
        # `Mac` (peer-A) was revoked in an earlier run. A new peer also called
        # `Mac` (peer-B) has since gone dormant past the threshold.
        self.seed("2026-08-01T00:00:00Z\trevoke\tMac\tpeer-A\tZAGROS-DEVELOPER")
        nb = FakeNetBird([dormant("Mac", "peer-A"), dormant("Mac", "peer-B")])
        out = self.run_revoke(nb)
        self.assertEqual(nb.deleted, ["peer-B"], out)
        rows = self.ledger_rows()
        self.assertEqual([r[3] for r in rows if r[1] == "revoke"], ["peer-A", "peer-B"])
        self.assertIn("skip (already done) Mac (peer-A)", out)

    def test_the_same_peer_is_not_revoked_twice(self):
        self.seed("2026-08-01T00:00:00Z\trevoke\tMac\tpeer-A\t")
        nb = FakeNetBird([dormant("Mac", "peer-A")])
        out = self.run_revoke(nb)
        self.assertEqual(nb.deleted, [], out)
        self.assertEqual(len(self.ledger_rows()), 1)

    def test_a_quarantine_row_does_not_count_as_revoked(self):
        # Same peer, same ID, but the ledger only says it was quarantined.
        self.seed("2026-08-01T00:00:00Z\tquarantine\tMac\tpeer-A\tZAGROS-DEVELOPER")
        nb = FakeNetBird([dormant("Mac", "peer-A")])
        out = self.run_revoke(nb)
        self.assertEqual(nb.deleted, ["peer-A"], out)

    def test_a_row_without_an_id_column_marks_nothing_done(self):
        # No such row has ever been written by this tool; a hand-edited one
        # must not be read as "every peer with this name is done".
        self.seed("2026-08-01T00:00:00Z\trevoke\tMac")
        nb = FakeNetBird([dormant("Mac", "peer-A")])
        out = self.run_revoke(nb)
        self.assertEqual(nb.deleted, ["peer-A"], out)

    def test_a_row_with_an_empty_id_column_marks_nothing_done(self):
        # A hand-edited row with a bare trailing tab: the column is there but
        # holds no ID, so it names no peer.
        self.seed("2026-08-01T00:00:00Z\trevoke\tMac\t\t")
        nb = FakeNetBird([dormant("Mac", "peer-A")])
        out = self.run_revoke(nb)
        self.assertEqual(nb.deleted, ["peer-A"], out)

    def test_a_fresh_host_with_no_ledger_revokes_and_starts_one(self):
        nb = FakeNetBird([dormant("Mac", "peer-A")])
        out = self.run_revoke(nb)
        self.assertEqual(nb.deleted, ["peer-A"], out)
        self.assertEqual([r[3] for r in self.ledger_rows()], ["peer-A"])

    def test_a_skipped_peer_does_not_spend_the_limit(self):
        # peer-A is already revoked and still listed ahead of peer-B. With
        # --limit 1 the one revoke this run may make belongs to peer-B.
        self.seed("2026-08-01T00:00:00Z\trevoke\tMac\tpeer-A\t")
        nb = FakeNetBird([dormant("Mac", "peer-A"), dormant("Mac", "peer-B")])
        out = self.run_revoke(nb, "--limit", "1")
        self.assertEqual(nb.deleted, ["peer-B"], out)


if __name__ == "__main__":
    unittest.main(verbosity=2)
