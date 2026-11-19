# SPDX-License-Identifier: Apache-2.0
"""python3 -m unittest discover ops/door-admit/tests   (expand tests need PyYAML)"""
import base64
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import door_admit as d  # noqa: E402


def jwt(claims):
    enc = lambda b: base64.urlsafe_b64encode(b).rstrip(b"=").decode()
    return "Bearer " + ".".join([enc(b'{"alg":"RS256"}'), enc(json.dumps(claims).encode()), enc(b"sig")])


ADMIT = {"aff_ids": ["org-alpha", "org-alpha-data", "nation-b-air"], "groups": ["SITE-PLATFORM-ADMIN"],
         "emails": ["Person@Example.org"]}

REGISTRY = {"orgs": [
    {"id": "org-alpha", "nation": "ZZA", "aliases": ["orgalpha"]},
    {"id": "org-alpha-data", "nation": "ZZA", "parent": "org-alpha"},
    {"id": "deep-child", "nation": "ZZA", "parent": "org-alpha-data"},
    {"id": "nation-b-forces", "nation": "ZZB"},
    {"id": "nation-b-air", "nation": "ZZB", "parent": "nation-b-forces"},
    {"id": "vendor-b", "nation": "ZZB"},
    {"id": "other", "nation": "ZZC"},
    {"id": "retired-b", "nation": "ZZB", "status": "retired"}]}


class Decide(unittest.TestCase):
    def test_org(self):
        self.assertTrue(d.decide({"aff": {"id": "org-alpha-data"}}, ADMIT)[0])

    def test_group(self):
        ok, why = d.decide({"aff": {"id": "other"}, "groups": ["SITE-PLATFORM-ADMIN"]}, ADMIT)
        self.assertTrue(ok)
        self.assertIn("group", why)

    def test_email_case_insensitive(self):
        self.assertTrue(d.decide({"email": "person@example.ORG"}, ADMIT)[0])

    def test_refused_and_reason(self):
        ok, why = d.decide({"aff": {"id": "other"}, "email": "x@y.z"}, ADMIT)
        self.assertFalse(ok)
        self.assertIn("other", why)

    def test_no_token(self):
        self.assertEqual(d.decide({}, ADMIT), (False, "no sign-in token reached the door"))

    def test_claims_from(self):
        self.assertEqual(d.claims_from(jwt({"email": "a@b"}))["email"], "a@b")
        self.assertEqual(d.claims_from("Bearer not.a"), {})
        self.assertEqual(d.claims_from(None), {})


class Expand(unittest.TestCase):
    def setUp(self):
        self.x = None
        try:
            import expand_rule as x
        except ImportError:
            self.skipTest("PyYAML not installed")
        self.x = x

    def test_orgs_and_nations(self):
        ids, matched = self.x.expand(REGISTRY, {"orgs": ["org-alpha"], "nations": ["ZZB"]})
        self.assertEqual(set(ids), {"org-alpha", "orgalpha", "org-alpha-data", "deep-child",
                                    "nation-b-forces", "nation-b-air", "vendor-b"})
        self.assertNotIn("retired-b", ids)
        self.assertNotIn("other", ids)

    def test_unknown_refuses(self):
        with self.assertRaises(ValueError):
            self.x.expand(REGISTRY, {"orgs": ["no-such-org"]})
        with self.assertRaises(ValueError):
            self.x.expand(REGISTRY, {"nations": ["XXX"]})


class EndToEnd(unittest.TestCase):
    """door-admit in front of a tiny upstream: admitted requests pass without the token, others get 403."""

    def test_proxy(self):
        seen = {}

        class Up(BaseHTTPRequestHandler):
            def do_GET(self):
                seen["auth"] = self.headers.get("Authorization")
                body = b"upstream says hi"
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *a):
                pass

        up = ThreadingHTTPServer(("127.0.0.1", 0), Up)
        threading.Thread(target=up.serve_forever, daemon=True).start()
        s = socket.socket(); s.bind(("127.0.0.1", 0)); port = s.getsockname()[1]; s.close()
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump({"service": "test", "rule_text": "test rule", "admit": ADMIT}, f)
        p = subprocess.Popen([sys.executable, os.path.join(HERE, "..", "door_admit.py"), "--listen", f"127.0.0.1:{port}",
                              "--upstream", f"http://127.0.0.1:{up.server_address[1]}", "--rules", f.name],
                             stdout=subprocess.DEVNULL)
        try:
            for _ in range(50):
                try:
                    urllib.request.urlopen(f"http://127.0.0.1:{port}/door-admit/healthz", timeout=1); break
                except OSError:
                    time.sleep(0.1)
            req = urllib.request.Request(f"http://127.0.0.1:{port}/", headers={"Authorization": jwt({"aff": {"id": "nation-b-air"}})})
            self.assertEqual(urllib.request.urlopen(req, timeout=5).read(), b"upstream says hi")
            self.assertIsNone(seen["auth"])
            req = urllib.request.Request(f"http://127.0.0.1:{port}/", headers={"Authorization": jwt({"aff": {"id": "other"}})})
            with self.assertRaises(urllib.error.HTTPError) as cm:
                urllib.request.urlopen(req, timeout=5)
            self.assertEqual(cm.exception.code, 403)
            raw = socket.create_connection(("127.0.0.1", port), timeout=5)
            raw.sendall(b"GET http://evil.example/ HTTP/1.1\r\nHost: x\r\n\r\n")
            self.assertIn(b" 400 ", raw.recv(200))
            raw.close()
        finally:
            p.terminate(); p.wait(); up.shutdown()


if __name__ == "__main__":
    unittest.main()
