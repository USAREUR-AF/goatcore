#!/usr/bin/env python3
"""relay-peer-names — resolve relay per-peer hashed IDs to friendly NetBird names (ADR 1055).

The relay per-peer stats key each peer by the relay PeerID, which the stats endpoint
renders as hex:  hex("sha-") + sha256(wg_pubkey)  (verified end-to-end: the relay
client hashes myPrivateKey.PublicKey().String() via messages.HashID, and HashID is
[]byte("sha-") ++ sha256(peerID)). That hash is one-way, so a dashboard can only show
the hex unless something holds the pubkey->name preimage. The NetBird management store
(Postgres) does — and only the Thor can reach it.

This runs on the Thor, reads (key,name) for every peer, computes the exact hex the relay
emits, and publishes a {hex: name} map to the relay-health ingest(s) so BOTH the Thor
mesh dashboard and the cloud dashboard render names with the hex revealed on hover:
  - writes peer-names.json into the local ingest dir (the Thor's own www/ingest), and
  - POSTs the map to any remote ingest URLs (the cloud box), reusing the ADR 1058
    bearer-gated channel (POST <base>/peer-names).

The map is only pubkey-derived hashes + display names — never a pubkey, never a cert.
The served file sits behind the same ADMINS oauth2-proxy gate as the dashboard.

Env:
  RELAY_PEER_NAMES_PG_CONTAINER   mgmt postgres container (default postgres-a)
  RELAY_PEER_NAMES_PG_DB          database (default netbird)
  RELAY_PEER_NAMES_PG_USER        db user (default postgres)
  RELAY_PEER_NAMES_LOCAL_DIR      local ingest dir to write peer-names.json into (optional)
  RELAY_PEER_NAMES_INGEST_URLS    comma-sep remote ingest base URLs (…/relay-metrics-ingest) (optional)
  RELAY_METRICS_BEARER_TOKEN      bearer for the remote POST (required when URLS set)
"""
import hashlib
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

# The shared peer -> person client (ops/identity/peerowner.py). This resolver
# had its own inline POST and its own partial reading of the wire format; so did
# peerview, and panoscope_core::peerowner is the same thing again in Rust. One
# module per language, not one per consumer — the four-state attribution is the
# part worth not re-deriving. Optional: a deploy that has not shipped it keeps
# working, without ory ids.
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                "..", "..", "identity"))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
try:
    import peerowner
except ImportError:
    peerowner = None

PG_C = os.environ.get("RELAY_PEER_NAMES_PG_CONTAINER", "postgres-a")
PG_DB = os.environ.get("RELAY_PEER_NAMES_PG_DB", "netbird")
PG_U = os.environ.get("RELAY_PEER_NAMES_PG_USER", "postgres")
LOCAL_DIR = os.environ.get("RELAY_PEER_NAMES_LOCAL_DIR", "")
URLS = [u.strip() for u in os.environ.get("RELAY_PEER_NAMES_INGEST_URLS", "").split(",") if u.strip()]
TOKEN = os.environ.get("RELAY_METRICS_BEARER_TOKEN", "")
PREFIX = b"sha-".hex()  # 7368612d — the relay renders the PeerID as raw hex bytes
# Peer→person deep-link (peer-identity-resolution.md, Option B). When set, the
# resolver batch-asks the portal to map each card's peer name to its owning Ory
# identity id and bakes it into the card, so the dashboard can deep-link the
# id-admin-console person page. Best-effort: portal down/unset -> no ory id, no link.
PORTAL_URL = os.environ.get("RELAY_PEER_NAMES_PORTAL_URL", "")          # explicit portal base (wins if set)
PORTAL_KEY = os.environ.get("RELAY_PEER_NAMES_PORTAL_KEY", "")          # operator api key (server-to-server)
ID_CONSOLE_URL = os.environ.get("RELAY_PEER_NAMES_ID_CONSOLE_URL", "")  # id-admin-console base, for the card link
# The Thor resolver can't reach the portal via its mesh hostname (mesh DNS doesn't
# resolve it from the Thor itself) and localhost:8088 is the oauth2/ADMINS gate, not
# the app. The app answers the operator Bearer key on its own port inside the docker
# network. So when PORTAL_URL is unset, resolve the portal container's current IP on
# its docker network at runtime (survives container restarts, no host port publish).
PORTAL_CONTAINER = os.environ.get("RELAY_PEER_NAMES_PORTAL_CONTAINER", "")  # e.g. goat-zenoh-portal
PORTAL_NET = os.environ.get("RELAY_PEER_NAMES_PORTAL_NET", "mgmt")          # docker network the app listens on
PORTAL_PORT = os.environ.get("RELAY_PEER_NAMES_PORTAL_PORT", "8090")        # app (not oauth2) port
# The ownership reconcile (ADR 1249 decision 5). OWNER_TEXTFILE: a node-exporter
# textfile path for the goat_peer_owner_* metrics (empty = off).
# OWNER_EXCEPTIONS: the goatnet's owner exceptions (decision 7), a file in the
# private overlay: one entry per peer the operator has excepted from the owner
# rule for now, with a reason. An excepted peer is shown as an exception on
# every surface and counted on its own, never as recorded and never hidden.
# Schema: ops/identity/peer-owner-exceptions.schema.json.
OWNER_TEXTFILE = os.environ.get("RELAY_PEER_NAMES_OWNER_TEXTFILE", "")
OWNER_EXCEPTIONS = os.environ.get("RELAY_PEER_NAMES_OWNER_EXCEPTIONS", "")


