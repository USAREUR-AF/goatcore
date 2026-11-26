"""Phase 14 bound by probe id: the controller fires, the edge returns only its lines.

A peering driven from two machines (docs/design/federation-two-drivers.md) runs
phase 14 in two parts. With FED_ACCEPTANCE_TRACE=deferred, acceptance.sh fires every
probe, reads no edge log, writes <work>/acceptance/ids.json and leaves the rows that
need the edge's ACL trace pending. edge_trace_filter.py keeps only the edge log's
lines that carry one of those ids, in the window widened by 5 minutes each side.
With FED_EDGE_TRACE_FILE, acceptance.sh decides the pending rows from the recorded
evidence and those lines, fires nothing, and refuses a file for other ids or another
window. Everything runs against the one-process fabric and a stand-in edge log.

Run: python3 -m pytest ops/federation/standup/test_acceptance_trace_by_id.py
"""
import datetime
import http.server
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "acceptance.sh"
FILTER = HERE / "edge_trace_filter.py"

_spec = importlib.util.spec_from_file_location("acceptance_fakefabric", HERE / "acceptance_fakefabric.py")
fakefabric = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fakefabric)

SCRUB = [k for k in os.environ if k.startswith("FED_")]
# The rows that read our edge's ACL trace.
TRACE_ROWS = ("refusal/control-allowed", "refusal/out-of-lane-denied", "refusal/spoof-denied",
              "refusal/forged-name", "refusal/impersonation")
# The fake edge stamps every line with this time; the stand-in log moves it to now.
FAKE_STAMP = "2026-11-09T00:00:00Z"


def stamp(t):
    return t.strftime("%Y-%m-%dT%H:%M:%SZ")


def run_filter(args, stdin=""):
    p = subprocess.run([sys.executable, str(FILTER)] + args, input=stdin, capture_output=True, text=True,
                       timeout=60)
    return p.returncode, p.stdout, p.stderr


