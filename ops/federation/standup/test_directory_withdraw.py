"""directory_withdraw asks for exactly our slot for one counterparty, signed as saltlick
requires, against a local stand-in directory that serves several slots."""
import http.server
import importlib.util
import json
from pathlib import Path
import tempfile
import threading
import unittest
import urllib.request

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


dw = load("directory_withdraw")
directory = load("directory")
SEED = bytes(range(32))


class StandIn(http.server.BaseHTTPRequestHandler):
    """Hosts xaa's entry and its slots for xab and xac, and xab's own entry. A DELETE
    is honored only when it is signed by xaa's key over the saltlick statement."""
    slots = {}
    seen = []
    key = None

    def log_message(self, *a):
        pass

    def do_DELETE(self):  # noqa: N802
        StandIn.seen.append(("DELETE", self.path))
        path = self.path[len("/v1/parties/"):]
        sig = self.headers.get("X-Saltlick-Signature", "")
        import base64
        ok = directory.verify(StandIn.key, directory.request_statement("DELETE", path, b"", None),
                              base64.b64decode(sig or "AA=="))
        if not ok:
            self.send_response(401)
            self.end_headers()
            return
        if path not in StandIn.slots:
            self.send_response(404)
            self.end_headers()
            return
        del StandIn.slots[path]
        self.send_response(204)
        self.end_headers()


class Withdraw(unittest.TestCase):
    def setUp(self):
        StandIn.slots = {"xaa": b"single", "xaa/packages/xab": b"for-xab", "xaa/packages/xac": b"for-xac",
                         "xab": b"theirs", "xab/packages/xaa": b"theirs-for-us"}
        StandIn.seen = []
        StandIn.key = directory.public_key(SEED)
        self.srv = http.server.HTTPServer(("127.0.0.1", 0), StandIn)
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.srv.server_port}"
        self.key = tempfile.NamedTemporaryFile("w", delete=False, suffix=".key")
        self.key.write(SEED.hex())
        self.key.close()

    def tearDown(self):
        self.srv.shutdown()

    @classmethod
    def tearDownClass(cls):
        # urllib builds its process-wide opener, and with it the TLS trust store, on the
        # first urlopen. Drop it so a later test that trusts its own CA through
        # SSL_CERT_FILE gets an opener built after that variable is set.
        urllib.request.install_opener(None)

    def test_only_our_slot_for_that_counterparty_goes(self):
        r = dw.withdraw(self.url, "xaa", "xac", self.key.name, False)
        self.assertEqual(r["outcome"], "withdrawn", r)
        self.assertEqual(StandIn.seen, [("DELETE", "/v1/parties/xaa/packages/xac")])
        self.assertEqual(sorted(StandIn.slots), ["xaa", "xaa/packages/xab", "xab", "xab/packages/xaa"])

    def test_twice_is_absent(self):
        dw.withdraw(self.url, "xaa", "xac", self.key.name, False)
        r = dw.withdraw(self.url, "xaa", "xac", self.key.name, False)
        self.assertEqual(r["outcome"], "absent", r)

    def test_dry_run_sends_nothing(self):
        r = dw.withdraw(self.url, "xaa", "xac", self.key.name, True)
        self.assertEqual(r["outcome"], "would-withdraw")
        self.assertEqual(StandIn.seen, [])

    def test_no_key_is_a_failure_and_sends_nothing(self):
        r = dw.withdraw(self.url, "xaa", "xac", None, False)
        self.assertEqual(r["outcome"], "failed")
        self.assertEqual(StandIn.seen, [])

    def test_ids_are_checked_before_anything_is_sent(self):
        for us, them in (("xaa", "xaa"), ("xaa", "../xab"), ("XAA", "xab")):
            self.assertEqual(dw.withdraw(self.url, us, them, self.key.name, False)["outcome"], "failed")
        self.assertEqual(StandIn.seen, [])

    def test_an_unreachable_directory_is_reported(self):
        r = dw.withdraw("http://127.0.0.1:9", "xaa", "xac", self.key.name, False, timeout=2)
        self.assertEqual(r["outcome"], "failed")

    def test_the_command_line_prints_one_json_line(self):
        import contextlib
        import io
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = dw.main(["--url", self.url, "--us", "xaa", "--them", "xab", "--key", self.key.name])
        self.assertEqual(rc, 0)
        self.assertEqual(json.loads(out.getvalue())["outcome"], "withdrawn")


if __name__ == "__main__":
    unittest.main()
