"""Onboarding passes the resolved URL through the existing --api-url interface."""

import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "ops/scripts/peer-attribution.py"


class FakeNetBird(http.server.BaseHTTPRequestHandler):
    seen = []

    def do_GET(self):
        FakeNetBird.seen.append((self.path, self.headers.get("Authorization")))
        body = json.dumps([]).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


class PeerAttributionUrlTests(unittest.TestCase):
    def setUp(self):
        FakeNetBird.seen = []
        self.server = http.server.HTTPServer(("127.0.0.1", 0), FakeNetBird)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        self.env = {k: v for k, v in os.environ.items() if not k.endswith("_NB_API_TOKEN")}
        self.env["TESTNET_NB_API_TOKEN"] = "fake-token"

    def run_script(self, *args):
        return subprocess.run([sys.executable, "-I", str(SCRIPT), *args],
                              env=self.env, capture_output=True, text=True, timeout=30)

    def test_url_flag_reaches_an_unlisted_goatnet(self):
        r = self.run_script("testnet", "--handle", "demo-person", "--api-url", self.url)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn(("/api/peers", "Token fake-token"), FakeNetBird.seen)

    def test_onboarding_passes_the_existing_url_option(self):
        import yaml
        play = yaml.safe_load((ROOT / "ops/ansible/operator-onboard.yml").read_text())[0]
        text = (ROOT / "ops/ansible/operator-onboard.yml").read_text()
        self.assertIn("['--api-url', netbird_site_url_map[item]]", text)
        self.assertNotIn("['--url',", text)
        self.assertIn("goatnet_netbird_site_url_map", text)


if __name__ == "__main__":
    unittest.main()
