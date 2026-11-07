#!/usr/bin/env python3
"""Helpers for the island-mode partition harness (run.sh in this directory).

Every input is read from the file that owns it, never copied:
  - repset table lists: ops/ansible/roles/spock-mesh-peering/defaults/main.yml
    (spock_mesh_netbird_mgmt_tables) and ops/ansible/roles/ory-narrow-repset/
    defaults/main.yml (ory_spock_inclusion_tables)
  - image pins: ops/pins/fleet-pins.yaml (netbird-mgmt) and the Ory role
    default version, digest from the goatnet that digest-pins it
  - scenario names: docs/design/island-mode-write-inventory.md §10

Subcommands:
  pins                       shell assignments NB_IMAGE, KRATOS_IMAGE, HYDRA_IMAGE
  tables <db>                replicated tables of netbird | kratos | hydra
  drift-env <out> <portA> <portB> <pwvar>
                             env file for ops/ansible/roles/spock-drift-watch
  account <expect> <dump>    match spock.exception_log / spock.resolutions rows
                             against a scenario's expected set
  coverage <results.tsv>     coverage report; exit 1 on any gate failure
"""
import json
import re
import shlex
import sys
from collections import Counter
from pathlib import Path

import yaml

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[4]
ROLES = REPO / "ops/ansible/roles"
INVENTORY = REPO / "docs/design/island-mode-write-inventory.md"
SCENARIOS = HERE / "scenarios"

# Mirrors the spock-drift-watch defaults: compare all replicated tables of
# these three databases.
DATABASES = ("netbird", "kratos", "hydra")
REPSET = {"netbird": "netbird_mgmt", "kratos": "ory_narrow", "hydra": "ory_narrow"}


def load(path):
    return yaml.safe_load(path.read_text())


def tables(db):
    if db == "netbird":
        return list(load(ROLES / "spock-mesh-peering/defaults/main.yml")["spock_mesh_netbird_mgmt_tables"])
    inc = load(ROLES / "ory-narrow-repset/defaults/main.yml")["ory_spock_inclusion_tables"][db]
    return [t.split(".", 1)[1] if "." in t else t for t in inc]


def pins():
    fleet = load(REPO / "ops/pins/fleet-pins.yaml")["components"]["netbird-mgmt"]["fleet"]
    nb = fleet.replace("{{ ghcr_org }}", "usareur-af")
    ory = load(ROLES / "ory/defaults/main.yml")
    out = {"NB_IMAGE": nb}
    # The Ory role default is the fleet version (a tag). Take its digest from
    # the goatnet that pins one, and refuse if that goatnet runs another tag.
    # A checkout that declares no goatnet (this repository on its own) has no
    # such file; the role defaults' tags are then the images.
    pinned_file = REPO / "ops/ansible/group_vars/zagros_dev_hosts/vars.yml"
    pinned = pinned_file.read_text() if pinned_file.is_file() else ""
    for var, key in (("ory_kratos_image", "KRATOS_IMAGE"), ("ory_hydra_image", "HYDRA_IMAGE")):
        tag = ory[var]
        m = re.search(rf'^{var}:\s*"([^"]+)"', pinned, re.M)
        if m and m.group(1).split("@")[0] == tag:
            out[key] = m.group(1)
        else:
            out[key] = tag
    for k, v in out.items():
        print(f"{k}='{v}'")


def drift_env(out, port_a, port_b, pwvar):
    peers = [
        {"name": f"site_{s}", "host": "127.0.0.1", "port": int(p), "dbname": "postgres",
         "user": "replicator", "password_env": pwvar}
        for s, p in (("a", port_a), ("b", port_b))
    ]
    where = {(t["db"], t["table"]): t["where"]
             for t in load(ROLES / "spock-drift-watch/defaults/main.yml")["spock_drift_watch_tables"] if "where" in t}
    tabs = [{"db": db, "table": t, **({"where": where[(db, t)]} if (db, t) in where else {})}
            for db in DATABASES for t in tables(db)]
    Path(out).write_text(
        "SPOCK_DRIFT_SITE=harness\n"
        f"SPOCK_DRIFT_TEXTFILE={Path(out).with_suffix('.prom')}\n"
        f"SPOCK_DRIFT_PEERS={shlex.quote(json.dumps(peers))}\n"
        f"SPOCK_DRIFT_TABLES={shlex.quote(json.dumps(tabs))}\n"
    )


def _kind(row, source):
    """One observed row as '<table> <type>'."""
    if source == "resolution":
        table = row.get("relname") or row.get("table_name") or "?"
        typ = (row.get("conflict_type") or "?").lower()
        res = (row.get("conflict_resolution") or "").lower()
        typ = f"{typ}:{res}" if res else typ
    else:
        table = row.get("table_name") or row.get("relname") or "?"
        op = (row.get("operation") or "?").upper()
        msg = (row.get("error_message") or "").lower()
        if "duplicate key" in msg or "unique" in msg:
            cls = "unique"
        elif "foreign key" in msg:
            cls = "fk"
        elif "insert-only" in msg:
            cls = "ledger"
        elif not msg:
            cls = "none"
        else:
            cls = "other"
        typ = f"{op}:{cls}"
    return f"{str(table).split('.')[-1]} {typ}"


