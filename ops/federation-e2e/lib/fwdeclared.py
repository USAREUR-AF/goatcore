#!/usr/bin/env python3
"""fwdeclared — the ports a loaded goat-host-firewall ruleset admits from one source.

    fwdeclared.py --ruleset RULESET.nft --saddr A --iif IF

RULESET.nft is the file the ref's goat-host-firewall role rendered and loaded on a host
(/etc/goat-host-firewall/ruleset.nft, written by the role's own renderer,
roles/goat-host-firewall/files/goat-host-firewall-render.py). Its accept rules in
`chain input` have one shape:

    [iifname "<if>"] [ip|ip6 saddr { <cidr>, ... }] tcp|udp dport { <port>, ... } accept comment "<rule>"

A rule admits a new connection from A arriving on IF when its interface is absent or
matches IF (nftables wildcards), and its sources are absent or hold A. Fixed-expression
rules (loopback, established, ICMP, DHCP replies, container bridges) admit nothing a scan
from another host's address opens, and are not counted. The `published` chain holds the
same rules for published container ports, so the input chain is the whole declaration.

Prints one JSON object: {"tcp": [..], "udp": [..], "by_port": {"tcp/22": ["ssh-management"]},
"rules": [every accept rule parsed], "changes": [the renderer's "# tightened"/"# loosened"
lines, with their reasons], "sha256": the renderer's own hash line}. Exits 1 when the file
holds no goat-host-firewall table.
"""
import argparse
import fnmatch
import ipaddress
import json
import re
import sys

RULE = re.compile(
    r'^\s*(?:iifname "(?P<iif>[^"]+)"\s+)?'
    r'(?:(?P<fam>ip6?) saddr (?:\{ (?P<srcs>[^}]+) \}|(?P<src>\S+))\s+)?'
    r'(?P<proto>tcp|udp) dport (?:\{ (?P<ports>[^}]+) \}|(?P<port>[0-9-]+))\s+'
    r'accept comment "(?P<name>[^"]+)"\s*$')


def ports_of(text):
    out = []
    for p in text.split(","):
        p = p.strip()
        if "-" in p:
            lo, hi = p.split("-", 1)
            out.extend(range(int(lo), int(hi) + 1))
        elif p:
            out.append(int(p))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ruleset", required=True)
    ap.add_argument("--saddr", required=True)
    ap.add_argument("--iif", default="")
    a = ap.parse_args()
    text = open(a.ruleset, encoding="utf-8").read()
    if "table inet goat_host_firewall {" not in text:
        print(f"{a.ruleset}: no table inet goat_host_firewall", file=sys.stderr)
        return 1
    saddr = ipaddress.ip_address(a.saddr)
    rules, admitted = [], {"tcp": set(), "udp": set()}
    by_port = {}
    in_input = False
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("chain "):
            in_input = s.startswith("chain input ")
            continue
        if not in_input:
            continue
        m = RULE.match(line)
        if not m:
            continue
        srcs = [x.strip() for x in (m.group("srcs") or m.group("src") or "").split(",") if x.strip()]
        r = {"name": m.group("name"), "proto": m.group("proto"), "interface": m.group("iif"),
             "family": m.group("fam"), "sources": srcs,
             "ports": ports_of(m.group("ports") or m.group("port"))}
        rules.append(r)
        if r["interface"] and not fnmatch.fnmatchcase(a.iif, r["interface"]):
            continue
        if r["family"] and (r["family"] == "ip6") != (saddr.version == 6):
            continue
        if srcs and not any(saddr in ipaddress.ip_network(x, strict=False) for x in srcs):
            continue
        for p in r["ports"]:
            admitted[r["proto"]].add(p)
            by_port.setdefault(f"{r['proto']}/{p}", []).append(r["name"])
    changes = [ln[2:].strip() for ln in text.splitlines() if re.match(r"^# (tightened|loosened) ", ln)]
    sha = re.search(r"^# sha256 ([0-9a-f]{64})$", text, re.M)
    print(json.dumps({"tcp": sorted(admitted["tcp"]), "udp": sorted(admitted["udp"]), "by_port": by_port,
                      "rules": rules, "changes": changes, "sha256": sha.group(1) if sha else ""}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
