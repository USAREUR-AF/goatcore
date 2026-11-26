#!/usr/bin/env python3
"""spock-reseed: seed a Spock member again after a site invalidated its slot.

One step per call, as `goatbuilder join-node <node> --reseed` runs them
(docs/design/goatbuilder-hardware-measurement.md, "How long a phone may be
off"). The NODE is the member that was away (a phone node); the SITE is the
site whose slot for it was invalidated (the founder). Each side is reached by
a command that runs psql there and reads SQL on stdin, for example

    --node-psql "ssh goat@100.127.0.20 sudo docker exec -i postgres-a psql -U postgres"
    --site-psql "docker exec -i rcph-a psql -U postgres"

Steps, in order; each refuses with its catalog code and exit 1:

  lost       the site's slot for one of the node's subscriptions reads
             wal_status = lost; refused when none does (GB-345)
  caught-up  the site has taken the node's own writes: its subscription from
             the node reads replicating and has confirmed the node's WAL
             position (GB-346), so the truncate below loses nothing
  size       the tables to copy are, in total, at most --size-fraction of the
             node's own slot cap (GB-347): the copy is written as WAL on the
             node, and a copy near its cap invalidates the site's slot there
  reseed     for each lost subscription: drop it on the node (which drops the
             lost slot on the site), create it again with the same provider,
             replication sets and forwarded origins and no initial copy, and
             copy each table of its replication sets again after a local
             truncate (spock.sub_resync_table; the truncate is not replicated)
  synced     every table of each new subscription reads 'r' in
             spock.local_sync_status, the subscription reads replicating, and
             no slot on the node reads lost (GB-348)
  converged  each copied table holds the same rows on the node and the site,
             by count and digest (GB-349)

Exit 2 when a side cannot be reached or a statement fails: that is not a
refusal, and the journal records the line without a code.

The detector, `slots`, changes nothing and keeps no journal. For the node, in
every database with a Spock node, it lists each subscription the node holds
(its status, and the wal_status of its slot at the provider, when the provider
is one of the sides given) and each slot the node keeps for a subscriber (its
wal_status and whether it is active, and the subscription it feeds, when the
subscriber is one of the sides given). A slot that reads wal_status = lost (or
conflicting) can never be read again: the line names it and what rebuilds it.
It exits 0 when every slot can still recover, and 1 with GB-387 when one
cannot. --site-psql and --peer-psql are optional for it; without them the
node's own slots are still read, and a subscription's slot at its provider
reads 'not read'.

--journal FILE records each step's outcome as JSON, so a stopped run resumes
at the step it stopped at: the subscriptions the lost step found, the tables
the reseed step copied again, and a refusal's code and line. No connection
string is written to it or printed: the provider's is read from the node's
subscription before it is dropped and held in memory, or, for a run that
stopped between the drop and the create, from the site's own interface.

  spock-reseed.py <step> --node-psql CMD --site-psql CMD [--journal FILE]
                  [--wait SECONDS] [--size-fraction F]
  spock-reseed.py slots --node-psql CMD [--site-psql CMD]
                  [--peer-psql NAME=CMD ...] [--json]
  spock-reseed.py --self-test
"""
import argparse
import datetime as dt
import json
import os
import re
import shlex
import subprocess
import sys
import time

STEPS = ("lost", "caught-up", "size", "reseed", "synced", "converged")
SCHEMA = "goatbuilder.spock-reseed/v1"


class Refused(Exception):
    def __init__(self, code, line):
        super().__init__(f"{code}: {line}")
        self.code, self.line = code, line


def now():
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def lit(s):
    """A SQL string literal."""
    return "'" + str(s).replace("'", "''") + "'"


def arr(items):
    return "ARRAY[" + ",".join(lit(i) for i in items) + "]::text[]"


