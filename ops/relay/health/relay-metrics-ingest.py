#!/usr/bin/env python3
"""relay-metrics-ingest — token-gated sink for relay self-published metrics (ADR 1058).

Runs as a small container on the Thor's mgmt network. Each relay POSTs its own
host metrics (the relay-health "deep" shape, read locally — no SSM) to
  POST /relay-metrics-ingest/<relay-name>   Authorization: Bearer <token>
This validates the bearer token + that <relay-name> is a known relay, then
writes <name>.json into INGEST_DIR for the relay-health prober to fold in
(relay-health-probe.sh --ingest-dir). Stdlib only, so it runs under a stock
pinned python image with the script bind-mounted — no custom image to build.

Fail-closed: missing/wrong token -> 401; unknown relay -> 404; oversized or
non-JSON body -> 400. Never trusts the relay's clock — stamps server received_at.

Env:
  RELAY_METRICS_BEARER_TOKEN   shared bearer token (required; refuses to start if unset)
  RELAY_METRICS_ALLOWED_NAMES  comma-separated known relay names (required)
  RELAY_METRICS_INGEST_DIR     output dir (default /ingest)
  RELAY_METRICS_LISTEN_PORT    listen port (default 8090)
  RELAY_METRICS_MAX_BYTES      max body size (default 262144)
"""
import json
import os
import re
import sqlite3
import stat
import sys
import tempfile
import threading
import time
from functools import wraps
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

TOKEN = os.environ.get("RELAY_METRICS_BEARER_TOKEN", "")
ALLOWED = {n.strip() for n in os.environ.get("RELAY_METRICS_ALLOWED_NAMES", "").split(",") if n.strip()}
# Site network hosts (mgmt hosts) allowed to publish host-net health
# (host-net-publish.sh). Separate allowlist: empty = the feature is off and
# /host-net-ingest 404s. Same bearer token — one trust domain.
HOST_ALLOWED = {n.strip() for n in os.environ.get("HOST_NET_ALLOWED_NAMES", "").split(",") if n.strip()}
# Off-mesh reachability canaries (canary-publish.sh) allowed to publish. Separate
# allowlist: empty = the feature is off and /canary-ingest 404s. Same bearer
# token — one trust domain. Deliberately holds MULTIPLE canaries so the aggregate
# is a quorum, not one box's word.
CANARY_ALLOWED = {n.strip() for n in os.environ.get("CANARY_ALLOWED_NAMES", "").split(",") if n.strip()}

# --- cross-goatnet summary (goatnets.html, the level above the fleet page) ---
# Sibling goatnets POST a COMPACT rollup here so one page can show every
# goatnet. Deliberately NOT on the main TOKEN: this crosses a goatnet boundary,
# and a production goatnet should not hand out its full ingest credential to a
# sibling. CROSS_FLEET_TOKEN authorizes exactly one thing — writing a six-field
# rollup with no topology, no per-relay detail, no addresses. Unset ⇒ the route
# 404s and the feature is simply off.
CROSS_FLEET_TOKEN = os.environ.get("CROSS_FLEET_TOKEN", "")
CROSS_FLEET_ALLOWED = {n.strip() for n in os.environ.get("CROSS_FLEET_ALLOWED_NAMES", "").split(",") if n.strip()}
# The whole permitted vocabulary. Anything else in the payload is dropped rather
# than stored: an ingest that accepts arbitrary keys from another goatnet is a
# write primitive, not a health band.
CROSS_FLEET_FIELDS = ("generated", "vantage", "relays_up", "relays_total", "worst_lat_ms", "stale_after_s")
# Zenoh routers (zenoh-health-probe.sh, F-248/F-253) allowed to publish their
# SERVICE health. Separate allowlist: empty = the feature is off and
# /zenoh-health-ingest 404s. Same bearer token — one trust domain. The router is
# on the site host (co-located with this ingest); this is a site-host-vantage aspect.
ZENOH_ALLOWED = {n.strip() for n in os.environ.get("ZENOH_HEALTH_ALLOWED_NAMES", "").split(",") if n.strip()}
# Allowlist auto-sync (ADR 1093 Phase 1): a trusted publisher on the site host (which
# has Kratos) POSTs the ACTIVE-roster email list here; we write it in-place to the
# oauth2-proxy authenticated-emails file so "active member ⇒ dashboard access,
# revoked ⇒ removed" becomes structural instead of a manual redeploy. Separate
# publisher allowlist; empty = feature off and /relay-health-allowlist-ingest 404s.
# Same bearer token — one trust domain.
ALLOWLIST_PUBLISHERS = {n.strip() for n in os.environ.get("ALLOWLIST_PUBLISHERS", "").split(",") if n.strip()}
# The oauth2-proxy AUTHENTICATED_EMAILS_FILE, bind-mounted here read-WRITE. We
# write IN-PLACE (truncate+write, never os.replace) because oauth2-proxy bind-mounts
# this exact file: a rename would swap the inode and oauth2-proxy would keep reading
# the stale one. In-place preserves the inode; oauth2-proxy re-reads on change.
ALLOWLIST_FILE = os.environ.get("ALLOWLIST_FILE", "/allowlist/authenticated-emails.txt")
# Fail-closed floor: refuse a suspiciously short list (a broken publisher / empty
# Kratos read must NOT lock the fleet out). Keeps the last-good file on refusal.
ALLOWLIST_MIN = int(os.environ.get("ALLOWLIST_MIN", "20"))
# Domain labels are dot-separated and contain no dots, so the match is linear.
# The earlier `[^@\s]+\.[^@\s]+` let a long dotted token backtrack
# quadratically, and a 256 KiB body can be a single token: one POST pinned the
# CPU and stalled every other ingest route. The length cap (the SMTP path
# limit) bounds the work even if the pattern changes again.
EMAIL_RE = re.compile(r"^[^@\s]+@[^@\s.]+(?:\.[^@\s.]+)+$")
EMAIL_MAX = 254
INGEST_DIR = os.environ.get("RELAY_METRICS_INGEST_DIR", "/ingest")
PORT = int(os.environ.get("RELAY_METRICS_LISTEN_PORT", "8090"))
MAX_BYTES = int(os.environ.get("RELAY_METRICS_MAX_BYTES", "262144"))
NAME_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,31}$")

