# SPDX-License-Identifier: Apache-2.0
"""The monitor's API: every path the OpenAPI document names is served, and the docs page loads it."""
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import bn_monitor  # noqa: E402


def _free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


class MonitorApi(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.dir = tempfile.mkdtemp()
        doc = dict(bn_monitor._STATUS_EXAMPLE[0], now=time.time())
        with open(os.path.join(cls.dir, "simple-bn.json"), "w") as f:
            json.dump(doc, f)
        cls.port = _free_port()
        cls.proc = subprocess.Popen([sys.executable, os.path.join(os.path.dirname(HERE), "bn_monitor.py"),
                                     "--bind", "127.0.0.1", "--port", str(cls.port), "--status-dir", cls.dir])
        for _ in range(50):
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{cls.port}/healthz", timeout=1)
                break
            except OSError:
                time.sleep(0.1)

    @classmethod
    def tearDownClass(cls):
        cls.proc.terminate()
        cls.proc.wait()

    def get(self, path):
        try:
            r = urllib.request.urlopen(f"http://127.0.0.1:{self.port}{path}", timeout=3)
            return r.status, r.read()
        except urllib.error.HTTPError as e:
            return e.code, e.read()

    def test_openapi_served_and_valid_shape(self):
        code, body = self.get("/api/v1/openapi.json")
        self.assertEqual(code, 200)
        spec = json.loads(body)
        self.assertEqual(spec["openapi"][:2], "3.")
        self.assertEqual(set(spec["paths"]), {"/api/v1/status", "/api/v1/feeds/{feed}", "/api/v1/feedback", "/api/v1/health", "/api/v1/modifications",
                                              "/api/v1/sim-status"})

    def test_every_documented_path_answers(self):
        for path in ("/api/v1/status", "/api/v1/feeds/tpq53", "/api/v1/feedback", "/api/v1/health",
                     "/api/v1/sim-status"):
            code, body = self.get(path)
            self.assertEqual(code, 200, path)
            json.loads(body)
        self.assertEqual(self.get("/api/v1/feeds/nope")[0], 404)

    def test_sim_status_lists_every_feed(self):
        code, body = self.get("/api/v1/sim-status")
        rows = json.loads(body)
        self.assertEqual([r["feed"] for r in rows], ["tpq53"])
        self.assertEqual(rows[0]["declared"], "simulated")
        self.assertEqual(rows[0]["node"], "boundary-host-a")

    def test_docs_page_points_at_the_spec(self):
        code, body = self.get("/api/docs")
        self.assertEqual(code, 200)
        self.assertIn(b'url:"/api/v1/openapi.json"', body)
        self.assertIn(b"integrity=", body)
        self.assertIn(b"tryItOutEnabled:true", body)


if __name__ == "__main__":
    unittest.main()
