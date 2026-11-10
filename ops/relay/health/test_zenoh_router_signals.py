"""Unit tests for zenoh-router-signals.py (stdlib only; run with unittest discover)."""
import importlib.util
import io
import ipaddress
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("signals", HERE / "zenoh-router-signals.py")
signals = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(signals)

CLOSE = ("[2m2026-09-17T15:59:12Z[0m ERROR rx-2750 zenoh_transport::unicast::universal::tx: "
         "Unable to push non droppable network message to 640ff4cb623a3aea8b6b09404a013963. Closing transport!\n")
ORPHAN = "2026-09-17T16:05:06Z WARN zenoh::api::session: Received ReplyData for unknown Query: 18410\n"
NOISE = "2026-09-17T16:05:06Z WARN zenoh_plugin_rest: Encoding is JSON but data is not JSON\n"


def jline(text):
    return json.dumps({"log": text, "stream": "stdout", "time": "2026-09-17T16:00:00Z"}) + "\n"


class TailTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.log = os.path.join(self.dir.name, "router-json.log")
        with open(self.log, "w") as f:
            f.write(jline(CLOSE) * 50)  # history before the first run

    def tearDown(self):
        self.dir.cleanup()

    def append(self, text):
        with open(self.log, "a") as f:
            f.write(text)

    def test_first_run_counts_nothing(self):
        state = {}
        lines, ok = signals.tail_log(self.log, state)
        self.assertTrue(ok)
        self.assertEqual(lines, [])
        self.assertEqual(state["offset"], os.path.getsize(self.log))

    def test_counts_only_appended_lines(self):
        state = {}
        signals.tail_log(self.log, state)
        self.append(jline(CLOSE) * 3 + jline(ORPHAN) * 2 + jline(NOISE))
        closes, orphans, zids = signals.scan(signals.tail_log(self.log, state)[0])
        self.assertEqual((closes, orphans), (3, 2))
        self.assertEqual(zids, {"640ff4cb623a3aea8b6b09404a013963": 3})
        self.assertEqual(signals.tail_log(self.log, state)[0], [])

    def test_partial_trailing_line_waits_for_next_run(self):
        state = {}
        signals.tail_log(self.log, state)
        whole = jline(CLOSE)
        self.append(whole[:20])
        self.assertEqual(signals.tail_log(self.log, state)[0], [])
        self.append(whole[20:])
        self.assertEqual(signals.scan(signals.tail_log(self.log, state)[0])[0], 1)

    def test_rotation_restarts_at_new_file_head(self):
        state = {}
        signals.tail_log(self.log, state)
        os.replace(self.log, self.log + ".1")
        with open(self.log, "w") as f:
            f.write(jline(CLOSE) * 2)
        state["inode"] = -1  # Windows may reuse inode numbers; force the rotation branch
        self.assertEqual(signals.scan(signals.tail_log(self.log, state)[0])[0], 2)

    def test_truncation_restarts_at_head(self):
        state = {}
        signals.tail_log(self.log, state)
        with open(self.log, "w") as f:
            f.write(jline(ORPHAN))
        self.assertEqual(signals.scan(signals.tail_log(self.log, state)[0])[1], 1)

    def test_missing_log_is_unmeasured(self):
        lines, ok = signals.tail_log(os.path.join(self.dir.name, "absent.log"), {})
        self.assertFalse(ok)


class CensusTests(unittest.TestCase):
    ADMIN = {"sessions": [
        {"peer": "aaa", "whatami": "client", "links": [{"dst": "tls/100.127.0.41:5000"}]},
        {"peer": "bbb", "whatami": "router", "links": [{"dst": "tls/100.127.0.107:58674"}]},
        {"peer": "ccc", "whatami": "router", "links": [{"dst": "tls/100.127.0.106:44936"}]},
        {"peer": "ddd", "whatami": "peer", "links": [{"dst": "tls/172.19.0.14:57122"}]},
    ]}
    NETS = [ipaddress.ip_network("172.19.0.0/16")]

    def test_counts_modes_and_undeclared_remote_routers(self):
        by_kind, remote = signals.session_census(self.ADMIN, set(), self.NETS)
        self.assertEqual(by_kind, {"client": 1, "router": 2, "peer": 1})
        self.assertEqual(sorted(remote), ["bbb", "ccc"])

    def test_allow_list_excludes_declared_routers(self):
        _, remote = signals.session_census(self.ADMIN, {"bbb"}, self.NETS)
        self.assertEqual(remote, ["ccc"])

    def test_allow_list_accepts_addresses_because_zids_are_not_stable(self):
        # bbb is 100.127.0.107, ccc is 100.127.0.106. Declaring bbb BY ADDRESS
        # must exclude it even though its ZID is not listed — the whole point,
        # since a third-party router's ZID changes when it restarts.
        zids, nets = signals.parse_allow("100.127.0.107")
        self.assertEqual((zids, [str(n) for n in nets]), (set(), ["100.127.0.107/32"]))
        _, remote = signals.session_census(self.ADMIN, zids, self.NETS, nets)
        self.assertEqual(remote, ["ccc"])

        # A CIDR covers a range; mixing a ZID and an address works.
        zids2, nets2 = signals.parse_allow("bbb, 100.127.0.105/24")
        self.assertEqual(zids2, {"bbb"})
        _, remote2 = signals.session_census(self.ADMIN, zids2, self.NETS, nets2)
        self.assertEqual(remote2, [])

    def test_empty_allow_list_parses_to_nothing(self):
        self.assertEqual(signals.parse_allow(""), (set(), []))
        self.assertEqual(signals.parse_allow(None), (set(), []))

    def test_list_shaped_admin_reply(self):
        by_kind, _ = signals.session_census([{"key": "@/x/router", "value": self.ADMIN}], set(), self.NETS)
        self.assertEqual(by_kind["router"], 2)

    def test_no_sessions_is_unmeasured(self):
        self.assertEqual(signals.session_census({}, set(), self.NETS), (None, None))


