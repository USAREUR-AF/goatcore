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
        for k in ("pod_slot", "state", "pending", "lapsed", "ledger_unsent", "connected", "generation", "generation_age_seconds", "max_rule_hours", "local_authority", "you", "sources"):
            self.assertIn(k, st)

    def test_allow_then_pending(self):
        s, out = self.req("POST", "/v1/rules", {"source": m.LIDAR + ".g2", "until": until(4), "reason": "survey"})
        self.assertEqual(s, 201)
        self.assertEqual(out["entry"]["action"], "allow")
        self.assertEqual(out["rule"]["origin"], "local")
        self.assertEqual(out["rule"]["ratification"], "pending")
        _, p = self.req("GET", "/v1/pending")
        self.assertEqual([x["action"] for x in p["pending"]], ["allow"])
        self.assertEqual(p["pending"][0]["authority"], {"identity": m.AUTHORITY, "cert_cn": m.AUTHORITY_CN})

    def test_limits(self):
        cases = [
            ({"source_slot": m.LIDAR, "until": until(4)}, "carriage_reason_required"),
            ({"source_slot": m.LIDAR, "until": until(4), "reason": "  "}, "carriage_reason_required"),
            ({"source_slot": m.LIDAR, "reason": "x"}, "carriage_until_required"),
            ({"source_slot": m.LIDAR, "until": until(73), "reason": "x"}, "carriage_over_max_length"),
            ({"source_slot": m.LIDAR, "until": until(-1), "reason": "x"}, "carriage_until_past"),
            ({"source_slot": "not-a-slot", "until": until(4), "reason": "x"}, "invalid_request"),
            ({"source": "00112233445566778899aabbccddeeff", "until": until(4), "reason": "x"}, "invalid_request"),
        ]
        for body, code in cases:
            s, out = self.req("POST", "/v1/rules", body)
            self.assertEqual(s, 400, body)
            self.assertEqual(out["error"]["code"], code)
            for k in ("message", "what", "why", "fix"):
                self.assertTrue(out["error"][k])
        _, p = self.req("GET", "/v1/pending")
        self.assertEqual(p["pending"], [])

    def test_bare_slot_accepted_once_carried_and_rollback_warns(self):
        s, _ = self.req("POST", "/v1/rules", {"source": m.RADAR, "until": until(4), "reason": "x"})
        self.assertEqual(s, 201)
        self.pod.rollback = True
        s, out = self.req("POST", "/v1/rules/c-102/extend", {"until": until(4), "reason": "x"})
        self.assertEqual(s, 202)
        self.assertIn("warning", out)

    def test_look_only_refused(self):
        self.pod.may_change = False
        s, out = self.req("POST", "/v1/sources/%s/revoke" % m.CAMERA, {"reason": "x"})
        self.assertEqual(s, 403)
        self.assertEqual(out["error"]["code"], "carriage_not_authorized")

    def test_revoke_extend_reactivate(self):
        s, out = self.req("POST", "/v1/sources/%s/revoke" % m.CAMERA, {"reason": "lost", "until": until(4)})
        self.assertEqual((s, out["error"]["code"]), (400, "invalid_request"))
        s, out = self.req("POST", "/v1/rules/c-101/reactivate", {"until": until(4), "reason": "x"})
        self.assertEqual((s, out["error"]["code"]), (409, "carriage_not_archived"))
        s, out = self.req("POST", "/v1/sources/%s/revoke" % m.CAMERA, {"reason": "lost"})
        self.assertEqual((s, out["rule"], out["entry"]["action"]), (200, None, "revoke"))
        _, rl = self.req("GET", "/v1/rules")
        self.assertEqual({r["id"]: r["state"] for r in rl["rules"]}["c-102"], "revoked")
        self.assertEqual(rl["revocations"][0]["source_slot"], m.CAMERA)
        _, st = self.req("GET", "/v1/status")
        self.assertEqual({x["slot"]: x["carriage"] for x in st["sources"]}[m.CAMERA], "refused")
        s, out = self.req("POST", "/v1/rules/c-102/extend", {"until": until(4), "reason": "x"})
        self.assertEqual(out["error"]["code"], "carriage_rule_expired")
        s, out = self.req("POST", "/v1/rules/c-090/reactivate", {"until": until(24), "reason": "back"})
        self.assertEqual((s, out["rule"]["state"]), (200, "active"))
        s, out = self.req("POST", "/v1/rules/c-090/extend", {"until": until(72), "reason": "longer"})
        self.assertEqual(s, 200)
        _, p = self.req("GET", "/v1/pending")
        self.assertEqual([x["action"] for x in p["pending"]], ["revoke", "reactivate", "extend"])


if __name__ == "__main__":
    unittest.main(verbosity=1)