def portal_base():
    """Where the Thor resolver reaches the portal API. Delegates to the shared
    client so the docker-inspect fallback lives in one place."""
    if peerowner is None:
        return PORTAL_URL.rstrip("/") if PORTAL_URL else ""
    return peerowner.portal_base(PORTAL_URL, PORTAL_CONTAINER, PORTAL_NET,
                                 PORTAL_PORT, log=log)


def log(m):
    sys.stderr.write("relay-peer-names: " + m + "\n")
    sys.stderr.flush()


# Card fields, in the column order of the SELECT below. 'key' (the pubkey) is
# column 0 and never leaves this process — only its hash is published. Removed
# with log() in the shared-client refactor (#2696) while both were still used,
# so every run raised NameError until they were restored.
_CARD_COLS = ["name", "id", "aff", "ip", "os", "host", "seen", "conn"]


def fetch_peers():
    """Return {relay_hex_id: {name,id,aff,ip,os,host,seen,conn}} — the baseball-card
    fields for every peer with a pubkey. Affiliation is the peer's NetBird groups
    minus the default 'All' (e.g. ZAGROS-DEVELOPER/ZAGROS-OBSERVER); '' when only 'All'."""
    sql = (
        "SELECT p.\"key\", p.name, p.id, "
        "COALESCE(string_agg(g.name, ',' ORDER BY g.name) "
        "  FILTER (WHERE g.name IS NOT NULL AND g.name <> 'All'), ''), "
        "p.ip, COALESCE(NULLIF(p.meta_os_version,''), p.meta_os), p.meta_hostname, "
        "p.peer_status_last_seen, p.peer_status_connected "
        "FROM peers p "
        "LEFT JOIN group_peers gp ON gp.peer_id = p.id "
        "LEFT JOIN groups g ON g.id = gp.group_id "
        "WHERE p.\"key\" <> '' "
        "GROUP BY p.id;"
    )
    out = subprocess.check_output(
        ["docker", "exec", PG_C, "psql", "-U", PG_U, "-d", PG_DB,
         "-t", "-A", "-F", "\t", "-c", sql],
        text=True, timeout=30,
    )
    m = {}
    for line in out.splitlines():
        parts = line.split("\t")
        if len(parts) < 2 or not parts[0].strip():
            continue
        key = parts[0].strip()
        hid = PREFIX + hashlib.sha256(key.encode()).hexdigest()
        vals = (parts[1:] + [""] * len(_CARD_COLS))[: len(_CARD_COLS)]
        # some columns (e.g. ip) come back wrapped in literal double-quotes.
        card = {c: (v.strip().strip('"') if v else "") for c, v in zip(_CARD_COLS, vals)}
        if not card["name"]:
            card["name"] = hid[:14]
        card["conn"] = card["conn"] in ("t", "true", "True")
        m[hid] = card
    return m