def account(expect_path, dump_path):
    """Exit 0 when the observed rows are exactly the expected multiset."""
    want = Counter()
    for line in Path(expect_path).read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        *key, n = line.split()
        want[" ".join(key)] += int(n)
    got = Counter()
    detail = []
    for line in Path(dump_path).read_text().splitlines():
        if not line.strip():
            continue
        site, db, source, payload = line.split(" ", 3)
        for row in json.loads(payload) or []:
            k = f"{site} {db} {source} {_kind(row, source)}"
            got[k] += 1
            msg = row.get("error_message") or row.get("conflict_resolution") or ""
            detail.append(f"{k} :: {str(msg)[:200]}")
    ok = True
    for k in sorted(set(want) | set(got)):
        if want[k] != got[k]:
            ok = False
            print(f"    {'unexpected' if got[k] > want[k] else 'missing'}: {k} {got[k]} (expected {want[k]})")
    if not ok:
        print("    observed rows (paste the ones the scenario accounts for into its EXPECT):")
        for k, n in sorted(got.items()):
            print(f"      {k} {n}")
        for d in detail:
            print(f"      | {d}")
    sys.exit(0 if ok else 1)


def section10_names():
    text = INVENTORY.read_text()
    body = text.split("## 10. Harness scenarios", 1)[1].split("\n## ", 1)[0]
    return [m.group(1) for m in re.finditer(r"^\| `([a-z0-9-]+)` \|", body, re.M)]


def read_tsv(path):
    rows = {}
    if path.exists():
        for line in path.read_text().splitlines():
            if line.strip() and not line.startswith("#"):
                name, _, reason = line.partition("\t")
                rows[name.strip()] = reason.strip()
    return rows


def coverage(results_path):
    names = section10_names()
    files = {p.stem for p in SCENARIOS.glob("*.sh")}
    pending = read_tsv(SCENARIOS / "pending.tsv")
    xfail = read_tsv(SCENARIOS / "expected-failing.tsv")
    results = {}
    if Path(results_path).exists():
        for line in Path(results_path).read_text().splitlines():
            if line.strip():
                n, st, *rest = line.split("\t")
                results[n] = (st, rest[0] if rest else "")
    gate = []
    rows = []
    for n in names:
        if n in files and n in pending:
            gate.append(f"{n}: both a scenario file and a pending entry")
        if n in files:
            st, why = results.get(n, ("NOT RUN", ""))
            if st == "PASS":
                state = "implemented, passing"
                if n in xfail:
                    gate.append(f"{n}: passes but is listed in expected-failing.tsv; remove the entry")
            elif st == "NOT RUN":
                state = "implemented, NOT RUN"
            else:
                state = "implemented, FAILING"
                if n in xfail:
                    why = f"known: {xfail[n]}"
                else:
                    gate.append(f"{n}: failed ({why})")
            rows.append((n, state, why))
        elif n in pending:
            rows.append((n, "pending", pending[n]))
        else:
            rows.append((n, "UNREGISTERED", ""))
            gate.append(f"{n}: in inventory §10 with neither a scenario file nor a pending entry")
    for extra in sorted((files | set(pending)) - set(names)):
        gate.append(f"{extra}: not a scenario name in inventory §10")
    for n in sorted(set(xfail) - files):
        gate.append(f"{n}: in expected-failing.tsv without a scenario file")

    width = max(len(n) for n, _, _ in rows)
    print("== coverage (inventory §10)")
    for n, state, why in rows:
        print(f"  {n:<{width}}  {state}{'  - ' + why if why else ''}")
    c = Counter(state for _, state, _ in rows)
    print(f"  total {len(rows)}: "
          f"{c['implemented, passing']} passing, {c['implemented, FAILING']} failing, "
          f"{c['implemented, NOT RUN']} not run, {c['pending']} pending, {c['UNREGISTERED']} unregistered")
    if gate:
        print("== coverage gate: FAIL")
        for g in gate:
            print(f"  {g}")
        sys.exit(1)
    print("== coverage gate: ok")


def main(argv):
    cmd, *args = argv or ["help"]
    if cmd == "pins":
        pins()
    elif cmd == "tables":
        print("\n".join(tables(args[0])))
    elif cmd == "drift-env":
        drift_env(*args)
    elif cmd == "account":
        account(*args)
    elif cmd == "coverage":
        coverage(args[0] if args else "/dev/null")
    elif cmd == "names":
        print("\n".join(section10_names()))
    else:
        print(__doc__)
        sys.exit(2)


if __name__ == "__main__":
    main(sys.argv[1:])
