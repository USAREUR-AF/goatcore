#!/usr/bin/env python3
"""burst — a burst on one lane, and what the receiving edge did with it (ADR 1244).

The receiving edge enforces a lane's agreed rate with one zenoh `downsampling` rule per
lane: at most one message per 1/hz across the whole lane, the rest dropped. A sender that
paces to 1/hz loses nothing to it; one that does not loses a burst's later messages. This
file sends the burst, reads the receiving edge's log, and judges the result for
units/burst.sh.

    burst.py send --base URL --prefix KEY --count N --spacing-ms MS --out SENT
        PUTs N values on one keep-alive connection to URL (the sending core's REST
        surface), MS apart: KEY/b1 .. KEY/bN, each value its own last chunk. Appends one
        line per PUT to SENT: <time, seconds since the epoch> TAB <key> TAB <HTTP code>.

    burst.py edge --log LOG --key-prefix PREFIX [--iface-prefix wg-]
        LOG is the receiving edge router's `docker logs -t`, with the ACL and downsampling
        modules at trace. A burst key (under PREFIX, the wire name) has CROSSED when the
        ACL allows its ingress on the partner link (an interface starting with
        --iface-prefix): zenoh's downsampling runs before that line, so a dropped message
        has none. Each "Message dropped by the downsampling interceptor: Push(KEY)" under
        PREFIX is a drop. Prints one JSON object: crossed, dropped, dropped_keys, the gaps
        between successive crossings (min_gap_ms), out_of_order (crossings whose sample
        number is lower than the one that crossed before it; first_out_of_order names the
        first), and the INFO drop lines in the log.

    burst.py judge --sent SENT --edge EDGE --hz HZ --stored K
                   --queue-full-before X --queue-full-after Y [--overflow --warned 0|1]
        EDGE is `edge`'s JSON, K how many burst values the receiving core stores, X and Y
        the sending republisher's dropped_total{reason="queue_full"} around the burst.
        Either way, a sample that crossed out of put order fails it.
        Paced (the default): PASS only when every PUT was accepted, every sample crossed
        and was stored, the edge dropped none, the sender counted no queue_full drop, and
        no two crossings were closer than 1/hz (less 1 ms for the log's timing).
        --overflow (a burst larger than the sender's queue): PASS only when the edge
        dropped none, everything that crossed was stored, every PUT either crossed or was
        counted as queue_full, and the republisher warned.
        Prints a JSON summary and a last line "burst: PASS|FAIL <why>". Exits 0 or 1.

    burst.py metric < /metrics
        The sum of every goat_fed_republisher_dropped_total series with reason="queue_full";
        0 when there is none.

    burst.py hz --config EDGE-CONFIG --key 'from/<sender>/**'
        The freq of the edge config's downsampling rule for that key expression. Exits 1
        when the config has none.

Exit 2 when an input cannot be read.
"""
import argparse
import http.client
import json
import re
import sys
import time
import urllib.parse

ANSI = re.compile(r"\x1b\[[0-9;]*m")
# `docker logs -t` time, then zenoh's own event time, then the level.
LINE = re.compile(r"^(\S+Z)\s+(?:(\d{4}-\d\d-\d\dT\S+Z)\s+)?(TRACE|DEBUG|INFO|WARN|ERROR)\s+(.*)$")
ALLOW = re.compile(r" on Interface\(([^)]*)\)\S* is authorized to Put \(ingress\) on (\S+)\s*$")
DROP = re.compile(r"Message dropped by the downsampling interceptor: Push\(([^)\s]+)\)")
INFO_DROP = "Some message(s) have been dropped by the downsampling interceptor"
NUMBER = re.compile(r"/b([0-9]+)$")
QUEUE_FULL = re.compile(r'^goat_fed_republisher_dropped_total\{([^}]*)\}\s+([0-9.eE+-]+)\s*$')
TIMING_SLACK_MS = 1.0