class Side:
    """One Postgres, reached by a command that runs psql and reads SQL on stdin."""

    def __init__(self, name, prefix):
        self.name, self.prefix = name, shlex.split(prefix)

    def q(self, db, sql):
        # -z separates fields with a zero byte: no argument that a shell on the
        # way (ssh) would split or interpret.
        r = subprocess.run(self.prefix + ["-d", db, "-tAXqz", "-v", "ON_ERROR_STOP=1"],
                           input=sql, capture_output=True, text=True, timeout=300)
        if r.returncode != 0:
            # psql's own error, without the SQL (it may carry a connection string).
            # psql's error lines, with any password in them removed.
            err = re.sub(r"password=\S+", "password=<removed>", " ".join(r.stderr.split()))[-600:]
            raise RuntimeError(f"{self.name}: psql in {db} failed: {err or r.returncode}")
        return [line.split("\0") for line in r.stdout.splitlines() if line != ""]

    def one(self, db, sql):
        rows = self.q(db, sql)
        return rows[0][0] if rows and rows[0] else ""


def local_node(side, db):
    return side.one(db, "SELECT n.node_name FROM spock.local_node l JOIN spock.node n ON n.node_id = l.node_id;")


def spock_dbs(side):
    dbs = [r[0] for r in side.q("postgres", "SELECT datname FROM pg_database WHERE datallowconn AND NOT datistemplate ORDER BY 1;")]
    return [d for d in dbs if side.one(d, "SELECT count(*) FROM pg_extension WHERE extname = 'spock';") == "1"
            and local_node(side, d)]


def subs_from(side, db, origin):
    """The subscriptions on `side` in `db` whose provider is the node `origin`."""
    rows = side.q(db, f"""SELECT s.sub_name, s.sub_slot_name, array_to_string(s.sub_replication_sets, ','),
                                 array_to_string(s.sub_forward_origins, ',')
                          FROM spock.subscription s JOIN spock.node o ON o.node_id = s.sub_origin
                          WHERE o.node_name = {lit(origin)} ORDER BY 1;""")
    return [{"db": db, "sub": r[0], "slot": r[1], "sets": [x for x in r[2].split(",") if x],
             "forward_origins": [x for x in r[3].split(",") if x]} for r in rows]


def tables_of(side, db, sets):
    return [r[0] for r in side.q(db, f"""SELECT DISTINCT t.set_reloid::regclass::text
                                         FROM spock.replication_set_table t
                                         JOIN spock.replication_set r ON r.set_id = t.set_id
                                         JOIN spock.local_node l ON l.node_id = r.set_nodeid
                                         WHERE r.set_name = ANY({arr(sets)}) ORDER BY 1;""")]


class Journal:
    def __init__(self, path):
        self.path = path
        self.doc = {"schema": SCHEMA, "steps": {}}
        if path and os.path.exists(path):
            with open(path) as f:
                self.doc = json.load(f)

    def get(self, step):
        return self.doc["steps"].get(step, {})

    def put(self, step, **kw):
        self.doc["steps"][step] = {"at": now(), **kw}
        if self.path:
            tmp = f"{self.path}.{os.getpid()}.tmp"
            with open(tmp, "w") as f:
                json.dump(self.doc, f, indent=2, sort_keys=True)
                f.write("\n")
            os.replace(tmp, self.path)


# ---- the steps -------------------------------------------------------------

def step_lost(node, site, j, a):
    found = []
    for db in spock_dbs(node):
        s_name = local_node(site, db)
        if not s_name:
            continue
        for s in subs_from(node, db, s_name):
            st = site.one(db, f"SELECT coalesce(wal_status, '') FROM pg_replication_slots WHERE slot_name = {lit(s['slot'])};")
            print(f"lost: {db} {s['sub']}: the site's slot {s['slot']} reads '{st or 'absent'}'")
            if st == "lost":
                found.append({**s, "site_node": s_name, "node_node": local_node(node, db)})
    if not found:
        raise Refused("GB-345", "no slot the site keeps for the node's subscriptions reads wal_status = lost: there is nothing to seed again, and copying a healthy node again only costs the copy")
    return {"subs": found}