# The cloud relay-health box gets the names it always had and the owner STATE
# only: no person, identity id, organization or nationality leaves the
# management host (operator decision, 2026-11-16). The full owner block stays
# in the on-host file.
_REMOTE_CARD_KEYS = tuple(_CARD_COLS)
# The cloud ingest's body cap (relay-metrics-ingest.py RELAY_METRICS_MAX_BYTES).
REMOTE_MAX_BYTES = int(os.environ.get("RELAY_PEER_NAMES_REMOTE_MAX_BYTES", "262144"))
# A reserved key in the flat names map: an ingest that predates it stores it as
# one more entry, which no card lookup matches; a current ingest lifts it to
# the top of its file.
REMOTE_META_KEY = "__peer_names_meta__"


def remote_body(names, owner_read, generated_at):
    """The body POSTed to the cloud ingest: each card's own fields and its
    owner state (`source`, and whether an owner exception covers it)."""
    out = {}
    for hid, c in names.items():
        card = {k: c[k] for k in _REMOTE_CARD_KEYS if k in c}
        o = c.get("owner") or {}
        card["owner"] = {"source": o.get("source") or "unread",
                         "exception": bool(c.get("owner_exception"))}
        out[hid] = card
    out[REMOTE_META_KEY] = {"generated_at": generated_at,
                            "owner_read": {k: v for k, v in (owner_read or {}).items()
                                           if k in ("state", "reason", "at", "counts")}}
    return json.dumps(out).encode()