class LinklessTests(unittest.TestCase):
    # #1922: entries whose teardown never finished keep a table slot and no link.
    ADMIN = {"sessions": [
        {"peer": "aaa", "whatami": "client", "links": [{"dst": "tls/100.127.0.41:5000"}]},
        {"peer": "bbb", "whatami": "client", "links": []},
        {"peer": "ccc", "whatami": "client"},
        {"peer": "ddd", "whatami": "router", "links": [{"dst": "tls/100.127.0.107:58674"}]},
    ]}

    def test_counts_sessions_with_no_link(self):
        self.assertEqual(signals.linkless_sessions(self.ADMIN), 2)

    def test_list_shaped_admin_reply(self):
        self.assertEqual(signals.linkless_sessions([{"key": "@/x/router", "value": self.ADMIN}]), 2)

    def test_all_live_is_zero_not_unmeasured(self):
        self.assertEqual(signals.linkless_sessions({"sessions": [self.ADMIN["sessions"][0]]}), 0)

    def test_no_sessions_is_unmeasured(self):
        self.assertIsNone(signals.linkless_sessions({}))
        self.assertIsNone(signals.linkless_sessions(None))

    def test_main_reports_linkless_and_null_when_unmeasured(self):
        def run(stdin_text, d):
            buf, old = io.StringIO(), sys.stdin
            sys.stdin = io.StringIO(stdin_text)
            try:
                with redirect_stdout(buf):
                    signals.main(["--state-dir", d])
            finally:
                sys.stdin = old
            return json.loads(buf.getvalue())
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(run(json.dumps(self.ADMIN), d)["linkless_sessions"], 2)
            self.assertIsNone(run("", d)["linkless_sessions"])


class WholeKeyspaceTests(unittest.TestCase):
    """The shape test the ACL cannot express."""

    def test_all_wildcard_keys_match_everything(self):
        for k in ("**", "**/**", "*/**", "*/*/**", "*", "*/*"):
            self.assertTrue(signals.matches_everything(k), k)

    def test_one_literal_chunk_is_a_filter_not_everything(self):
        for k in ("**/basemap_vendor/**", "*/geo/track/v1/*", "**/tracks/v1",
                  "abc123/**", "ops/**", "abc123/*/telemetry/**", "ops"):
            self.assertFalse(signals.matches_everything(k), k)

    def test_empty_key_is_not_everything(self):
        self.assertFalse(signals.matches_everything(""))

    def test_census_groups_whole_keyspace_keys_by_session(self):
        def row(declared, clients):
            return {"key": f"@/router1/router/subscriber/{declared}",
                    "value": {"clients": clients, "peers": [], "routers": []}}
        census = [
            row("**", ["aaa", "bbb"]),
            row("*/**", ["ccc"]),
            row("**/basemap_vendor/**", ["ddd"]),          # a filter: not counted
            row("abc123/tracks/v1", ["eee"]),       # ordinary: not counted
        ]
        found = signals.fabric_wide_subscribers(census)
        self.assertEqual(sorted(found), ["aaa", "bbb", "ccc"])
        self.assertEqual(found["ccc"], ["*/**"])

    def test_census_without_subscriber_rows_is_unmeasured(self):
        self.assertIsNone(signals.fabric_wide_subscribers([{"key": "@/router1/router/linkstate"}]))

    def test_census_with_rows_but_no_whole_keyspace_is_empty_not_null(self):
        census = [{"key": "@/r/router/subscriber/abc/def", "value": {"clients": ["a"]}}]
        self.assertEqual(signals.fabric_wide_subscribers(census), {})


class MainTests(unittest.TestCase):
    def test_counters_accumulate_across_runs(self):
        with tempfile.TemporaryDirectory() as d:
            log = os.path.join(d, "r.log")
            Path(log).write_text("")
            state = os.path.join(d, "state")

            def run():
                buf = io.StringIO()
                old = sys.stdin
                sys.stdin = io.StringIO(json.dumps(CensusTests.ADMIN))
                try:
                    with redirect_stdout(buf):
                        signals.main(["--log-path", log, "--state-dir", state,
                                      "--local-net", "172.19.0.0/16"])
                finally:
                    sys.stdin = old
                return json.loads(buf.getvalue())

            first = run()
            self.assertEqual(first["slow_consumer_closes_total"], 0)
            with open(log, "a") as f:
                f.write(jline(CLOSE) * 4)
            self.assertEqual(run()["slow_consumer_closes_total"], 4)
            with open(log, "a") as f:
                f.write(jline(CLOSE))
            out = run()
            self.assertEqual(out["slow_consumer_closes_total"], 5)
            self.assertIn("640ff4cb623a3aea8b6b09404a013963", out["recent_close_zids"])
            self.assertEqual(out["remote_router_sessions"], 2)
            self.assertTrue(out["log_tail_ok"])

    def test_unreadable_log_reports_null_not_zero(self):
        with tempfile.TemporaryDirectory() as d:
            buf = io.StringIO()
            old = sys.stdin
            sys.stdin = io.StringIO("")
            try:
                with redirect_stdout(buf):
                    signals.main(["--log-path", os.path.join(d, "nope"), "--state-dir", d])
            finally:
                sys.stdin = old
            out = json.loads(buf.getvalue())
            self.assertFalse(out["log_tail_ok"])
            self.assertIsNone(out["slow_consumer_closes_total"])
            self.assertIsNone(out["remote_router_sessions"])


if __name__ == "__main__":
    unittest.main()
