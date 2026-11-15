#!/usr/bin/env python3
"""samples — the numbered samples of a flow, as a reader received them (gate row B3).

The proving scenario of docs/design/federation-key-rotation.md §6 has a publisher on each
side write one numbered sample a second (lib/flow.sh flow_start), and a reader on the
other side record each number it receives with its arrival time. This file is the reader's
recorder and the check of §6's `samples-<key>` row.

    samples.py record OUT [--prefix P]
        Reads a Server-Sent Events stream on stdin (the core router's REST subscription,
        `curl -N -H 'Accept: text/event-stream'`) and appends one line per sample to OUT:
        <arrival time, seconds since the epoch> TAB <the key's last chunk>. A line that is
        not an event carrying a key, or whose last chunk does not start with P, is skipped;
        OUT is flushed after every sample, so a reader that is stopped loses nothing it
        received.

    samples.py check --sent SENT --arrived ARRIVED [--sent SENT --arrived ARRIVED]...
                     --from T0 --to T1 [--gap [--rotation START END]...]
        SENT is lib/flow.sh's flow-<label>.sent (<release time> TAB <key> TAB <HTTP code>),
        ARRIVED this recorder's file for the same flow. The step is the time window
        [T0, T1]. Every sample released inside the window counts, at every reader:
          - a release the sending core did not accept (a code other than 2xx) fails;
          - a number that never reached the reader is missing, and missing fails;
          - with --gap (the transport and carrier keys, whose step has a gap by design,
            §3), a late number passes, and so does a number lost inside a rotation's gap:
            a run of consecutive late or lost numbers at least GAP_MIN_S long, from the
            last number on time before it to the first on time after it. The fabric does
            not re-deliver what was released while the tunnel was down (measured in the
            ADR 1229 trial, §6), so each gap is recorded with its numbers, its length in
            seconds and how many it lost. A run that lost numbers counts as a gap only
            when the flow recovers after it (an on-time number follows), it is no longer
            than GAP_MAX_S, and it lies inside a rotated side's window (--rotation, one
            gap per window, and no more such runs than windows). A number lost outside a
            gap fails: no rotation explains it. Each run refused as a gap is listed in
            "not_gaps" with why.
        A number is late when it arrived more than LATE_S seconds after its release.
        Prints one JSON object per flow and a last line "samples: PASS|FAIL <why>".
        Exits 0 on PASS, 1 on FAIL, 2 when an input cannot be read.

The key's last chunk carries the number: lib/flow.sh names each sample fl<label><epoch>n<i>,
and <i> counts up from 1 for the whole flow.
"""
import argparse
import json
import re
import sys
import time

LATE_S = 5.0
# A rotation's gap is the link restarting onto a new key until the far side takes it:
# tens of seconds, measured (§6). A shorter run of losses is not one.
GAP_MIN_S = 5.0
# The longest a rotation's gap may be: the harness's own tunnel-back check gives up 60 s
# after the partner's step begins (units/rotation.sh, rot-tunnel-back), and that is
# about three times the ~21.5 s measured (§6). A longer run of losses is an outage.
GAP_MAX_S = 60.0
NUMBER = re.compile(r"n([0-9]+)$")


def record(out_path, prefix="", stream=sys.stdin):
    """Append <arrival time> TAB <key's last chunk> for every event carrying a key."""
    with open(out_path, "a", encoding="utf-8") as out:
        for line in stream:
            line = line.strip()
            if not line.startswith("data:"):
                continue
            try:
                ev = json.loads(line[5:].strip())
            except ValueError:
                continue
            key = ev.get("key") if isinstance(ev, dict) else None
            if not isinstance(key, str) or not key:
                continue
            last = key.rsplit("/", 1)[-1]
            if not last.startswith(prefix):
                continue
            out.write(f"{time.time():.3f}\t{last}\n")
            out.flush()


def number(key):
    m = NUMBER.search(key)
    return int(m.group(1)) if m else None


def read_rows(path):
    with open(path, encoding="utf-8") as fh:
        return [r.rstrip("\n").split("\t") for r in fh if r.strip()]