def seconds(stamp):
    """2026-11-15T11:49:47.614999123Z -> seconds since the epoch, to the microsecond."""
    day, _, rest = stamp.rstrip("Z").partition("T")
    hms, _, frac = rest.partition(".")
    t = time.strptime(day + "T" + hms, "%Y-%m-%dT%H:%M:%S")
    return time.mktime(t) - time.timezone + float("0." + (frac or "0")[:6])


def edge(log_path, prefix, iface_prefix="wg-"):
    crossed, drops, info = {}, [], 0
    with open(log_path, encoding="utf-8", errors="replace") as fh:
        for raw in fh:
            m = LINE.match(ANSI.sub("", raw.rstrip("\n")))
            if not m:
                continue
            when = seconds(m.group(2) or m.group(1))
            msg = m.group(4)
            if INFO_DROP in msg:
                info += 1
            d = DROP.search(msg)
            if d and d.group(1).startswith(prefix):
                drops.append(d.group(1))
                continue
            a = ALLOW.search(msg)
            if a and a.group(1).startswith(iface_prefix) and a.group(2).startswith(prefix):
                crossed.setdefault(a.group(2), when)
    order = sorted(crossed, key=lambda k: crossed[k])
    times = [crossed[k] for k in order]
    gaps = [round((b - a) * 1000, 1) for a, b in zip(times, times[1:])]
    nums = [int(m.group(1)) if m else 0 for m in (NUMBER.search(k) for k in order)]
    late = [order[i] for i in range(1, len(order)) if nums[i] < nums[i - 1]]
    return {"crossed": len(crossed), "dropped": len(drops), "dropped_keys": drops,
            "min_gap_ms": min(gaps) if gaps else None, "gaps_ms": gaps,
            "out_of_order": len(late), "first_out_of_order": late[0] if late else None,
            "info_drop_lines": info}


def read_sent(path):
    rows = [ln.rstrip("\n").split("\t") for ln in open(path, encoding="utf-8") if ln.strip()]
    return rows, sum(1 for r in rows if len(r) > 2 and r[2].startswith("2"))


def judge(a):
    rows, accepted = read_sent(a.sent)
    with open(a.edge, encoding="utf-8") as fh:
        e = json.load(fh)
    n = len(rows)
    qf = int(round(a.queue_full_after - a.queue_full_before))
    interval = 1000.0 / a.hz if a.hz > 0 else 0.0
    why = []
    if n == 0:
        why.append("nothing was sent")
    if accepted != n:
        why.append(f"the sending core accepted {accepted} of {n} PUTs")
    if e["dropped"]:
        why.append(f"{e['dropped']} dropped at the receiving edge ({', '.join(e['dropped_keys'][:3])}"
                   f"{', ...' if e['dropped'] > 3 else ''})")
    if e.get("out_of_order"):
        why.append(f"{e['out_of_order']} crossed out of put order (first: {e.get('first_out_of_order')})")
    gap = e.get("min_gap_ms")
    if gap is not None and gap < interval - TIMING_SLACK_MS:
        why.append(f"two crossings {gap} ms apart, under 1/hz ({interval:.1f} ms)")
    if a.overflow:
        if e["crossed"] != a.stored:
            why.append(f"{e['crossed']} crossed the edge but {a.stored} stored")
        if e["crossed"] + qf != accepted:
            why.append(f"{accepted - e['crossed'] - qf} sample(s) unaccounted: {accepted} sent, "
                       f"{e['crossed']} crossed, {qf} counted queue_full")
        if qf <= 0:
            why.append("the sender counted no queue_full drop for a burst larger than its queue")
        if not a.warned:
            why.append("no queue-full warning in the republisher's log")
    else:
        if e["crossed"] != n:
            why.append(f"{e['crossed']} of {n} crossed the edge")
        if a.stored != n:
            why.append(f"{a.stored} of {n} stored at the receiving core")
        if qf:
            why.append(f"{qf} queue_full drop(s) counted by the sender")
    summary = {"sent": n, "accepted": accepted, "crossed": e["crossed"], "dropped_at_edge": e["dropped"],
               "stored": a.stored, "queue_full": qf, "min_gap_ms": gap, "interval_ms": round(interval, 1),
               "overflow": a.overflow}
    print(json.dumps(summary))
    if why:
        print("burst: FAIL " + "; ".join(why))
        return 1
    if a.overflow:
        print(f"burst: PASS {e['crossed']} crossed spaced, {qf} queue_full counted and warned, none dropped at the edge")
    else:
        print(f"burst: PASS all {n} crossed in put order and stored, none dropped, smallest gap {gap} ms (1/hz {interval:.1f} ms)")
    return 0