def publish(names, owner_read=None, now=None):
    """Write the on-host file (full owner blocks) and POST the stripped body to
    each cloud ingest. Returns the number of remote failures the operator must
    see: a body over the ingest's cap, or an ingest that refused it. A cloud
    box that cannot be reached is logged and not counted: it must not fail the
    management host's own write."""
    generated_at = int(now if now is not None else time.time())
    if LOCAL_DIR and os.path.isdir(LOCAL_DIR):
        # id_console_url is Thor-local only (the id-admin-console is mesh-internal;
        # the cloud box can't reach it, so the deep-link stays off the cloud dashboard).
        doc = {"names": names, "generated_at": generated_at, "count": len(names),
               "id_console_url": ID_CONSOLE_URL, "owner_read": owner_read}
        tmp = os.path.join(LOCAL_DIR, ".peer-names.json.tmp")
        dst = os.path.join(LOCAL_DIR, "peer-names.json")
        with open(tmp, "w") as f:
            json.dump(doc, f)
        os.replace(tmp, dst)
        log(f"wrote {dst} count={len(names)}")
    if not URLS:
        return 0
    body = remote_body(names, owner_read, generated_at)
    if len(body) > REMOTE_MAX_BYTES:
        log(f"remote peer-names NOT sent: body {len(body)} bytes is over the ingest cap {REMOTE_MAX_BYTES}")
        return len(URLS)
    failures = 0
    for base in URLS:
        url = base.rstrip("/") + "/peer-names"
        req = urllib.request.Request(
            url, data=body, method="POST",
            headers={"Authorization": "Bearer " + TOKEN, "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(req, timeout=20) as r:
                log(f"POST {url} -> {r.status}")
        except urllib.error.HTTPError as e:
            log(f"POST {url} REFUSED: HTTP {e.code}")
            failures += 1
        except Exception as e:  # noqa: BLE001 — a down cloud box must not fail the Thor write
            log(f"POST {url} FAILED: {e}")
    return failures


def enrich_owners(cards):
    """Ask the portal who owns every peer and write the answer onto each card:
    `owner` (the resolver's owner block: where the owner came from, the
    organization and its nation, the responsible person's handle, the person's
    own nationality, each with a state) and, as before, `ory_id`.

    Every peer is asked about, machines and infrastructure included: the owner
    of a `machine-` peer is its recorded custodian, and a name that carries no
    owner is itself the answer a surface must show.

    Never leaves a card without an owner state, and never turns a failed read
    into "no owner". When the portal cannot be asked (no key on this host,
    refused, unreachable, a malformed answer, a portal too old to say, or a
    portal whose own owner sources failed), the cards carry
    `owner: {source: "unread", reason}`, and when every card is unread the file
    says so once (`owner_read`).

    Returns the file-level `owner_read` record."""
    at = int(time.time())
    if peerowner is None:
        why = "the shared client ops/identity/peerowner.py is not installed on this host"
    elif not PORTAL_KEY:
        why = "no portal key is configured for this writer (relay_peer_names_portal_key)"
    else:
        why = ""
    if why:
        return _all_unread(cards, why, at)
    names = sorted({c["name"] for c in cards.values() if c.get("name")})
    try:
        rows = peerowner.resolve(portal_base(), PORTAL_KEY, names)
    except Exception as e:  # noqa: BLE001 — a client defect is an unread cycle, not a crash
        return _all_unread(cards, f"the owner lookup failed ({e})", at)
    counts = {}
    for c in cards.values():
        row = rows.get(c.get("name") or "")
        if row is None:
            c["owner"] = {"source": "unread", "reason": "the resolver returned no row for this name"}
        else:
            c["owner"] = row.get("owner") or {"source": "unread", "reason": "the resolver returned no owner"}
            if row.get("ory_id"):
                c["ory_id"] = row["ory_id"]
        src = c["owner"].get("source") or "unread"
        counts[src] = counts.get(src, 0) + 1
    if counts and set(counts) == {"unread"}:
        why = next((c["owner"].get("reason") for c in cards.values() if c["owner"].get("reason")),
                   "the portal could not be asked")
        log(f"owners not read: {why}")
        return {"state": "unread", "reason": why, "at": at}
    log("owners: " + ", ".join(f"{k}={v}" for k, v in sorted(counts.items())))
    return {"state": "read", "counts": counts, "at": at}


def _all_unread(cards, why, at):
    log(f"owners not read: {why}")
    for c in cards.values():
        c["owner"] = {"source": "unread", "reason": why}
    return {"state": "unread", "reason": why, "at": at}


# ADR 1249: only the record (or a declaration) makes a peer compliant. A roster
# answer is a lead, not a record, so it is counted unrecorded like no answer.
_COMPLIANT = {"record", "declared"}
EXCEPTIONS_SCHEMA = "goat.peer-owner-exceptions/v1"
_EXCEPTION_FIELDS = ("reason", "granted_by", "granted_on", "review_by")


def peer_class(name):
    if name.startswith("machine-"):
        return "machine"
    if name.startswith("peer-"):
        return "person"
    return "other"


_EXC_REQUIRED = ("peer_id", "name", "reason", "granted_by", "granted_on")
_EXC_ALLOWED = set(_EXC_REQUIRED) | {"review_by"}


def _is_date(v):
    import datetime
    if not isinstance(v, str) or len(v) != 10:
        return False
    try:
        datetime.date.fromisoformat(v)
        return True
    except ValueError:
        return False


class Exceptions(dict):
    """{peer_id: entry}, plus how the file loaded: `configured` (a path is
    set), `loaded` (the file was read and has the schema), `refused` (entries
    not used, each logged with why), `problems` (the reasons, for
    --check-exceptions)."""

    def __init__(self, configured=False):
        super().__init__()
        self.configured, self.loaded, self.refused, self.problems = configured, False, 0, []


def load_exceptions(path):
    """The goatnet's owner exceptions, checked against
    ops/identity/peer-owner-exceptions.schema.json. Never raises.

    A file that is missing, unreadable, not the schema, or not the right shape
    is not loaded: no exception applies and every unrecorded peer is counted,
    which errs toward reporting; the metrics say the file did not load. An
    entry that breaks the schema (a required field missing or empty, a date
    that is not YYYY-MM-DD, a key the schema does not have) is refused and
    counted. Two entries for one peer id are both refused: neither is the
    decision."""
    out = Exceptions(configured=bool(path))
    if not path:
        return out

    def problem(m):
        out.problems.append(m)
        log(f"owner exceptions {path}: {m}")

    try:
        with open(path) as f:
            doc = json.load(f)
    except Exception as e:  # noqa: BLE001
        problem(f"not read ({e}); no exception applies")
        return out
    if not isinstance(doc, dict) or doc.get("schema") != EXCEPTIONS_SCHEMA:
        problem(f"schema is not {EXCEPTIONS_SCHEMA}; no exception applies")
        return out
    entries = doc.get("exceptions")
    if not isinstance(entries, list) or set(doc) - {"schema", "exceptions"}:
        problem("not {schema, exceptions: [...]}; no exception applies")
        return out
    out.loaded = True
    good, seen = [], {}
    for e in entries:
        if not isinstance(e, dict):
            out.refused += 1
            problem("an entry that is not an object was refused")
            continue
        who = e.get("name") or e.get("peer_id") or "?"
        why = [f"{k} missing" for k in _EXC_REQUIRED if not isinstance(e.get(k), str) or not e.get(k)]
        why += [f"{k} is not YYYY-MM-DD" for k in ("granted_on", "review_by")
                if k in e and e.get(k) and not _is_date(e.get(k))]
        why += [f"unknown key {k}" for k in sorted(set(e) - _EXC_ALLOWED)]
        if why:
            out.refused += 1
            problem(f"exception for {who} refused: {', '.join(why)}")
            continue
        good.append(e)
        seen[e["peer_id"]] = seen.get(e["peer_id"], 0) + 1
    for e in good:
        if seen[e["peer_id"]] > 1:
            out.refused += 1
            problem(f"exception for {e['name']} refused: peer_id {e['peer_id']} is listed {seen[e['peer_id']]} times")
            continue
        out[e["peer_id"]] = e
    return out


def grade(card, exceptions):
    """recorded | excepted | exception_stale | lead | unrecorded | unread.

    An exception applies to the peer it names by its stable NetBird peer id.
    Once the record names an owner, the exception is stale: the peer counts as
    recorded and the exception is reported so it can be removed."""
    src = (card.get("owner") or {}).get("source") or "none"
    exc = exceptions.get(card.get("id") or "")
    if src == "unread":
        return "unread"
    if src in _COMPLIANT:
        return "exception_stale" if exc else "recorded"
    if exc:
        return "excepted"
    return "lead" if src == "roster" else "unrecorded"


def mark_exceptions(cards, exceptions):
    """Put each applying exception on its card, so every surface that reads
    peer-names.json shows "owner exception" with its reason."""
    for c in cards.values():
        e = exceptions.get(c.get("id") or "")
        if not e:
            continue
        stale = grade(c, exceptions) == "exception_stale"
        c["owner_exception"] = dict({k: e.get(k) for k in _EXCEPTION_FIELDS if e.get(k)},
                                    state="stale" if stale else "active")


_CARRIED = ("goat_peer_owner_unrecorded{", "goat_peer_owner_excepted{",
            "goat_peer_owner_exception_stale{", "goat_peer_owner_counts_timestamp_seconds ")


def _carried(previous):
    """The count lines of the last cycle that read owners, from the previous
    textfile, so an unread cycle keeps the last known count instead of 0."""
    out = []
    for line in (previous or "").splitlines():
        if line.startswith(_CARRIED):
            out.append(line)
    return out


def owner_metrics(cards, owner_read, exceptions, now=None, previous=None):
    """The goat_peer_owner_* textfile. Counts only, no names: the names of the
    unrecorded peers go to the journal (relay-peer-names log lines).

    A cycle that did not read owners writes no fresh count: it carries the
    last read cycle's unrecorded, excepted and stale counts and the time they
    were read (`goat_peer_owner_counts_timestamp_seconds`), so a failed lookup
    neither clears the critical page nor raises one. With no previous read,
    the counts are absent."""
    now = int(now if now is not None else time.time())
    read = (owner_read or {}).get("state") == "read"
    present = {c.get("id") for c in cards.values()}
    counts, unrecorded, excepted, stale = {}, {}, {}, 0
    for c in cards.values():
        st = grade(c, exceptions)
        cl = peer_class(c.get("name") or "")
        key = (st, cl, "true" if c.get("conn") else "false")
        counts[key] = counts.get(key, 0) + 1
        if st in ("unrecorded", "lead"):
            unrecorded[cl] = unrecorded.get(cl, 0) + 1
        elif st == "excepted":
            excepted[cl] = excepted.get(cl, 0) + 1
        elif st == "exception_stale":
            stale += 1
    gone = sum(1 for pid in exceptions if pid not in present)
    out = [
        "# HELP goat_peer_owner_peers Mesh peers by owner state (ADR 1249): recorded, excepted, exception_stale, lead, unrecorded, unread.",
        "# TYPE goat_peer_owner_peers gauge",
    ]
    for (st, cl, conn), n in sorted(counts.items()):
        out.append(f'goat_peer_owner_peers{{state="{st}",class="{cl}",connected="{conn}"}} {n}')
    help_ = [
        "# HELP goat_peer_owner_unrecorded Mesh peers with no recorded or declared owner and no owner exception. Expected 0. Carried from the last read cycle when this one did not read owners.",
        "# TYPE goat_peer_owner_unrecorded gauge",
        "# HELP goat_peer_owner_excepted Mesh peers under an owner exception the operator granted, by class.",
        "# TYPE goat_peer_owner_excepted gauge",
        "# HELP goat_peer_owner_exception_stale Owner exceptions to remove: the peer now has a recorded owner (reason=recorded) or is no longer on the mesh (reason=peer_gone).",
        "# TYPE goat_peer_owner_exception_stale gauge",
        "# HELP goat_peer_owner_counts_timestamp_seconds When the owner counts above were read.",
        "# TYPE goat_peer_owner_counts_timestamp_seconds gauge",
    ]
    if read:
        out += help_
        for cl in ("machine", "person", "other"):
            out.append(f'goat_peer_owner_unrecorded{{class="{cl}"}} {unrecorded.get(cl, 0)}')
        for cl in ("machine", "person", "other"):
            out.append(f'goat_peer_owner_excepted{{class="{cl}"}} {excepted.get(cl, 0)}')
        out += [f'goat_peer_owner_exception_stale{{reason="recorded"}} {stale}',
                f'goat_peer_owner_exception_stale{{reason="peer_gone"}} {gone}',
                f"goat_peer_owner_counts_timestamp_seconds {now}"]
    else:
        carried = _carried(previous)
        if carried:
            out += help_ + carried
    out += [
        "# HELP goat_peer_owner_read 1 when the portal's owner resolver answered this cycle, 0 when it could not be asked.",
        "# TYPE goat_peer_owner_read gauge",
        f"goat_peer_owner_read {1 if read else 0}",
        "# HELP goat_peer_owner_exceptions_configured 1 when this host is given an owner exceptions file.",
        "# TYPE goat_peer_owner_exceptions_configured gauge",
        f"goat_peer_owner_exceptions_configured {1 if getattr(exceptions, 'configured', False) else 0}",
        "# HELP goat_peer_owner_exceptions_loaded 1 when the owner exceptions file was read and has the schema.",
        "# TYPE goat_peer_owner_exceptions_loaded gauge",
        f"goat_peer_owner_exceptions_loaded {1 if getattr(exceptions, 'loaded', False) else 0}",
        "# HELP goat_peer_owner_exceptions_refused Owner exception entries not used because they break the schema or repeat a peer id.",
        "# TYPE goat_peer_owner_exceptions_refused gauge",
        f"goat_peer_owner_exceptions_refused {getattr(exceptions, 'refused', 0)}",
        "# HELP goat_peer_owner_last_run_timestamp_seconds When the reconcile last ran.",
        "# TYPE goat_peer_owner_last_run_timestamp_seconds gauge",
        f"goat_peer_owner_last_run_timestamp_seconds {now}",
    ]
    return "\n".join(out) + "\n"


def reconcile(cards, owner_read, exceptions=None):
    """Write the textfile and log the names an operator must act on."""
    if exceptions is None:
        exceptions = load_exceptions(OWNER_EXCEPTIONS)
    if (owner_read or {}).get("state") == "read":
        bad = sorted(c["name"] for c in cards.values() if grade(c, exceptions) in ("unrecorded", "lead"))
        if bad:
            log(f"owner reconcile: {len(bad)} peer(s) with no recorded owner and no owner exception: "
                + ", ".join(bad[:50]) + (" …" if len(bad) > 50 else ""))
        stale = sorted(c["name"] for c in cards.values() if grade(c, exceptions) == "exception_stale")
        if stale:
            log(f"owner reconcile: {len(stale)} owner exception(s) to remove, the peer now has a recorded owner: "
                + ", ".join(stale[:50]))
    present = {c.get("id") for c in cards.values()}
    gone = sorted((e.get("name") or pid) for pid, e in exceptions.items() if pid not in present)
    if gone:
        log(f"owner reconcile: {len(gone)} owner exception(s) to remove, the peer has left the mesh: "
            + ", ".join(gone[:50]))
    unread = sorted(c["name"] for c in cards.values() if grade(c, exceptions) == "unread")
    if unread and (owner_read or {}).get("state") == "read":
        log(f"owner reconcile: {len(unread)} peer(s) whose owner was not read: " + ", ".join(unread[:50]))
    if not OWNER_TEXTFILE:
        return
    try:
        with open(OWNER_TEXTFILE) as f:
            previous = f.read()
    except OSError:
        previous = ""
    body = owner_metrics(cards, owner_read, exceptions, previous=previous)
    tmp = OWNER_TEXTFILE + ".tmp"
    with open(tmp, "w") as f:
        f.write(body)
    os.replace(tmp, OWNER_TEXTFILE)


def write_exceptions(cards, owner_read, path, granted_by, reason, granted_on):
    """Write the goatnet's initial owner exceptions once: one entry for every
    peer present now without a recorded or declared owner. Refuses when the
    owners were not read (a file made blind would except everyone) and never
    overwrites (later changes are made by hand, in the overlay). The operator
    states the grant date; review_by is never set here, only the operator sets
    it."""
    if (owner_read or {}).get("state") != "read":
        raise SystemExit("refusing to write owner exceptions: owners were not read this run")
    if not granted_by or not reason:
        raise SystemExit("--granted-by and --reason are required: an exception is a decision someone made")
    if not _is_date(granted_on):
        raise SystemExit("--granted-on YYYY-MM-DD is required: the date the operator granted the exceptions")
    entries = [{"peer_id": c["id"], "name": c["name"], "reason": reason,
                "granted_by": granted_by, "granted_on": granted_on}
               for c in sorted(cards.values(), key=lambda c: c["name"])
               if c.get("id") and grade(c, {}) in ("unrecorded", "lead")]
    try:
        f = open(path, "x")
    except FileExistsError:
        raise SystemExit(f"refusing to overwrite {path}: edit the exceptions by hand in the overlay")
    with f:
        json.dump({"schema": EXCEPTIONS_SCHEMA, "exceptions": entries}, f, indent=1)
        f.write("\n")
    log(f"wrote {len(entries)} owner exception(s) to {path} (granted_on {granted_on}, no review_by)")


def main(argv):
    import argparse
    ap = argparse.ArgumentParser(prog="relay-peer-names.py",
                                 description="Publish peer names and owners; reconcile peer owners (ADR 1249).")
    ap.add_argument("--write-exceptions", metavar="PATH",
                    help="write the goatnet's initial owner exceptions file and exit")
    ap.add_argument("--granted-by", help="who granted the exceptions (with --write-exceptions)")
    ap.add_argument("--reason", help="why these peers are excepted (with --write-exceptions)")
    ap.add_argument("--granted-on", help="YYYY-MM-DD the operator granted them (with --write-exceptions)")
    ap.add_argument("--check-exceptions", metavar="PATH",
                    help="check an owner exceptions file against the schema and exit (0 = every entry is used)")
    a = ap.parse_args(argv)
    if a.check_exceptions:
        exc = load_exceptions(a.check_exceptions)
        if not exc.loaded or exc.refused:
            return 1
        log(f"owner exceptions {a.check_exceptions}: {len(exc)} entr{'y' if len(exc) == 1 else 'ies'}, all used")
        return 0
    if a.write_exceptions and not (a.granted_by and a.reason and _is_date(a.granted_on)):
        ap.error("--write-exceptions needs --granted-by, --reason and --granted-on YYYY-MM-DD")
    names = fetch_peers()
    if not names:
        log("no peers resolved — refusing to publish an empty map")
        return 1
    log(f"resolved {len(names)} peer names")
    owner_read = enrich_owners(names)
    if a.write_exceptions:
        write_exceptions(names, owner_read, a.write_exceptions, a.granted_by, a.reason, a.granted_on)
        return 0
    exceptions = load_exceptions(OWNER_EXCEPTIONS)
    mark_exceptions(names, exceptions)
    failures = publish(names, owner_read)
    reconcile(names, owner_read, exceptions)
    return 3 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
