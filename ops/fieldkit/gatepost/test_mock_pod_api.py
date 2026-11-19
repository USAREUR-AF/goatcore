#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Tests for the Gatepost mock of the pod API: it must refuse what the pod refuses.

Run: python3 ops/fieldkit/gatepost/test_mock_pod_api.py
"""
from __future__ import annotations

import json
import sys
import threading
import unittest
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mock_pod_api as m  # noqa: E402


def until(hours: float) -> str:
    return (datetime.now(timezone.utc) + timedelta(hours=hours)).strftime("%Y-%m-%dT%H:%M:%SZ")


class MockPodApi(unittest.TestCase):
    def setUp(self):
        self.pod = m.Pod()
        self.srv = m.serve(0, self.pod)
        self.base = f"http://127.0.0.1:{self.srv.server_address[1]}"
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()

    def tearDown(self):
        self.srv.shutdown()
        self.srv.server_close()

    def req(self, method, path, body=None):
        data = json.dumps(body).encode() if body is not None else None
        r = urllib.request.Request(self.base + path, data=data, method=method, headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(r) as resp:
                raw = resp.read()
                return resp.status, json.loads(raw) if raw and resp.headers.get_content_type() == "application/json" else raw
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read())

    def test_serves_gatepost_and_status(self):
        s, page = self.req("GET", "/")
        self.assertEqual(s, 200)
        self.assertIn(b"Fieldkit Gatepost", page)
        s, st = self.req("GET", "/v1/status")
        self.assertEqual(s, 200)
        for k in ("pod_slot", "connected", "generation", "generation_age_seconds", "max_rule_hours", "local_authority", "you", "sources"):
            self.assertIn(k, st)

    def test_allow_then_pending(self):
        s, out = self.req("POST", "/v1/rules", {"source_slot": m.LIDAR, "until": until(4), "reason": "survey"})
        self.assertEqual(s, 201)
        self.assertEqual(out["rule"]["origin"], "local")
        self.assertEqual(out["rule"]["ratification"], "pending")
        _, p = self.req("GET", "/v1/pending")
        self.assertEqual([x["action"] for x in p["pending"]], ["allow"])
        self.assertEqual(p["pending"][0]["authority"], m.AUTHORITY)

    def test_limits(self):
        cases = [
            ({"source_slot": m.LIDAR, "until": until(4)}, "E_POD_REASON_MISSING"),
            ({"source_slot": m.LIDAR, "until": until(4), "reason": "  "}, "E_POD_REASON_MISSING"),
            ({"source_slot": m.LIDAR, "reason": "x"}, "E_POD_UNTIL_MISSING"),
            ({"source_slot": m.LIDAR, "until": until(73), "reason": "x"}, "E_POD_UNTIL_TOO_LONG"),
            ({"source_slot": m.LIDAR, "until": until(-1), "reason": "x"}, "E_POD_UNTIL_PAST"),
            ({"source_slot": "not-a-slot", "until": until(4), "reason": "x"}, "E_POD_SOURCE_INVALID"),
        ]
        for body, code in cases:
            s, out = self.req("POST", "/v1/rules", body)
            self.assertEqual(s, 400, body)
            self.assertEqual(out["error"]["code"], code)
            for k in ("what", "why", "fix"):
                self.assertTrue(out["error"][k])
        _, p = self.req("GET", "/v1/pending")
        self.assertEqual(p["pending"], [])

    def test_look_only_refused(self):
        self.pod.may_change = False
        s, out = self.req("POST", "/v1/sources/%s/revoke" % m.CAMERA, {"reason": "x"})
        self.assertEqual(s, 403)
        self.assertEqual(out["error"]["code"], "E_POD_NOT_LOCAL_AUTHORITY")

    def test_revoke_extend_reactivate(self):
        s, out = self.req("POST", "/v1/sources/%s/revoke" % m.CAMERA, {"reason": "lost"})
        self.assertEqual((s, out["rule"]["state"]), (200, "archived"))
        _, st = self.req("GET", "/v1/status")
        self.assertEqual({x["slot"]: x["carriage"] for x in st["sources"]}[m.CAMERA], "refused")
        s, out = self.req("POST", "/v1/rules/c-102/extend", {"until": until(4), "reason": "x"})
        self.assertEqual(out["error"]["code"], "E_POD_RULE_NOT_ACTIVE")
        s, out = self.req("POST", "/v1/rules/c-090/reactivate", {"until": until(24), "reason": "back"})
        self.assertEqual((s, out["rule"]["state"]), (200, "active"))
        s, out = self.req("POST", "/v1/rules/c-090/extend", {"until": until(72), "reason": "longer"})
        self.assertEqual(s, 200)
        _, p = self.req("GET", "/v1/pending")
        self.assertEqual([x["action"] for x in p["pending"]], ["revoke", "reactivate", "extend"])


if __name__ == "__main__":
    unittest.main(verbosity=1)
