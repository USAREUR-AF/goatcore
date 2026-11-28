#!/usr/bin/env python3
"""spock-reseed: rebuild a Spock member after a slot it needs was invalidated.

One step per call, as `goatbuilder join-node <node> --reseed` runs them
(docs/operations/spock-site-rebuild.md; docs/design/goatbuilder-hardware-measurement.md,
"How long a phone may be off"). The NODE is the member being rebuilt: a joiner
site's Postgres or a phone node. The SITE is its founder, the healthy peer it
is copied from. A PEER is another site the node replicates with. Each side is
reached by a command that runs psql there and reads SQL on stdin, for example

    --node-psql "ssh goat@100.127.0.20 sudo docker exec -i postgres-a psql -U postgres"
    --site-psql "docker exec -i rcph-a psql -U postgres"
    --peer-psql "other-site=ssh goat@100.127.0.30 sudo docker exec -i postgres-a psql -U postgres"

Every site writes (ADR 1165), so the node may hold rows the founder lacks. The
rebuild never drops them silently: it exports them first, and replays them or
hands them back afterwards.

Steps, in order; each refuses with its catalog code and exit 1:

  lost       in every database with a Spock node: a slot at a provider for
             one of the node's subscriptions, or the node's slot for the
             founder's subscription from it, reads wal_status = lost; refused
             when none does (GB-345), and when the node subscribes to a site
             that is not one of the sides given, or has no subscription from
             the founder (GB-390)
  caught-up  the founder has taken the node's own writes (GB-346), so nothing
             written here is only here; passed with a note when the founder's
             slot on the node is lost, since then it never will and the export
             carries them
  export     with no client but this run connected (GB-389), each table of
             the founder's replication sets is compared with the founder by
             primary key and row digest (a table with no primary key is
             refused, GB-390). A row only the node holds, or holds changed,
             written here after the founder's last applied commit from the
             node (spock.progress) is to be REPLAYED after the copy; any other
             such row is for REVIEW (it came from another site, or the founder
             removed it since). Both go to <journal>.export.json, mode 0600,
             with a digest; the step refuses (GB-388) until run with
             --accept <that digest>
  size       the tables to copy are, in total, at most --size-fraction of the
             node's own slot cap (GB-347): the copy is written as WAL on the
             node, and a copy near its cap invalidates the site's slot there
  reseed     the export is still what was accepted (GB-388 otherwise, and the
             export step is opened again), and no client is connected
             (GB-389). The node's other healthy subscriptions are disabled for
             the copy. Each lost subscription of the node is dropped (which
             drops the lost slot) and created again with the same provider,
             replication sets and forwarded origins and no initial copy; so is
             the founder's subscription from the node, when its slot on the
             node is lost. Then, with the subscription from the founder
             disabled, every table of its replication sets is truncated here
             in one statement in repair mode (not replicated), copied again
             (spock.sub_resync_table without its own truncate, which cascades
             to the tables that reference the one copied, and those are not
             copied again), and the subscription is enabled
  synced     every table of the subscription from the founder reads 'r' in
             spock.local_sync_status and it reads replicating, no slot on the
             node reads lost (GB-348); the disabled subscriptions are enabled
             again, and they and the founder's re-created subscription from the
             node read replicating
  replay     each row to replay is written again here, parents before
             children, each in its own statement; it then replicates to the
             founder. A row already here as exported is counted, not written.
             A row that cannot be written, with the reason, and the rows for
             review go to <journal>.handback.json, mode 0600; the step refuses
             (GB-391) while a row was not written, until run with --accept
             <the hand-back's digest>
  converged  each table holds the same rows on the node and the founder, by
             count and digest (GB-349). A peer whose subscription from the
             node is lost too is named: it is rebuilt next, from the founder

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
at the step it stopped at: the subscriptions the lost step found, the
subscriptions the reseed step disabled, and a refusal's code and line. A
journal another version of this helper wrote is refused (exit 2). No
connection string is written to it, to the export or to the hand-back file,
or printed: a provider's is read from the subscription before it is dropped
and held in memory, or, for a run that stopped between the drop and the
create, from the provider's own interface. The export and hand-back files
hold row contents, so they are written mode 0600 beside the journal.

  spock-reseed.py <step> --node-psql CMD --site-psql CMD [--peer-psql NAME=CMD ...]
                  [--journal FILE] [--accept DIGEST] [--wait SECONDS] [--size-fraction F]
  spock-reseed.py slots --node-psql CMD [--site-psql CMD]
                  [--peer-psql NAME=CMD ...] [--json]
  spock-reseed.py --self-test
"""
import argparse
import datetime as dt
import hashlib
import json
import os
import re
import shlex
import subprocess
import sys
import time

