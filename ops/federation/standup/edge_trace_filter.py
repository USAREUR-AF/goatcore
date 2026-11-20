#!/usr/bin/env python3
"""Phase 14's edge half: only this run's lines leave the edge (docs/design/federation-two-drivers.md).

A peering driven from two machines runs phase 14 in two parts. The controller fires
every probe first (acceptance.sh with FED_ACCEPTANCE_TRACE=deferred) and writes the
ids it fired and the window it fired them in to <work>/acceptance/ids.json. The edge
driver then runs this filter over its own edge router's log:

    edge_trace_filter.py --ids ID,ID,... --window START/END < edge.log > out.json

It keeps ONLY the lines that carry one of the ids and whose timestamp falls in the
window widened by WIDEN seconds on each side (the clock skew phase 3 also allows). A
line with no timestamp it can read is dropped: nothing places it in the window. It
writes {"schema": "goatbuilder.phase14-edge/v1", "ids": [...], "window": {"start",
"end"}, "lines": [...]}, with the ids and window exactly as given, so the controller
can tell the file answers its own ids.json. Nothing else from the log is written.

acceptance.sh also calls two helpers here, so the file's shape lives in one place:

    edge_trace_filter.py --write-ids OUT --ids ID,ID,... --window START/END
        writes ids.json (schema goatbuilder.acceptance-ids/v1)
    edge_trace_filter.py --verify TRACE.json --against IDS.json
        refuses (exit 2, the reason on stderr) a file whose schema, ids or window
        differ from ids.json, or that holds a line the filter would not have kept;
        else prints its lines, one per line, for the decisions to read

Exit status: 0 done, 2 refused or unreadable input.
"""
import argparse
import datetime
import json
import os
import re
import sys
import tempfile

SCHEMA_IDS = "goatbuilder.acceptance-ids/v1"
SCHEMA_EDGE = "goatbuilder.phase14-edge/v1"
WIDEN = datetime.timedelta(minutes=5)
# An id is a run id or a zenoh session id: letters, digits, dot, underscore, hyphen.
# Shorter than 8 characters, an id would match lines by accident.
ID = re.compile(r"[A-Za-z0-9._-]{8,128}")
WHEN = re.compile(r"(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2}:\d{2})(\.\d+)?(Z|[+-]\d{2}:?\d{2})")
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
# Only at the start of a line, after colour codes and blanks: `docker logs -t` puts
# its own timestamp first, and a router line begins with its own.
LEAD = 64


class Refused(ValueError):
    pass


def parse_time(text):
    """An RFC 3339 time with a zone, as a datetime in UTC; None when it is not one."""
    m = WHEN.fullmatch(text.strip())
    if not m:
        return None
    return _when(m)


def _when(m):
    day, clock, frac, zone = m.groups()
    # Six digits after the dot, as every Python's fromisoformat reads them.
    frac = "." + (frac or ".")[1:7].ljust(6, "0")
    zone = "+00:00" if zone == "Z" else (zone if ":" in zone else zone[:3] + ":" + zone[3:])
    try:
        t = datetime.datetime.fromisoformat("%sT%s%s%s" % (day, clock, frac, zone))
    except ValueError:
        return None
    return t.astimezone(datetime.timezone.utc)


def line_time(line):
    """The timestamp a log line starts with, or None."""
    head = ANSI.sub("", line).lstrip()[:LEAD]
    m = WHEN.match(head)
    return _when(m) if m else None


def parse_ids(text):
    ids = [i for i in (text or "").split(",")]
    if not ids or any(not ID.fullmatch(i) for i in ids):
        raise Refused("--ids takes ids of 8 to 128 letters, digits, '.', '_' or '-', separated by commas: %r" % text)
    if len(set(ids)) != len(ids):
        raise Refused("--ids names an id twice: %r" % text)
    return ids