def step_caught_up(node, site, j, a):
    subs = j.get("lost").get("subs", [])
    for s in subs:
        db = s["db"]
        row = site.q(db, f"SELECT status, slot_name FROM spock.sub_show_status() WHERE provider_node = {lit(s['node_node'])};")
        if not row:
            raise Refused("GB-346", f"{db}: the site has no subscription from the node {s['node_node']}, so the node's own writes cannot be shown to have reached it")
        status, slot = row[0][0], row[0][1]
        target = node.one(db, "SELECT pg_current_wal_lsn();")
        deadline = time.time() + a.wait
        while True:
            status = site.one(db, f"SELECT status FROM spock.sub_show_status() WHERE provider_node = {lit(s['node_node'])};")
            behind = node.one(db, f"SELECT pg_wal_lsn_diff({lit(target)}::pg_lsn, confirmed_flush_lsn)::bigint FROM pg_replication_slots WHERE slot_name = {lit(slot)};")
            if status == "replicating" and behind != "" and int(behind) <= 0:
                break
            if time.time() > deadline:
                raise Refused("GB-346", f"{db}: the site's subscription from the node reads '{status}' and its slot on the node is {behind or 'unknown'} bytes behind the node's WAL position {target} after {a.wait}s: the node's own writes have not all reached the site, and the truncate would lose them")
            time.sleep(2)
        print(f"caught-up: {db}: the site's subscription from the node reads replicating and has confirmed {target}")
    return {}


def step_size(node, site, j, a):
    subs = j.get("lost").get("subs", [])
    total = 0
    for s in subs:
        n = site.one(s["db"], f"""SELECT coalesce(sum(pg_total_relation_size(t.set_reloid)), 0)::bigint
                                  FROM spock.replication_set_table t
                                  JOIN spock.replication_set r ON r.set_id = t.set_id
                                  JOIN spock.local_node l ON l.node_id = r.set_nodeid
                                  WHERE r.set_name = ANY({arr(s['sets'])});""")
        total += int(n or 0)
    cap = int(node.one("postgres", "SELECT pg_size_bytes(current_setting('max_slot_wal_keep_size'));"))
    if cap < 0:
        print(f"size: {total} bytes to copy; the node's slot cap is unlimited")
        return {"bytes": total, "cap": cap}
    limit = int(cap * a.size_fraction)
    print(f"size: {total} bytes to copy; the node's slot cap is {cap} bytes, and the copy may use {limit} ({a.size_fraction:g} of it)")
    if total > limit:
        raise Refused("GB-347", f"the tables to copy hold {total} bytes, more than {a.size_fraction:g} of the node's slot cap ({cap} bytes): the copy is written as WAL on the node, and one near its cap invalidates the site's slot there, so the node's next writes could not reach the site")
    return {"bytes": total, "cap": cap}