STEPS = ("lost", "caught-up", "export", "size", "reseed", "synced", "replay", "converged")
SCHEMA = "goatbuilder.spock-reseed/v2"
EXPORT_SCHEMA = "goatbuilder.spock-reseed-export/v1"
HANDBACK_SCHEMA = "goatbuilder.spock-reseed-handback/v1"


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

    def run(self, db, script):
        """A script that may have statements refused (ON_ERROR_ROLLBACK): its
        error lines, without passwords. A side that cannot be reached is not
        a refused statement: psql's exit 2 raises."""
        r = subprocess.run(self.prefix + ["-d", db, "-tAXq"], input=script, capture_output=True, text=True, timeout=600)
        if r.returncode == 2:
            err = re.sub(r"password=\S+", "password=<removed>", " ".join(r.stderr.split()))[-600:]
            raise RuntimeError(f"{self.name}: psql in {db} failed: {err}")
        return [re.sub(r"password=\S+", "password=<removed>", line.strip())
                for line in r.stderr.splitlines() if line.startswith(("ERROR:", "DETAIL:"))]

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
            if self.doc.get("schema") != SCHEMA:
                raise RuntimeError(f"{path} was written by another version of this helper ({self.doc.get('schema')}): finish that re-seed with the helper it began with, or move the journal aside to start again at the lost step")

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

def qi(name):
    """A SQL identifier, quoted."""
    return '"' + name.replace('"', '""') + '"'


def subs_all(side, db):
    """Every subscription on `side` in `db`, with its provider's node name."""
    rows = side.q(db, """SELECT s.sub_name, s.sub_slot_name, array_to_string(s.sub_replication_sets, ','),
                                array_to_string(s.sub_forward_origins, ','), o.node_name, s.sub_enabled::text
                         FROM spock.subscription s JOIN spock.node o ON o.node_id = s.sub_origin ORDER BY 1;""")
    return [{"db": db, "sub": r[0], "slot": r[1], "sets": [x for x in r[2].split(",") if x],
             "forward_origins": [x for x in r[3].split(",") if x], "provider": r[4],
             "enabled": r[5] == "true"} for r in rows]


def lost_dbs(j):
    return j.get("lost").get("dbs", [])


def step_lost(node, site, peers, j, a):
    dbs = []
    for db in spock_dbs(node):
        s_name = local_node(site, db)
        if not s_name:
            continue
        me = local_node(node, db)
        sides = {s_name: ("the founder", site)}
        for name, p in peers.items():
            n = local_node(p, db)
            if n:
                sides[n] = (name, p)
        founder_sub, lost_in, other_in = None, [], []
        for s in subs_all(node, db):
            prov = s["provider"]
            if prov not in sides:
                raise Refused("GB-390", f"{db}: the node subscribes to {prov} ({s['sub']}), which is not one of the sides given: name the site whose Spock node is {prov} with --peer, so its slot for the node is read")
            st, _ = slot_state(sides[prov][1], s["slot"])
            print(f"lost: {db} {s['sub']} from {prov}: its slot {s['slot']} there reads '{st}'")
            s = {**s, "wal_status": st}
            if prov == s_name:
                founder_sub = s
            if st == "lost":
                lost_in.append(s)
            elif prov != s_name:
                other_in.append(s)
        out_founder, out_peers = None, []
        for n, (name, side) in sides.items():
            for s in subs_from(side, db, me):
                st, _ = slot_state(node, s["slot"])
                print(f"lost: {db} {name}'s subscription {s['sub']} from the node: its slot {s['slot']} on the node reads '{st}'")
                if st == "lost":
                    if side is site:
                        out_founder = {**s, "subscriber": n}
                    else:
                        out_peers.append({**s, "subscriber": n, "site": name})
        if not lost_in and not out_founder:
            continue
        if founder_sub is None:
            raise Refused("GB-390", f"{db}: the node has no subscription from the founder {s_name}: the rebuild copies the node's tables from the founder, through that subscription")
        dbs.append({"db": db, "node_node": me, "site_node": s_name, "founder_sub": founder_sub,
                    "lost_in": lost_in, "other_in": other_in, "out_founder": out_founder, "out_peers": out_peers})
    if not dbs:
        raise Refused("GB-345", "no slot the site keeps for the node's subscriptions reads wal_status = lost, and neither does the node's slot for the site's subscription from it: there is nothing to seed again, and copying a healthy node again only costs the copy")
    for d in dbs:
        for o in d["out_peers"]:
            print(f"lost: {d['db']}: {o['site']}'s subscription {o['sub']} from the node is lost too; this rebuild leaves it, and {o['site']} is rebuilt after this one, from the founder")
    return {"dbs": dbs}


