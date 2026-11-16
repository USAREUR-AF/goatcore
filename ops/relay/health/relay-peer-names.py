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


def publish(names, owner_read=None):
    if LOCAL_DIR and os.path.isdir(LOCAL_DIR):
        # id_console_url is Thor-local only (the id-admin-console is mesh-internal;
        # the cloud box can't reach it, so the deep-link stays off the cloud dashboard).
        doc = {"names": names, "generated_at": int(time.time()), "count": len(names),
               "id_console_url": ID_CONSOLE_URL, "owner_read": owner_read}
        tmp = os.path.join(LOCAL_DIR, ".peer-names.json.tmp")
        dst = os.path.join(LOCAL_DIR, "peer-names.json")
        with open(tmp, "w") as f:
            json.dump(doc, f)
        os.replace(tmp, dst)
        log(f"wrote {dst} count={len(names)}")
    body = json.dumps(names).encode()
    for base in URLS:
        url = base.rstrip("/") + "/peer-names"
        req = urllib.request.Request(
            url, data=body, method="POST",
            headers={"Authorization": "Bearer " + TOKEN, "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(req, timeout=20) as r:
                log(f"POST {url} -> {r.status}")
        except Exception as e:  # best-effort — a down cloud box must not fail the Thor write
            log(f"POST {url} FAILED: {e}")


def enrich_owners(cards):
    """Ask the portal who owns every peer, in one batch, and write the answer
    onto each card: `owner` (the resolver's owner block: where the owner came
    from, the organization and its nation, the responsible person's handle, the
    person's own nationality, each with a state) and, as before, `ory_id`.

    Every peer is asked about, machines and infrastructure included: the owner
    of a `machine-` peer is its recorded custodian, and a name that carries no
    owner is itself the answer a surface must show.

    Never leaves a card without an owner state. When the portal cannot be
    asked (no key on this host, refused, unreachable), every card carries
    `owner: {source: "unread", reason}` and the file says so once
    (`owner_read`), so a refused key is a visible state on every surface that
    reads this file and never reads as "no owner".

    Returns the file-level `owner_read` record."""
    at = int(time.time())
    if peerowner is None:
        why = "the shared client ops/identity/peerowner.py is not installed on this host"
    elif not PORTAL_KEY:
        why = "no portal key is configured for this writer (relay_peer_names_portal_key)"
    else:
        why = ""
    if why:
        log(f"owners not read: {why}")
        for c in cards.values():
            c["owner"] = {"source": "unread", "reason": why}
        return {"state": "unread", "reason": why, "at": at}
    names = sorted({c["name"] for c in cards.values() if c.get("name")})
    rows = peerowner.resolve(portal_base(), PORTAL_KEY, names)
    unread = [r for r in rows.values() if r["attribution"] == peerowner.UNREAD]
    if unread and len(unread) == len(rows):
        why = unread[0]["reason"] or "the portal could not be asked"
        log(f"owners not read: {why}")
        for c in cards.values():
            c["owner"] = {"source": "unread", "reason": why}
        return {"state": "unread", "reason": why, "at": at}
    counts = {}
    for c in cards.values():
        row = rows.get(c.get("name") or "")
        if row is None:
            c["owner"] = {"source": "unread", "reason": "the resolver returned no row for this name"}
        else:
            c["owner"] = row["owner"] or {"source": "none"}
            if row["ory_id"]:
                c["ory_id"] = row["ory_id"]
        src = c["owner"].get("source") or "none"
        counts[src] = counts.get(src, 0) + 1
    log("owners: " + ", ".join(f"{k}={v}" for k, v in sorted(counts.items())))
    return {"state": "read", "counts": counts, "at": at}


if __name__ == "__main__":
    names = fetch_peers()
    if not names:
        log("no peers resolved — refusing to publish an empty map")
        sys.exit(1)
    log(f"resolved {len(names)} peer names")
    owner_read = enrich_owners(names)
    publish(names, owner_read)