def step_reseed(node, site, j, a):
    subs = j.get("lost").get("subs", [])
    done = j.get("reseed").get("copied", [])
    for s in subs:
        db, sub = s["db"], s["sub"]
        exists = node.one(db, f"SELECT count(*) FROM spock.subscription WHERE sub_name = {lit(sub)};") == "1"
        slot_now = site.one(db, f"SELECT coalesce(wal_status, '') FROM pg_replication_slots WHERE slot_name = {lit(s['slot'])};")
        dsn = None
        if exists and slot_now == "lost":
            # The provider's connection string as the subscription had it, held
            # in this process only: sub_drop removes the provider's node and
            # interface on the node with the subscription.
            dsn = node.one(db, f"""SELECT i.if_dsn FROM spock.subscription s
                                   JOIN spock.node_interface i ON i.if_id = s.sub_origin_if
                                   WHERE s.sub_name = {lit(sub)};""")
            node.q(db, f"SELECT spock.sub_drop({lit(sub)});")
            print(f"reseed: {db} {sub}: dropped (the site's lost slot with it)")
            exists = False
        if not exists:
            if not dsn:
                # A run that stopped between the drop and the create: the
                # site's own interface, the one its node was created with.
                dsn = site.one(db, """SELECT i.if_dsn FROM spock.local_node l
                                      JOIN spock.node_interface i ON i.if_id = l.node_local_interface;""")
            node.q(db, f"""SELECT spock.sub_create(
                    subscription_name := {lit(sub)},
                    provider_dsn      := {lit(dsn)},
                    replication_sets  := {arr(s['sets'])},
                    synchronize_data  := false,
                    forward_origins   := {arr(s['forward_origins'])});""")
            print(f"reseed: {db} {sub}: created again with replication sets {','.join(s['sets'])}, no initial copy")
        for t in tables_of(node, db, s["sets"]):
            key = f"{db}:{sub}:{t}"
            if key in done:
                continue
            node.q(db, f"SELECT spock.sub_resync_table({lit(sub)}, {lit(t)}::regclass, true);")
            done.append(key)
            j.put("reseed", ok=False, copied=done)
            print(f"reseed: {db} {sub}: {t} truncated here and copied again")
    return {"copied": done}


def step_synced(node, site, j, a):
    subs = j.get("lost").get("subs", [])
    deadline = time.time() + a.wait
    for s in subs:
        db, sub = s["db"], s["sub"]
        while True:
            status = node.one(db, f"SELECT status FROM spock.sub_show_status() WHERE subscription_name = {lit(sub)};")
            pending = node.q(db, f"""SELECT sync_nspname || '.' || sync_relname, sync_status FROM spock.local_sync_status ls
                                     JOIN spock.subscription s ON s.sub_id = ls.sync_subid
                                     WHERE s.sub_name = {lit(sub)} AND ls.sync_relname IS NOT NULL AND ls.sync_status <> 'r';""")
            lost = node.q("postgres", "SELECT slot_name FROM pg_replication_slots WHERE wal_status = 'lost';")
            if lost:
                raise Refused("GB-348", f"{db}: a slot on the node reads lost during the copy ({', '.join(r[0] for r in lost)}): the site can no longer take the node's writes from it")
            if status == "replicating" and not pending:
                break
            if time.time() > deadline:
                left = ", ".join(f"{r[0]}={r[1]}" for r in pending) or "none"
                raise Refused("GB-348", f"{db} {sub}: after {a.wait}s the subscription reads '{status}' and tables not yet copied: {left}")
            time.sleep(2)
        print(f"synced: {db} {sub}: replicating, every table copied")
    return {}


def digest(side, db, t):
    return side.one(db, f"SELECT count(*) || ' ' || coalesce(md5(string_agg(md5(x::text), ',' ORDER BY md5(x::text))), '') FROM {t} x;")


def step_converged(node, site, j, a):
    subs = j.get("lost").get("subs", [])
    checked = []
    for s in subs:
        db = s["db"]
        for t in tables_of(node, db, s["sets"]):
            deadline = time.time() + a.wait
            while True:
                dn, ds = digest(node, db, t), digest(site, db, t)
                if dn == ds:
                    break
                if time.time() > deadline:
                    raise Refused("GB-349", f"{db} {t}: the node holds '{dn}' and the site '{ds}' (count and digest) after {a.wait}s")
                time.sleep(2)
            print(f"converged: {db} {t}: {dn.split()[0]} rows, the same on both")
            checked.append(f"{db}:{t}")
    return {"tables": checked}


# ---- the detector -----------------------------------------------------------

