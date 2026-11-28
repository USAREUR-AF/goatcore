#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Zenoh client for the mixed-version candidate test (ADR 1262).

Two roles, run at different eclipse-zenoh versions against one router:

  client.py serve SECS   subscriber on cand/data, queryable cand/q (replies
                         "pong:<version>"), liveliness token cand/live/serve,
                         advanced publisher on cand/adv with a 10-sample cache
                         (it publishes 5 samples before the driver arrives).
                         Prints SERVE_RESULT {"received": N, ...} on exit.
  client.py drive        waits for the token, puts 20 samples on cand/data,
                         queries cand/q, reads liveliness, and opens an advanced
                         subscriber on cand/adv that must receive the history.
                         Prints DRIVE_RESULT {...} and exits 1 on any miss.

Environment: ZENOH_ENDPOINT (default tcp/127.0.0.1:7447), ZENOH_MODE (client).
"""

import json
import os
import sys
import time

import zenoh

ENDPOINT = os.environ.get("ZENOH_ENDPOINT", "tcp/127.0.0.1:7447")
MODE = os.environ.get("ZENOH_MODE", "client")
N = 20


def version() -> str:
    try:
        from importlib.metadata import version as v
        return v("eclipse-zenoh")
    except Exception:  # noqa: BLE001
        return "unknown"


def open_session():
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps(MODE))
    conf.insert_json5("timestamping/enabled", "true")  # advanced publisher sequences by timestamp
    if MODE == "client":
        conf.insert_json5("connect/endpoints", json.dumps([ENDPOINT]))
    else:
        conf.insert_json5("listen/endpoints", json.dumps([ENDPOINT]) if os.environ.get("ZENOH_LISTEN") else "[]")
        conf.insert_json5("connect/endpoints", "[]" if os.environ.get("ZENOH_LISTEN") else json.dumps([ENDPOINT]))
        conf.insert_json5("scouting/multicast/enabled", "false")
    return zenoh.open(conf)


def advanced():
    try:
        from zenoh import ext  # noqa: F401
        return ext
    except ImportError:
        return None


def serve(secs: float) -> int:
    s = open_session()
    got = []
    sub = s.declare_subscriber("cand/data", lambda sample: got.append(sample.payload.to_bytes()))
    me = version()
    qbl = s.declare_queryable("cand/q", lambda q: q.reply("cand/q", f"pong:{me}".encode()))
    tok = s.liveliness().declare_token("cand/live/serve")
    ext = advanced()
    adv = None
    if ext is not None:
        adv = ext.declare_advanced_publisher(s, "cand/adv", cache=ext.CacheConfig(max_samples=10),
                                             publisher_detection=True)
        for i in range(5):
            adv.put(f"hist-{i}".encode())
    deadline = time.time() + secs
    while time.time() < deadline and len(got) < N:
        time.sleep(0.1)
    time.sleep(1.0)  # let the driver finish its advanced-subscriber read
    print("SERVE_RESULT " + json.dumps({"version": me, "received": len(got), "advanced": adv is not None}),
          flush=True)
    for h in (adv, tok, qbl, sub):
        if h is not None:
            h.undeclare()
    s.close()
    return 0 if len(got) == N else 1


def drive() -> int:
    s = open_session()
    me = version()
    res = {"version": me}
    t0 = time.time()
    while time.time() - t0 < 20:
        if list(s.liveliness().get("cand/live/**", timeout=2.0)):
            break
        time.sleep(0.5)
    tokens = [r.ok.key_expr for r in s.liveliness().get("cand/live/**", timeout=3.0) if r.ok]
    res["liveliness"] = [str(k) for k in tokens]
    for i in range(N):
        s.put("cand/data", f"d-{i}".encode())
        time.sleep(0.02)
    replies = [r.ok.payload.to_bytes().decode() for r in s.get("cand/q", timeout=5.0) if r.ok]
    res["query"] = replies
    ext = advanced()
    hist = []
    if ext is not None:
        asub = ext.declare_advanced_subscriber(
            s, "cand/adv", lambda sample: hist.append(sample.payload.to_bytes().decode()),
            history=ext.HistoryConfig(detect_late_publishers=True))
        t1 = time.time()
        while time.time() - t1 < 5 and len(hist) < 5:
            time.sleep(0.1)
        asub.undeclare()
    res["advanced_history"] = hist
    s.close()
    ok = (res["liveliness"] == ["cand/live/serve"]
          and len(replies) == 1 and replies[0].startswith("pong:")
          and (ext is None or len(hist) >= 5))
    res["advanced_api"] = ext is not None
    res["ok"] = ok
    print("DRIVE_RESULT " + json.dumps(res), flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    if sys.argv[1] == "serve":
        sys.exit(serve(float(sys.argv[2]) if len(sys.argv) > 2 else 30))
    sys.exit(drive())
