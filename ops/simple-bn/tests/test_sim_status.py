# SPDX-License-Identifier: Apache-2.0
"""Out-of-band sim/real status: the declaration tool, and what the sender shows for each feed."""
import json
import os
import socket
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import sim_status  # noqa: E402
import simple_bn_sender as s  # noqa: E402

SYS_REF = "44.20,28.60"   # a made-up system reference point


def free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as t:
        t.bind(("127.0.0.1", 0))
        return t.getsockname()[1]


def relay_feed(simulated=None, **extra):
    """A relayed real feed reshaped to system tracks (simulated None: no reshape, records pass unchanged)."""
    os.environ["TEST_SYS_REF"] = SYS_REF
    p = {"src_port": free_port(), "cats": [62], "data_class": "real", "expect_sac": 7, "expect_sic": 5, **extra}
    if simulated is not None:
        p["reshape"] = {"to": "system", "reference_env": "TEST_SYS_REF", "simulated": simulated}
    return s.RelayFeed("drone-a", None, {"drone-a": p}, "127.0.0.1", ["127.0.0.1"])


class DeclarationTool(unittest.TestCase):
    def setUp(self):
        self.path = os.path.join(tempfile.mkdtemp(), "sim-status.json")

    def run_tool(self, *args):
        return subprocess.run([sys.executable, os.path.join(os.path.dirname(HERE), "sim_status.py"),
                               "--file", self.path, *args], capture_output=True, text=True)

    def test_set_then_clear_keeps_history(self):
        r = self.run_tool("set", "drone-a", "simulated", "--by", "ops desk", "--note", "source reports a sim run")
        self.assertEqual(r.returncode, 0, r.stderr)
        doc = sim_status.load(self.path)
        self.assertEqual(doc["feeds"]["drone-a"]["mode"], "simulated")
        self.assertEqual(oct(os.stat(self.path).st_mode & 0o777), "0o644")
        self.assertEqual(self.run_tool("clear", "drone-a", "--by", "ops desk").returncode, 0)
        doc = sim_status.load(self.path)
        self.assertNotIn("drone-a", doc["feeds"])
        self.assertEqual([h["mode"] for h in doc["history"]], ["simulated", None])

    def test_refuses_bad_input(self):
        self.assertNotEqual(self.run_tool("set", "drone-a", "maybe", "--by", "x").returncode, 0)
        self.assertNotEqual(self.run_tool("set", "../etc", "real", "--by", "x").returncode, 0)
        self.assertNotEqual(self.run_tool("set", "drone-a", "real", "--by", " ").returncode, 0)
        self.assertFalse(os.path.exists(self.path))

    def test_history_is_bounded(self):
        doc = sim_status.load(self.path)
        for i in range(sim_status.HISTORY + 20):
            sim_status.apply(doc, "drone-a", "real", "x", now=i)
        self.assertEqual(len(doc["history"]), sim_status.HISTORY)


class SenderShowsIt(unittest.TestCase):
    def tearDown(self):
        for f in getattr(self, "feeds", []):
            f.isock.close()

    def feed(self, *a, **k):
        f = relay_feed(*a, **k)
        self.feeds = getattr(self, "feeds", []) + [f]
        return f

    def test_real_feed_with_no_declaration_reads_real_from_the_profile(self):
        st = s.sim_status_doc(self.feed(simulated=False, source_can_simulate=True), {})
        self.assertEqual((st["declared"], st["basis"], st["sim_bit_sent"], st["agrees"]), ("real", "profile", False, True))
        self.assertTrue(st["source_can_simulate"])
        self.assertFalse(st["source_marks_simulation"])

    def test_declared_simulated_on_a_real_feed_disagrees_and_changes_nothing_sent(self):
        f = self.feed(simulated=False, source_can_simulate=True)
        st = s.sim_status_doc(f, {"drone-a": {"mode": "simulated", "set_at": 1.0, "set_by": "ops desk", "note": "n"}})
        self.assertEqual((st["declared"], st["basis"], st["set_by"], st["agrees"]), ("simulated", "operator", "ops desk", False))
        self.assertFalse(f.reshape.simulated)

    def test_unknown_and_pass_through_have_no_verdict(self):
        st = s.sim_status_doc(self.feed(simulated=False), {"drone-a": {"mode": "unknown"}})
        self.assertIsNone(st["agrees"])
        st = s.sim_status_doc(self.feed(), {})
        self.assertIsNone(st["sim_bit_sent"])
        self.assertIsNone(st["agrees"])

    def test_a_bad_declaration_falls_back_to_the_profile(self):
        st = s.sim_status_doc(self.feed(simulated=False), {"drone-a": {"mode": "sometimes"}})
        self.assertEqual(st["basis"], "profile")

    def test_reading_the_file(self):
        d = tempfile.mkdtemp()
        self.assertEqual(s.read_sim_status(os.path.join(d, "absent.json"))[1]["error"], None)
        bad = os.path.join(d, "bad.json")
        with open(bad, "w") as f:
            f.write("{not json")
        feeds, info = s.read_sim_status(bad)
        self.assertEqual(feeds, {})
        self.assertFalse(info["loaded"])
        self.assertTrue(info["error"])
        good = os.path.join(d, "good.json")
        with open(good, "w") as f:
            json.dump({"feeds": {"drone-a": {"mode": "real"}}}, f)
        self.assertEqual(s.read_sim_status(good)[0], {"drone-a": {"mode": "real"}})
        self.assertEqual(s.read_sim_status("none"), ({}, {"path": None, "loaded": False, "error": None}))