def slot_state(side, slot):
    """(wal_status, active) of a slot on `side`, ("absent", "") when it has none.
    A slot Postgres invalidated for a conflict (`conflicting`, a logical slot
    on a standby) reads 'lost' too: neither can be read again."""
    rows = side.q("postgres", f"""SELECT coalesce(wal_status, ''), active::text, coalesce(conflicting, false)::text
                                 FROM pg_replication_slots WHERE slot_name = {lit(slot)};""")
    if not rows:
        return "absent", ""
    st, active, conflicting = rows[0][0], rows[0][1], rows[0][2]
    return ("lost" if conflicting == "true" else st or "unknown"), active


def detect(node, others):
    """Every subscription and slot of the node, in every database with a Spock
    node. `others` maps a side's name to its Side: the founder and any peers."""
    found = []
    for db in spock_dbs(node):
        me = local_node(node, db)
        names = {}
        for name, side in others.items():
            n = local_node(side, db)
            if n:
                names[n] = (name, side)
        for r in node.q(db, """SELECT s.sub_name, s.sub_slot_name, o.node_name, st.status
                               FROM spock.subscription s JOIN spock.node o ON o.node_id = s.sub_origin
                               LEFT JOIN spock.sub_show_status() st ON st.subscription_name = s.sub_name
                               ORDER BY 1;"""):
            sub, slot, provider, status = r[0], r[1], r[2], r[3] or "unknown"
            if provider in names:
                wal, _ = slot_state(names[provider][1], slot)
            else:
                wal = "not read"
            found.append({"db": db, "direction": "in", "subscription": sub, "provider": provider,
                          "subscriber": me, "slot": slot, "slot_at": provider, "status": status,
                          "wal_status": wal})
        fed = {}
        for n, (name, side) in names.items():
            for s in subs_from(side, db, me):
                st = side.one(db, f"SELECT status FROM spock.sub_show_status() WHERE subscription_name = {lit(s['sub'])};")
                fed[s["slot"]] = (n, s["sub"], st or "unknown")
        for r in node.q("postgres", f"""SELECT slot_name, coalesce(wal_status, ''), active::text, coalesce(conflicting, false)::text
                                       FROM pg_replication_slots WHERE database = {lit(db)} AND slot_type = 'logical'
                                       ORDER BY 1;"""):
            slot, wal, active, conflicting = r[0], r[1] or "unknown", r[2], r[3]
            if conflicting == "true":
                wal = "lost"
            subscriber, sub, status = fed.get(slot, ("", "", "not read"))
            found.append({"db": db, "direction": "out", "subscription": sub, "provider": me,
                          "subscriber": subscriber, "slot": slot, "slot_at": me, "status": status,
                          "wal_status": wal, "active": active})
    for f in found:
        f["verdict"] = verdict(f)
    return found


def verdict(f):
    """What a line of the detector means. 'unreserved' still recovers: the WAL
    is held until the next checkpoint removes it."""
    if f["wal_status"] in ("reserved", "extended", "unreserved"):
        return "can recover"
    if f["wal_status"] != "lost":
        return "not read"
    if f["direction"] == "in":
        return f"cannot recover: {f['provider']} no longer holds this node's changes; rebuild this node (join-node --reseed)"
    return (f"cannot recover: {f['subscriber'] or 'its subscriber'} can no longer read this node's writes; "
            "the rebuild of this node re-creates its founder's subscription, and another peer's is rebuilt with that peer")


def step_slots(node, others, a):
    found = detect(node, others)
    lost = [f for f in found if f["wal_status"] == "lost"]
    if a.json:
        print(json.dumps({"schema": "goatbuilder.spock-slots/v1", "slots": found,
                          "lost": len(lost)}, indent=2, sort_keys=True))
    else:
        cols = ("db", "direction", "subscription", "provider", "subscriber", "slot", "status", "wal_status", "verdict")
        rows = [cols] + [tuple(str(f.get(c, "")) for c in cols) for f in found]
        widths = [max(len(r[i]) for r in rows) for i in range(len(cols) - 1)]
        for r in rows:
            print("  ".join(v.ljust(widths[i]) for i, v in enumerate(r[:-1])) + "  " + r[-1])
        print(f"slots: {len(found)} subscriptions and slots read, {len(lost)} cannot recover")
    if lost:
        names = ", ".join(f"{f['db']} {f['slot']} ({'at ' + f['slot_at']})" for f in lost)
        raise Refused("GB-387", f"{len(lost)} Spock slot(s) read wal_status = lost and can never be read again: {names}")
    return 0