if not TOKEN or len(TOKEN) < 16:
    sys.stderr.write("relay-metrics-ingest: refusing to start — RELAY_METRICS_BEARER_TOKEN unset/too short\n")
    sys.exit(2)
if not ALLOWED:
    sys.stderr.write("relay-metrics-ingest: refusing to start — RELAY_METRICS_ALLOWED_NAMES empty\n")
    sys.exit(2)
os.makedirs(INGEST_DIR, exist_ok=True)

# Requests select a configured output; request strings never construct paths.
RELAY_OUTPUTS = {n: os.path.join(INGEST_DIR, f"{n}.json") for n in ALLOWED}
HOST_OUTPUTS = {n: os.path.join(INGEST_DIR, f"host-{n}.json") for n in HOST_ALLOWED}
ZENOH_OUTPUTS = {n: os.path.join(INGEST_DIR, f"zenoh-{n}.json") for n in ZENOH_ALLOWED}
CANARY_OUTPUTS = {n: os.path.join(INGEST_DIR, f"canary-{n}.json") for n in CANARY_ALLOWED}
GOATNET_OUTPUTS = {n: os.path.join(INGEST_DIR, f"goatnet-{n}.json") for n in CROSS_FLEET_ALLOWED}


def log(msg):
    sys.stderr.write(f"relay-metrics-ingest: {msg}\n")
    sys.stderr.flush()


def atomic_json_write(path, doc):
    """Publish a complete document from an exclusively owned sibling file."""
    root = os.path.abspath(INGEST_DIR)
    if os.path.dirname(os.path.abspath(path)) != root:
        raise ValueError("JSON output must be directly inside the ingest directory")
    path = os.path.join(root, os.path.basename(path))
    try:
        mode = stat.S_IMODE(os.stat(path).st_mode)
    except FileNotFoundError:
        mode = 0o644
    tmp = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", dir=os.path.dirname(path),
                                         prefix="." + os.path.basename(path) + ".",
                                         suffix=".tmp", delete=False) as f:
            tmp = f.name
            os.fchmod(f.fileno(), mode)
            json.dump(doc, f)
        os.replace(tmp, path)
        tmp = None  # Rename consumed our file; the old pathname is no longer ours.
    finally:
        if tmp is not None:
            try:
                os.unlink(tmp)
            except FileNotFoundError:
                pass


def serialized(lock=None):
    """Serialize one output's entire read/build/publish operation, not others."""
    lock = lock if lock is not None else threading.RLock()

    def decorate(func):
        @wraps(func)
        def locked(*args, **kwargs):
            with lock:
                return func(*args, **kwargs)
        return locked
    return decorate


# ---------------------------------------------------------------------------
# Per-peer time-series retention (ADR 1055 Part B). Each relay-metrics POST
# carries per-peer fwd_bps/rcv_bps; we append them into a bounded, downsampled
# SQLite store (stdlib) with three tiers, evicting past each window. A debounced
# export writes peer-series.json (aggregated across relays), which nginx serves
# behind the same ADMINS gate — the dashboard fetches it to draw sparklines.
# Self-contained on THIS box (no fleet TSDB); each box keeps its own history, so
# the private-mesh (site host) and out-of-band (AWS) stores never mix.
# TIERS: (bucket_step_seconds, retention_window_seconds)
TIERS = [(60, 3600), (900, 86400), (7200, 604800)]  # 1h@1m, 24h@15m, 7d@2h
_TKEY = ["h", "d", "w"]
DB_PATH = os.path.join(INGEST_DIR, "peer-series.db")
SERIES_JSON = os.path.join(INGEST_DIR, "peer-series.json")
EXPORT_MIN_INTERVAL = 30  # seconds; debounce the JSON export
_DB_LOCK = threading.Lock()
_last_export = [0]


def init_series_db():
    with _DB_LOCK:
        con = sqlite3.connect(DB_PATH, timeout=10)
        con.execute(
            "CREATE TABLE IF NOT EXISTS peer_series("
            "relay TEXT, peer TEXT, tier INTEGER, bucket INTEGER, "
            "fwd REAL, rcv REAL, n INTEGER, "
            "PRIMARY KEY(relay, peer, tier, bucket))"
        )
        con.commit()
        con.close()


