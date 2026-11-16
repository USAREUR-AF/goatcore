#!/usr/bin/env python3
"""Loopback HTTP checks for independent current-health and history processing."""
import http.client
import importlib.util
import io
import json
import os
import sqlite3
import sys
import tempfile
import threading
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).resolve().parent


class IngestTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        env = {"RELAY_METRICS_BEARER_TOKEN": "synthetic-local-fixture-token",
               "RELAY_METRICS_ALLOWED_NAMES": "relay-a", "HOST_NET_ALLOWED_NAMES": "host-a",
               "ZENOH_HEALTH_ALLOWED_NAMES": "router-a", "RELAY_METRICS_INGEST_DIR": str(self.root)}
        spec = importlib.util.spec_from_file_location("health_ingest_fixture", HERE / "relay-metrics-ingest.py")
        self.m = importlib.util.module_from_spec(spec)
        with patch.dict(os.environ, env):
            spec.loader.exec_module(self.m)
        self.m.init_host_series_db()
        self.m.init_zenoh_series_db()
        self.logs = self.enterContext(redirect_stderr(io.StringIO()))
        self.clock = self.enterContext(patch.object(self.m.time, "time", return_value=1000))
        self.errors = []
        errors = self.errors

        class Server(self.m.ThreadingHTTPServer):
            def handle_error(self, request, client_address):
                errors.append(sys.exc_info()[1])

        self.server = Server(("127.0.0.1", 0), self.m.Handler)
        self.thread = threading.Thread(target=lambda: self.server.serve_forever(poll_interval=0.01))
        self.thread.start()
        self.addCleanup(self.stop_server)

    def stop_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(3)
        self.assertFalse(self.thread.is_alive())

    def route(self, kind):
        return "/host-net-ingest/host-a" if kind == "host" else "/zenoh-health-ingest/router-a"

    def payload(self, kind, state):
        if kind == "host":
            return {"host_net": {"state": state, "wan": {"loss_pct": 0}}}
        return {"zenoh": {"state": state, "container": {
            "health": "unhealthy" if state == "new" else "healthy", "uptime_s": 100}}}

    def post(self, kind, state="new", *, auth=None, body=None, route=None):
        client = http.client.HTTPConnection("127.0.0.1", self.server.server_port, timeout=3)
        try:
            client.request("POST", route or self.route(kind),
                           json.dumps(self.payload(kind, state) if body is None else body),
                           {"Authorization": auth if auth is not None else "Bearer " + self.m.TOKEN,
                            "Content-Type": "application/json"})
            response = client.getresponse()
            response.read()
            return response.status
        finally:
            client.close()

    def aggregate(self, kind):
        return Path(self.m.HOSTS_JSON if kind == "host" else self.m.ZENOH_HEALTH_JSON)

    def current(self, kind):
        doc = json.loads(self.aggregate(kind).read_text())
        return doc["hosts"]["host-a"]["host_net"] if kind == "host" else doc["routers"]["router-a"]["zenoh"]

    def timer(self, kind):
        return self.m._last_host_export if kind == "host" else self.m._last_zenoh_export

    def history(self, kind):
        return Path(self.m.HOST_SERIES_JSON if kind == "host" else self.m.ZENOH_SERIES_JSON)

    def name(self, kind, operation):
        return {"record": f"record_{kind}_series", "export": f"export_{kind}_series",
                "aggregate": "export_hosts_aggregate" if kind == "host" else "export_zenoh_aggregate"}[operation]

    def assert_wedged(self):
        doc = json.loads(Path(self.m.ZENOH_WEDGES_JSON).read_text())
        self.assertTrue(doc["routers"]["router-a"]["currently_wedged"])
        self.assertTrue(any(e["type"] == "wedged" for e in doc["events"]))

    def test_peer_names_lifts_the_writer_record_and_keeps_cards_bare(self):
        # Review of #977, item 4: the cloud copy carried no owner state and no
        # generation time. The writer sends them in a reserved key.
        body = {"7368612d01": {"name": "machine-box-1", "owner": {"source": "record", "exception": False}},
                "__peer_names_meta__": {"generated_at": 990, "owner_read": {"state": "unread", "reason": "401"}}}
        self.assertEqual(self.post("peer", body=body, route="/relay-metrics-ingest/peer-names"), 204)
        doc = json.loads((self.root / "peer-names.json").read_text())
        self.assertEqual(doc["generated_at"], 990)
        self.assertEqual(doc["owner_read"]["state"], "unread")
        self.assertEqual(list(doc["names"]), ["7368612d01"], "the reserved key is not a card")
        self.assertEqual(doc["count"], 1)

    def test_normal_ingest_updates_individual_aggregate_history_and_wedge_timeline(self):
        for kind in ("host", "zenoh"):
            with self.subTest(kind=kind):
                self.assertEqual(self.post(kind), 204)
                individual = self.root / ("host-host-a.json" if kind == "host" else "zenoh-router-a.json")
                self.assertEqual(json.loads(individual.read_text())["received_at"], 1000)
                self.assertEqual(self.current(kind)["state"], "new")
                self.assertEqual(json.loads(self.history(kind).read_text())["generated_at"], 1000)
                self.assertEqual(self.timer(kind)[0], 1000)
        self.assert_wedged()

    def test_record_failure_keeps_current_outputs_and_recovers(self):
        for kind in ("host", "zenoh"):
            with self.subTest(kind=kind):
                self.clock.return_value = 1000
                self.assertEqual(self.post(kind, "old"), 204)
                self.clock.return_value = 1031
                with patch.object(self.m, self.name(kind, "record"),
                                  side_effect=sqlite3.OperationalError("synthetic history outage")), \
                        patch.object(self.m, self.name(kind, "export")) as export:
                    self.assertEqual(self.post(kind), 204)
                self.assertEqual(self.current(kind)["state"], "new")
                self.assertEqual(self.timer(kind)[0], 1000)
                export.assert_not_called()
                self.assertIn(f"{kind} series record failed", self.logs.getvalue())
                if kind == "zenoh":
                    self.assert_wedged()
                self.clock.return_value = 1062
                self.assertEqual(self.post(kind, "recovered"), 204)
                self.assertEqual(self.current(kind)["state"], "recovered")
                self.assertEqual(self.timer(kind)[0], 1062)

    def test_history_export_failure_keeps_current_outputs_and_retries(self):
        for kind in ("host", "zenoh"):
            with self.subTest(kind=kind):
                self.clock.return_value = 1000
                self.assertEqual(self.post(kind, "old"), 204)
                self.clock.return_value = 1031
                with patch.object(self.m, self.name(kind, "export"),
                                  side_effect=OSError("synthetic history export failure")):
                    self.assertEqual(self.post(kind), 204)
                self.assertEqual(self.current(kind)["state"], "new")
                self.assertEqual(self.timer(kind)[0], 1000)
                self.clock.return_value = 1032
                self.assertEqual(self.post(kind, "retry"), 204)
                self.assertEqual(self.timer(kind)[0], 1032)
                self.assertEqual(json.loads(self.history(kind).read_text())["generated_at"], 1032)

    def test_aggregate_failure_does_not_block_history_or_wedges(self):
        for kind in ("host", "zenoh"):
            with self.subTest(kind=kind):
                self.clock.return_value = 1000
                self.assertEqual(self.post(kind, "old"), 204)
                before = self.aggregate(kind).read_bytes()
                self.clock.return_value = 1031
                with patch.object(self.m, self.name(kind, "aggregate"),
                                  side_effect=OSError("synthetic aggregate failure")):
                    self.assertEqual(self.post(kind), 204)
                self.assertEqual(self.aggregate(kind).read_bytes(), before)
                self.assertEqual(self.timer(kind)[0], 1031)
                self.assertEqual(json.loads(self.history(kind).read_text())["generated_at"], 1031)
                self.assertIn(f"{kind} aggregate failed", self.logs.getvalue())
                if kind == "zenoh":
                    self.assert_wedged()

    def test_wedge_failure_does_not_block_current_aggregate_or_history(self):
        with patch.object(self.m, "track_zenoh_wedges", side_effect=OSError("synthetic wedge failure")):
            self.assertEqual(self.post("zenoh"), 204)
        self.assertEqual(self.current("zenoh")["state"], "new")
        self.assertEqual(self.timer("zenoh")[0], 1000)
        self.assertIn("zenoh wedge tracking failed", self.logs.getvalue())

    def test_current_outputs_are_visible_before_slow_history_returns(self):
        for kind in ("host", "zenoh"):
            with self.subTest(kind=kind):
                self.assertEqual(self.post(kind, "old"), 204)
                entered, release = threading.Event(), threading.Event()
                record = getattr(self.m, self.name(kind, "record"))
                results, errors = [], []

                def delayed_record(*args):
                    entered.set()
                    if not release.wait(3):
                        raise RuntimeError("fixture history timeout")
                    return record(*args)

                def request():
                    try:
                        results.append(self.post(kind))
                    except Exception as error:
                        errors.append(error)

                with patch.object(self.m, self.name(kind, "record"), side_effect=delayed_record):
                    worker = threading.Thread(target=request)
                    worker.start()
                    try:
                        self.assertTrue(entered.wait(3))
                        self.assertEqual(self.current(kind)["state"], "new")
                        if kind == "zenoh":
                            self.assert_wedged()
                    finally:
                        release.set()
                        worker.join(4)
                self.assertFalse(worker.is_alive())
                self.assertEqual(errors, [])
                self.assertEqual(results, [204])

    def test_successful_export_is_throttled_until_exact_interval(self):
        for kind in ("host", "zenoh"):
            with self.subTest(kind=kind):
                self.clock.return_value = 1000
                self.assertEqual(self.post(kind), 204)
                export = self.name(kind, "export")
                with patch.object(self.m, export, wraps=getattr(self.m, export)) as call:
                    self.clock.return_value = 1029
                    self.assertEqual(self.post(kind), 204)
                    call.assert_not_called()
                    self.clock.return_value = 1030
                    self.assertEqual(self.post(kind), 204)
                    call.assert_called_once()
                self.assertEqual(self.timer(kind)[0], 1030)

    def test_required_individual_write_failure_is_not_acknowledged(self):
        real_replace = self.m.os.replace
        for kind in ("host", "zenoh"):
            with self.subTest(kind=kind):
                dst = self.root / ("host-host-a.json" if kind == "host" else "zenoh-router-a.json")

                def failing_replace(src, target):
                    if str(target) == str(dst):
                        raise OSError("synthetic required output failure")
                    return real_replace(src, target)

                with patch.object(self.m.os, "replace", side_effect=failing_replace), \
                        patch.object(self.m, self.name(kind, "record")) as record:
                    with self.assertRaises((http.client.RemoteDisconnected, ConnectionResetError)):
                        self.post(kind)
                record.assert_not_called()
                self.assertFalse(dst.exists())
        self.assertEqual(len(self.errors), 2)
        self.assertTrue(all(isinstance(error, OSError) for error in self.errors))

    def test_auth_allowlist_and_payload_refusals_write_no_report(self):
        for kind in ("host", "zenoh"):
            with self.subTest(kind=kind):
                self.assertEqual(self.post(kind, auth="Bearer wrong-fixture-token"), 401)
                self.assertEqual(self.post(kind, route=self.route(kind).replace("-a", "-unknown")), 404)
                self.assertEqual(self.post(kind, body={}), 400)
                self.assertFalse(self.aggregate(kind).exists())
        self.assertFalse((self.root / "host-host-a.json").exists())
        self.assertFalse((self.root / "zenoh-router-a.json").exists())


if __name__ == "__main__":
    unittest.main()