def parse_window(text):
    start, sep, end = (text or "").partition("/")
    s, e = parse_time(start), parse_time(end)
    if not sep or s is None or e is None:
        raise Refused("--window takes START/END, two RFC 3339 times with a zone: %r" % text)
    if e < s:
        raise Refused("--window ends before it starts: %r" % text)
    return {"start": start.strip(), "end": end.strip()}, s, e


def keep(line, ids, start, end):
    """Whether the filter keeps LINE: it carries an id and falls in the widened window."""
    if not any(i in line for i in ids):
        return False
    t = line_time(line)
    return t is not None and start - WIDEN <= t <= end + WIDEN


def filter_lines(stream, ids, start, end):
    out = []
    for raw in stream:
        line = raw.rstrip("\r\n")
        if keep(line, ids, start, end):
            out.append(line)
    return out


def write_json(path, doc):
    d = os.path.dirname(os.path.abspath(path))
    os.makedirs(d, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".tmp-")
    with os.fdopen(fd, "w") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")
    os.replace(tmp, path)


def load(path, what):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError) as e:
        raise Refused("%s %s cannot be read: %s" % (what, path, e))


def verify(trace_path, ids_path):
    """The lines of a returned edge file, once it is shown to answer ids.json."""
    want = load(ids_path, "the ids file")
    got = load(trace_path, "the edge's file")
    if not isinstance(want, dict) or want.get("schema") != SCHEMA_IDS:
        raise Refused("%s is not %s" % (ids_path, SCHEMA_IDS))
    if not isinstance(got, dict) or got.get("schema") != SCHEMA_EDGE:
        raise Refused("%s is not %s" % (trace_path, SCHEMA_EDGE))
    ids = want.get("ids")
    if not isinstance(ids, list) or not ids or not all(isinstance(i, str) and ID.fullmatch(i) for i in ids):
        raise Refused("%s holds no well-formed ids" % ids_path)
    gids = got.get("ids")
    if not isinstance(gids, list) or sorted(gids) != sorted(ids) or len(set(gids)) != len(gids):
        raise Refused("the edge's file answers other ids than this run fired: it names %r, %s names %r"
                      % (gids, os.path.basename(ids_path), ids))
    window = want.get("window")
    if not isinstance(window, dict):
        raise Refused("%s holds no window" % ids_path)
    _, start, end = parse_window("%s/%s" % (window.get("start"), window.get("end")))
    if got.get("window") != {"start": window.get("start"), "end": window.get("end")}:
        raise Refused("the edge's file covers another window than this run's: it names %r, %s names %r"
                      % (got.get("window"), os.path.basename(ids_path), window))
    lines = got.get("lines")
    if not isinstance(lines, list) or not all(isinstance(x, str) and "\n" not in x for x in lines):
        raise Refused("%s holds no list of single lines" % trace_path)
    for n, line in enumerate(lines, 1):
        if not keep(line, ids, start, end):
            raise Refused("line %d of %s carries none of this run's ids, or falls outside the window: "
                          "the edge returned more than its filter keeps" % (n, trace_path))
    return lines


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--ids")
    ap.add_argument("--window")
    ap.add_argument("--write-ids", metavar="OUT")
    ap.add_argument("--verify", metavar="TRACE")
    ap.add_argument("--against", metavar="IDS")
    a = ap.parse_args(argv)
    try:
        if a.verify:
            if not a.against:
                raise Refused("--verify needs --against <ids.json>")
            for line in verify(a.verify, a.against):
                sys.stdout.write(line + "\n")
            return 0
        ids = parse_ids(a.ids)
        window, start, end = parse_window(a.window)
        if a.write_ids:
            write_json(a.write_ids, {"schema": SCHEMA_IDS, "ids": ids, "window": window})
            return 0
        stdin = open(sys.stdin.fileno(), "r", encoding="utf-8", errors="replace", closefd=False)
        doc = {"schema": SCHEMA_EDGE, "ids": ids, "window": window,
               "lines": filter_lines(stdin, ids, start, end)}
        json.dump(doc, sys.stdout, indent=2)
        sys.stdout.write("\n")
        return 0
    except Refused as e:
        print("edge trace filter: %s" % e, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