class Filter(unittest.TestCase):
    """edge_trace_filter.py on a stand-in edge log."""

    def setUp(self):
        self.start = datetime.datetime(2026, 11, 15, 12, 0, 0, tzinfo=datetime.timezone.utc)
        self.end = self.start + datetime.timedelta(minutes=2)
        self.window = "%s/%s" % (stamp(self.start), stamp(self.end))
        self.ids = ["acc-20261115T120000Z-4242-0a0b0c0d", "f" + "1" * 31]

    def at(self, minutes):
        return stamp(self.start + datetime.timedelta(minutes=minutes))

    def test_only_lines_carrying_an_id_inside_the_widened_window_leave_the_edge(self):
        log = "\n".join([
            "%s INFO access_control: %s is authorized to Put (ingress) on from/xab/acceptance-probe/%s"
            % (self.at(1), "a" * 32, self.ids[0]),
            "%s INFO access_control: ACL: peer %s did not match any configured ACL subject" % (self.at(-4), self.ids[1]),
            "%s INFO access_control: %s is authorized to Put (ingress) on from/xab/telemetry/x" % (self.at(1), "c" * 32),
            "%s INFO access_control: a line of another run acc-20261115T110000Z-1-ffffffff" % self.at(1),
            "%s INFO access_control: %s too early" % (self.at(-6), self.ids[0]),
            "%s INFO access_control: %s too late" % (self.at(8), self.ids[0]),
            "%s INFO access_control: %s at the edge of the widened window" % (self.at(7), self.ids[1]),
            "no timestamp at all, but it carries %s" % self.ids[0],
            "\x1b[2m%s\x1b[0m \x1b[32m INFO\x1b[0m coloured %s" % (self.at(0), self.ids[0]),
            "2026-11-15T12:01:00.123456789Z 2026-11-15T12:01:00.123Z INFO docker -t prefix %s" % self.ids[1],
            "2026-11-15T13:01:00+01:00 INFO an offset zone %s" % self.ids[0],
        ]) + "\n"
        rc, out, err = run_filter(["--ids", ",".join(self.ids), "--window", self.window], log)
        self.assertEqual(rc, 0, err)
        doc = json.loads(out)
        self.assertEqual(doc["schema"], "goatbuilder.phase14-edge/v1")
        self.assertEqual(doc["ids"], self.ids)
        self.assertEqual(doc["window"], {"start": stamp(self.start), "end": stamp(self.end)})
        kept = doc["lines"]
        self.assertEqual(len(kept), 6, kept)
        for line in kept:
            self.assertTrue(any(i in line for i in self.ids), line)
        joined = "\n".join(kept)
        for gone in ("telemetry", "another run", "too early", "too late", "no timestamp"):
            self.assertNotIn(gone, joined)
        for there in ("acceptance-probe", "did not match", "edge of the widened window", "coloured",
                      "docker -t prefix", "an offset zone"):
            self.assertIn(there, joined)

    def test_ids_and_window_that_are_not_well_formed_are_refused(self):
        for args in (["--ids", "", "--window", self.window],
                     ["--ids", "short", "--window", self.window],
                     ["--ids", "a" * 32 + ",x y z" + "0" * 8, "--window", self.window],
                     ["--ids", ",".join([self.ids[0], self.ids[0]]), "--window", self.window],
                     ["--ids", self.ids[0], "--window", stamp(self.start)],
                     ["--ids", self.ids[0], "--window", "%s/%s" % (stamp(self.end), stamp(self.start))],
                     ["--ids", self.ids[0], "--window", "2026-11-15T12:00:00/2026-11-15T12:02:00"]):
            rc, out, err = run_filter(args, "")
            self.assertEqual((rc, out), (2, ""), (args, err))
            self.assertIn("edge trace filter:", err)

    def test_the_ids_file_and_a_returned_file_are_checked_against_each_other(self):
        d = tempfile.mkdtemp(prefix="trace-filter-")
        self.addCleanup(shutil.rmtree, d, True)
        ids_json = os.path.join(d, "acceptance", "ids.json")
        rc, _, err = run_filter(["--write-ids", ids_json, "--ids", ",".join(self.ids), "--window", self.window])
        self.assertEqual(rc, 0, err)
        with open(ids_json) as f:
            self.assertEqual(json.load(f), {"schema": "goatbuilder.acceptance-ids/v1", "ids": self.ids,
                                            "window": {"start": stamp(self.start), "end": stamp(self.end)}})
        line = "%s INFO %s is authorized" % (self.at(1), self.ids[0])
        rc, out, _ = run_filter(["--ids", ",".join(self.ids), "--window", self.window], line + "\n")
        good = json.loads(out)

        def verify(doc):
            p = os.path.join(d, "edge.json")
            with open(p, "w") as f:
                json.dump(doc, f)
            return run_filter(["--verify", p, "--against", ids_json])

        rc, out, err = verify(good)
        self.assertEqual((rc, out), (0, line + "\n"), err)
        # The same ids in another order are the same ids.
        rc, _, err = verify(dict(good, ids=list(reversed(self.ids))))
        self.assertEqual(rc, 0, err)
        for bad, why in ((dict(good, ids=self.ids[:1]), "other ids"),
                         (dict(good, ids=self.ids + ["e" * 32]), "other ids"),
                         (dict(good, window={"start": stamp(self.start), "end": self.at(3)}), "another window"),
                         (dict(good, lines=good["lines"] + ["%s INFO unrelated" % self.at(1)]), "none of this run's ids"),
                         (dict(good, lines=["%s INFO %s" % (self.at(30), self.ids[0])]), "outside the window"),
                         (dict(good, schema="something/v1"), "is not goatbuilder.phase14-edge/v1")):
            rc, out, err = verify(bad)
            self.assertEqual((rc, out), (2, ""), (why, err))
            self.assertIn(why, err)


class CountingServer:
    """A REST surface that only counts what reaches it: in trace-file mode nothing may."""

    def __init__(self):
        self.hits = []
        owner = self

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def _any(self):
                owner.hits.append((self.command, self.path))
                self.send_response(500)
                self.end_headers()

            do_GET = do_PUT = do_POST = do_DELETE = _any

        self.srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.url = "http://127.0.0.1:%d" % self.srv.server_address[1]

    def stop(self):
        self.srv.shutdown()
        self.srv.server_close()


