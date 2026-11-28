#!/usr/bin/env python3
"""Turn the raw output of run-matrix.sh into per-run rows and result tables.

    summarize.py [--csv runs.csv] <placement dir> [<placement dir> ...]

Each placement dir is one run-matrix.sh output directory. Prints Markdown:
one table per transport, a memory table, and a list of runs whose measured
transport or TLS path did not match the configuration (those runs are left
out of the tables). Every number comes from the files in the given dirs.
"""
import csv
import json
import os
import re
import statistics
import sys

TRANSPORT_TITLE = {
    ("ws", "kernel"): "WebSocket over TLS, kernel TLS (kTLS)",
    ("ws", "user"): "WebSocket over TLS, TLS in user space",
    ("quic", "user"): "QUIC",
}
RELAY_NAME = {"goat": "goat-relay", "go": "NetBird relay (Go)"}
CLIENT_NAME = {"stock": "stock", "pr6774": "with #6774"}


def read_snmp(path):
    """Return {(table, field): int} from /proc/net/snmp + netstat dumps."""
    vals = {}
    with open(path) as f:
        lines = f.read().split("--- tls_stat")[0].splitlines()
    for i in range(0, len(lines) - 1, 2):
        h, v = lines[i].split(), lines[i + 1].split()
        if not h or h[0] != v[0]:
            continue
        table = h[0].rstrip(":")
        for k, x in zip(h[1:], v[1:]):
            try:
                vals[(table, k)] = int(x)
            except ValueError:
                pass
    return vals


def read_tls(path):
    with open(path) as f:
        text = f.read()
    out = {}
    if "--- tls_stat" in text:
        part = text.split("--- tls_stat", 1)[1].split("--- tls_path", 1)[0]
        for line in part.splitlines():
            m = re.match(r"(\w+)\s+(\d+)", line.strip())
            if m:
                out[m.group(1)] = int(m.group(2))
    m = re.search(r"tls_module_loaded=(\w+)", text)
    out["module"] = m.group(1) if m else "?"
    return out


def read_samples(path):
    clk = 100
    rows = []
    with open(path) as f:
        for line in f:
            if line.startswith("#"):
                m = re.search(r"clk_tck=(\d+)", line)
                if m:
                    clk = int(m.group(1))
                continue
            if line.startswith("t_ms") or not line.strip():
                continue
            rows.append([int(x) for x in line.strip().split(",")])
    return clk, rows


def vm(path, key):
    try:
        with open(path) as f:
            for line in f:
                if line.startswith(key):
                    return int(line.split()[1])
    except OSError:
        pass
    return None


