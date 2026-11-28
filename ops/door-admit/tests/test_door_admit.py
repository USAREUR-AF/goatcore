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
try:
    import expand_rule  # noqa: E402  (needs PyYAML)
except ImportError:
    expand_rule = None


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

    def test_claims_from_requires_an_object(self):
        # a payload of JSON null or a list is not a claim set
        self.assertEqual(d.claims_from("Bearer a.bnVsbA.c"), {})
        self.assertEqual(d.claims_from("Bearer a.W10.c"), {})
        self.assertEqual(d.claims_from("Bearer a.IngiCg.c"), {})


class Expand(unittest.TestCase):
    def setUp(self):
        if expand_rule is None:
            self.skipTest("PyYAML not installed")
        self.x = expand_rule

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



class Door:
    """door-admit as a subprocess in front of an upstream that records every request it gets."""

    def __enter__(self):
        self.seen = []
        seen = self.seen

        class Up(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def _any(self):
                n = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(n) if n else b""
                seen.append({"method": self.command, "path": self.path, "body": body,
                             "multi": self.headers.get_all("X-Multi") or [],
                             "auth": self.headers.get("Authorization")})
                out = b"upstream says hi"
                self.send_response(200)
                self.send_header("Content-Length", str(len(out)))
                self.end_headers()
                self.wfile.write(out)

            do_GET = do_POST = _any

            def log_message(self, *a):
                pass

        self.up = ThreadingHTTPServer(("127.0.0.1", 0), Up)
        threading.Thread(target=self.up.serve_forever, daemon=True).start()
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        self.port = s.getsockname()[1]
        s.close()
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump({"service": "test", "rule_text": "test rule", "admit": ADMIT}, f)
        self.rules = f.name
        self.p = subprocess.Popen([sys.executable, os.path.join(HERE, "..", "door_admit.py"), "--listen",
                                   f"127.0.0.1:{self.port}", "--upstream", f"http://127.0.0.1:{self.up.server_address[1]}",
                                   "--rules", f.name], stdout=subprocess.DEVNULL)
        for _ in range(50):
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{self.port}/door-admit/healthz", timeout=1).close()
                break
            except OSError:
                time.sleep(0.1)
        return self

    def __exit__(self, *exc):
        self.p.terminate()
        self.p.wait()
        self.up.shutdown()
        self.up.server_close()
        os.unlink(self.rules)

    def raw(self, data, timeout=3):
        """Send raw bytes on one connection; return everything read until the door closes it or goes quiet."""
        c = socket.create_connection(("127.0.0.1", self.port), timeout=timeout)
        c.sendall(data)
        out = b""
        try:
            while True:
                chunk = c.recv(65536)
                if not chunk:
                    break
                out += chunk
        except socket.timeout:
            out += b"<<timeout>>"
        c.close()
        time.sleep(0.2)
        return out


ADMITTED = jwt({"aff": {"id": "nation-b-air"}, "email": "in@example.org"})
REFUSED_TOKEN = jwt({"aff": {"id": "other"}, "email": "out@example.org"})


class RequestParsing(unittest.TestCase):
    """One request in is at most one request out, and a refusal is safe to render."""

    def test_chunked_body_is_not_read_as_a_second_request(self):
        inner = b"GET /smuggled HTTP/1.1\r\nHost: localhost\r\n\r\n"
        req = (b"POST / HTTP/1.1\r\nHost: x\r\nAuthorization: " + ADMITTED.encode() +
               b"\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n" + inner)
        with Door() as door:
            out = door.raw(req)
            self.assertTrue(out.startswith(b"HTTP/1.1 411 "), out[:80])
            self.assertNotIn(b"<<timeout>>", out, "the connection must be closed after the refusal")
            self.assertEqual([r["path"] for r in door.seen if r["path"] == "/smuggled"], [])

    def test_refused_body_is_not_read_as_a_second_request(self):
        inner = (b"GET /forged HTTP/1.1\r\nHost: x\r\nAuthorization: " + ADMITTED.encode() + b"\r\n\r\n")
        req = (b"POST / HTTP/1.1\r\nHost: x\r\nAuthorization: " + REFUSED_TOKEN.encode() +
               b"\r\nContent-Length: " + str(len(inner)).encode() + b"\r\n\r\n" + inner)
        with Door() as door:
            out = door.raw(req)
            self.assertTrue(out.startswith(b"HTTP/1.1 403 "), out[:80])
            self.assertNotIn(b"upstream says hi", out)
            self.assertNotIn(b"<<timeout>>", out)
            self.assertEqual(door.seen, [])

    def test_healthz_closes_the_connection(self):
        inner = (b"GET /forged HTTP/1.1\r\nHost: x\r\nAuthorization: " + ADMITTED.encode() + b"\r\n\r\n")
        req = (b"POST /door-admit/healthz HTTP/1.1\r\nHost: x\r\nContent-Length: " + str(len(inner)).encode() +
               b"\r\n\r\n" + inner)
        with Door() as door:
            out = door.raw(req)
            self.assertNotIn(b"<<timeout>>", out)
            self.assertEqual(door.seen, [])

    def test_refusal_page_escapes_the_caller(self):
        req = (b"GET / HTTP/1.1\r\nHost: x\r\nX-Forwarded-Email: <script>alert(1)</script>\r\n\r\n")
        with Door() as door:
            out = door.raw(req)
            self.assertTrue(out.startswith(b"HTTP/1.1 403 "), out[:80])
            self.assertNotIn(b"<script>", out)
            self.assertIn(b"&lt;script&gt;", out)

    def test_negative_and_malformed_content_length(self):
        with Door() as door:
            for value in (b"-1", b"abc", b"\xb2", b"99999999999999"):
                req = (b"POST / HTTP/1.1\r\nHost: x\r\nAuthorization: " + ADMITTED.encode() +
                       b"\r\nContent-Length: " + value + b"\r\n\r\n")
                out = door.raw(req)
                self.assertRegex(out[:13], rb"HTTP/1.1 4(00|13)", (value, out[:80]))
                self.assertNotIn(b"<<timeout>>", out, value)
            self.assertEqual(door.seen, [])

    def test_null_payload_token_is_refused_not_a_crash(self):
        req = b"GET / HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer a.bnVsbA.c\r\n\r\n"
        with Door() as door:
            self.assertTrue(door.raw(req).startswith(b"HTTP/1.1 403 "))

    def test_admitted_post_keeps_repeated_headers_and_the_body(self):
        req = (b"POST /x HTTP/1.1\r\nHost: x\r\nAuthorization: " + ADMITTED.encode() +
               b"\r\nX-Multi: one\r\nX-Multi: two\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello")
        with Door() as door:
            out = door.raw(req)
            self.assertIn(b"upstream says hi", out)
            self.assertEqual(len(door.seen), 1)
            self.assertEqual(door.seen[0]["multi"], ["one", "two"])
            self.assertEqual(door.seen[0]["body"], b"hello")
            self.assertIsNone(door.seen[0]["auth"])


if __name__ == "__main__":
    unittest.main()
