"""The round-trip check (ops/federation/roundtrip/) against a fabric in one process.

fedcheck.sh sends a probe with a fresh nonce once per gap until one is acknowledged
or the timeout passes, so a probe the receiving edge's downsampling drops (a second
sample inside its interval, with nothing logged) does not fail the round trip; it
never sends faster than one probe per gap. It fails at once when our core refuses
the probe. responder.py keeps running when one acknowledgement cannot be written.

Run: python3 -m pytest ops/federation/standup/test_roundtrip.py
Needs bash, curl and openssl.
"""
import http.server
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HERE = Path(__file__).resolve().parent
FEDCHECK = HERE.parent / "roundtrip" / "fedcheck.sh"
RESPONDER = HERE.parent / "roundtrip" / "responder.py"

_spec = importlib.util.spec_from_file_location("acceptance_fakefabric", HERE / "acceptance_fakefabric.py")
fakefabric = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fakefabric)

# The fake's arrival times are taken when its server handles each PUT; allow for
# the scheduling of that thread, never for a resend that came early.
JITTER = 0.05


@unittest.skipUnless(shutil.which("curl") and shutil.which("openssl"), "needs curl and openssl")
class Fedcheck(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="roundtrip-test-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.fabric = fakefabric.Fabric(fakefabric.agreement(), "xaa", self.tmp)
        self.addCleanup(self.fabric.stop)
        self.env = None

    def fedcheck(self, timeout="5", gap="0.5", base=None, extra_args=()):
        if self.env is None:
            self.env = self.fabric.start()
        env = dict(os.environ, BASE=base or self.env["FED_OURS_REST"])
        t0 = time.monotonic()
        args = ["bash", str(FEDCHECK), "release/all", "from/xab", timeout] + ([gap] if gap is not None else [])
        p = subprocess.run(args + list(extra_args), env=env, capture_output=True, text=True, timeout=120)
        return p.returncode, p.stdout + p.stderr, time.monotonic() - t0

    def probe_times(self):
        return [t for t, side, key in self.fabric.probe_log if side == "xaa" and key.startswith("release/all/fedcheck/")]

    def assert_spaced(self, times, gap):
        for a, b in zip(times, times[1:]):
            self.assertGreaterEqual(b - a, gap - JITTER, "probes %.3fs apart, gap %.2fs" % (b - a, gap))

    def test_a_working_round_trip_sends_one_probe(self):
        rc, out, _ = self.fedcheck()
        self.assertEqual(rc, 0, out)
        self.assertIn("PASS  round trip complete in ", out)
        self.assertEqual(len(self.probe_times()), 1, out)

    def test_a_dropped_first_probe_still_completes_the_round_trip(self):
        self.fabric.faults.drop_probes["xab"] = 1
        rc, out, _ = self.fedcheck(gap="0.5")
        self.assertEqual(rc, 0, out)
        self.assertIn("PASS  round trip complete in ", out)
        self.assertIn("probe 2 of 2", out)
        times = self.probe_times()
        self.assertEqual(len(times), 2, out)
        self.assert_spaced(times, 0.5)

    def test_it_never_sends_faster_than_one_probe_per_gap(self):
        self.fabric.faults.responder_down.add("xab")
        rc, out, _ = self.fedcheck(timeout="4", gap="1.2")
        self.assertEqual(rc, 1, out)
        self.assertIn("no acknowledgement in 4s", out)
        times = self.probe_times()
        self.assertGreaterEqual(len(times), 2, out)
        self.assertLessEqual(len(times), 4, out)
        self.assert_spaced(times, 1.2)
        self.assertIn("%d probes sent" % len(times), out)

    def test_a_gap_shorter_than_its_poll_is_held_to_the_poll(self):
        self.fabric.faults.responder_down.add("xab")
        rc, out, _ = self.fedcheck(timeout="2", gap="0")
        self.assertEqual(rc, 1, out)
        self.assert_spaced(self.probe_times(), 0.5)

    def test_without_a_gap_it_waits_a_second_between_probes(self):
        self.fabric.faults.responder_down.add("xab")
        rc, out, _ = self.fedcheck(timeout="3", gap=None)
        self.assertEqual(rc, 1, out)
        self.assert_spaced(self.probe_times(), 1.0)

    def test_an_acknowledgement_of_an_earlier_probe_counts(self):
        # The first probe is acknowledged only after a second was sent: either answer
        # completes the round trip, and the digest is checked against that probe.
        self.fabric.faults.responder_down.add("xab")
        self.env = self.fabric.start()

        def late_ack():
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                keys = [k for _, s, k in self.fabric.probe_log if s == "xaa"]
                if len(keys) >= 2:
                    first = keys[0]
                    wire = "from/xaa/" + first[len("release/"):]
                    with self.fabric.cores["xab"].lock:
                        raw = self.fabric.cores["xab"].store[wire]
                    self.fabric.faults.responder_down.discard("xab")
                    self.fabric.respond("xab", wire, raw)
                    return
                time.sleep(0.05)

        t = threading.Thread(target=late_ack, daemon=True)
        t.start()
        rc, out, _ = self.fedcheck(timeout="8", gap="0.5")
        t.join()
        self.assertEqual(rc, 0, out)
        self.assertIn("probe 1 of ", out)

    def test_a_probe_our_core_refuses_fails_at_once(self):
        self.fabric.faults.refuse_probe_put.add("xaa")
        rc, out, took = self.fedcheck(timeout="10")
        self.assertEqual(rc, 1, out)
        self.assertIn("HTTP 403", out)
        self.assertLess(took, 5, out)
        self.assertNotIn("PASS", out)

    def test_a_core_that_does_not_answer_is_named(self):
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
        s.close()
        rc, out, _ = self.fedcheck(timeout="2", base="http://127.0.0.1:%d" % port)
        self.assertEqual(rc, 1, out)
        self.assertIn("our core did not answer", out)

    def test_a_gap_that_is_not_a_number_is_refused(self):
        rc, out, _ = self.fedcheck(gap="soon")
        self.assertEqual(rc, 2, out)
        self.assertIn("not a number of seconds", out)


class Responder(unittest.TestCase):
    """responder.py against a REST surface that refuses its first acknowledgement."""

    def test_a_failed_acknowledgement_does_not_stop_the_responder(self):
        puts = []
        probe = {"key": "from/xab/all/fedcheck/rt1", "value": "ZmVkY2hlY2stcnQx", "encoding": "zenoh/bytes"}

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                body = json.dumps([probe] if "/fedcheck/" in self.path else []).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_PUT(self):
                self.rfile.read(int(self.headers.get("Content-Length") or 0))
                puts.append(self.path)
                self.send_response(500 if len(puts) == 1 else 200)
                self.send_header("Content-Length", "0")
                self.end_headers()

        srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.addCleanup(srv.server_close)
        self.addCleanup(srv.shutdown)
        env = dict(os.environ, BASE="http://127.0.0.1:%d" % srv.server_address[1])
        p = subprocess.Popen([sys.executable, str(RESPONDER), "from/xab", "release/all"], env=env,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(p.kill)
        deadline = time.monotonic() + 10
        while len(puts) < 2 and time.monotonic() < deadline and p.poll() is None:
            time.sleep(0.05)
        alive = p.poll() is None
        p.kill()
        _, err = p.communicate()
        self.assertTrue(alive, err)
        self.assertEqual(puts[:2], ["/release/all/fedcheck-ack/rt1"] * 2, err)
        self.assertIn("rt1", err)


if __name__ == "__main__":
    unittest.main()