@unittest.skipUnless(shutil.which("curl") and shutil.which("openssl"), "needs curl and openssl")
class TraceById(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="acceptance-trace-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.ag = fakefabric.agreement()
        self.fabric = fakefabric.Fabric(self.ag, "xaa", self.tmp)
        # Every edge line carries FAKE_STAMP; the stand-in log moves it to now.
        self.fabric.faults.trace_stamp = FAKE_STAMP
        self.addCleanup(self.fabric.stop)
        self.agreement = os.path.join(self.tmp, "agreement.json")
        with open(self.agreement, "w") as f:
            json.dump(self.ag, f)
        self.ids_json = os.path.join(self.tmp, "acceptance", "ids.json")
        self.read_marker = os.path.join(self.tmp, "edge-log-was-read")

    def env(self, extra=None, fabric=True):
        env = {k: v for k, v in os.environ.items() if k not in SCRUB}
        if fabric:
            env.update(self.fabric.start())
        env.update({"FED_ACCEPTANCE_WAIT": "0.3", "FED_ACCEPTANCE_GAP": "0", "FED_ACCEPTANCE_RT_TIMEOUT": "3"})
        env.update(extra or {})
        return env

    def run_acceptance(self, env):
        p = subprocess.run(["bash", str(SCRIPT), self.agreement, "--as", "xaa"], env=env,
                           capture_output=True, text=True, timeout=240)
        return p.returncode, p.stdout + p.stderr

    def bundle(self):
        with open(os.path.join(self.tmp, "evidence", "acceptance", "result.json")) as f:
            return json.load(f)

    def checks(self):
        return {c["name"]: c for c in self.bundle()["checks"]}

    def deferred(self, extra=None):
        """The controller's half: the fabric's probes fire, the edge log is never read."""
        env = self.env(dict({"FED_ACCEPTANCE_TRACE": "deferred",
                             "FED_EDGE_LOG_OURS": "touch '%s'; cat '%s'" % (self.read_marker, self.fabric.trace_path)},
                            **(extra or {})))
        return self.run_acceptance(env)

    def edge_log(self):
        """The stand-in edge router's log: the fake edge's lines, stamped now, among
        lines of other sessions and of earlier runs that must never leave the edge."""
        now = stamp(datetime.datetime.now(datetime.timezone.utc))
        with open(self.fabric.trace_path) as f:
            ours = f.read().replace(FAKE_STAMP, now)
        noise = ("%s INFO access_control: %s on Interface(eth0)+CertCommonName(peering-xab)+Transport(Tls)+%s "
                 "is authorized to Put (ingress) on from/xab/telemetry/position\n" % (now, "c" * 32, "c" * 32)
                 + "%s INFO access_control: %s is unauthorized to Put (ingress) on ops/fabric/acceptance-probe-"
                 "acc-20261101T000000Z-1-00000000\n" % (now, "c" * 32)
                 + "%s INFO zenoh::net::runtime: new session with %s\n" % (now, "c" * 32))
        return noise + ours + noise

    def edge_half(self, log=None):
        """The edge driver's half: the agreed filter over the edge log, as a file."""
        with open(self.ids_json) as f:
            ids = json.load(f)
        rc, out, err = run_filter(["--ids", ",".join(ids["ids"]),
                                   "--window", "%s/%s" % (ids["window"]["start"], ids["window"]["end"])],
                                  self.edge_log() if log is None else log)
        self.assertEqual(rc, 0, err)
        path = os.path.join(self.tmp, "handoff-14-edge.json")
        with open(path, "w") as f:
            f.write(out)
        return path, json.loads(out)

    def decide(self, trace_file):
        """The controller again, with the edge's file: every surface counts requests."""
        counter = CountingServer()
        self.addCleanup(counter.stop)
        env = self.env({"FED_EDGE_TRACE_FILE": trace_file,
                        "FED_OURS_REST": counter.url, "FED_THEIRS_REST": counter.url,
                        "FED_PROBE_THEIRS_REST": counter.url, "FED_PROBE_FORGED_REST": counter.url,
                        "FED_PROBE_IMPERSONATE_REST": counter.url,
                        "FED_PROBE_FORGED_ZID": "0" * 32, "FED_PROBE_IMPERSONATE_ZID": "0" * 32,
                        "FED_EDGE_LOG_OURS": "touch '%s'" % self.read_marker,
                        "FED_SPIRE_OURS": "touch '%s'; false" % self.read_marker,
                        "FED_SPIRE_THEIRS": "touch '%s'; false" % self.read_marker}, fabric=False)
        rc, out = self.run_acceptance(env)
        return rc, out, counter.hits

    # --- the controller's half ------------------------------------------------
    def test_deferred_fires_every_probe_reads_no_edge_log_and_leaves_the_trace_rows_pending(self):
        rc, out = self.deferred()
        # A pending row is not a pass: the controller's half alone is INCOMPLETE.
        self.assertEqual(rc, 3, out)
        self.assertEqual(self.bundle()["verdict"], "INCOMPLETE", out)
        self.assertEqual(self.bundle()["counts"]["PENDING"], len(TRACE_ROWS), out)
        self.assertFalse(os.path.exists(self.read_marker), "the edge log was read: " + out)
        checks = self.checks()
        for name in TRACE_ROWS:
            c = checks[name]
            self.assertEqual((c["verdict"], c["required"]), ("PENDING", True), (name, out))
            self.assertEqual(c["observations"].get("pending"), "edge-trace", (name, out))
            self.assertNotIn("trace", c["observations"], name)
        for name, c in checks.items():
            if name not in TRACE_ROWS:
                self.assertEqual(c["verdict"], "PASS", (name, out))
        # The probes did reach our edge: its log holds this run's lines.
        with open(self.fabric.trace_path) as f:
            self.assertIn("acceptance-probe", f.read())
        with open(self.ids_json) as f:
            ids = json.load(f)
        self.assertEqual(ids["schema"], "goatbuilder.acceptance-ids/v1")
        run_id = self.bundle()["run"]["id"]
        self.assertEqual(ids["ids"][0], run_id)
        self.assertEqual(set(ids["ids"][1:]), {self.fabric.zids["forged"], self.fabric.zids["impersonate"]})
        start = datetime.datetime.strptime(ids["window"]["start"], "%Y-%m-%dT%H:%M:%SZ")
        end = datetime.datetime.strptime(ids["window"]["end"], "%Y-%m-%dT%H:%M:%SZ")
        self.assertLessEqual(start, end)

    def test_the_agreed_probes_own_id_is_one_of_the_ids_when_it_is_known(self):
        rc, out = self.deferred({"FED_PROBE_THEIRS_ZID": self.fabric.agreed_zid})
        self.assertEqual(rc, 3, out)
        with open(self.ids_json) as f:
            self.assertIn(self.fabric.agreed_zid, json.load(f)["ids"])

    def test_without_either_variable_no_ids_file_is_written(self):
        rc, out = self.run_acceptance(self.env())
        self.assertEqual(rc, 0, out)
        self.assertFalse(os.path.exists(self.ids_json))
        self.assertEqual({c["verdict"] for c in self.checks().values()}, {"PASS"})

    # --- the whole round: deferred, the edge's filter, the decision -------------
    def test_the_edges_lines_matched_by_id_decide_the_pending_rows_and_nothing_fires(self):
        rc, out = self.deferred({"FED_PROBE_THEIRS_ZID": self.fabric.agreed_zid})
        self.assertEqual(rc, 3, out)
        deferred = self.checks()
        path, doc = self.edge_half()
        # Only this run's lines left the edge.
        self.assertTrue(doc["lines"])
        self.assertFalse([x for x in doc["lines"] if "telemetry" in x or "c" * 32 in x], doc["lines"])
        rc, out, hits = self.decide(path)
        self.assertEqual(rc, 0, out)
        self.assertEqual(hits, [], "trace-file mode reached a surface: " + out)
        self.assertFalse(os.path.exists(self.read_marker), "trace-file mode ran an edge or SPIRE command: " + out)
        checks = self.checks()
        self.assertEqual(set(checks), set(deferred))
        self.assertEqual({c["verdict"] for c in checks.values()}, {"PASS"}, out)
        for name in TRACE_ROWS:
            self.assertTrue(checks[name]["required"], name)
            self.assertNotIn("pending", checks[name]["observations"], name)
        for name in ("refusal/control-allowed", "refusal/out-of-lane-denied", "refusal/spoof-denied"):
            self.assertEqual(checks[name]["observations"]["session"], self.fabric.agreed_zid, name)
        self.assertIn("is unauthorized to Put (ingress) on ops/fabric/",
                      checks["refusal/out-of-lane-denied"]["observations"]["trace"])
        # Every row that did not need the trace is the controller's own, as recorded.
        for name, c in deferred.items():
            if name not in TRACE_ROWS:
                self.assertEqual(checks[name], c, name)
        self.assertTrue(self.bundle()["inputs"]["edge_trace_file"])

    def test_a_denial_the_edge_recorded_for_another_session_still_fails(self):
        self.fabric.faults.deny_from_other_session = True
        rc, out = self.deferred()
        self.assertEqual(rc, 3, out)
        path, _ = self.edge_half()
        rc, out, hits = self.decide(path)
        self.assertEqual((rc, hits), (1, []), out)
        v = {n: c["verdict"] for n, c in self.checks().items()}
        self.assertEqual(v["refusal/control-allowed"], "PASS", out)
        self.assertEqual(v["refusal/out-of-lane-denied"], "FAIL", out)
        self.assertIn("another session", out)

    def test_lines_from_before_the_window_do_not_decide_anything(self):
        rc, out = self.deferred()
        self.assertEqual(rc, 3, out)
        # The edge's own lines keep their old time: the filter keeps none of them.
        with open(self.fabric.trace_path) as f:
            stale = f.read()
        path, doc = self.edge_half(stale)
        self.assertEqual(doc["lines"], [])
        rc, out, hits = self.decide(path)
        self.assertEqual((rc, hits), (1, []), out)
        v = {n: c["verdict"] for n, c in self.checks().items()}
        self.assertEqual(v["refusal/control-allowed"], "FAIL", out)
        self.assertEqual(v["refusal/out-of-lane-denied"], "SKIP", out)
        self.assertEqual(v["refusal/forged-name"], "BLOCKED", out)

    def test_a_file_for_other_ids_or_another_window_is_refused(self):
        rc, out = self.deferred()
        self.assertEqual(rc, 3, out)
        path, doc = self.edge_half()
        before = self.bundle()
        for bad in (dict(doc, ids=doc["ids"][:1]),
                    dict(doc, window={"start": doc["window"]["start"], "end": "2027-01-01T00:00:00Z"}),
                    dict(doc, lines=doc["lines"] + ["%s INFO unrelated" % doc["window"]["start"]])):
            with open(path, "w") as f:
                json.dump(bad, f)
            rc, out, hits = self.decide(path)
            self.assertEqual((rc, hits), (2, []), out)
            self.assertIn("acceptance: the edge's trace file", out)
            self.assertEqual(self.bundle(), before, "a refused file changed the evidence")

    def test_a_trace_file_with_no_ids_file_is_refused(self):
        path = os.path.join(self.tmp, "edge.json")
        with open(path, "w") as f:
            json.dump({"schema": "goatbuilder.phase14-edge/v1", "ids": ["a" * 32],
                       "window": {"start": "2026-11-15T00:00:00Z", "end": "2026-11-15T00:01:00Z"}, "lines": []}, f)
        rc, out, hits = self.decide(path)
        self.assertEqual((rc, hits), (2, []), out)
        self.assertIn("ids.json", out)

    def test_both_variables_or_a_derived_session_are_refused(self):
        rc, out = self.run_acceptance(self.env({"FED_ACCEPTANCE_TRACE": "deferred",
                                                "FED_EDGE_TRACE_FILE": os.path.join(self.tmp, "x.json")}))
        self.assertEqual(rc, 2, out)
        rc, out = self.run_acceptance(self.env({"FED_ACCEPTANCE_TRACE": "later"}))
        self.assertEqual(rc, 2, out)
        self.assertIn("FED_ACCEPTANCE_TRACE", out)
        rc, out = self.run_acceptance(self.env({"FED_ACCEPTANCE_TRACE": "deferred", "FED_ACCEPTANCE_ATTEMPT": "1",
                                                "FED_ACCEPTANCE_SIGNER": "true", "FED_ACCEPTANCE_VERIFIER": "true"}))
        self.assertEqual(rc, 2, out)
        self.assertIn("derived session", out)

    # --- the split run is for a drill's probes (B10), not a session's own (B2) ------
    def test_a_split_run_with_our_own_launcher_or_in_a_session_is_refused_before_anything_fires(self):
        calls = os.path.join(self.tmp, "launcher-calls")
        launcher = os.path.join(self.tmp, "launcher-ours")
        with open(launcher, "w") as f:
            f.write("#!/bin/sh\necho \"$@\" >> '%s'\nexit 1\n" % calls)
        os.chmod(launcher, 0o700)  # the owner runs it; nobody else needs to read it
        trace_file = os.path.join(self.tmp, "x.json")
        for extra, named in (
                ({"FED_ACCEPTANCE_TRACE": "deferred", "FED_PROBE_LAUNCHER_OURS": launcher}, "FED_PROBE_LAUNCHER_OURS"),
                ({"FED_ACCEPTANCE_TRACE": "deferred", "FED_ACCEPTANCE_SESSION": "s"}, "FED_ACCEPTANCE_SESSION"),
                ({"FED_EDGE_TRACE_FILE": trace_file, "FED_PROBE_LAUNCHER_OURS": launcher}, "FED_PROBE_LAUNCHER_OURS"),
                ({"FED_EDGE_TRACE_FILE": trace_file, "FED_ACCEPTANCE_SESSION": "s"}, "FED_ACCEPTANCE_SESSION")):
            rc, out = self.run_acceptance(self.env(extra))
            self.assertEqual(rc, 2, (extra, out))
            self.assertIn(named, out, extra)
            self.assertIn("B10", out, extra)
            self.assertFalse(os.path.exists(calls), (extra, "the launcher was called"))
            self.assertFalse(os.path.exists(self.ids_json), (extra, "a refused run wrote ids.json"))

    # --- "could not be read" is the read failing, never an empty answer --------------
    def test_an_edge_log_command_that_fails_blocks_the_rows_as_unread(self):
        rc, out = self.run_acceptance(self.env({"FED_EDGE_LOG_OURS": "cat '%s'; false" % self.fabric.trace_path}))
        v = self.checks()
        for name in ("refusal/forged-name", "refusal/impersonation"):
            self.assertEqual(v[name]["verdict"], "BLOCKED", out)
            self.assertIn("could not be read", v[name]["reason"], out)

    def test_an_edge_log_that_reads_but_names_no_session_is_not_unread(self):
        rc, out = self.run_acceptance(self.env({"FED_EDGE_LOG_OURS": "true"}))
        v = self.checks()
        for name in ("refusal/forged-name", "refusal/impersonation"):
            self.assertEqual(v[name]["verdict"], "BLOCKED", out)
            self.assertIn("names that session nowhere", v[name]["reason"], out)
            self.assertNotIn("could not be read", v[name]["reason"], out)

    def test_a_file_with_no_line_for_a_probe_says_nowhere_not_unread(self):
        rc, out = self.deferred()
        self.assertEqual(rc, 3, out)
        with open(self.fabric.trace_path) as f:
            stale = f.read()
        path, _ = self.edge_half(stale)
        rc, out, hits = self.decide(path)
        v = self.checks()
        self.assertIn("names that session nowhere", v["refusal/forged-name"]["reason"], out)
        self.assertNotIn("could not be read", v["refusal/forged-name"]["reason"], out)


if __name__ == "__main__":
    unittest.main()