def send(a):
    u = urllib.parse.urlsplit(a.base)
    conn = http.client.HTTPConnection(u.hostname, u.port or 80, timeout=8)
    with open(a.out, "a", encoding="utf-8") as out:
        for i in range(1, a.count + 1):
            key = f"{a.prefix}/b{i}"
            try:
                conn.request("PUT", "/" + key, body=f"b{i}".encode())
                r = conn.getresponse()
                r.read()
                code = str(r.status)
            except (OSError, http.client.HTTPException):
                conn.close()
                conn = http.client.HTTPConnection(u.hostname, u.port or 80, timeout=8)
                code = "000"
            out.write(f"{time.time():.6f}\t{key}\t{code}\n")
            out.flush()
            time.sleep(a.spacing_ms / 1000.0)
    conn.close()
    return 0


def metric(stream):
    total = 0.0
    for ln in stream:
        m = QUEUE_FULL.match(ln.strip())
        if m and re.search(r'(^|,)reason="queue_full"', m.group(1)):
            total += float(m.group(2))
    print(int(total) if total == int(total) else total)
    return 0


def hz(config, key):
    text = open(config, encoding="utf-8").read()
    m = re.search(r'key_expr:\s*"' + re.escape(key) + r'"\s*,\s*freq:\s*([0-9.]+)', text)
    if not m:
        print(f"no downsampling rule for {key} in {config}", file=sys.stderr)
        return 1
    f = float(m.group(1))
    print(int(f) if f == int(f) else f)
    return 0


def main(argv):
    ap = argparse.ArgumentParser(prog="burst.py")
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("send")
    s.add_argument("--base", required=True)
    s.add_argument("--prefix", required=True)
    s.add_argument("--count", type=int, required=True)
    s.add_argument("--spacing-ms", type=float, default=1.0)
    s.add_argument("--out", required=True)
    e = sub.add_parser("edge")
    e.add_argument("--log", required=True)
    e.add_argument("--key-prefix", required=True)
    e.add_argument("--iface-prefix", default="wg-")
    j = sub.add_parser("judge")
    j.add_argument("--sent", required=True)
    j.add_argument("--edge", required=True)
    j.add_argument("--hz", type=float, required=True)
    j.add_argument("--stored", type=int, required=True)
    j.add_argument("--queue-full-before", type=float, required=True)
    j.add_argument("--queue-full-after", type=float, required=True)
    j.add_argument("--overflow", action="store_true")
    j.add_argument("--warned", type=int, default=0)
    sub.add_parser("metric")
    h = sub.add_parser("hz")
    h.add_argument("--config", required=True)
    h.add_argument("--key", required=True)
    a = ap.parse_args(argv)
    try:
        if a.cmd == "send":
            return send(a)
        if a.cmd == "edge":
            print(json.dumps(edge(a.log, a.key_prefix, a.iface_prefix)))
            return 0
        if a.cmd == "judge":
            return judge(a)
        if a.cmd == "metric":
            return metric(sys.stdin)
        return hz(a.config, a.key)
    except (OSError, ValueError, KeyError) as err:
        print(f"burst.py {a.cmd}: {err}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
