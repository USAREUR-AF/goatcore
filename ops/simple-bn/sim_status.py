#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Declare, per feed, whether its source is running real or simulated data: the out-of-band sim/real status.

Some sources can run a simulation and send it in the same form as real data, with no flag a receiver
or this node can read. For those, the operator declares here what the source is doing, from what the
source's operators say. The sender reads the file every 5 s and shows the declaration in the status
document; the monitor serves it at GET /api/v1/sim-status. The declaration changes nothing that is
sent: the SIM bit stays as the feed's profile sets it, and the status shows whether the two agree.

  simple-bn-sim-status show
  simple-bn-sim-status set <feed> real|simulated|unknown --by <who> [--note <text>]
  simple-bn-sim-status clear <feed> --by <who>        back to the profile's default

File: /etc/simple-bn/sim-status.json (root writes, mode 644; holds no secret). Python standard library only.
"""
import argparse
import json
import os
import re
import sys
import tempfile
import time

DEFAULT_FILE = "/etc/simple-bn/sim-status.json"
MODES = ("real", "simulated", "unknown")
HISTORY = 100
FEED_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")


def load(path):
    """The declaration document; an empty one when the file is absent."""
    try:
        with open(path) as f:
            doc = json.load(f)
    except FileNotFoundError:
        return {"schema": "goat.sim-status/v1", "feeds": {}, "history": []}
    if not isinstance(doc, dict) or not isinstance(doc.get("feeds"), dict):
        raise ValueError(f"{path}: not a goat.sim-status/v1 document")
    doc.setdefault("history", [])
    return doc


def save(path, doc):
    d = os.path.dirname(path) or "."
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".sim-status.")
    with os.fdopen(fd, "w") as f:
        json.dump(doc, f, indent=1, sort_keys=True)
        f.write("\n")
    os.chmod(tmp, 0o644)
    os.replace(tmp, path)


def apply(doc, feed, mode, by, note="", now=None):
    """Record one declaration (mode None clears it) and its history entry. Returns the document."""
    if not FEED_RE.match(feed):
        raise ValueError(f"feed name {feed!r}: letters, digits, '.', '_' and '-' only")
    if mode is not None and mode not in MODES:
        raise ValueError(f"mode {mode!r}: one of {', '.join(MODES)}")
    if not by.strip():
        raise ValueError("--by is required: who declared it")
    now = time.time() if now is None else now
    entry = {"feed": feed, "mode": mode, "set_at": now, "set_by": by.strip()[:100], "note": note.strip()[:300]}
    if mode is None:
        doc["feeds"].pop(feed, None)
    else:
        doc["feeds"][feed] = {k: entry[k] for k in ("mode", "set_at", "set_by", "note")}
    doc["history"] = (doc.get("history", []) + [entry])[-HISTORY:]
    doc["schema"] = "goat.sim-status/v1"
    return doc


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--file", default=DEFAULT_FILE)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("show")
    s = sub.add_parser("set")
    s.add_argument("feed")
    s.add_argument("mode", choices=MODES)
    s.add_argument("--by", required=True)
    s.add_argument("--note", default="")
    c = sub.add_parser("clear")
    c.add_argument("feed")
    c.add_argument("--by", required=True)
    c.add_argument("--note", default="")
    a = ap.parse_args(argv)
    try:
        doc = load(a.file)
        if a.cmd == "set":
            save(a.file, apply(doc, a.feed, a.mode, a.by, a.note))
        elif a.cmd == "clear":
            save(a.file, apply(doc, a.feed, None, a.by, a.note))
    except (OSError, ValueError) as e:
        sys.exit(f"simple-bn-sim-status: {e}")
    json.dump({"feeds": doc["feeds"], "history": doc["history"][-5:]}, sys.stdout, indent=1, sort_keys=True)
    print()


if __name__ == "__main__":
    main()
