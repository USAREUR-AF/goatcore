#!/usr/bin/env python3
"""Close the operator step's open row in a standup timings file.

goat-standup.sh writes an open row (seconds and exit code `-`, cause
`operator`) when it stops at the operator step (phase 11). A run resumed with
--from <later phase> calls this first: when the file's last row is such a row
for an earlier phase, it appends the closing row, with the wall time from the
stop to now, and prints those seconds. Otherwise it changes nothing and prints
nothing. goatbuilder standup does the same (close_operator_stop in
crates/goatbuilder/src/standup.rs), so either driver's file reads back the
same way.

usage: standup-timings-close.py <timings file> <from phase> [--now YYYYMMDDTHHMMSSZ]
"""
import argparse
import datetime
import sys

FMT = "%Y%m%dT%H%M%SZ"


def main(argv):
    ap = argparse.ArgumentParser(description="close the operator step's open timings row")
    ap.add_argument("file")
    ap.add_argument("from_phase", type=int)
    ap.add_argument("--now", help="the closing time, " + FMT.replace("%", "%%") + " (default: now, UTC)")
    a = ap.parse_args(argv)
    try:
        with open(a.file, encoding="utf-8") as f:
            lines = f.read().splitlines()
    except OSError:
        return 0
    if not lines:
        return 0
    row = lines[-1].split("\t")
    if len(row) != 6 or row[3:] != ["-", "-", "operator"]:
        return 0
    ts, n, label = row[0], row[1], row[2]
    try:
        if int(n) >= a.from_phase:
            return 0
        stopped = datetime.datetime.strptime(ts, FMT)
    except ValueError:
        return 0
    now = datetime.datetime.strptime(a.now, FMT) if a.now else datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)
    secs = max(0, int((now - stopped).total_seconds()))
    with open(a.file, "a", encoding="utf-8") as f:
        f.write(f"{now.strftime(FMT)}\t{n}\t{label}\t{secs}\t0\toperator\n")
    print(secs)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