def record_series(relay, peers, ts):
    """Fold one relay's per-peer sample into the tiered buckets; evict stale."""
    rows = []
    for p in peers:
        pid = p.get("peer_id")
        if not isinstance(pid, str) or not pid:
            continue
        fwd = float(p.get("fwd_bps") or 0)
        rcv = float(p.get("rcv_bps") or 0)
        for tier, (step, _win) in enumerate(TIERS):
            rows.append((relay, pid, tier, (ts // step) * step, fwd, rcv))
    if not rows:
        return
    with _DB_LOCK:
        con = sqlite3.connect(DB_PATH, timeout=10)
        con.executemany(
            "INSERT INTO peer_series(relay,peer,tier,bucket,fwd,rcv,n) "
            "VALUES(?,?,?,?,?,?,1) "
            "ON CONFLICT(relay,peer,tier,bucket) DO UPDATE SET "
            "fwd=(fwd*n+excluded.fwd)/(n+1), rcv=(rcv*n+excluded.rcv)/(n+1), n=n+1",
            rows,
        )
        for tier, (_step, win) in enumerate(TIERS):
            con.execute("DELETE FROM peer_series WHERE tier=? AND bucket < ?", (tier, ts - win))
        con.commit()
        con.close()


# ---------------------------------------------------------------------------
# Host-net health retention (outage RCA). Site network hosts POST
# HostNetHealthV1 to /host-net-ingest/<host>; we persist the LAST report as
# host-<name>.json + an aggregate hosts.json (so the dashboard shows last-known
# state while a host is dark — the whole point), and fold the degradation-
# relevant scalars into the same tiered/downsampled sqlite pattern the peer
# series uses. Export host-series.json debounced.
HOST_SERIES_JSON = os.path.join(INGEST_DIR, "host-series.json")
HOSTS_JSON = os.path.join(INGEST_DIR, "hosts.json")
PEERS_JSON = os.path.join(INGEST_DIR, "peers.json")  # canary quorum, dashboard peer band
# host_series row metrics, in column order (see record_host_series):
HOST_METRICS = ["rx", "tx", "loss", "rtt", "jitter", "tun_errs", "peers", "sig_rx", "sig_tx", "sig_msgs", "sig_nf"]
_last_host_export = [0]


def init_host_series_db():
    with _DB_LOCK:
        con = sqlite3.connect(DB_PATH, timeout=10)
        con.execute(
            "CREATE TABLE IF NOT EXISTS host_series("
            "host TEXT, tier INTEGER, bucket INTEGER, "
            + ", ".join(f"{m} REAL" for m in HOST_METRICS)
            + ", n INTEGER, PRIMARY KEY(host, tier, bucket))"
        )
        con.commit()
        con.close()


def record_host_series(host, hn, ts):
    """Fold one host-net sample into the tiered buckets; evict past windows."""
    def g(d, k):
        v = (d or {}).get(k)
        try:
            return float(v)
        except (TypeError, ValueError):
            return None
    vals = [
        g(hn.get("iface"), "rx_bps"), g(hn.get("iface"), "tx_bps"),
        g(hn.get("wan"), "loss_pct"), g(hn.get("wan"), "rtt_avg_ms"), g(hn.get("wan"), "jitter_ms"),
        g(hn.get("tunnels"), "errs_15m"), g(hn.get("mesh"), "peers_connected"),
        g(hn.get("control"), "signal_rx_bps"), g(hn.get("control"), "signal_tx_bps"),
        g(hn.get("control"), "signal_msgs_60s"), g(hn.get("control"), "signal_notfound_60s"),
    ]
    vals = [0.0 if v is None else v for v in vals]
    cols = ", ".join(HOST_METRICS)
    upd = ", ".join(f"{m}=({m}*n+excluded.{m})/(n+1)" for m in HOST_METRICS)
    with _DB_LOCK:
        con = sqlite3.connect(DB_PATH, timeout=10)
        for tier, (step, _win) in enumerate(TIERS):
            con.execute(
                f"INSERT INTO host_series(host,tier,bucket,{cols},n) "
                f"VALUES(?,?,?,{','.join('?' * len(HOST_METRICS))},1) "
                f"ON CONFLICT(host,tier,bucket) DO UPDATE SET {upd}, n=n+1",
                [host, tier, (ts // step) * step] + vals,
            )
        for tier, (_step, win) in enumerate(TIERS):
            con.execute("DELETE FROM host_series WHERE tier=? AND bucket < ?", (tier, ts - win))
        con.commit()
        con.close()


@serialized()
def export_host_series():
    """host-series.json: {series:{host:{h|d|w:[[bucket,<HOST_METRICS...>],...]}},
    metrics:[...]} — the dashboard's drill-down history source."""
    cols = ", ".join(HOST_METRICS)
    with _DB_LOCK:
        con = sqlite3.connect(DB_PATH, timeout=10)
        rows = con.execute(
            f"SELECT host, tier, bucket, {cols} FROM host_series ORDER BY host, tier, bucket"
        ).fetchall()
        con.close()
    series = {}
    for row in rows:
        host, tier, bucket = row[0], row[1], row[2]
        series.setdefault(host, {"h": [], "d": [], "w": []})[_TKEY[tier]].append(
            [bucket] + [round(v or 0, 2) for v in row[3:]]
        )
    doc = {"series": series, "metrics": HOST_METRICS, "generated_at": int(time.time())}
    atomic_json_write(HOST_SERIES_JSON, doc)


@serialized()
def export_hosts_aggregate():
    """hosts.json: every host's LAST full report in one fetch. Rebuilt from the
    per-host files on disk, so a restart keeps last-known state (that state
    surviving the reporter being down is the feature)."""
    hosts = {}
    for n in sorted(HOST_ALLOWED):
        p = os.path.join(INGEST_DIR, f"host-{n}.json")
        try:
            with open(p) as f:
                hosts[n] = json.load(f)
        except Exception:
            pass  # skip hosts we cannot read or parse
    doc = {"hosts": hosts, "generated_at": int(time.time())}
    atomic_json_write(HOSTS_JSON, doc)


# ---------------------------------------------------------------------------
# Zenoh router SERVICE-health retention (F-248 FD exhaustion / F-253 admission
# seizure). The site host POSTs its local router's health (zenoh-health-probe.sh) to
# /zenoh-health-ingest/<router>; we persist the LAST report as zenoh-<name>.json
# + an aggregate zenoh-health.json (so the dashboard shows last-known state while
# the router is bounced), and fold the degradation-relevant scalars into the same
# tiered/downsampled sqlite the peer/host series use. Because that store lives
# HERE (a different container from the router), the history survives the router
# restarting — the entire reason this exists. Export zenoh-series.json debounced.
ZENOH_HEALTH_JSON = os.path.join(INGEST_DIR, "zenoh-health.json")
ZENOH_SERIES_JSON = os.path.join(INGEST_DIR, "zenoh-series.json")
# zenoh_series row metrics, in column order (see record_zenoh_series). open_ms is
# THE seizure signal (a stalled new-session open spikes toward the 10s timeout);
# close_wait is the leading wedge indicator; fd_used vs fd_cap is F-248.
# tls_bps / quic_dps = transport-split throughput: TLS byte/s vs
# QUIC datagram/s — zenoh 1.9.0 has no per-transport message counters, so these
# transport-level rates are the honest proxy.
ZENOH_METRICS = ["open_ms", "rest_ms", "close_wait", "time_wait", "estab",
                 "fd_used", "fd_cap", "cpu", "mem_mb", "storages", "rocksdb_mb",
                 "tls_bps", "quic_dps"]
_last_zenoh_export = [0]


def init_zenoh_series_db():
    with _DB_LOCK:
        con = sqlite3.connect(DB_PATH, timeout=10)
        con.execute(
            "CREATE TABLE IF NOT EXISTS zenoh_series("
            "router TEXT, tier INTEGER, bucket INTEGER, "
            + ", ".join(f"{m} REAL" for m in ZENOH_METRICS)
            + ", n INTEGER, PRIMARY KEY(router, tier, bucket))"
        )
        # Forward-migrate an older table: add any metric column it lacks (never
        # drop — the whole point is that the history survives). Idempotent.
        have = {r[1] for r in con.execute("PRAGMA table_info(zenoh_series)")}
        for m in ZENOH_METRICS:
            if m not in have:
                con.execute(f"ALTER TABLE zenoh_series ADD COLUMN {m} REAL")
        con.commit()
        con.close()


def record_zenoh_series(router, z, ts):
    """Fold one router-health sample into the tiered buckets; evict past windows."""
    def g(d, k):
        v = (d or {}).get(k)
        try:
            return float(v)
        except (TypeError, ValueError):
            return None
    adm, wed, ses, cap, sto = (z.get("admission"), z.get("wedge"),
                               z.get("sessions"), z.get("capacity"), z.get("storage"))
    vals = [
        g(adm, "open_ms"), g(adm, "rest_ms"),
        g(wed, "close_wait"), g(wed, "time_wait"),
        g(ses, "estab"),
        g(cap, "fd_used"), g(cap, "fd_cap"), g(cap, "cpu_pct"), g(cap, "mem_mb"),
        g(cap, "storages"), g(sto, "rocksdb_mb"),
        g(ses, "tls_bps_tx"), g(ses, "quic_dps_tx"),
    ]
    vals = [0.0 if v is None else v for v in vals]
    cols = ", ".join(ZENOH_METRICS)
    upd = ", ".join(f"{m}=({m}*n+excluded.{m})/(n+1)" for m in ZENOH_METRICS)
    with _DB_LOCK:
        con = sqlite3.connect(DB_PATH, timeout=10)
        for tier, (step, _win) in enumerate(TIERS):
            con.execute(
                f"INSERT INTO zenoh_series(router,tier,bucket,{cols},n) "
                f"VALUES(?,?,?,{','.join('?' * len(ZENOH_METRICS))},1) "
                f"ON CONFLICT(router,tier,bucket) DO UPDATE SET {upd}, n=n+1",
                [router, tier, (ts // step) * step] + vals,
            )
        for tier, (_step, win) in enumerate(TIERS):
            con.execute("DELETE FROM zenoh_series WHERE tier=? AND bucket < ?", (tier, ts - win))
        con.commit()
        con.close()


@serialized()
def export_zenoh_series():
    """zenoh-series.json: {series:{router:{h|d|w:[[bucket,<ZENOH_METRICS...>],...]}},
    metrics:[...]} — the dashboard's router-health drill-down history source."""
    cols = ", ".join(ZENOH_METRICS)
    with _DB_LOCK:
        con = sqlite3.connect(DB_PATH, timeout=10)
        rows = con.execute(
            f"SELECT router, tier, bucket, {cols} FROM zenoh_series ORDER BY router, tier, bucket"
        ).fetchall()
        con.close()
    series = {}
    for row in rows:
        router, tier, bucket = row[0], row[1], row[2]
        series.setdefault(router, {"h": [], "d": [], "w": []})[_TKEY[tier]].append(
            [bucket] + [round(v or 0, 2) for v in row[3:]]
        )
    doc = {"series": series, "metrics": ZENOH_METRICS, "generated_at": int(time.time())}
    atomic_json_write(ZENOH_SERIES_JSON, doc)


@serialized()
def export_zenoh_aggregate():
    """zenoh-health.json: every router's LAST full report in one fetch. Rebuilt
    from the per-router files on disk, so a restart (of the ingest OR the router)
    keeps last-known state visible — the point of a durable store."""
    routers = {}
    for n in sorted(ZENOH_ALLOWED):
        p = os.path.join(INGEST_DIR, f"zenoh-{n}.json")
        try:
            with open(p) as f:
                routers[n] = json.load(f)
        except Exception:
            pass  # skip routers we cannot read or parse
    doc = {"routers": routers, "generated_at": int(time.time())}
    atomic_json_write(ZENOH_HEALTH_JSON, doc)


# --- Zenoh WEDGE-EVENT surfacing ----------------------
# The router deadlocks recurrently. One mechanism is (TLS accept-throttle,
# fixed); a SECOND userspace lock-deadlock remains
# (main thread on a futex, REST plane blocked) — see the playbook
# docs/operations/zenoh-wedge-investigation-playbook.md. A wedge-watch watchdog
# `docker restart`s the wedged router, so the RELIABLE wedge marker is an UPTIME
# RESET between consecutive reports (Docker RestartCount stays 0 for a manual
# restart, which is why it looked flat). We also flag a LIVE wedge (health
# unhealthy / open-probe failing). Events persist in zenoh-wedges.json, reloaded
# on boot so the event timeline survives an ingest restart.
ZENOH_WEDGES_JSON = os.path.join(INGEST_DIR, "zenoh-wedges.json")
WEDGE_OPEN_MS = 8000           # a new-session open this slow = admission wedged
WEDGE_UPTIME_DROP_S = 60       # uptime dropping by more than this = a restart happened
WEDGE_EVENTS_MAX = 300
WEDGE_EVENTS_KEEP_S = 14 * 86400
_zenoh_wedge_last = {}         # router -> {"uptime","health","wedged","wedged_since"}
_zenoh_wedge_events = []       # bounded list of {ts,router,type,detail,open_ms,close_wait,uptime_s}
_WEDGE_LOCK = threading.RLock()


def _zenoh_is_wedged(z):
    """Is this report a live wedge? Returns (bool, reason). Ordered by directness."""
    c, a = z.get("container") or {}, z.get("admission") or {}
    tls_rejected = a.get("open_reason") == "tls_error"
    if c.get("health") == "unhealthy":
        return True, "container unhealthy (REST healthcheck timing out)"
    # Otherwise the bridge's currently_wedged -> WedgedStuck critical alert
    # would bypass F-568's classified-TLS guard after five minutes. Keep older
    # reports (without a reason) and genuine connection failures detectable.
    if a.get("open_ok") is False and not tls_rejected:
        return True, "TCP/TLS admission proxy FAILED"
    if a.get("rest_ok") is False:
        return True, "REST adminspace not answering"
    om = a.get("open_ms")
    cw = (z.get("wedge") or {}).get("close_wait")
    corroborated = not tls_rejected or isinstance(cw, (int, float)) and cw > 20
    if isinstance(om, (int, float)) and om >= WEDGE_OPEN_MS and corroborated:
        return True, f"TCP/TLS admission proxy stalled ({int(om)} ms)"
    return False, ""


@serialized(_WEDGE_LOCK)
def track_zenoh_wedges(name, z, ts):
    """Fold one report into the per-router wedge-event timeline; persist."""
    global _zenoh_wedge_events
    c, a, w = z.get("container") or {}, z.get("admission") or {}, z.get("wedge") or {}
    uptime, health = c.get("uptime_s"), c.get("health")
    wedged_now, why = _zenoh_is_wedged(z)
    prev = _zenoh_wedge_last.get(name) or {}

    def ev(kind, detail):
        _zenoh_wedge_events.append({
            "ts": ts, "router": name, "type": kind, "detail": detail,
            "open_ms": a.get("open_ms"), "close_wait": w.get("close_wait"), "uptime_s": uptime})

    # RESTART: uptime fell -> the router bounced. Distinguish a WEDGE-RECOVERY
    # restart (the router was wedged in the prior probe) from a CLEAN restart
    # (it was healthy — a planned deploy or manual bounce). Only wedge-recovery
    # restarts count toward the wedge total + the ZenohRouterWedging alert; a
    # clean/deploy restart is recorded for visibility but must NOT read as a wedge.
    pu = prev.get("uptime")
    if isinstance(pu, (int, float)) and isinstance(uptime, (int, float)) and uptime < pu - WEDGE_UPTIME_DROP_S:
        since = prev.get("wedged_since")
        if since:
            ev("restart", f"wedge-recovery restart (watchdog); prior wedge lasted ~{ts - since}s")
        else:
            ev("restart-clean", "clean restart while healthy (planned deploy / manual) — not a wedge")
    # WEDGE START: healthy -> wedged (probe caught it live, pre-restart).
    if wedged_now and not prev.get("wedged"):
        ev("wedged", why)
    # RECOVER without a restart (health returned on its own — uncommon).
    elif not wedged_now and prev.get("wedged") and isinstance(pu, (int, float)) \
            and isinstance(uptime, (int, float)) and uptime >= pu:
        ev("recovered", "health returned (no restart)")

    _zenoh_wedge_last[name] = {
        "uptime": uptime, "health": health, "wedged": wedged_now,
        "wedged_since": (prev.get("wedged_since") or ts) if wedged_now else None}
    cutoff = ts - WEDGE_EVENTS_KEEP_S
    _zenoh_wedge_events = [e for e in _zenoh_wedge_events if e["ts"] >= cutoff][-WEDGE_EVENTS_MAX:]
    export_zenoh_wedges(ts)


@serialized(_WEDGE_LOCK)
def export_zenoh_wedges(now):
    """zenoh-wedges.json: per-router recent events + restart counts + live-wedge flag."""
    routers = {}
    for n in sorted(ZENOH_ALLOWED):
        evs = [e for e in _zenoh_wedge_events if e["router"] == n]
        last = _zenoh_wedge_last.get(n) or {}
        # Only WEDGE-recovery restarts count (clean/deploy restarts are type
        # "restart-clean" and are excluded from the wedge total + last_wedge_ts).
        restarts = [e for e in evs if e["type"] == "restart"]
        routers[n] = {
            "events": evs[-60:],
            "restarts_24h": sum(1 for e in restarts if e["ts"] >= now - 86400),
            "restarts_7d": sum(1 for e in restarts if e["ts"] >= now - 7 * 86400),
            "last_wedge_ts": max((e["ts"] for e in evs if e["type"] in ("wedged", "restart")), default=None),
            "currently_wedged": bool(last.get("wedged")),
            "wedged_since": last.get("wedged_since"),
        }
    doc = {"routers": routers, "events": _zenoh_wedge_events[-WEDGE_EVENTS_MAX:], "generated_at": now}
    atomic_json_write(ZENOH_WEDGES_JSON, doc)


def load_zenoh_wedges():
    """Reload the wedge timeline on boot so the event history survives a restart."""
    global _zenoh_wedge_events
    try:
        with open(ZENOH_WEDGES_JSON) as f:
            d = json.load(f)
        _zenoh_wedge_events = d.get("events") or []
        for n, r in (d.get("routers") or {}).items():
            _zenoh_wedge_last[n] = {"uptime": None, "health": None,
                                    "wedged": bool(r.get("currently_wedged")),
                                    "wedged_since": r.get("wedged_since")}
    except Exception:
        pass  # unreadable saved timeline: retain initialized or already restored state


@serialized()
def export_canaries_aggregate():
    """peers.json: every canary's LAST report + a QUORUM verdict across them.

    The whole reason to run multiple external canaries is that no single one is
    trusted — one box doing something idiosyncratic (its own uplink flaking, a
    local resolver quirk) must not read as a mesh outage. So the aggregate carries
    a `quorum` block: the front-door verdict is the MAJORITY across fresh canaries,
    and a lone dissenter is surfaced as `outliers`, not as truth. Fresh = reported
    within CANARY_STALE_S; stale canaries are excluded from the vote (never counted
    as down just for being quiet).
    """
    stale_s = int(os.environ.get("CANARY_STALE_S", "180"))
    now = int(time.time())
    peers, fresh = {}, []
    for n in sorted(CANARY_ALLOWED):
        p = os.path.join(INGEST_DIR, f"canary-{n}.json")
        try:
            with open(p) as f:
                rep = json.load(f)
            peers[n] = rep
            if now - int(rep.get("reported_at") or rep.get("received_at") or 0) <= stale_s:
                fresh.append(rep)
        except Exception:
            pass  # skip peers we cannot read or parse
    # Quorum on the front-door reachability (the no-mesh-needed external signal).
    def fd(rep):
        return (((rep.get("journey") or {}).get("front_door") or {}).get("st")) or "grey"
    votes = [fd(r) for r in fresh]
    up = sum(1 for v in votes if v == "green")
    down = sum(1 for v in votes if v == "red")
    part = sum(1 for v in votes if v == "amber")
    if not fresh:
        verdict = "unknown"
    elif up > down and up >= (len(votes) + 1) // 2:
        verdict = "up"
    elif down > up and down >= (len(votes) + 1) // 2:
        verdict = "down"
    else:
        verdict = "degraded"
    outliers = [r.get("peer") for r in fresh if (verdict == "up" and fd(r) == "red") or (verdict == "down" and fd(r) == "green")]
    quorum = {"verdict": verdict, "fresh": len(fresh), "total": len(CANARY_ALLOWED),
              "up": up, "down": down, "degraded": part, "outliers": outliers}
    doc = {"peers": peers, "quorum": quorum, "generated_at": now}
    atomic_json_write(PEERS_JSON, doc)


@serialized()
def export_series():
    """Write peer-series.json: {series:{hex:{h|d|w:[[bucket,fwd,rcv],...]}}}.
    Rates are summed across relays per (peer,tier,bucket) = the peer's fleet total."""
    with _DB_LOCK:
        con = sqlite3.connect(DB_PATH, timeout=10)
        rows = con.execute(
            "SELECT peer, tier, bucket, SUM(fwd), SUM(rcv) FROM peer_series "
            "GROUP BY peer, tier, bucket ORDER BY peer, tier, bucket"
        ).fetchall()
        con.close()
    series = {}
    for peer, tier, bucket, fwd, rcv in rows:
        series.setdefault(peer, {"h": [], "d": [], "w": []})[_TKEY[tier]].append(
            [bucket, round(fwd or 0, 1), round(rcv or 0, 1)]
        )
    doc = {"series": series, "generated_at": int(time.time()), "peers": len(series)}
    atomic_json_write(SERIES_JSON, doc)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _reply(self, code, body=b"", close=False):
        self.send_response(code)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Content-Type", "application/json")
        if close:
            self.send_header("Connection", "close")
            self.close_connection = True
        self.end_headers()
        if body:
            self.wfile.write(body)

    def do_GET(self):
        # Unauthenticated liveness only; never serves ingested data.
        if self.path == "/healthz":
            self._reply(200, b'{"ok":true}')
        else:
            self._reply(404, b'{"error":"not found"}')

    def do_POST(self):
        # Drain the request body FIRST, always. With HTTP/1.1 keep-alive + a
        # proxy (Traefik) reusing the backend connection, an UN-read body
        # corrupts the NEXT request on that connection — it gets parsed as a
        # bogus request line → a spurious 501. So read the (capped) body before
        # any reply; reject oversize with Connection: close so we never have to
        # drain an attacker-sized body.
        length = int(self.headers.get("Content-Length", "0") or "0")
        if length > MAX_BYTES:
            return self._reply(413, b'{"error":"too large"}', close=True)
        raw = self.rfile.read(length) if length > 0 else b""
        hm = re.match(r"^/host-net-ingest/([^/?]+)", self.path)
        cm = re.match(r"^/canary-ingest/([^/?]+)", self.path)
        am = re.match(r"^/relay-health-allowlist-ingest/([^/?]+)", self.path)
        zm = re.match(r"^/zenoh-health-ingest/([^/?]+)", self.path)
        gm = re.match(r"^/goatnet-summary-ingest/([^/?]+)", self.path)
        m = re.match(r"^/relay-metrics-ingest/([^/?]+)", self.path)
        if not m and not hm and not cm and not am and not zm and not gm:
            return self._reply(404, b'{"error":"not found"}')
        # Constant-ish token check (fixed-length compare via hmac).
        auth = self.headers.get("Authorization", "")
        sent = auth[7:] if auth.startswith("Bearer ") else ""
        import hmac
        # The cross-goatnet route is authorized by its OWN credential, so a
        # sibling never needs this box's full ingest token.
        expect = CROSS_FLEET_TOKEN if gm else TOKEN
        if not expect or not hmac.compare_digest(sent, expect):
            log(f"401 bad token from {self.client_address[0]} path={self.path}")
            return self._reply(401, b'{"error":"unauthorized"}')
        if gm:
            # Cross-goatnet rollup for goatnets.html. Whitelist-copied field by
            # field: never store what a sibling sent verbatim.
            name = gm.group(1)
            if name not in CROSS_FLEET_ALLOWED or not NAME_RE.match(name):
                log(f"404 unknown goatnet name={name!r}")
                return self._reply(404, b'{"error":"unknown goatnet"}')
            try:
                doc = json.loads(raw.decode("utf-8"))
                if not isinstance(doc, dict):
                    raise ValueError("not an object")
            except Exception as e:
                log(f"400 bad goatnet summary from {name}: {e}")
                return self._reply(400, b'{"error":"bad json"}')
            out = {}
            for k in CROSS_FLEET_FIELDS:
                v = doc.get(k)
                if isinstance(v, (int, float)) or (isinstance(v, str) and len(v) <= 64):
                    out[k] = v
            out["received_at"] = int(time.time())
            path = GOATNET_OUTPUTS[name]
            atomic_json_write(path, out)
            log(f"200 goatnet summary from {name}: {out.get('relays_up')}/{out.get('relays_total')} up")
            return self._reply(200, b'{"ok":true}')
        if am:
            # Allowlist auto-sync: the site host publishes the active-roster emails.
            # Body = {"emails": ["a@b", ...]} (or newline-separated text).
            pub = am.group(1)
            if pub not in ALLOWLIST_PUBLISHERS or not NAME_RE.match(pub):
                log(f"404 unknown allowlist publisher name={pub!r}")
                return self._reply(404, b'{"error":"unknown publisher"}')
            try:
                txt = raw.decode("utf-8")
                doc = json.loads(txt)
                emails = doc.get("emails") if isinstance(doc, dict) else doc
                if not isinstance(emails, list):
                    raise ValueError("emails not a list")
            except Exception:
                # tolerate newline/comma-separated plaintext
                emails = [e for e in re.split(r"[\s,]+", raw.decode("utf-8", "replace")) if e]
            # normalize + validate; fail-closed on a suspiciously short/invalid list
            clean = sorted({e.strip().lower() for e in emails if isinstance(e, str)})
            clean = [e for e in clean if len(e) <= EMAIL_MAX and EMAIL_RE.match(e)]
            if len(clean) < ALLOWLIST_MIN:
                log(f"409 allowlist too short ({len(clean)} < {ALLOWLIST_MIN}) from {pub} — keeping last-good")
                return self._reply(409, b'{"error":"allowlist below floor; last-good kept"}')
            body_out = ("\n".join(clean) + "\n").encode("utf-8")
            # IN-PLACE write (truncate+write, NOT os.replace): oauth2-proxy bind-mounts
            # this exact file and watches it; a rename would strand it on the old inode.
            try:
                with open(ALLOWLIST_FILE, "r+b") as f:
                    f.seek(0)
                    f.write(body_out)
                    f.truncate()
                    f.flush()
                    os.fsync(f.fileno())
            except FileNotFoundError:
                with open(ALLOWLIST_FILE, "wb") as f:
                    f.write(body_out)
                    f.flush()
                    os.fsync(f.fileno())
            log(f"200 allowlist synced from {pub}: {len(clean)} emails")
            return self._reply(200, b'{"ok":true,"count":%d}' % len(clean))
        if hm:
            # Host-net health (HostNetHealthV1, host-net-publish.sh).
            name = hm.group(1)
            if name not in HOST_ALLOWED or not NAME_RE.match(name):
                log(f"404 unknown host name={name!r}")
                return self._reply(404, b'{"error":"unknown host"}')
            if length <= 0:
                return self._reply(400, b'{"error":"empty body"}')
            try:
                doc = json.loads(raw)
                assert isinstance(doc, dict) and isinstance(doc.get("host_net"), dict)
            except Exception:
                log(f"400 non-JSON / no host_net block from {name}")
                return self._reply(400, b'{"error":"bad payload"}')
            doc["received_at"] = int(time.time())
            doc["host"] = name
            dst = HOST_OUTPUTS[name]
            atomic_json_write(dst, doc)
            try:
                export_hosts_aggregate()
            except Exception as e:
                log(f"host aggregate failed for {name}: {e}")
            try:
                record_host_series(name, doc["host_net"], doc["received_at"])
                if doc["received_at"] - _last_host_export[0] >= EXPORT_MIN_INTERVAL:
                    export_host_series()
                    _last_host_export[0] = doc["received_at"]
            except Exception as e:
                log(f"host series record failed for {name}: {e}")  # never fail the ingest on it
            hn = doc["host_net"]
            log(f"200 host {name} bytes={length} loss={(hn.get('wan') or {}).get('loss_pct')} tun_errs={(hn.get('tunnels') or {}).get('errs_15m')}")
            return self._reply(204)
        if zm:
            # Zenoh router service health (zenoh-health-probe.sh, F-248/F-253).
            name = zm.group(1)
            if name not in ZENOH_ALLOWED or not NAME_RE.match(name):
                log(f"404 unknown zenoh router name={name!r}")
                return self._reply(404, b'{"error":"unknown router"}')
            if length <= 0:
                return self._reply(400, b'{"error":"empty body"}')
            try:
                doc = json.loads(raw)
                assert isinstance(doc, dict) and isinstance(doc.get("zenoh"), dict)
            except Exception:
                log(f"400 non-JSON / no zenoh block from {name}")
                return self._reply(400, b'{"error":"bad payload"}')
            doc["received_at"] = int(time.time())
            doc["router"] = name
            dst = ZENOH_OUTPUTS[name]
            atomic_json_write(dst, doc)
            try:
                export_zenoh_aggregate()
            except Exception as e:
                log(f"zenoh aggregate failed for {name}: {e}")
            try:
                track_zenoh_wedges(name, doc["zenoh"], doc["received_at"])
            except Exception as e:
                log(f"zenoh wedge tracking failed for {name}: {e}")
            try:
                record_zenoh_series(name, doc["zenoh"], doc["received_at"])
                if doc["received_at"] - _last_zenoh_export[0] >= EXPORT_MIN_INTERVAL:
                    export_zenoh_series()
                    _last_zenoh_export[0] = doc["received_at"]
            except Exception as e:
                log(f"zenoh series record failed for {name}: {e}")  # never fail the ingest on it
            z = doc["zenoh"]
            log(f"200 zenoh {name} bytes={length} health={(z.get('container') or {}).get('health')} "
                f"open_ms={(z.get('admission') or {}).get('open_ms')} close_wait={(z.get('wedge') or {}).get('close_wait')}")
            return self._reply(204)
        if cm:
            # Off-mesh reachability canary (canary-publish.sh). Body = {published_at,
            # canary:{peer,goatnet,vantage,reported_at,overall,journey{...},implication}}.
            # Persist per-canary + rebuild the quorum aggregate.
            name = cm.group(1)
            if name not in CANARY_ALLOWED or not NAME_RE.match(name):
                log(f"404 unknown canary name={name!r}")
                return self._reply(404, b'{"error":"unknown canary"}')
            if length <= 0:
                return self._reply(400, b'{"error":"empty body"}')
            try:
                body = json.loads(raw)
                rep = body.get("canary") if isinstance(body, dict) else None
                assert isinstance(rep, dict) and isinstance(rep.get("journey"), dict)
            except Exception:
                log(f"400 non-JSON / no canary block from {name}")
                return self._reply(400, b'{"error":"bad payload"}')
            rep["received_at"] = int(time.time())
            rep["peer"] = name
            dst = CANARY_OUTPUTS[name]
            atomic_json_write(dst, rep)
            try:
                export_canaries_aggregate()
            except Exception as e:
                log(f"canary aggregate failed for {name}: {e}")  # never fail the ingest on it
            log(f"200 canary {name} overall={rep.get('overall')} fd={((rep.get('journey') or {}).get('front_door') or {}).get('detail')}")
            return self._reply(204)
        name = m.group(1)
        if name == "peer-names":
            # Reserved (ADR 1055 names layer): the site host-side resolver publishes the
            # relay-peer-id -> friendly NetBird name map here. Same bearer gate; the
            # body is a flat {hashed_id: name} JSON object (the relay hex id, never a
            # pubkey). Written as peer-names.json into the ingest dir, which nginx
            # serves behind the same ADMINS gate — the dashboard fetches it to render
            # names with the hex on hover. The prober folds relays by name lookup
            # (<relay>.json), so this file is never mis-parsed as a relay.
            if length <= 0:
                return self._reply(400, b'{"error":"empty body"}')
            try:
                nm = json.loads(raw)
                # value is the friendly name (legacy) OR a baseball-card object.
                assert isinstance(nm, dict) and all(
                    isinstance(k, str) and isinstance(v, (str, dict)) for k, v in nm.items())
            except Exception:
                log("400 bad peer-names payload")
                return self._reply(400, b'{"error":"bad payload"}')
            # The writer's file-level record (owner_read, generated_at) rides in a
            # reserved key so an ingest older than it stores it as one more entry
            # that no card lookup matches. Lifted to the top of the file here, so
            # a reader can tell a stale map from a fresh one and an unread owner
            # cycle from a read one.
            meta = nm.pop("__peer_names_meta__", None)
            doc = {"names": nm, "received_at": int(time.time()), "count": len(nm)}
            if isinstance(meta, dict):
                if isinstance(meta.get("generated_at"), int):
                    doc["generated_at"] = meta["generated_at"]
                if isinstance(meta.get("owner_read"), dict):
                    doc["owner_read"] = meta["owner_read"]
            dst = os.path.join(INGEST_DIR, "peer-names.json")
            atomic_json_write(dst, doc)
            log(f"200 peer-names count={len(nm)}")
            return self._reply(204)
        if name not in ALLOWED or not NAME_RE.match(name):
            log(f"404 unknown relay name={name!r}")
            return self._reply(404, b'{"error":"unknown relay"}')
        if length <= 0:
            return self._reply(400, b'{"error":"empty body"}')
        try:
            doc = json.loads(raw)
            assert isinstance(doc, dict) and isinstance(doc.get("deep"), dict)
        except Exception:
            log(f"400 non-JSON / no deep block from {name}")
            return self._reply(400, b'{"error":"bad payload"}')
        doc["received_at"] = int(time.time())
        doc["relay"] = name
        dst = RELAY_OUTPUTS[name]
        atomic_json_write(dst, doc)
        # ADR 1055 Part B — fold per-peer rates into the time-series + debounced export.
        try:
            peers = (doc.get("deep") or {}).get("peers") or []
            if peers:
                record_series(name, peers, doc["received_at"])
                now = doc["received_at"]
                if now - _last_export[0] >= EXPORT_MIN_INTERVAL:
                    export_series()
                    _last_export[0] = now
        except Exception as e:
            log(f"series record failed for {name}: {e}")  # never fail the ingest on it
        log(f"200 {name} bytes={length}")
        self._reply(204)
        return None

    def log_message(self, *args):
        pass  # silence default access log; we log what we care about


if __name__ == "__main__":
    init_series_db()
    if HOST_ALLOWED:
        init_host_series_db()
        export_hosts_aggregate()   # restart keeps last-known host state visible
    if ZENOH_ALLOWED:
        init_zenoh_series_db()
        export_zenoh_aggregate()   # restart keeps last-known router state visible
        load_zenoh_wedges()        # reload the wedge-event timeline (survives ingest restart)
        export_zenoh_wedges(int(time.time()))
    log(f"listening :{PORT} dir={INGEST_DIR} relays={sorted(ALLOWED)} hosts={sorted(HOST_ALLOWED)} "
        f"zenoh={sorted(ZENOH_ALLOWED)} series_db={DB_PATH}")
    ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