RUN = {"lost": step_lost, "caught-up": step_caught_up, "size": step_size,
       "reseed": step_reseed, "synced": step_synced, "converged": step_converged}


def self_test():
    assert lit("a'b") == "'a''b'"
    assert arr(["default", "x'y"]) == "ARRAY['default','x''y']::text[]"
    assert list(RUN) == list(STEPS)
    j = Journal(None)
    j.put("lost", ok=True, subs=[{"db": "netbird"}])
    assert j.get("lost")["subs"][0]["db"] == "netbird" and j.get("size") == {}
    assert verdict({"direction": "in", "provider": "site_a", "wal_status": "lost"}).startswith("cannot recover: site_a")
    assert verdict({"direction": "out", "subscriber": "", "wal_status": "lost"}).startswith("cannot recover: its subscriber")
    assert verdict({"direction": "out", "wal_status": "unreserved"}) == "can recover"
    assert verdict({"direction": "in", "wal_status": "not read"}) == "not read"
    e = Refused("GB-345", "x")
    assert str(e) == "GB-345: x"
    print("spock-reseed: self-test ok")
    return 0


def main():
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("step", choices=STEPS + ("slots",))
    ap.add_argument("--node-psql", required=True)
    ap.add_argument("--site-psql")
    ap.add_argument("--peer-psql", action="append", default=[], metavar="NAME=CMD")
    ap.add_argument("--journal")
    ap.add_argument("--wait", type=int, default=600)
    ap.add_argument("--size-fraction", type=float, default=0.25)
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()
    node = Side("node", a.node_psql)
    if a.step == "slots":
        others = {}
        if a.site_psql:
            others["site"] = Side("site", a.site_psql)
        for p in a.peer_psql:
            name, sep, cmd = p.partition("=")
            if not sep or not name or not cmd:
                print(f"spock-reseed: --peer-psql takes NAME=CMD, not {p!r}", file=sys.stderr)
                return 2
            others[name] = Side(name, cmd)
        try:
            return step_slots(node, others, a)
        except Refused as e:
            print(f"{e.code}: {e.line}", file=sys.stderr)
            return 1
        except (RuntimeError, subprocess.TimeoutExpired, OSError) as e:
            print(f"spock-reseed: slots: {e}", file=sys.stderr)
            return 2
    if not a.site_psql:
        print(f"spock-reseed: {a.step} needs --site-psql", file=sys.stderr)
        return 2
    site = Side("site", a.site_psql)
    j = Journal(a.journal)
    if a.step != "lost" and not j.get("lost").get("ok"):
        print("GB-345: the lost step has not passed in this journal; run it first", file=sys.stderr)
        return 1
    try:
        out = RUN[a.step](node, site, j, a)
    except Refused as e:
        j.put(a.step, ok=False, code=e.code, line=e.line, **({"copied": j.get("reseed").get("copied", [])} if a.step == "reseed" else {}))
        print(f"{e.code}: {e.line}", file=sys.stderr)
        return 1
    except (RuntimeError, subprocess.TimeoutExpired, OSError) as e:
        # Not a refusal: a side could not be reached or a statement failed.
        j.put(a.step, ok=False, code="", line=str(e), **({"copied": j.get("reseed").get("copied", [])} if a.step == "reseed" else {}))
        print(f"spock-reseed: {a.step}: {e}", file=sys.stderr)
        return 2
    j.put(a.step, ok=True, **out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
