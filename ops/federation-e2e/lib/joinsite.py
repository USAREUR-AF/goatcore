#!/usr/bin/env python3
"""joinsite.py — a joining site's declared service set against what join-node's plan does.

    joinsite.py <model.json> <plan.json> <node>

model.json is `endstate.py model <site>`; plan.json is `goatbuilder -o json join-node ...
--dry-run`. For every service the model marks present on <node>, this prints one line:

    built   <service>  by <play>          a command of the plan runs that play on the node
    named   <service>  <the plan's line>  the plan names it under "does not run yet"
    SILENT  <service>                     neither: declared, not built, and not said
    BOTH    <service>                     built and named: the plan contradicts itself

and exits 0 only when there is no SILENT or BOTH line (gate row I3: join-node builds every
declared service, or names it with what closes the gap).

PLAYS is this harness's own record of which play brings each service up on a joining
site's core node. It is kept apart from join-node's BUILDERS table on purpose, so that the
plan is judged against a second account of the same facts; a service missing here reads
as SILENT until someone adds it after reading the play.
"""
import json
import sys

PLAYS = {
    "traefik": ["lan-traefik-routes.yml", "traefik-acme.yml"],
    "netbird-mgmt": ["site-bootstrap-for.yml", "site-bootstrap.yml"],
    "signal-server": ["site-bootstrap-for.yml", "site-bootstrap.yml"],
    "postgres-spock": ["site-bootstrap-for.yml", "site-bootstrap.yml"],
    "pgbackrest": ["site-bootstrap-for.yml", "site-bootstrap.yml"],
    "admit-audit": ["site-bootstrap-for.yml", "site-bootstrap.yml"],
    "ory": ["ory-deploy.yml"],
    "kratos-selfservice-ui": ["ory-deploy.yml"],
    "federation-proxy": ["signal-federation-transport.yml"],
    "zenoh-router": ["zenoh-deploy.yml"],
    "mesh-coredns": ["goat-coredns.yml"],
    "fabric-health": ["fabric-health-deploy.yml"],
    "mesh-health": ["mesh-health-deploy.yml"],
    "presence-health": ["presence-health-deploy.yml"],
    "time-health": ["time-health-deploy.yml"],
    "consistency-health": ["consistency-health-deploy.yml"],
    "fabric-crawler": ["fabric-crawler-deploy.yml"],
}


def main(argv):
    if len(argv) != 4:
        print(__doc__.strip().splitlines()[2].strip(), file=sys.stderr)
        return 2
    with open(argv[1], encoding="utf-8") as fh:
        model = json.load(fh)
    with open(argv[2], encoding="utf-8") as fh:
        plan = json.load(fh)
    node = argv[3]
    declared = sorted(
        name for name, sv in (model.get("services") or {}).items()
        if sv.get("present") is True and node in (sv.get("hosts") or [])
    )
    checks = plan.get("checks") or []
    named = {}
    for c in checks:
        if c.get("name") == "not-run":
            d = c.get("detail") or ""
            svc, sep, _ = d.partition(" — ")
            if sep:
                named[svc] = d
    cmds = [c.get("detail") or "" for c in checks if c.get("name") == "cmd"]

    def built_by(svc):
        for play in PLAYS.get(svc, []):
            for cmd in cmds:
                if play in cmd.split() and (f"-l {node}" in cmd or f"--limit {node}" in cmd or f"-l '{node}" in cmd):
                    return play
        return None

    bad = 0
    if not declared:
        print(f"SILENT  (none)  the model places no service on {node}; nothing to judge")
        return 1
    for svc in declared:
        play, line = built_by(svc), named.get(svc)
        if play and line:
            print(f"BOTH    {svc}  by {play}; and named: {line}")
            bad += 1
        elif play:
            print(f"built   {svc}  by {play}")
        elif line:
            print(f"named   {svc}  {line}")
        else:
            print(f"SILENT  {svc}  declared at {node}, no plan command builds it and the plan does not name it")
            bad += 1
    print(f"{len(declared)} declared at {node}; {bad} silent or contradictory")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
