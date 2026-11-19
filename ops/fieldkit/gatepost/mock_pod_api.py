#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""mock_pod_api.py — a local stand-in for the pod API, for building and testing Gatepost.

It answers the routes of docs/design/pod-carriage-and-provenance.md §7.4 with the
same JSON shapes ops/goat-pod-api serves, from synthetic in-memory data, and it
serves gatepost.html at `/`. It enforces the same limits the pod does (a reason
on every change, an end time no later than the longest rule length, only local
authority may change), so the page can be tested against refusals too. It holds
no certificate: the identity is a flag.

Not a pod. Nothing here is used on a goatnet.

Usage:
  python3 ops/fieldkit/gatepost/mock_pod_api.py [--port 8790] [--look-only] [--connected]
"""
from __future__ import annotations

import argparse
import json
import re
import threading
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
PAGE = HERE / "gatepost.html"

POD = "9d0c4b7e2a1f4c3d8e6b5a4f3c2d1e0f"
RADAR = "3f2a9c1e0b7d4e5f8a6b9c0d1e2f3a4b"
CAMERA = "b81e44d0c2a94f6e9a7d3c5b1e0f2a6c"
LIDAR = "e5d4c3b2a1f04e9d8c7b6a5f4e3d2c1b"
AUTHORITY = "zagros-op-alpha"


def now() -> datetime:
    return datetime.now(timezone.utc).replace(microsecond=0)


def iso(t: datetime | None) -> str | None:
    return t.strftime("%Y-%m-%dT%H:%M:%SZ") if t else None


def parse(s: object) -> datetime | None:
    if not isinstance(s, str):
        return None
    try:
        return datetime.fromisoformat(s.replace("Z", "+00:00")).astimezone(timezone.utc)
    except ValueError:
        return None


class Refusal(Exception):
    def __init__(self, status: int, code: str, what: str, why: str, fix: str):
        super().__init__(what)
        self.status, self.body = status, {"error": {"code": code, "what": what, "why": why, "fix": fix}}


class Pod:
    """In-memory pod state. One lock; every change appends a ledger line."""

    def __init__(self, may_change: bool = True, connected: bool = False, max_hours: int = 72):
        self.lock = getattr(self, "lock", None) or threading.Lock()
        self.may_change = may_change
        self.connected = connected
        self.max_hours = max_hours
        self.started = now()
        t = now()
        self.rules = [
            {"id": "c-101", "source_slot": RADAR, "source_name": "radar-east", "kind": "org", "origin": "central",
             "state": "active", "until": None, "reason": "org rule", "ratification": None},
            {"id": "c-102", "source_slot": CAMERA, "source_name": "camera-gate", "kind": "named", "origin": "central",
             "state": "active", "until": iso(t + timedelta(hours=20)), "reason": "gate watch", "ratification": None},
            {"id": "c-090", "source_slot": LIDAR, "source_name": "lidar-north", "kind": "temporary", "origin": "central",
             "state": "archived", "until": iso(t - timedelta(days=3)), "reason": "last exercise", "ratification": None},
        ]
        self.names = {RADAR: "radar-east", CAMERA: "camera-gate", LIDAR: "lidar-north"}
        self.connected_sources = {RADAR, CAMERA, LIDAR}
        self.ledger: list[dict] = []
        self.next_id = 1

    # -- reads -----------------------------------------------------------------
    def carriage(self, slot: str) -> str:
        active = [r for r in self.rules if r["source_slot"] == slot and r["state"] == "active"]
        if not active:
            return "refused"
        if all(r["until"] and parse(r["until"]) - now() < timedelta(hours=6) for r in active):
            return "expiring"
        return "ok"

    def status(self) -> dict:
        age = int((now() - self.started).total_seconds()) + (0 if self.connected else 5 * 3600)
        return {
            "pod_slot": POD,
            "connected": self.connected,
            "cut_off_since": None if self.connected else iso(self.started - timedelta(hours=5)),
            "generation": 41,
            "generation_age_seconds": age,
            "max_rule_hours": self.max_hours,
            "local_authority": [AUTHORITY],
            "you": {"identity": AUTHORITY if self.may_change else "zagros-viewer-bravo", "may_change": self.may_change},
            "sources": [{"slot": s, "name": self.names.get(s), "connected": True, "carriage": self.carriage(s)}
                        for s in sorted(self.connected_sources)],
        }

    def pending(self) -> dict:
        return {"pending": [e for e in self.ledger if e["state"] == "pending"]}

    # -- checks ----------------------------------------------------------------
    def need_authority(self) -> None:
        if not self.may_change:
            raise Refusal(403, "E_POD_NOT_LOCAL_AUTHORITY", "You may look, but not change rules on this pod.",
                          "Only people named as local authority in the pod's rules may change them.",
                          f"Ask one of them to do it: {AUTHORITY}.")

    def need_reason(self, body: dict) -> str:
        reason = body.get("reason")
        if not isinstance(reason, str) or not reason.strip():
            raise Refusal(400, "E_POD_REASON_MISSING", "A reason is needed.",
                          "Every change on a pod is written down with its reason, for the org lead who checks it later.",
                          "Type a short reason and try again.")
        return reason.strip()

    def need_until(self, body: dict) -> str:
        t = parse(body.get("until"))
        if t is None:
            raise Refusal(400, "E_POD_UNTIL_MISSING", "An end time is needed.",
                          "A rule made on a pod always ends.", "Pick 4, 24 or 72 hours.")
        if t <= now():
            raise Refusal(400, "E_POD_UNTIL_PAST", "The end time is already past.", "A rule must end in the future.",
                          "Pick a duration again; the page sets the end time from now.")
        if t > now() + timedelta(hours=self.max_hours, minutes=1):
            raise Refusal(400, "E_POD_UNTIL_TOO_LONG", f"That is longer than {self.max_hours} hours.",
                          f"A rule made on a pod runs at most {self.max_hours} hours.", "Pick a shorter time.")
        return iso(t)

    def rule(self, rule_id: str) -> dict:
        for r in self.rules:
            if r["id"] == rule_id:
                return r
        raise Refusal(404, "E_POD_RULE_NOT_FOUND", f"No rule {rule_id} on this pod.", "It may have been removed.",
                      "Press Refresh and pick again.")

    def record(self, action: str, slot: str, rule_id: str | None, until: str | None, reason: str) -> None:
        self.ledger.append({"id": f"l-{len(self.ledger) + 1}", "action": action, "source_slot": slot,
                            "rule_id": rule_id, "until": until, "reason": reason, "authority": AUTHORITY,
                            "at": iso(now()), "state": "pending"})

    # -- changes ---------------------------------------------------------------
    def allow(self, body: dict) -> dict:
        self.need_authority()
        slot = body.get("source_slot")
        if not isinstance(slot, str) or not re.fullmatch(r"[0-9a-f]{32}", slot):
            raise Refusal(400, "E_POD_SOURCE_INVALID", "That source is not a slot.",
                          "A slot is 32 letters and numbers (0-9, a-f).", "Copy the slot from the device's label or pick one from the list.")
        until, reason = self.need_until(body), self.need_reason(body)
        r = {"id": f"l-r{self.next_id}", "source_slot": slot, "source_name": self.names.get(slot), "kind": "local",
             "origin": "local", "state": "active", "until": until, "reason": reason, "ratification": "pending"}
        self.next_id += 1
        self.rules.append(r)
        self.record("allow", slot, r["id"], until, reason)
        return {"rule": r}

    def extend(self, rule_id: str, body: dict) -> dict:
        self.need_authority()
        r = self.rule(rule_id)
        until, reason = self.need_until(body), self.need_reason(body)
        if r["state"] != "active":
            raise Refusal(409, "E_POD_RULE_NOT_ACTIVE", "That rule is not on.", "Only a rule that is on can be extended.",
                          "Use Bring back for a rule that ended.")
        r["until"] = until
        self.record("extend", r["source_slot"], r["id"], until, reason)
        return {"rule": r}

    def revoke(self, slot: str, body: dict) -> dict:
        self.need_authority()
        reason = self.need_reason(body)
        hit = None
        for r in self.rules:
            if r["source_slot"] == slot and r["state"] == "active":
                r["state"], r["until"], hit = "archived", iso(now()), r
        self.record("revoke", slot, hit["id"] if hit else None, None, reason)
        return {"rule": hit}

    def reactivate(self, rule_id: str, body: dict) -> dict:
        self.need_authority()
        r = self.rule(rule_id)
        until, reason = self.need_until(body), self.need_reason(body)
        if r["state"] != "archived":
            raise Refusal(409, "E_POD_RULE_ACTIVE", "That rule is already on.", "Only a rule that ended can be brought back.",
                          "Use Extend to give it a later end time.")
        r["state"], r["until"] = "active", until
        self.record("reactivate", r["source_slot"], r["id"], until, reason)
        return {"rule": r}


ROUTES = [
    ("GET", re.compile(r"^/v1/status$"), lambda pod, m, b: (200, pod.status())),
    ("GET", re.compile(r"^/v1/rules$"), lambda pod, m, b: (200, {"rules": pod.rules})),
    ("GET", re.compile(r"^/v1/pending$"), lambda pod, m, b: (200, pod.pending())),
    ("POST", re.compile(r"^/v1/rules$"), lambda pod, m, b: (201, pod.allow(b))),
    ("POST", re.compile(r"^/v1/rules/([\w-]+)/extend$"), lambda pod, m, b: (200, pod.extend(m.group(1), b))),
    ("POST", re.compile(r"^/v1/rules/([\w-]+)/reactivate$"), lambda pod, m, b: (200, pod.reactivate(m.group(1), b))),
    ("POST", re.compile(r"^/v1/sources/([0-9a-f]{32})/revoke$"), lambda pod, m, b: (200, pod.revoke(m.group(1), b))),
]


def make_handler(pod: Pod):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *a):  # quiet
            pass

        def send(self, status: int, body: bytes, ctype: str) -> None:
            self.send_response(status)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def handle_any(self, method: str) -> None:
            path = self.path.split("?", 1)[0]
            if method == "GET" and path in ("/", "/index.html"):
                self.send(200, PAGE.read_bytes(), "text/html; charset=utf-8")
                return
            body: dict = {}
            if method == "POST":
                n = int(self.headers.get("Content-Length") or 0)
                try:
                    body = json.loads(self.rfile.read(n) or b"{}")
                except json.JSONDecodeError:
                    body = None  # type: ignore[assignment]
                if not isinstance(body, dict):
                    self.send(400, json.dumps({"error": {"code": "E_POD_BAD_REQUEST", "what": "The request was not JSON.",
                                                         "why": "", "fix": "Reload the page."}}).encode(), "application/json")
                    return
            if method == "POST" and path == "/_mock/reset":
                # Test isolation only: put the synthetic data back. The real pod has no such route.
                with pod.lock:
                    pod.__init__(may_change=body.get("may_change", True), connected=body.get("connected", False),
                                 max_hours=body.get("max_hours", 72))
                self.send(204, b"", "application/json")
                return
            for m_, rx, fn in ROUTES:
                m = rx.match(path)
                if m and m_ == method:
                    try:
                        with pod.lock:
                            status, out = fn(pod, m, body)
                    except Refusal as r:
                        status, out = r.status, r.body
                    self.send(status, json.dumps(out).encode(), "application/json")
                    return
            self.send(404, json.dumps({"error": {"code": "E_POD_NO_ROUTE", "what": "No such page on this pod.",
                                                 "why": path, "fix": "Reload the page."}}).encode(), "application/json")

        def do_GET(self):
            self.handle_any("GET")

        def do_POST(self):
            self.handle_any("POST")

    return Handler


def serve(port: int, pod: Pod) -> ThreadingHTTPServer:
    return ThreadingHTTPServer(("127.0.0.1", port), make_handler(pod))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--port", type=int, default=8790)
    ap.add_argument("--look-only", action="store_true", help="sign in as someone who is not local authority")
    ap.add_argument("--connected", action="store_true", help="report the pod as connected (default: cut off)")
    ap.add_argument("--max-hours", type=int, default=72)
    a = ap.parse_args()
    srv = serve(a.port, Pod(may_change=not a.look_only, connected=a.connected, max_hours=a.max_hours))
    print(f"mock pod API on http://127.0.0.1:{a.port}/ (Gatepost at /)", flush=True)
    srv.serve_forever()


if __name__ == "__main__":
    main()