def step_caught_up(node, site, peers, j, a):
    for d in lost_dbs(j):
        db = d["db"]
        if d["out_founder"]:
            print(f"caught-up: {db}: the founder's slot on the node ({d['out_founder']['slot']}) is lost, so the founder cannot take the node's writes; the export step finds every row only the node holds")
            continue
        row = site.q(db, f"SELECT status, slot_name FROM spock.sub_show_status() WHERE provider_node = {lit(d['node_node'])};")
        if not row:
            raise Refused("GB-346", f"{db}: the site has no subscription from the node {d['node_node']}, so the node's own writes cannot be shown to have reached it")
        slot = row[0][1]
        target = node.one(db, "SELECT pg_current_wal_lsn();")
        deadline = time.time() + a.wait
        while True:
            status = site.one(db, f"SELECT status FROM spock.sub_show_status() WHERE provider_node = {lit(d['node_node'])};")
            behind = node.one(db, f"SELECT pg_wal_lsn_diff({lit(target)}::pg_lsn, confirmed_flush_lsn)::bigint FROM pg_replication_slots WHERE slot_name = {lit(slot)};")
            if status == "replicating" and behind != "" and int(behind) <= 0:
                break
            if time.time() > deadline:
                raise Refused("GB-346", f"{db}: the site's subscription from the node reads '{status}' and its slot on the node is {behind or 'unknown'} bytes behind the node's WAL position {target} after {a.wait}s: the node's own writes have not all reached the site, and the truncate would lose them")
            time.sleep(2)
        print(f"caught-up: {db}: the site's subscription from the node reads replicating and has confirmed {target}")
    return {}


def quiet(node, dbs):
    """Refuse while a client other than this run is connected to a database
    the rebuild truncates: a write it makes after the export would be lost."""
    rows = node.q("postgres", f"""SELECT datname, coalesce(usename, ''), coalesce(nullif(application_name, ''), '(no application name)'),
                                         coalesce(client_addr::text, 'local'), count(*)
                                  FROM pg_stat_activity
                                  WHERE backend_type = 'client backend' AND pid <> pg_backend_pid() AND datname = ANY({arr(dbs)})
                                  GROUP BY 1, 2, 3, 4 ORDER BY 1, 2, 3, 4;""")
    if rows:
        who = "; ".join(f"{r[0]}: {r[1]} {r[2]} from {r[3]} ({r[4]})" for r in rows)
        raise Refused("GB-389", f"clients are connected to the databases the rebuild truncates, and a write they make after the export would be lost: {who}")


def pk_cols(side, db, t):
    return [r[0] for r in side.q(db, f"""SELECT a.attname FROM pg_index i
                                         JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey)
                                         WHERE i.indrelid = {lit(t)}::regclass AND i.indisprimary
                                         ORDER BY array_position(i.indkey::int2[], a.attnum);""")]


# The commit timestamp and origin of a row's last write. Spock records the
# origin node's id as the origin of a change it applies; 0 is a write made
# here. A transaction whose commit timestamp is no longer kept reads NULL.
CTS = """CREATE FUNCTION pg_temp.cts(x xid, OUT ts timestamptz, OUT roident oid) LANGUAGE plpgsql AS $f$
BEGIN SELECT c.timestamp, c.roident INTO ts, roident FROM pg_xact_commit_timestamp_origin(x) c;
EXCEPTION WHEN OTHERS THEN ts := NULL; roident := NULL; END $f$;
"""


def fk_edges(side, db):
    """Every foreign key in the database, as (referencing, referenced) tables."""
    return [(r[0], r[1]) for r in side.q(db, """SELECT conrelid::regclass::text, confrelid::regclass::text
                                               FROM pg_constraint WHERE contype = 'f' AND conrelid <> confrelid;""")]


def referrers(edges, start):
    """The tables in `start`, and every table whose foreign keys reach one of them."""
    out, changed = set(start), True
    while changed:
        changed = False
        for child, parent in edges:
            if parent in out and child not in out:
                out.add(child)
                changed = True
    return out


def plan_copy(node, site, d):
    """Which tables the copy takes. A table that already holds the same rows as
    the founder is left alone. A table that differs is copied again, and so is
    every replicated table whose foreign keys reach it, since its truncate
    cascades to them. A table that is not replicated and whose foreign keys
    reach one copied is emptied by that cascade and not copied: it is named,
    with its row count, so the operator accepts it first."""
    db = d["db"]
    tables = tables_of(node, db, d["founder_sub"]["sets"])
    differ = [t for t in tables if digest(node, db, t) != digest(site, db, t)]
    reach = referrers(fk_edges(node, db), differ)
    copy = sorted(t for t in reach if t in tables)
    emptied = {t: int(node.one(db, f"SELECT count(*) FROM {t};") or 0) for t in sorted(reach - set(tables))}
    return {"copy": copy, "emptied": emptied, "same": sorted(set(tables) - set(copy))}