def one_test(fd, config):
    relay, mode, transport, client = config
    try:
        with open(os.path.join(fd, "iperf.json")) as f:
            ip = json.load(f)
    except (OSError, ValueError):
        return None, "iperf output missing or unreadable"
    if "error" in ip:
        return None, f"iperf error: {ip['error']}"
    end = ip["end"]
    mbps = end["sum_received"]["bits_per_second"] / 1e6
    retr = end.get("sum_sent", {}).get("retransmits")
    t0 = ip["start"]["timestamp"]["timesecs"]
    omit = ip["start"]["test_start"].get("omit", 0)
    dur = ip["start"]["test_start"]["duration"]
    lo, hi = (t0 + omit) * 1000, (t0 + omit + dur) * 1000
    clk, rows = read_samples(os.path.join(fd, "relay-samples.csv"))
    win = [r for r in rows if lo <= r[0] <= hi + 1500]
    cpu = host = None
    if len(win) >= 2:
        a, b = win[0], win[-1]
        secs = (b[0] - a[0]) / 1000
        cpu = (b[1] - a[1]) / clk / secs * 100
        host = (b[4] - a[4]) / max(1, b[5] - a[5]) * 100
    rss_max = max((r[2] for r in rows), default=None)
    before = read_snmp(os.path.join(fd, "relay-counters-before.txt"))
    after = read_snmp(os.path.join(fd, "relay-counters-after.txt"))
    d_udp = after.get(("Udp", "InDatagrams"), 0) - before.get(("Udp", "InDatagrams"), 0)
    d_tcp = after.get(("Tcp", "InSegs"), 0) - before.get(("Tcp", "InSegs"), 0)
    carried = "quic" if d_udp > 10 * d_tcp else ("ws" if d_tcp > 10 * d_udp else "mixed")
    tls = read_tls(os.path.join(fd, "relay-counters-before.txt"))
    ktls_live = min(tls.get("TlsCurrTxSw", 0), tls.get("TlsCurrRxSw", 0))
    if transport == "quic":
        path = "user"
    else:
        path = "kernel" if ktls_live >= 2 else "user"
    row = dict(relay=relay, mode=mode, transport=transport, client=client,
               mbps=mbps, retransmits=retr, relay_cpu_pct=cpu, host_cpu_pct=host,
               relay_rss_max_kib=rss_max, udp_in=d_udp, tcp_in=d_tcp, carried=carried,
               tls_path=path, ktls_sessions=ktls_live, tls_module=tls.get("module"))
    problems = []
    if carried != transport:
        problems.append(f"relay counters show {carried}, configured {transport}")
    if transport == "ws" and mode == "ktls" and relay == "goat" and path != "kernel":
        problems.append("configured kTLS but no kernel TLS session was live")
    if transport == "ws" and mode == "user" and path != "user":
        problems.append("configured user-space TLS but kernel TLS sessions were live")
    return row, "; ".join(problems)


def walk(placements):
    rows, bad, mem = [], [], []
    for pi, pdir in enumerate(placements, 1):
        pname = os.path.basename(os.path.normpath(pdir))
        for cid in sorted(os.listdir(pdir)):
            cdir = os.path.join(pdir, cid)
            parts = cid.split("-")
            if not os.path.isdir(cdir) or len(parts) != 4 or parts[0] not in RELAY_NAME:
                continue
            config = tuple(parts)
            for rname in sorted(os.listdir(cdir)):
                rdir = os.path.join(cdir, rname)
                if not rname.startswith("r"):
                    continue
                if os.path.exists(os.path.join(rdir, "FAILED")):
                    bad.append((pname, cid, rname, "-", "peers did not connect through the relay"))
                    continue
                mem.append(dict(placement=pname, config=config, run=rname,
                                no_peers=vm(os.path.join(rdir, "relay-mem-no-peers.txt"), "VmRSS"),
                                idle=vm(os.path.join(rdir, "relay-mem-idle-2-peers.txt"), "VmRSS")))
                for fname in sorted(os.listdir(rdir)):
                    if not re.fullmatch(r"f\d+", fname):
                        continue
                    row, problem = one_test(os.path.join(rdir, fname), config)
                    if row is None or problem:
                        bad.append((pname, cid, rname, fname, problem))
                        if row is None:
                            continue
                    row.update(placement=pname, run=rname, flows=int(fname[1:]), excluded=bool(problem))
                    rows.append(row)
    return rows, bad, mem


def fmt(x, nd=0):
    return "n/a" if x is None else f"{x:.{nd}f}"


def cell(vals, nd=0):
    vals = [v for v in vals if v is not None]
    if not vals:
        return "n/a"
    med = statistics.median(vals)
    return f"{med:.{nd}f} ({min(vals):.{nd}f}–{max(vals):.{nd}f}, n={len(vals)})"


