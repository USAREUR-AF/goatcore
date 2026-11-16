#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""relay-peer-names run as a program, the way the timer and an operator run it
(review of the reconcile, items 1, 2, 3, 6, 7, 9, 11, 12 and the dates).

A fake `docker` on PATH answers the NetBird store query; a fake portal answers
`POST /api/access/peers`. Nothing here reaches a real host."""
import http.server
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
WRITER = os.path.join(HERE, "relay-peer-names.py")

# key, name, id, aff, ip, os, host, seen, conn
ROWS = [
    ("k1", "machine-anvil-pod-7", "p1", "", "100.127.0.11", "linux", "pod-7", "2026-01-05", "t"),
    ("k2", "edge-gw-unlisted", "p2", "", "100.127.0.12", "linux", "gw", "2026-01-05", "t"),
    ("k3", "old-kiosk", "p3", "", "100.127.0.13", "linux", "kiosk", "2026-01-05", "f"),
]


def owners(ok=True):
    peers = {"machine-anvil-pod-7": {"resolved": True, "handle": "rivera-j",
                                     "machine": {"system_handle": "anvil-pod-7"},
                                     "owner": {"source": "record", "via": "machine-custody"}},
             "edge-gw-unlisted": {"resolved": False, "owner": {"source": "none", "reason": "x"}},
             "old-kiosk": {"resolved": False, "owner": {"source": "none", "reason": "x"}}}
    return {"peers": peers, "notes": [],
            "owner_sources": {"ok": ok, "failed": [] if ok else ["identity_layer_error"]}}


class Portal(http.server.BaseHTTPRequestHandler):
    answer = owners()

    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length") or 0))
        body = json.dumps(type(self).answer).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


class CLI(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp(dir=os.environ.get("TEST_SCRATCH"))
        self.addCleanup(shutil.rmtree, self.d, True)
        bin_ = os.path.join(self.d, "bin")
        os.mkdir(bin_)
        docker = os.path.join(bin_, "docker")
        with open(docker, "w") as f:
            f.write("#!/bin/sh\ncat <<'ROWS'\n" + "\n".join("\t".join(r) for r in ROWS) + "\nROWS\n")
        os.chmod(docker, os.stat(docker).st_mode | stat.S_IEXEC)
        Portal.answer = owners()
        self.srv = http.server.HTTPServer(("127.0.0.1", 0), Portal)
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.addCleanup(self.srv.server_close)
        self.addCleanup(self.srv.shutdown)
        self.www = os.path.join(self.d, "www")
        os.mkdir(self.www)
        self.prom = os.path.join(self.d, "peer-owners.prom")
        self.exc = os.path.join(self.d, "owner-exceptions.json")
        self.env = dict(os.environ, PATH=bin_ + os.pathsep + os.environ["PATH"],
                        RELAY_PEER_NAMES_LOCAL_DIR=self.www, RELAY_PEER_NAMES_INGEST_URLS="",
                        RELAY_PEER_NAMES_PORTAL_URL=f"http://127.0.0.1:{self.srv.server_port}",
                        RELAY_PEER_NAMES_PORTAL_KEY="synthetic-key",
                        RELAY_PEER_NAMES_OWNER_TEXTFILE=self.prom,
                        RELAY_PEER_NAMES_OWNER_EXCEPTIONS=self.exc)

    def run_(self, *args):
        return subprocess.run([sys.executable, WRITER, *args], env=self.env,
                              capture_output=True, text=True, timeout=60)

    def metrics(self):
        out = {}
        with open(self.prom) as f:
            for line in f:
                if line and not line.startswith("#"):
                    k, v = line.rsplit(" ", 1)
                    out[k] = float(v)
        return out

    def write(self, doc):
        with open(self.exc, "w") as f:
            json.dump(doc, f)

    # 2: the documented command works.
    def test_write_exceptions_runs_as_documented(self):
        r = self.run_("--write-exceptions", self.exc, "--granted-by", "ops-lead",
                      "--reason", "present before the rule", "--granted-on", "2026-01-05")
        self.assertEqual(r.returncode, 0, r.stderr)
        with open(self.exc) as f:
            doc = json.load(f)
        self.assertEqual(sorted(e["name"] for e in doc["exceptions"]), ["edge-gw-unlisted", "old-kiosk"])
        self.assertEqual({e["granted_on"] for e in doc["exceptions"]}, {"2026-01-05"})
        self.assertTrue(all("review_by" not in e for e in doc["exceptions"]))
        # 12: never overwrites.
        r = self.run_("--write-exceptions", self.exc, "--granted-by", "x", "--reason", "y",
                      "--granted-on", "2026-01-05")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("refusing to overwrite", r.stderr)

    # dates: the operator states the date; a bad one is refused.
    def test_granted_on_is_required_and_checked(self):
        r = self.run_("--write-exceptions", self.exc, "--granted-by", "x", "--reason", "y")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(os.path.exists(self.exc))
        r = self.run_("--write-exceptions", self.exc, "--granted-by", "x", "--reason", "y",
                      "--granted-on", "2026-13-40")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(os.path.exists(self.exc))

    # 1 (end to end): the portal says its owner sources failed. No exception
    # file, no unrecorded count, no "read".
    def test_failed_owner_sources_write_no_exceptions_and_no_unrecorded(self):
        Portal.answer = owners(ok=False)
        r = self.run_("--write-exceptions", self.exc, "--granted-by", "x", "--reason", "y",
                      "--granted-on", "2026-01-05")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(os.path.exists(self.exc), "a blind cycle must not except everyone")
        r = self.run_()
        self.assertEqual(r.returncode, 0, r.stderr)
        m = self.metrics()
        self.assertEqual(m["goat_peer_owner_read"], 0)
        self.assertFalse(any(k.startswith("goat_peer_owner_unrecorded") for k in m),
                         "no unrecorded count from a cycle that read nothing")

    # 3: an unread cycle carries the last read count; it never writes 0.
    def test_an_unread_cycle_carries_the_last_count(self):
        self.assertEqual(self.run_().returncode, 0)
        m = self.metrics()
        self.assertEqual(m['goat_peer_owner_unrecorded{class="other"}'], 2)
        read_at = m["goat_peer_owner_counts_timestamp_seconds"]
        Portal.answer = owners(ok=False)
        self.assertEqual(self.run_().returncode, 0)
        m = self.metrics()
        self.assertEqual(m["goat_peer_owner_read"], 0)
        self.assertEqual(m['goat_peer_owner_unrecorded{class="other"}'], 2, "carried, not 0")
        self.assertEqual(m["goat_peer_owner_counts_timestamp_seconds"], read_at)

    # 6, 7, 9: a malformed exceptions file never crashes the writer; it says
    # whether the file loaded and how many entries it refused.
    def test_a_malformed_exceptions_file_is_reported_not_a_crash(self):
        for doc in ([], {"schema": "goat.peer-owner-exceptions/v1", "exceptions": {}},
                    {"schema": "goat.peer-owner-exceptions/v1", "exceptions": ["x"]},
                    {"schema": "wrong", "exceptions": []}):
            self.write(doc)
            r = self.run_()
            self.assertEqual(r.returncode, 0, (doc, r.stderr))
            self.assertTrue(os.path.exists(os.path.join(self.www, "peer-names.json")))
            m = self.metrics()
            self.assertEqual(m["goat_peer_owner_exceptions_configured"], 1, doc)
            if doc == {"schema": "goat.peer-owner-exceptions/v1", "exceptions": ["x"]}:
                self.assertEqual(m["goat_peer_owner_exceptions_loaded"], 1)
                self.assertEqual(m["goat_peer_owner_exceptions_refused"], 1)
            else:
                self.assertEqual(m["goat_peer_owner_exceptions_loaded"], 0, doc)

    def test_the_loader_enforces_the_schema(self):
        ok = {"peer_id": "p2", "name": "edge-gw-unlisted", "reason": "r", "granted_by": "g",
              "granted_on": "2026-01-05"}
        bad = [dict(ok, peer_id="p3", name=""),                     # name required
               dict(ok, peer_id="p3", name="old-kiosk", granted_on="whenever"),
               dict(ok, peer_id="p3", name="old-kiosk", review_by="soon"),
               dict(ok, peer_id="p3", name="old-kiosk", extra=1)]
        for e in bad:
            self.write({"schema": "goat.peer-owner-exceptions/v1", "exceptions": [ok, e]})
            self.assertEqual(self.run_().returncode, 0)
            m = self.metrics()
            self.assertEqual(m["goat_peer_owner_exceptions_refused"], 1, e)
            self.assertEqual(m['goat_peer_owner_excepted{class="other"}'], 1, e)
            r = self.run_("--check-exceptions", self.exc)
            self.assertNotEqual(r.returncode, 0, e)
        # a duplicate peer_id is refused, both entries: neither decision is the one
        self.write({"schema": "goat.peer-owner-exceptions/v1", "exceptions": [ok, dict(ok, reason="other")]})
        self.assertEqual(self.run_().returncode, 0)
        m = self.metrics()
        self.assertEqual(m["goat_peer_owner_exceptions_refused"], 2)
        self.assertEqual(m['goat_peer_owner_excepted{class="other"}'], 0)
        self.write({"schema": "goat.peer-owner-exceptions/v1", "exceptions": [ok]})
        self.assertEqual(self.run_("--check-exceptions", self.exc).returncode, 0)

    # 11: the names of exceptions whose peer left the mesh are in the journal.
    def test_peer_gone_names_are_logged(self):
        self.write({"schema": "goat.peer-owner-exceptions/v1", "exceptions": [
            {"peer_id": "p-left", "name": "laptop-that-left", "reason": "r", "granted_by": "g",
             "granted_on": "2026-01-05"}]})
        r = self.run_()
        self.assertEqual(r.returncode, 0)
        self.assertIn("laptop-that-left", r.stderr)
        self.assertEqual(self.metrics()['goat_peer_owner_exception_stale{reason="peer_gone"}'], 1)


if __name__ == "__main__":
    unittest.main()