def classify(node, site, d):
    """The rows of each table of the founder's replication sets that only the
    node holds, or holds with other contents, against the founder, with each
    row's origin and whether it was written after the founder's last applied
    commit from the node (the sync point)."""
    db = d["db"]
    sync = site.one(db, f"""SELECT p.remote_commit_ts FROM spock.progress p
                            JOIN spock.node n ON n.node_id = p.remote_node_id
                            JOIN spock.local_node l ON l.node_id = p.node_id
                            WHERE n.node_name = {lit(d['node_node'])};""")
    origins = {r[0]: r[1] for r in node.q(db, "SELECT node_id::text, node_name FROM spock.node;")}
    newer = f"(c.ts > {lit(sync)}::timestamptz)" if sync else "true"
    out = {"sync_point": sync or None, "site_only": {}, "replaced": {}, "added": {}}
    for t in tables_of(node, db, d["founder_sub"]["sets"]):
        cols = pk_cols(node, db, t)
        if not cols:
            raise Refused("GB-390", f"{db} {t} has no primary key: the rebuild cannot tell which of its rows only the node holds, so it does not truncate it")
        key = "ROW(" + ",".join(f"x.{qi(c)}" for c in cols) + ")::text"
        mine = {r[0]: r[1:] for r in node.q(db, CTS + f"""SELECT {key}, md5(x::text), coalesce(c.roident::text, ''),
                                                                coalesce({newer}, false)::text
                                                         FROM {t} x LEFT JOIN LATERAL pg_temp.cts(x.xmin) c ON true;""")}
        theirs = {r[0]: r[1] for r in site.q(db, f"SELECT {key}, md5(x::text) FROM {t} x;")}
        replay, review, replaced = [], [], []
        for k in sorted(mine):
            md5, ro, new = mine[k]
            local, new = ro == "0", new == "true"
            origin = "this node" if local else (origins.get(ro) or (f"origin {ro}" if ro else "an unknown origin"))
            if k not in theirs:
                if local and new:
                    replay.append({"key": k, "md5": md5})
                elif local:
                    review.append({"key": k, "md5": md5, "reason": "written here before the founder's last applied commit from this node, and absent at the founder: removed there since, or never applied there"})
                else:
                    review.append({"key": k, "md5": md5, "reason": f"came from {origin}, and absent at the founder"})
            elif theirs[k] != md5:
                if local and new:
                    review.append({"key": k, "md5": md5, "reason": "changed here after the founder's last applied commit from this node; the founder's version replaces it"})
                else:
                    replaced.append(k)
        added = sorted(k for k in theirs if k not in mine)
        keep = replay + review
        if keep:
            rows = {r[0]: json.loads(r[1]) for r in node.q(db, f"SELECT {key}, row_to_json(x)::text FROM {t} x WHERE {key} = ANY({arr([r['key'] for r in keep])});")}
            for r in keep:
                r["row"] = rows.get(r["key"])
            out["site_only"][t] = {"key": cols, "replay": replay, "review": review}
        if replaced:
            out["replaced"][t] = replaced
        if added:
            out["added"][t] = {"count": len(added), "keys": added}
        print(f"export: {db} {t}: {len(replay)} to replay, {len(review)} for review, {len(replaced)} replaced by the founder's version, {len(added)} the copy adds")
    out["plan"] = plan_copy(node, site, d)
    pl = out["plan"]
    print(f"export: {db}: the copy takes {', '.join(pl['copy']) or 'no table'}; left alone, already the same as the founder's: {', '.join(pl['same']) or 'none'}")
    for t, n in pl["emptied"].items():
        print(f"export: {db}: {t} is not replicated and references a table the copy truncates: its {n} rows are emptied here and not copied")
    return out


