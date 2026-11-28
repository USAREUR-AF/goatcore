#!/usr/bin/env python3
"""How much of the mesh negotiates post-quantum key exchange.

READ-ONLY. Changes nothing, ever. Safe on a timer.

WHAT THIS MEASURES AND WHY IT IS PERMANENT. Rosenpass is a per-peer client
setting that survives upgrades and re-enrolment, so the share of the fleet
carrying it is a standing property of the goatnet, not a fact about any one
remediation. It will matter again the next time the setting is turned on
deliberately (the eventual PQC rollout), turned on by accident (a stale doc, a
copied command), or needs to be proven off for an audit.

Deliberately NOT modelled here: campaigns. An earlier version of this file
hardcoded a mailing cohort, a peer count "at the notice" and a deadline, and
exported `goat_rosenpass_peers_cleared_since_notice`. Every one of those is a
timestamp pretending to be a fact — correct for about a day, then quietly wrong
forever, on a dashboard nobody would think to re-read. Trend belongs in the
time-series database, which already stores it: `delta(goat_rosenpass_peers_enabled[24h])`
answers "how is this moving" for any window, at any time, without this script
remembering anything.

WHY PEER COUNT AND NOT BYTES. Attribution by traffic is not available: the
signal server exposes no byte-size metric, and the message body is an opaque
NaCl box, so no label distinguishing post-quantum traffic can exist. Peer count
is the one Rosenpass fact that is counted rather than inferred, which is why it
is the one published continuously.

usage:
  rosenpass-compliance-report.py                 # summary to stdout
  rosenpass-compliance-report.py --textfile PATH # prometheus textfile
  rosenpass-compliance-report.py --json          # machine-readable

Run ON the mgmt host (needs docker).
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time

NAMED = re.compile(r"^peer-(.+?)-[a-z0-9-]+-\d{4}-\d{2}-\d{2}[a-z]?-(.*)$")


def sh(*args):
    return subprocess.run(args, capture_output=True, text=True).stdout


def psql(db, sql):
    """Query via a temp file, formatted with FLAGS.

    Two traps, both of which return a wrong answer rather than an error:
    `\\pset fieldsep '|'` makes psql echo `Field separator is "|".` — a line
    containing the separator, which splits wrong and breaks every consumer. And
    mktemp yields 0600, which the container's postgres uid cannot read, so psql
    returns nothing — indistinguishable from "no peer has it enabled", the most
    dangerous possible wrong answer here.
    """
    fd, path = tempfile.mkstemp(suffix=".sql")
    os.write(fd, sql.encode())
    os.close(fd)
    os.chmod(path, 0o644)
    sh("sudo", "docker", "cp", path, "postgres-a:/tmp/rpc.sql")
    out = sh("sudo", "docker", "exec", "postgres-a", "psql", "-U", "postgres",
             "-d", db, "-A", "-t", "-F", "|", "-f", "/tmp/rpc.sql")
    os.unlink(path)
    return [r for r in (l.split("|") for l in out.splitlines() if "|" in l)
            if len(r) >= 3]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--textfile")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()

    # NetBird 0.83.0 stopped writing peers.peer_status_connected: connected is
    # now a peer_connections row held by a management instance that heartbeated
    # within 5 minutes. Both tables are site-local (ADR 1084 D6), so this counts
    # the peers connected to THIS site's management. The \if branch serves a
    # database before 0.83.0.
    rows = psql("netbird", """
SELECT to_regclass('public.peer_connections') IS NOT NULL AS has_pc \\gset
\\if :has_pc
SELECT p.name,
       CASE WHEN mi.last_seen > now() - interval '5 minutes' THEN 'up' ELSE 'down' END,
       COALESCE(p.meta_flags::json->>'RosenpassEnabled','false')
FROM peers p
LEFT JOIN peer_connections pc ON pc.peer_id = p.id
LEFT JOIN management_instances mi ON mi.id = pc.instance_id;
\\else
SELECT name,
       CASE WHEN peer_status_connected THEN 'up' ELSE 'down' END,
       COALESCE(meta_flags::json->>'RosenpassEnabled','false')
FROM peers;
\\endif
""")
    if not rows:
        print("FAILED to read the peer table. This is NOT an all-clear.",
              file=sys.stderr)
        return 2

    total = len(rows)
    total_up = len([r for r in rows if r[1] == "up"])
    on = [r for r in rows if r[2] == "true"]
    on_up = [r for r in on if r[1] == "up"]
    owners = set()
    for r in on:
        m = NAMED.match(r[0] or "")
        owners.add(m.group(1) if m else "\x00" + (r[0] or "?"))

    data = {
        "generated": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "peers_total": total,
        "peers_total_connected": total_up,
        "peers_enabled": len(on),
        "peers_enabled_connected": len(on_up),
        "owners": len(owners),
        "share": (len(on) / total) if total else None,
    }

    if a.textfile:
        lines = [
            "# HELP goat_rosenpass_peers_enabled Peers whose client negotiates Rosenpass post-quantum key exchange.\n",
            "# TYPE goat_rosenpass_peers_enabled gauge\n",
            "goat_rosenpass_peers_enabled %d\n" % len(on),
            "# HELP goat_rosenpass_peers_enabled_connected Of those, currently connected.\n",
            "# TYPE goat_rosenpass_peers_enabled_connected gauge\n",
            "goat_rosenpass_peers_enabled_connected %d\n" % len(on_up),
            "# HELP goat_rosenpass_owners Distinct peer owners holding at least one such peer.\n",
            "# TYPE goat_rosenpass_owners gauge\n",
            "goat_rosenpass_owners %d\n" % len(owners),
            # The denominator, published alongside, so share is computable in a
            # query rather than baked in here. A percentage with no visible
            # denominator is the shape of number this codebase keeps having to
            # un-learn.
            "# HELP goat_rosenpass_peers_known Peers in the management database — the denominator.\n",
            "# TYPE goat_rosenpass_peers_known gauge\n",
            "goat_rosenpass_peers_known %d\n" % total,
            "# HELP goat_rosenpass_peers_known_connected Connected peers — the denominator for the connected share.\n",
            "# TYPE goat_rosenpass_peers_known_connected gauge\n",
            "goat_rosenpass_peers_known_connected %d\n" % total_up,
        ]
        tmp = a.textfile + ".tmp"
        with open(tmp, "w") as fh:
            fh.writelines(lines)
        os.replace(tmp, a.textfile)

    if a.json:
        print(json.dumps(data, indent=1))
        return 0

    pct = ("%.1f%%" % (100 * data["share"])) if data["share"] is not None else "—"
    print("Post-quantum key exchange — %s" % data["generated"])
    print("  peers negotiating PQC     %d of %d known peers (%s)"
          % (len(on), total, pct))
    print("    connected now           %d of %d" % (len(on_up), total_up))
    print("  distinct owners           %d" % len(owners))
    print()
    print("  Trend is not computed here on purpose — Prometheus already stores it:")
    print("    delta(goat_rosenpass_peers_enabled[24h])")
    return 0


if __name__ == "__main__":
    sys.exit(main())