def judge(sent_rows, arrived_rows, t0, t1, gap, rotations=()):
    """One flow's verdict for the window [t0, t1]: (ok, report dict). `rotations` are the
    rotated sides' windows, (start, end) each: a lossy gap counts only inside one of them."""
    released = []  # (number, release time, code)
    for r in sent_rows:
        if len(r) < 3:
            continue
        t, n = float(r[0]), number(r[1])
        if n is not None and t0 <= t <= t1:
            released.append((n, t, r[2]))
    arrivals = {}
    for r in arrived_rows:
        if len(r) < 2:
            continue
        n = number(r[1])
        if n is not None and n not in arrivals:
            arrivals[n] = float(r[0])
    refused = [n for n, _, code in released if not code.startswith("2")]
    accepted = [(n, t) for n, t, code in released if code.startswith("2")]
    missing = [n for n, _ in accepted if n not in arrivals]
    late = [n for n, t in accepted if n in arrivals and arrivals[n] - t > LATE_S]
    # The gap: from the last on-time number before the first late or missing one, to the
    # first on-time number after the last of them, by release time.
    off = [i for i, (n, _) in enumerate(accepted) if n in missing or n in late]
    missing_set = set(missing)

    def span(first, last):
        before = accepted[first - 1][1] if first > 0 else accepted[first][1]
        after = accepted[last + 1][1] if last + 1 < len(accepted) else accepted[last][1]
        return round(after - before, 1)

    gap_s = span(off[0], off[-1]) if off else 0.0
    # Each run of consecutive late or lost numbers, and the runs long enough to be a
    # rotation's gap.
    runs = []
    for i in off:
        if runs and runs[-1][-1] == i - 1:
            runs[-1].append(i)
        else:
            runs.append([i])
    gaps, lost_outside, not_gaps = [], [], []
    windows = sorted(rotations)
    used = set()
    # The runs that lost numbers and are long enough to be a rotation's gap.
    lossy = [r for r in runs if any(accepted[i][0] in missing_set for i in r) and span(r[0], r[-1]) >= GAP_MIN_S]
    for r in runs:
        lost = [accepted[i][0] for i in r if accepted[i][0] in missing_set]
        s_ = span(r[0], r[-1])
        rec = {"first": accepted[r[0]][0], "last": accepted[r[-1]][0], "s": s_, "lost": len(lost)}
        # Late and complete is a gap the fabric re-delivered, however short.
        if gap and not lost:
            gaps.append(rec)
            continue
        if not gap:
            lost_outside += lost
            continue
        # A run that lost numbers is a rotation's gap only when it is long enough to be
        # one, the flow recovers after it (an on-time number follows), it is no longer
        # than GAP_MAX_S, and it lies inside a rotated side's window not already used by
        # another gap; and only as many such runs as there are rotated sides.
        lo, hi = accepted[r[0]][1], accepted[r[-1]][1]
        win = next((w for w in windows if w[0] <= lo and hi <= w[1] and w not in used), None)
        why = None
        if s_ < GAP_MIN_S:
            why = f"shorter than {GAP_MIN_S:g} s"
        elif r[-1] + 1 >= len(accepted):
            why = "not followed by an on-time number (the flow did not recover)"
        elif s_ > GAP_MAX_S:
            why = f"longer than {GAP_MAX_S:g} s"
        elif len(lossy) > len(windows):
            why = f"more lossy runs than rotated sides ({len(lossy)} runs, {len(windows)} rotation window(s))"
        elif win is None:
            why = "outside every rotation's window"
        if why:
            lost_outside += lost
            not_gaps.append(dict(rec, why=why))
        else:
            used.add(win)
            gaps.append(rec)
    report = {"released": len(released), "refused_by_sender": refused, "arrived": len(accepted) - len(missing),
              "missing": missing, "late": late, "gap_s": gap_s}
    if gap:
        report["gaps"] = gaps
        report["lost_outside"] = lost_outside
        report["not_gaps"] = not_gaps
    problems = []
    if not released:
        problems.append("no sample was released inside the window")
    if refused:
        problems.append(f"the sending core did not accept {len(refused)} release(s)")
    if missing and not gap:
        problems.append(f"{len(missing)} number(s) never reached the reader")
    if gap and lost_outside:
        problems.append(f"{len(lost_outside)} number(s) lost outside a rotation's gap: "
                        + ", ".join(str(n) for n in lost_outside[:20])
                        + "".join(f"; {g['first']}-{g['last']} ({g['s']} s) is {g['why']}" for g in not_gaps))
    if late and not gap:
        problems.append(f"{len(late)} number(s) arrived more than {LATE_S:g} s late, in a step that has no gap by design")
    report["problems"] = problems
    return not problems, report


def check(args):
    if len(args.sent) != len(args.arrived):
        print("samples: each --sent needs its --arrived", file=sys.stderr)
        return 2
    try:
        flows = [(read_rows(s), read_rows(a), s) for s, a in zip(args.sent, args.arrived)]
    except OSError as e:
        print(f"samples: {e}", file=sys.stderr)
        return 2
    ok_all, why = True, []
    for sent, arrived, name in flows:
        ok, rep = judge(sent, arrived, args.t0, args.t1, args.gap, [tuple(w) for w in args.rotation])
        rep["flow"] = name
        print(json.dumps(rep, sort_keys=True))
        if not ok:
            ok_all = False
            why += [f"{name}: {p}" for p in rep["problems"]]
        elif args.gap and rep["gaps"]:
            for g in rep["gaps"]:
                if g["lost"]:
                    why.append(f"{name}: {g['lost']} number(s) lost in a gap of {g['s']} s ({g['first']}-{g['last']})")
                else:
                    why.append(f"{name}: every number re-delivered after a gap of {g['s']} s")
    print(f"samples: {'PASS' if ok_all else 'FAIL'} {'; '.join(why) or 'every number arrived on time'}")
    return 0 if ok_all else 1


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("record")
    r.add_argument("out")
    r.add_argument("--prefix", default="")
    c = sub.add_parser("check")
    c.add_argument("--sent", action="append", required=True)
    c.add_argument("--arrived", action="append", required=True)
    c.add_argument("--from", dest="t0", type=float, required=True)
    c.add_argument("--to", dest="t1", type=float, required=True)
    c.add_argument("--gap", action="store_true")
    c.add_argument("--rotation", nargs=2, type=float, action="append", default=[], metavar=("START", "END"),
                   help="a rotated side's window; a lossy gap counts only inside one, one gap each")
    a = ap.parse_args(argv)
    if a.cmd == "record":
        record(a.out, a.prefix)
        return 0
    return check(a)


if __name__ == "__main__":
    sys.exit(main())