def sha(obj):
    return hashlib.sha256(json.dumps(obj, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def rows_digest(exported):
    """sha256 of the rows only the node holds, every database. It leaves out
    the founder's own rows, which change as the founder takes writes."""
    return sha({db: e["site_only"] for db, e in sorted(exported.items()) if e["site_only"]})


def emptied_names(exported):
    """The tables that are not replicated and that the truncate empties, those
    holding rows: emptying an empty one loses nothing."""
    out = {}
    for db, e in sorted(exported.items()):
        names = sorted(t for t, n in e.get("plan", {}).get("emptied", {}).items() if n)
        if names:
            out[db] = names
    return out


def accept_digest(exported):
    """What the operator accepts: the rows only the node holds, and the names
    of the tables that are not replicated and that the truncate empties."""
    return sha({"rows": rows_digest(exported), "emptied": emptied_names(exported)})


def write_private(path, doc):
    tmp = f"{path}.{os.getpid()}.tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(doc, f, indent=2, sort_keys=True)
        f.write("\n")
    os.replace(tmp, path)


def side_file(a, suffix):
    if a.journal:
        return re.sub(r"\.json$", "", a.journal) + suffix
    raise RuntimeError(f"no --journal, so there is nowhere to write the {suffix} file")


def export_all(node, site, j):
    return {d["db"]: classify(node, site, d) for d in lost_dbs(j)}


def step_export(node, site, peers, j, a):
    quiet(node, [d["db"] for d in lost_dbs(j)])
    exported = export_all(node, site, j)
    digest_ = accept_digest(exported)
    emptied = emptied_names(exported)
    path = side_file(a, ".export.json")
    write_private(path, {"schema": EXPORT_SCHEMA, "at": now(), "digest": digest_, "databases": exported})
    n = {c: sum(len(t[c]) for e in exported.values() for t in e["site_only"].values()) for c in ("replay", "review")}
    replaced = sum(len(v) for e in exported.values() for v in e["replaced"].values())
    added = sum(v["count"] for e in exported.values() for v in e["added"].values())
    lost_local = sum(n for e in exported.values() for n in e["plan"]["emptied"].values())
    print(f"export: {path}: {n['replay']} rows to replay after the copy, {n['review']} for review, {replaced} replaced by the founder's version, {added} the copy adds, {lost_local} rows of tables that are not replicated emptied; digest {digest_}")
    if (n["replay"] + n["review"] or emptied) and a.accept != digest_:
        also = ""
        if emptied:
            also = "; the truncate also empties " + ", ".join(f"{db} {t}" for db, ts in emptied.items() for t in ts) + f" ({lost_local} rows), which are not replicated"
        raise Refused("GB-388", f"{n['replay'] + n['review']} rows exist only at the node ({n['replay']} to replay after the copy, {n['review']} for review){also}: read {path}, then run again with --accept {digest_}")
    return {"export": path, "digest": digest_, "rows_digest": rows_digest(exported), "emptied": emptied,
            **n, "replaced": replaced, "added": added}


def step_size(node, site, peers, j, a):
    total = 0
    for d in lost_dbs(j):
        n = site.one(d["db"], f"""SELECT coalesce(sum(pg_total_relation_size(t.set_reloid)), 0)::bigint
                                  FROM spock.replication_set_table t
                                  JOIN spock.replication_set r ON r.set_id = t.set_id
                                  JOIN spock.local_node l ON l.node_id = r.set_nodeid
                                  WHERE r.set_name = ANY({arr(d['founder_sub']['sets'])});""")
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


def recreate(sub_side, prov_side, db, s, label):
    """Drop a subscription whose slot at its provider is lost (which drops the
    slot) and create it again with the same provider, replication sets and
    forwarded origins and no initial copy. The provider's connection string is
    read from the subscription before the drop and held in this process; a run
    that stopped between the drop and the create reads the provider's own
    interface."""
    sub = s["sub"]
    exists = sub_side.one(db, f"SELECT count(*) FROM spock.subscription WHERE sub_name = {lit(sub)};") == "1"
    slot_now, _ = slot_state(prov_side, s["slot"])
    dsn = None
    if exists and slot_now == "lost":
        dsn = sub_side.one(db, f"""SELECT i.if_dsn FROM spock.subscription s
                                   JOIN spock.node_interface i ON i.if_id = s.sub_origin_if
                                   WHERE s.sub_name = {lit(sub)};""")
        sub_side.q(db, f"SELECT spock.sub_drop({lit(sub)});")
        print(f"reseed: {db} {label} {sub}: dropped (the lost slot with it)")
        exists = False
    if not exists:
        if not dsn:
            dsn = prov_side.one(db, """SELECT i.if_dsn FROM spock.local_node l
                                       JOIN spock.node_interface i ON i.if_id = l.node_local_interface;""")
        sub_side.q(db, f"""SELECT spock.sub_create(
                subscription_name := {lit(sub)},
                provider_dsn      := {lit(dsn)},
                replication_sets  := {arr(s['sets'])},
                synchronize_data  := false,
                forward_origins   := {arr(s['forward_origins'])});""")
        print(f"reseed: {db} {label} {sub}: created again with replication sets {','.join(s['sets'])}, no initial copy")


def step_reseed(node, site, peers, j, a):
    dbs = lost_dbs(j)
    quiet(node, [d["db"] for d in dbs])
    # The rows only the node holds must be the ones the operator accepted: a
    # change since means the export no longer covers them.
    accepted = j.get("export").get("rows_digest", "")
    digest_ = rows_digest(export_all(node, site, j))
    if digest_ != accepted:
        j.put("export", ok=False, code="GB-388", line="the rows only the node holds changed after the export step")
        raise Refused("GB-388", f"the rows only the node holds changed after the export step (digest {digest_}, accepted {accepted or 'none'}): nothing was truncated; run again, from the export step")
    paused = j.get("reseed").get("paused", [])
    byname = {}
    for p in peers.values():
        for d in dbs:
            n = local_node(p, d["db"])
            if n:
                byname[(d["db"], n)] = p
    for d in dbs:
        db, fsub = d["db"], d["founder_sub"]["sub"]
        # The node's other subscriptions wait while its tables are copied: a
        # change they apply to a row not copied yet would be discarded.
        for s in d["other_in"]:
            key = f"{db}:{s['sub']}"
            if s["enabled"] and key not in paused:
                node.q(db, f"SELECT spock.sub_disable({lit(s['sub'])}, immediate := true);")
                paused.append(key)
                j.put("reseed", ok=False, paused=paused)
                print(f"reseed: {db} {s['sub']}: disabled while the tables are copied (its slot at {s['provider']} is kept)")
        for s in d["lost_in"]:
            prov = site if s["provider"] == d["site_node"] else byname[(db, s["provider"])]
            recreate(node, prov, db, s, "the node's")
        if d["out_founder"]:
            recreate(site, node, db, d["out_founder"], "the founder's")
        # The subscription from the founder waits while the tables are copied:
        # a change it streamed between the truncate and a table's copy would
        # meet the rows the copy brings. What to copy is read after it stops:
        # a table that is the same as the founder's then stays the same, since
        # every later change reaches it through the subscription.
        node.q(db, f"SELECT spock.sub_disable({lit(fsub)}, immediate := true);")
        plan = plan_copy(node, site, d)
        more = sorted(set(t for t, n in plan["emptied"].items() if n) - set(j.get("export").get("emptied", {}).get(db, [])))
        if more:
            j.put("export", ok=False, code="GB-388", line="the truncate would empty a table the accepted export does not name")
            raise Refused("GB-388", f"{db}: the truncate would now also empty {', '.join(more)}, which is not replicated and which the accepted export does not name: nothing was truncated; run again, from the export step")
        if plan["copy"]:
            # One truncate, in repair mode so it is not replicated. It cascades
            # to the tables that reference these, as the export named.
            # spock.sub_resync_table's own truncate would cascade one table at
            # a time, emptying tables already copied.
            node.q(db, f"BEGIN; SELECT spock.repair_mode(true); TRUNCATE {', '.join(plan['copy'])} CASCADE; COMMIT;")
            print(f"reseed: {db}: truncated here in repair mode (not replicated): {', '.join(plan['copy'])}"
                  + (f"; emptied by the cascade, not replicated: {', '.join(plan['emptied'])}" if plan["emptied"] else ""))
            # Parents first: Spock copies the tables one at a time in the order
            # they are marked, and a guard trigger that runs during the copy
            # (ENABLE ALWAYS) may drop a row whose parent is not there yet; the
            # revocation guard drops an identity's credential identifiers
            # whose credential is missing.
            for t in fk_order(node, db, plan["copy"]):
                node.q(db, f"SELECT spock.sub_resync_table({lit(fsub)}, {lit(t)}::regclass, false);")
        node.q(db, f"SELECT spock.sub_enable({lit(fsub)}, immediate := true);")
        print(f"reseed: {db} {fsub}: {len(plan['copy'])} tables copied again from the founder; {len(plan['same'])} left alone, the same as the founder's: {', '.join(plan['same']) or 'none'}")
    return {"paused": paused}


def step_synced(node, site, peers, j, a):
    deadline = time.time() + a.wait
    paused = j.get("reseed").get("paused", [])
    for d in lost_dbs(j):
        db, sub = d["db"], d["founder_sub"]["sub"]
        while True:
            status = node.one(db, f"SELECT status FROM spock.sub_show_status() WHERE subscription_name = {lit(sub)};")
            pending = node.q(db, f"""SELECT sync_nspname || '.' || sync_relname, sync_status FROM spock.local_sync_status ls
                                     JOIN spock.subscription s ON s.sub_id = ls.sync_subid
                                     WHERE s.sub_name = {lit(sub)} AND ls.sync_relname IS NOT NULL AND ls.sync_status <> 'r';""")
            lost = node.q("postgres", "SELECT slot_name FROM pg_replication_slots WHERE wal_status = 'lost';")
            lost = [r for r in lost if not any(o["slot"] == r[0] for o in d["out_peers"])]
            if lost:
                raise Refused("GB-348", f"{db}: a slot on the node reads lost during the copy ({', '.join(r[0] for r in lost)}): the site can no longer take the node's writes from it")
            if status == "replicating" and not pending:
                break
            if time.time() > deadline:
                left = ", ".join(f"{r[0]}={r[1]}" for r in pending) or "none"
                raise Refused("GB-348", f"{db} {sub}: after {a.wait}s the subscription reads '{status}' and tables not yet copied: {left}")
            time.sleep(2)
        print(f"synced: {db} {sub}: replicating, every table copied")
        waits = [(node, s["sub"], "the node's") for s in d["other_in"] if f"{db}:{s['sub']}" in paused]
        for _, s, _ in waits:
            node.q(db, f"SELECT spock.sub_enable({lit(s)}, immediate := true);")
        if d["out_founder"]:
            waits.append((site, d["out_founder"]["sub"], "the founder's"))
        for side, s, label in waits:
            while True:
                status = side.one(db, f"SELECT status FROM spock.sub_show_status() WHERE subscription_name = {lit(s)};")
                if status == "replicating":
                    break
                if time.time() > deadline:
                    raise Refused("GB-348", f"{db} {label} {s}: after {a.wait}s the subscription reads '{status}', not replicating")
                time.sleep(2)
            print(f"synced: {db} {label} {s}: replicating")
    return {}


def fk_order(side, db, tables):
    """The tables, each after the tables its foreign keys reference."""
    edges = side.q(db, f"""SELECT conrelid::regclass::text, confrelid::regclass::text FROM pg_constraint
                           WHERE contype = 'f' AND conrelid <> confrelid
                             AND conrelid::regclass::text = ANY({arr(tables)}) AND confrelid::regclass::text = ANY({arr(tables)});""")
    needs = {t: {p for c, p in edges if c == t} for t in tables}
    out = []
    while needs:
        ready = sorted(t for t, n in needs.items() if not n - set(out))
        if not ready:
            out.extend(sorted(needs))
            break
        for t in ready:
            out.append(t)
            del needs[t]
    return out


def step_replay(node, site, peers, j, a):
    path = j.get("export").get("export", "")
    with open(path) as f:
        doc = json.load(f)
    if doc.get("digest") != j.get("export").get("digest"):
        raise RuntimeError(f"{path} is not the export this journal accepted")
    applied = arrived = 0
    not_applied, review = {}, {}
    for db, e in sorted(doc["databases"].items()):
        tables = fk_order(node, db, list(e["site_only"]))
        for t in tables:
            entry = e["site_only"][t]
            if entry["review"]:
                review.setdefault(db, {})[t] = entry["review"]
            rows = entry["replay"]
            if not rows:
                continue
            key = "ROW(" + ",".join(f"x.{qi(c)}" for c in entry["key"]) + ")::text"
            keys = [r["key"] for r in rows]

            def present():
                return {r[0]: r[1] for r in node.q(db, f"SELECT {key}, md5(x::text) FROM {t} x WHERE {key} = ANY({arr(keys)});")}

            have = present()
            todo = [r for r in rows if r["key"] not in have]
            errors = []
            if todo:
                # Each row in its own statement: ON_ERROR_ROLLBACK keeps the
                # rows that apply when one is refused, and the refusal's
                # reason is kept with it.
                script = "\\set ON_ERROR_ROLLBACK on\nBEGIN;\n" + "".join(
                    f"INSERT INTO {t} OVERRIDING SYSTEM VALUE SELECT * FROM json_populate_record(NULL::{t}, {lit(json.dumps(r['row']))}::json);\n"
                    for r in todo) + "COMMIT;\n"
                errors = node.run(db, script)
                have = present()
            for r in rows:
                if have.get(r["key"]) == r["md5"]:
                    if r in todo:
                        applied += 1
                    else:
                        arrived += 1
                else:
                    why = ("present after the copy with other contents" if r not in todo and r["key"] in have
                           else "; ".join(errors) or "not present after the insert")
                    not_applied.setdefault(db, {}).setdefault(t, []).append({**r, "why": why})
            print(f"replay: {db} {t}: {len(rows)} rows: {sum(1 for r in rows if have.get(r['key']) == r['md5'])} present here as exported, {len(not_applied.get(db, {}).get(t, []))} handed back")
    digest_ = hashlib.sha256(json.dumps(not_applied, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    hb = side_file(a, ".handback.json")
    write_private(hb, {"schema": HANDBACK_SCHEMA, "at": now(), "digest": digest_, "not_applied": not_applied, "review": review})
    n_not = sum(len(v) for d in not_applied.values() for v in d.values())
    n_rev = sum(len(v) for d in review.values() for v in d.values())
    print(f"replay: {applied} rows written again here, {arrived} already here as exported, {n_not} not applied; {n_rev} rows for review are kept in {hb}")
    if n_not and a.accept != digest_:
        raise Refused("GB-391", f"{n_not} rows only the node held were not written again: read {hb} (each with the reason), apply or discard them by hand, then run again with --accept {digest_}")
    return {"handback": hb, "applied": applied, "arrived": arrived, "not_applied": n_not, "review": n_rev}


def digest(side, db, t):
    return side.one(db, f"SELECT count(*) || ' ' || coalesce(md5(string_agg(md5(x::text), ',' ORDER BY md5(x::text))), '') FROM {t} x;")


def step_converged(node, site, peers, j, a):
    checked = []
    for d in lost_dbs(j):
        db = d["db"]
        for t in tables_of(node, db, d["founder_sub"]["sets"]):
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
    for d in lost_dbs(j):
        for o in d["out_peers"]:
            print(f"converged: next, rebuild {o['site']}: its subscription {o['sub']} from the node is lost")
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


RUN = {"lost": step_lost, "caught-up": step_caught_up, "export": step_export, "size": step_size,
       "reseed": step_reseed, "synced": step_synced, "replay": step_replay, "converged": step_converged}


def self_test():
    assert lit("a'b") == "'a''b'"
    assert arr(["default", "x'y"]) == "ARRAY['default','x''y']::text[]"
    assert list(RUN) == list(STEPS)
    j = Journal(None)
    j.put("lost", ok=True, dbs=[{"db": "netbird"}])
    assert lost_dbs(j)[0]["db"] == "netbird" and j.get("size") == {}
    assert verdict({"direction": "in", "provider": "site_a", "wal_status": "lost"}).startswith("cannot recover: site_a")
    assert verdict({"direction": "out", "subscriber": "", "wal_status": "lost"}).startswith("cannot recover: its subscriber")
    assert verdict({"direction": "out", "wal_status": "unreserved"}) == "can recover"
    assert verdict({"direction": "in", "wal_status": "not read"}) == "not read"
    e = Refused("GB-345", "x")
    assert str(e) == "GB-345: x"

    class Fake:
        """A side whose only answer is the foreign keys of fk_order's query."""
        def __init__(self, edges):
            self.edges = edges

        def q(self, db, sql):
            return [list(x) for x in self.edges]

    # child -> parent, grandchild -> child: parents first, whatever the order given.
    order = fk_order(Fake([("grandchild", "child"), ("child", "parent")]), "db", ["grandchild", "child", "parent", "other"])
    assert order.index("parent") < order.index("child") < order.index("grandchild"), order
    assert set(order) == {"grandchild", "child", "parent", "other"}
    # A cycle is not dropped: its tables come last, in name order.
    order = fk_order(Fake([("a", "b"), ("b", "a")]), "db", ["a", "b", "c"])
    assert order == ["c", "a", "b"], order
    # The digest is over the rows only the node holds, not the founder's.
    one = {"netbird": {"site_only": {"t": {"key": ["id"], "replay": [{"key": "(1)"}], "review": []}},
                       "replaced": {}, "added": {"t": {"count": 1, "keys": ["(2)"]}}}}
    two = json.loads(json.dumps(one))
    two["netbird"]["added"]["t"] = {"count": 5, "keys": ["(2)", "(3)", "(4)", "(5)", "(6)"]}
    assert rows_digest(one) == rows_digest(two) and accept_digest(one) == accept_digest(two)
    two["netbird"]["site_only"]["t"]["replay"].append({"key": "(7)"})
    assert rows_digest(one) != rows_digest(two)
    assert rows_digest({"netbird": {"site_only": {}}}) == rows_digest({})
    # A table the truncate empties is part of what is accepted, by name, not count.
    three = json.loads(json.dumps(one))
    three["netbird"]["plan"] = {"copy": ["t"], "same": [], "emptied": {"public.sessions": 4}}
    assert accept_digest(three) != accept_digest(one) and rows_digest(three) == rows_digest(one)
    nine = json.loads(json.dumps(three))
    nine["netbird"]["plan"]["emptied"]["public.sessions"] = 9
    assert accept_digest(three) == accept_digest(nine)
    assert emptied_names(three) == {"netbird": ["public.sessions"]}
    three["netbird"]["plan"]["emptied"]["public.sessions"] = 0
    assert emptied_names(three) == {} and accept_digest(three) == accept_digest(one)
    # The tables a truncate reaches: referrers, transitively.
    edges = [("identity_credentials", "identities"), ("sessions", "identities"), ("identities", "networks"),
             ("session_devices", "sessions"), ("courier_messages", "networks")]
    assert referrers(edges, ["identities"]) == {"identities", "identity_credentials", "sessions", "session_devices"}
    assert referrers(edges, []) == set()
    # A journal another version wrote is refused, not resumed.
    import tempfile
    with tempfile.TemporaryDirectory() as d:
        old = os.path.join(d, "j.json")
        with open(old, "w") as f:
            json.dump({"schema": "goatbuilder.spock-reseed/v1", "steps": {"lost": {"ok": True}}}, f)
        try:
            Journal(old)
            raise AssertionError("a v1 journal was resumed")
        except RuntimeError as err:
            assert "another version" in str(err)

        class A:
            journal = os.path.join(d, "join-node-x-reseed.json")
        assert side_file(A, ".export.json").endswith("join-node-x-reseed.export.json")
        write_private(os.path.join(d, "e.json"), {"x": 1})
        assert os.stat(os.path.join(d, "e.json")).st_mode & 0o777 == 0o600
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
    ap.add_argument("--accept", default="", metavar="DIGEST")
    a = ap.parse_args()
    node = Side("node", a.node_psql)
    peers = {}
    for p in a.peer_psql:
        name, sep, cmd = p.partition("=")
        if not sep or not name or not cmd:
            print(f"spock-reseed: --peer-psql takes NAME=CMD, not {p!r}", file=sys.stderr)
            return 2
        peers[name] = Side(name, cmd)
    if a.step == "slots":
        others = dict(peers)
        if a.site_psql:
            others["site"] = Side("site", a.site_psql)
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
    try:
        j = Journal(a.journal)
    except (RuntimeError, ValueError, OSError) as e:
        print(f"spock-reseed: {e}", file=sys.stderr)
        return 2
    if a.step != "lost" and not j.get("lost").get("ok"):
        print("GB-345: the lost step has not passed in this journal; run it first", file=sys.stderr)
        return 1
    # What a step recorded before it stopped (the subscriptions the reseed
    # step paused) stays in its entry when it stops again.
    kept = {k: v for k, v in j.get(a.step).items() if k not in ("at", "ok", "code", "line")}
    try:
        out = RUN[a.step](node, site, peers, j, a)
    except Refused as e:
        kept.update({k: v for k, v in j.get(a.step).items() if k not in ("at", "ok", "code", "line")})
        j.put(a.step, ok=False, code=e.code, line=e.line, **kept)
        print(f"{e.code}: {e.line}", file=sys.stderr)
        return 1
    except (RuntimeError, subprocess.TimeoutExpired, OSError, KeyError, ValueError) as e:
        # Not a refusal: a side could not be reached or a statement failed.
        kept.update({k: v for k, v in j.get(a.step).items() if k not in ("at", "ok", "code", "line")})
        j.put(a.step, ok=False, code="", line=str(e), **kept)
        print(f"spock-reseed: {a.step}: {e}", file=sys.stderr)
        return 2
    j.put(a.step, ok=True, **out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