class FeedsServiceSource(unittest.TestCase):
    """Declarations from a feeds service merge with the host file: newest wins, each says where it came from."""
    def tearDown(self):
        for f in getattr(self, "feeds", []):
            f.isock.close()

    def feed(self, **k):
        f = relay_feed(simulated=False, source_can_simulate=True, **k)
        self.feeds = getattr(self, "feeds", []) + [f]
        return f

    def test_newest_wins_and_names_its_source(self):
        f = self.feed(sim_status_from="feed-17")
        host = {"drone-a": {"mode": "real", "set_at": 100.0, "set_by": "ops desk"}}
        svc = {"feed-17": {"mode": "simulated", "set_at": 200.0, "set_by": "pilot@example.org", "note": "sim run"}}
        st = s.sim_status_doc(f, s.merge_declarations([f], host, svc))
        self.assertEqual((st["declared"], st["source"], st["set_by"], st["agrees"]), ("simulated", "feeds-service", "pilot@example.org", False))
        st = s.sim_status_doc(f, s.merge_declarations([f], dict(host, **{"drone-a": dict(host["drone-a"], set_at=300.0)}), svc))
        self.assertEqual((st["declared"], st["source"]), ("real", "host"))

    def test_service_ignored_without_a_profile_link(self):
        f = self.feed()
        merged = s.merge_declarations([f], {}, {"drone-a": {"mode": "simulated", "set_at": 1.0}})
        self.assertEqual(merged, {})
        self.assertEqual(s.sim_status_doc(f, merged)["basis"], "profile")

    def test_reading_the_url(self):
        import http.server, threading
        body = json.dumps({"schema": "goat.sim-status/v1", "feeds": {"feed-17": {"mode": "unknown", "set_at": 5.0}}}).encode()

        class H(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200 if self.path == "/api/sim-status" else 500)
                self.end_headers()
                self.wfile.write(body if self.path == "/api/sim-status" else b"{")

            def log_message(self, *a):
                pass
        srv = http.server.HTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        base = "http://127.0.0.1:%d" % srv.server_address[1]
        try:
            feeds, info = s.read_sim_status_url(base + "/api/sim-status")
            self.assertEqual((feeds["feed-17"]["mode"], info["loaded"]), ("unknown", True))
            feeds, info = s.read_sim_status_url(base + "/broken")
            self.assertEqual((feeds, info["loaded"]), ({}, False))
            self.assertNotIn("127.0.0.1", info["error"] or "")
        finally:
            srv.shutdown()
        self.assertEqual(s.read_sim_status_url("none"), ({}, {"source": None, "loaded": False, "error": None}))
        feeds, info = s.read_sim_status_url("http://127.0.0.1:9/api/sim-status", timeout=0.5)
        self.assertFalse(info["loaded"])


class EndToEnd(unittest.TestCase):
    def test_status_document_carries_the_declaration(self):
        d = tempfile.mkdtemp()
        decl = os.path.join(d, "sim-status.json")
        sim_status.save(decl, sim_status.apply(sim_status.load(decl), "counterfire", "unknown", "ops desk"))
        status = os.path.join(d, "status.json")
        rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        rx.bind(("127.0.0.1", 0))
        r = subprocess.run([sys.executable, os.path.join(os.path.dirname(HERE), "simple_bn_sender.py"),
                            "--dest", "127.0.0.1:%d" % rx.getsockname()[1], "--label", "t", "--feeds", "counterfire",
                            "--status", status, "--sim-status", decl, "--duration", "6", "--seed", "1"],
                           capture_output=True, text=True, timeout=30)
        rx.close()
        self.assertEqual(r.returncode, 0, r.stderr)
        with open(status) as f:
            doc = json.load(f)
        self.assertTrue(doc["sim_status_file"]["loaded"])
        st = doc["feeds"][0]["sim_status"]
        self.assertEqual((st["declared"], st["basis"], st["sim_bit_sent"]), ("unknown", "operator", True))


if __name__ == "__main__":
    unittest.main()
