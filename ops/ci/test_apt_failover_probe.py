#!/usr/bin/env python3
"""apt-failover's probe judges what apt fetches, not a HEAD.

.github/actions/apt-failover/probe.sh decides whether the runner's Ubuntu mirror answers.
It used to send one HEAD for dists/<codename>/Release. On 19 Nov 2026 a go-vuln job's
probe passed in 1 s, and the apt-get update right after it timed out: the mirror answered
the HEAD and then gave nothing for noble and noble-updates InRelease. The probe now GETs
dists/<suite>/InRelease for <codename>, <codename>-updates and <codename>-security and
reads each to its end within the bound. Any one failing means the mirror does not answer,
and the job fails over to the cache (and exports no UBUNTU_MIRROR).

Each case runs the probe against a stand-in mirror on 127.0.0.1.

Run: python3 -m pytest ops/ci/test_apt_failover_probe.py
"""
import http.server
import socketserver
import subprocess
import threading
import time
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
PROBE = REPO / ".github/actions/apt-failover/probe.sh"
ACTION = REPO / ".github/actions/apt-failover/action.yml"
CODENAME = "noble"
SUITES = (CODENAME, CODENAME + "-updates", CODENAME + "-security")
BODY = b"-----BEGIN PGP SIGNED MESSAGE-----\nOrigin: Ubuntu\n" + b"x" * 20000 + b"\n"


def stand_in(behaviour):
    """A mirror at http://127.0.0.1:<port>/ubuntu whose answers `behaviour(suite, method)` picks:
    'ok', 'hang' (headers, then a body that never ends), 'silent' (no answer at all),
    or an HTTP status code."""

    class H(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.0"

        def log_message(self, *a):
            pass

        def answer(self, method):
            path = self.path
            suite = None
            for s in SUITES:
                if path in ("/ubuntu/dists/%s/InRelease" % s, "/ubuntu/dists/%s/Release" % s):
                    suite = s
            b = behaviour(suite, method) if suite else 404
            if b == "silent":
                time.sleep(30)
                return
            if isinstance(b, int):
                self.send_response(b)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            self.send_response(200)
            self.send_header("Content-Length", str(len(BODY)))
            self.end_headers()
            if method == "HEAD":
                return
            if b == "hang":
                self.wfile.write(BODY[:100])
                self.wfile.flush()
                time.sleep(30)
                return
            self.wfile.write(BODY)

        def do_GET(self):
            self.answer("GET")

        def do_HEAD(self):
            self.answer("HEAD")

    class S(socketserver.ThreadingMixIn, http.server.HTTPServer):
        daemon_threads = True

    srv = S(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


class Probe(unittest.TestCase):
    def run_probe(self, behaviour, seconds=3):
        srv = stand_in(behaviour)
        try:
            t0 = time.monotonic()
            r = subprocess.run(["bash", str(PROBE), "127.0.0.1", str(srv.server_address[1]), "/ubuntu",
                                CODENAME, str(seconds)], capture_output=True, text=True, timeout=60)
            return r.returncode, time.monotonic() - t0, r.stdout + r.stderr
        finally:
            srv.shutdown()

    def test_a_mirror_that_serves_every_inrelease_answers(self):
        rc, _, out = self.run_probe(lambda s, m: "ok")
        self.assertEqual(rc, 0, out)

    def test_a_head_that_answers_and_a_get_that_hangs_is_no_answer(self):
        # The 19 Nov shape: headers at once, then nothing.
        rc, took, out = self.run_probe(lambda s, m: "hang" if s == CODENAME else "ok")
        self.assertEqual(rc, 1, out)   # 1: no answer (not 127, a probe that did not run)
        self.assertLess(took, 3 * 3 + 10, "the probe ran past its bound")

    def test_one_suite_that_gives_no_answer_is_no_answer(self):
        rc, _, out = self.run_probe(lambda s, m: "silent" if s == CODENAME + "-updates" else "ok")
        self.assertEqual(rc, 1, out)   # 1: no answer (not 127, a probe that did not run)

    def test_one_suite_answering_404_is_no_answer(self):
        rc, _, out = self.run_probe(lambda s, m: 404 if s == CODENAME + "-security" else "ok")
        self.assertEqual(rc, 1, out)   # 1: no answer (not 127, a probe that did not run)

    def test_a_mirror_that_answers_only_head_is_no_answer(self):
        # The old probe passed this mirror.
        rc, _, out = self.run_probe(lambda s, m: "ok" if m == "HEAD" else "hang")
        self.assertEqual(rc, 1, out)   # 1: no answer (not 127, a probe that did not run)

    def test_nothing_listening_is_no_answer(self):
        r = subprocess.run(["bash", str(PROBE), "127.0.0.1", "1", "/ubuntu", CODENAME, "3"],
                           capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 1)

    def test_the_action_judges_the_mirror_with_this_probe(self):
        t = ACTION.read_text()
        self.assertTrue('probe.sh' in t, "the action does not run probe.sh")
        self.assertFalse("HEAD %s/dists/%s/Release" in t, "the action still has the HEAD probe")


if __name__ == "__main__":
    unittest.main()
