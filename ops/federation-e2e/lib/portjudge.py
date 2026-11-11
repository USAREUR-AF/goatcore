#!/usr/bin/env python3
"""portjudge — compare one port scan with the ports a node may answer on.

    portjudge.py --scan SCAN.json --counters NFT.txt --expect EXPECT.json --label TEXT --out EVIDENCE.json

SCAN.json is portscan.py's output. NFT.txt is the target's `nft list table inet fe2e_scan`
taken right after the scan: one counter per probed UDP port, counting the datagrams from
the scanning source that got past every other filter on the target (input and forward
hooks, priority 500, after any firewall's own chains). EXPECT.json:

  {"declared": {"tcp": [..], "udp": [..]},        the node's declared profile
   "spire":    {"tcp": [..], "udp": [..]},        SPIRE federation forwards the ref's peering roles open
   "drill":    {"tcp": [..], "udp": [..]},        surfaces only this harness runs
   "source": "where the declared set came from", "notes": ["..."]}

A TCP port answers when it completed a handshake. A UDP port answers when the scan got a
reply, or when its datagrams were delivered past every filter and no ICMP port
unreachable came back. Pass: the ports that answer are exactly declared + spire + drill,
every one of them answers, and nothing else does. Prints the open list per protocol and
what is extra or missing; writes the whole record to EVIDENCE.json; exits 0 on pass.
"""
import argparse
import json
import re
import sys


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--scan", required=True)
    ap.add_argument("--counters", required=True)
    ap.add_argument("--expect", required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    try:
        with open(a.scan, encoding="utf-8") as fh:
            scan = json.loads(fh.read().strip().splitlines()[-1])
    except Exception as e:  # the scan did not run or printed no result
        print(f"{a.label}: no scan result in {a.scan}: {e}")
        return 1
    with open(a.expect, encoding="utf-8") as fh:
        exp = json.load(fh)
    delivered = {}
    ok = False
    try:
        with open(a.counters, encoding="utf-8") as fh:
            for m in re.finditer(r"(?:dport|proto-dst) (\d+) counter packets (\d+)", fh.read()):
                delivered[m.group(1)] = delivered.get(m.group(1), 0) + int(m.group(2))
    except OSError:
        # No counters file: every UDP port is judged by its probe alone.
        pass
    udp_open, udp_state = [], {}
    for port, how in scan.get("udp", {}).items():
        d = delivered.get(port, 0)
        if how == "answered" or (how == "silent" and d > 0):
            state = "open"
            udp_open.append(int(port))
        elif how == "refused":
            state = "closed (ICMP port unreachable)"
        else:
            state = "filtered (not delivered, no reply)"
        udp_state[port] = {"probe": how, "delivered": d, "state": state}
    udp_open.sort()
    tcp_open = sorted(scan.get("tcp_open", []))

    def want(proto):
        out = set()
        for k in ("declared", "spire", "drill"):
            out |= set(int(p) for p in (exp.get(k) or {}).get(proto, []))
        return out

    extra_tcp = sorted(set(tcp_open) - want("tcp"))
    extra_udp = sorted(set(udp_open) - want("udp"))
    miss_tcp = sorted(want("tcp") - set(tcp_open))
    miss_udp = sorted(want("udp") - set(udp_open))
    errs = scan.get("tcp_errors") or []
    ok = not (extra_tcp or extra_udp or miss_tcp or miss_udp or errs)
    rec = {"label": a.label, "target": scan.get("target"), "tcp_range": scan.get("tcp_range"),
           "tcp_open": tcp_open, "tcp_closed": scan.get("tcp_closed"), "tcp_silent": scan.get("tcp_silent"),
           "udp_open": udp_open, "udp": udp_state, "expected": exp,
           "extra": {"tcp": extra_tcp, "udp": extra_udp}, "missing": {"tcp": miss_tcp, "udp": miss_udp},
           "outcome": "pass" if ok else "fail", "seconds": scan.get("seconds")}
    with open(a.out, "w", encoding="utf-8") as fh:
        json.dump(rec, fh, indent=2)
        fh.write("\n")
    print(f"{a.label}: target {scan.get('target')}, tcp {scan.get('tcp_range')} and udp {','.join(scan.get('udp', {}))}")
    print(f"  answers tcp {tcp_open or 'none'}  udp {udp_open or 'none'}")
    print(f"  declared ({exp.get('source', '?')}): tcp {exp['declared'].get('tcp', [])} udp {exp['declared'].get('udp', [])}")
    for k, what in (("spire", "SPIRE federation forwards the ref's peering roles open"), ("drill", "drill additions, harness only")):
        v = exp.get(k) or {}
        if v.get("tcp") or v.get("udp"):
            print(f"  {what}: tcp {v.get('tcp', [])} udp {v.get('udp', [])}")
    for n in exp.get("notes", []):
        print(f"  note: {n}")
    if extra_tcp or extra_udp:
        print(f"  EXTRA, answering beyond the declared set: tcp {extra_tcp} udp {extra_udp}")
    if miss_tcp or miss_udp:
        print(f"  MISSING, declared but not answering: tcp {miss_tcp} udp {miss_udp}")
    if errs:
        print(f"  INCOMPLETE: the scanner could not try {len(errs)} TCP ports ({scan.get('tcp_error')})")
    print("  " + ("exactly the declared ports" if ok else "not exactly the declared ports"))
    # One line a unit can lift into the check's reason.
    parts = []
    if errs:
        parts.append(f"scan incomplete: {len(errs)} TCP ports not tried ({scan.get('tcp_error')})")
    if extra_tcp or extra_udp:
        parts.append(f"answers beyond the declared set: tcp {extra_tcp} udp {extra_udp}")
    if miss_tcp or miss_udp:
        parts.append(f"declared but not answering: tcp {miss_tcp} udp {miss_udp}")
    for k in ("drill", "spire"):
        v = exp.get(k) or {}
        if v.get("tcp") or v.get("udp"):
            parts.append(f"{exp.get(k + '_why', k)} allowed and listed separately: tcp {v.get('tcp', [])} udp {v.get('udp', [])}")
    print("reason: " + ("; ".join(parts) or "exactly the declared ports"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