def tables(rows, mem, placements):
    good = [r for r in rows if not r["excluded"]]
    out = []
    keys = [("ws", "kernel"), ("ws", "user"), ("quic", "user")]
    for tkey in keys:
        sel = [r for r in good if (r["transport"], r["tls_path"]) == tkey]
        out.append(f"### {TRANSPORT_TITLE[tkey]}\n")
        if not sel:
            out.append("No runs.\n")
            continue
        out.append("Mbps and relay CPU: median (lowest–highest, number of runs). "
                   "Relay CPU is the relay process's CPU time during the test as a "
                   "percentage of one core.\n")
        out.append("| Relay | Client | Flows | Mbps | Relay CPU % | Mbps per relay core |")
        out.append("|---|---|---|---|---|---|")
        for relay in ("goat", "go"):
            for client in ("stock", "pr6774"):
                for flows in (1, 4, 8, 16):
                    rs = [r for r in sel if r["relay"] == relay and r["client"] == client and r["flows"] == flows]
                    if not rs:
                        continue
                    eff = [r["mbps"] / (r["relay_cpu_pct"] / 100) for r in rs if r["relay_cpu_pct"]]
                    out.append(f"| {RELAY_NAME[relay]} | {CLIENT_NAME[client]} | {flows} | "
                               f"{cell([r['mbps'] for r in rs])} | {cell([r['relay_cpu_pct'] for r in rs])} | "
                               f"{cell(eff)} |")
        if len(placements) > 1:
            out.append("\nMedian Mbps per placement, 16 flows:\n")
            names = [os.path.basename(os.path.normpath(p)) for p in placements]
            out.append("| Relay | Client | " + " | ".join(names) + " |")
            out.append("|---|---|" + "---|" * len(names))
            for relay in ("goat", "go"):
                for client in ("stock", "pr6774"):
                    vals = []
                    for n in names:
                        rs = [r["mbps"] for r in sel if r["relay"] == relay and r["client"] == client
                              and r["flows"] == 16 and r["placement"] == n]
                        vals.append(fmt(statistics.median(rs)) if rs else "n/a")
                    if any(v != "n/a" for v in vals):
                        out.append(f"| {RELAY_NAME[relay]} | {CLIENT_NAME[client]} | " + " | ".join(vals) + " |")
        out.append("")
    out.append("### Relay memory (resident set, MiB)\n")
    out.append("| Relay | Transport | TLS | Client | Started, no peers | Two peers connected, idle | Highest under 16 flows |")
    out.append("|---|---|---|---|---|---|---|")
    configs = sorted({m["config"] for m in mem})
    for cfg in configs:
        relay, mode, transport, client = cfg
        ms = [m for m in mem if m["config"] == cfg]
        hi = [r["relay_rss_max_kib"] for r in good
              if (r["relay"], r["mode"], r["transport"], r["client"]) == cfg and r["flows"] == 16]
        tls = "kTLS" if (transport == "ws" and mode == "ktls" and relay == "goat") else "user space"
        out.append(f"| {RELAY_NAME[relay]} | {transport.upper() if transport == 'quic' else 'WebSocket'} | {tls} | "
                   f"{CLIENT_NAME[client]} | {cell([m['no_peers'] / 1024 for m in ms if m['no_peers']], 1)} | "
                   f"{cell([m['idle'] / 1024 for m in ms if m['idle']], 1)} | "
                   f"{cell([h / 1024 for h in hi if h], 1)} |")
    return "\n".join(out)


def main(argv):
    csv_path = None
    if len(argv) > 2 and argv[1] == "--csv":
        csv_path = argv[2]
        argv = argv[:1] + argv[3:]
    placements = argv[1:]
    if not placements:
        print(__doc__, file=sys.stderr)
        return 2
    rows, bad, mem = walk(placements)
    if csv_path:
        cols = ["placement", "relay", "mode", "transport", "client", "run", "flows", "mbps",
                "retransmits", "relay_cpu_pct", "host_cpu_pct", "relay_rss_max_kib", "udp_in",
                "tcp_in", "carried", "tls_path", "ktls_sessions", "tls_module", "excluded"]
        with open(csv_path, "w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=cols)
            w.writeheader()
            for r in sorted(rows, key=lambda r: (r["placement"], r["relay"], r["mode"], r["transport"],
                                                 r["client"], r["run"], r["flows"])):
                w.writerow({k: (f"{r[k]:.2f}" if isinstance(r[k], float) else r[k]) for k in cols})
    print(tables(rows, mem, placements))
    if bad:
        print("\n### Runs left out\n")
        for b in bad:
            print(f"- {b[0]} {b[1]} {b[2]} {b[3]}: {b[4]}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
