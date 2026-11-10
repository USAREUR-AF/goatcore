#!/usr/bin/env python3
"""dormancy-report — REPORT-ONLY dormancy census (ADR 1089 Stage 3.2).

Sibling of identity-reconcile: reads the two activity signals the probation
lifecycle sheds on — NetBird peer last-seen and Ory Kratos session recency —
and emits Prometheus textfile metrics + a journald name report. It moves
NOTHING: the shed leg (Stage 3.3) stays gated behind `probation_enforce`
(default off) until this report has run across at least one full threshold
window and the counts have been eyeballed against the real roster
(membership-lifecycle-plan.md Stage 3 sequencing — the first real shed must
be calibrated, not a surprise mass-isolation).

Two independent axes, deliberately not joined (per-peer mesh dormancy is what
the engine's `--filter last_seen>Nd` will shed; per-identity login dormancy is
what the Ory `probation` marker rides — the join happens in the shed leg, not
here):
  - PEER dormancy: not connected AND last_seen older than threshold
    (never-seen counts as dormant; a connected peer is never dormant).
  - IDENTITY login dormancy: active Kratos identities whose newest session
    authenticated_at is older than threshold; never-logged-in counted
    separately (undelivered credentials dominate it — a delivery problem, not
    an inactivity signal, so it must not be lumped in).

Metrics (node-exporter textfile collector; NO PII in labels — names go to
stdout → journald only, same discipline as identity-reconcile):
    goat_dormancy_report_up{env}                                gauge
    goat_dormant_peers{env,threshold_days}                      gauge
    goat_dormant_person_identities{env,threshold_days}          gauge
    goat_identities_no_session_ever{env}                        gauge
    goat_dormancy_sessions_check_up{env}                        gauge
    goat_dormancy_report_last_run_timestamp_seconds             gauge
    goat_device_peer_expiry_enabled{env,class,kind}             gauge

The last one is not dormancy; it rides this report because it reads the same
peer list. It counts peers in a NETWORK, CONTROL or EDGE-SERVER group (ADR
1216) that NetBird is set to expire: kind="login" when the peer's login
expiration is on, kind="inactivity" when its inactivity expiration is on. Such
a device drops off the mesh when the timer runs out and comes back only when
someone re-enrolls it at the device, so the right count is zero. Every
class x kind series is written, zeros included, so the alert is a value
comparison and never an absent() form. Peer names go to journald only.

Usage: dormancy_report.py <output-dir>
  env: NETBIRD_MGMT_URL, NETBIRD_API_TOKEN, KRATOS_ADMIN_URL (default
       http://127.0.0.1:4434), GOAT_ENV (default unknown; obs.yml sets it),
       DORMANCY_THRESHOLDS (default "7,14,30" days)
Exit: 0 ok (report written), 1 config/fetch error (up=0 written).
"""

import datetime
import json
import os
import pathlib
import sys
import time
import urllib.parse
import urllib.request


def parse_ts(s: str) -> float | None:
    """RFC3339 → unix seconds; None on absent/unparseable (== never)."""
    if not s:
        return None
    try:
        return datetime.datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()
    except ValueError:
        return None


def fetch_json(url: str, headers: dict[str, str]) -> tuple[object, dict[str, str]]:
    req = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read().decode()), dict(r.headers)


def fetch_peers(base: str, token: str) -> list[dict]:
    data, _ = fetch_json(base.rstrip("/") + "/api/peers", {"Authorization": f"Token {token}"})
    return data if isinstance(data, list) else []


def fetch_identities(kratos: str) -> list[dict]:
    data, _ = fetch_json(kratos.rstrip("/") + "/admin/identities?per_page=500", {})
    return data if isinstance(data, list) else []


def fetch_latest_sessions(kratos: str) -> tuple[int, dict[str, float]]:
    """(check_up, identity_id → newest authenticated_at unix). Pages through
    /admin/sessions (active + expired — dormancy needs the LAST login, which
    may be an expired session). Degrades to check_up=0 on any failure (older
    Kratos without the endpoint, etc.) — the peer axis still reports."""
    latest: dict[str, float] = {}
    url = kratos.rstrip("/") + "/admin/sessions?page_size=250"
    try:
        for _ in range(40):  # bound: 10k sessions
            data, headers = fetch_json(url, {})
            if not isinstance(data, list):
                break
            for s in data:
                ident = (s.get("identity") or {}).get("id") or s.get("identity_id")
                ts = parse_ts(str(s.get("authenticated_at") or ""))
                if ident and ts is not None and ts > latest.get(ident, 0):
                    latest[ident] = ts
            nxt = None
            for part in (headers.get("Link") or "").split(","):
                if 'rel="next"' in part:
                    nxt = part[part.find("<") + 1 : part.find(">")]
            if not nxt or not data:
                break
            # Kratos returns the Link `next` as a RELATIVE path
            # (/admin/sessions?page_size=…&page_token=…); urlopen rejects that
            # with "unknown url type" and the sweep would degrade to check_up=0
            # on every run after the first page. Resolve it against the
            # page we just fetched.
            url = urllib.parse.urljoin(url, nxt)
        return 1, latest
    except Exception as e:  # noqa: BLE001 — degrade, never crash the cron
        print(f"dormancy-report: sessions fetch failed: {e}", file=sys.stderr)
        return 0, {}


# ADR 1216: the classes whose devices must never expire. Matched on the group
# SUFFIX, as ops/scripts/lifecycle-sweep.py does, so <SCOPE>-NETWORK counts and
# <SCOPE>-NETWORK-JUMP does not.
NON_EXPIRING_CLASSES = ("NETWORK", "CONTROL", "EDGE-SERVER")
EXPIRY_KINDS = (
    ("login", "login_expiration_enabled"),
    ("inactivity", "inactivity_expiration_enabled"),
)


def device_class(peer: dict) -> str | None:
    """The ADR 1216 class this peer's groups put it in, or None."""
    for g in peer.get("groups") or []:
        name = str(g.get("name", "") if isinstance(g, dict) else g).upper()
        for cls in NON_EXPIRING_CLASSES:
            if name == cls or name.endswith("-" + cls):
                return cls
    return None


def expirable_device_peers(peers: list[dict]) -> list[tuple[str, str, str]]:
    """(peer name, class, kind) for every NETWORK / CONTROL / EDGE-SERVER peer
    with a NetBird expiry turned on. One row per kind that is on."""
    rows = []
    for p in peers:
        cls = device_class(p)
        if cls is None:
            continue
        for kind, field in EXPIRY_KINDS:
            if p.get(field) is True:
                rows.append((str(p.get("name", "?")), cls, kind))
    return rows


def expiry_counts(rows: list[tuple[str, str, str]]) -> dict[tuple[str, str], int]:
    """Every class x kind pair, zeros included."""
    counts = {(c, k): 0 for c in NON_EXPIRING_CLASSES for k, _ in EXPIRY_KINDS}
    for _, cls, kind in rows:
        counts[(cls, kind)] += 1
    return counts


def atomic_write(out_path: pathlib.Path, body: str) -> None:
    tmp = out_path.with_suffix(out_path.suffix + f".tmp.{os.getpid()}")
    tmp.write_text(body)
    tmp.chmod(0o644)
    tmp.replace(out_path)


def render_prom(
    env: str,
    up: int,
    peers_by_thr: dict[int, int],
    idents_by_thr: dict[int, int],
    never: int,
    sessions_up: int,
    expiry: dict[tuple[str, str], int] | None = None,
) -> str:
    lines = [
        "# HELP goat_dormancy_report_up 1 if the NetBird peer fetch succeeded, else 0.",
        "# TYPE goat_dormancy_report_up gauge",
        f'goat_dormancy_report_up{{env="{env}"}} {up}',
        "# HELP goat_dormant_peers Mesh peers not connected whose last_seen exceeds the threshold (never-seen counts).",
        "# TYPE goat_dormant_peers gauge",
    ]
    for thr, n in sorted(peers_by_thr.items()):
        lines.append(f'goat_dormant_peers{{env="{env}",threshold_days="{thr}"}} {n}')
    lines += [
        "# HELP goat_dormant_person_identities Active Kratos identities whose newest session exceeds the threshold (never-logged-in excluded — see goat_identities_no_session_ever).",
        "# TYPE goat_dormant_person_identities gauge",
    ]
    for thr, n in sorted(idents_by_thr.items()):
        lines.append(f'goat_dormant_person_identities{{env="{env}",threshold_days="{thr}"}} {n}')
    lines += [
        "# HELP goat_identities_no_session_ever Active Kratos identities with no session on record (the F-241 undelivered-credential cohort dominates this — a delivery gap, not dormancy).",
        "# TYPE goat_identities_no_session_ever gauge",
        f'goat_identities_no_session_ever{{env="{env}"}} {never}',
        "# HELP goat_dormancy_sessions_check_up 1 if the Kratos sessions sweep ran, else 0 (peer axis unaffected).",
        "# TYPE goat_dormancy_sessions_check_up gauge",
        f'goat_dormancy_sessions_check_up{{env="{env}"}} {sessions_up}',
        "# HELP goat_dormancy_report_last_run_timestamp_seconds Unix time of the last run.",
        "# TYPE goat_dormancy_report_last_run_timestamp_seconds gauge",
        f"goat_dormancy_report_last_run_timestamp_seconds {int(time.time())}",
    ]
    # Written only when the peer list was read: with no list the count is
    # unknown, and goat_dormancy_report_up=0 already says so.
    if expiry is not None:
        lines += [
            "# HELP goat_device_peer_expiry_enabled NETWORK, CONTROL and EDGE-SERVER peers with a NetBird login or inactivity expiration turned on (ADR 1216; should be 0).",
            "# TYPE goat_device_peer_expiry_enabled gauge",
        ]
        for (cls, kind), n in sorted(expiry.items()):
            lines.append(
                f'goat_device_peer_expiry_enabled{{env="{env}",class="{cls}",kind="{kind}"}} {n}'
            )
    return "\n".join(lines) + "\n"


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: dormancy_report.py <output-dir>", file=sys.stderr)
        return 1
    outdir = pathlib.Path(sys.argv[1])
    outdir.mkdir(parents=True, exist_ok=True)
    out_path = outdir / "goat_dormancy_report.prom"

    env = os.environ.get("GOAT_ENV", "unknown")
    nb_url = os.environ.get("NETBIRD_MGMT_URL", "").strip()
    nb_token = os.environ.get("NETBIRD_API_TOKEN", "").strip()
    kratos = os.environ.get("KRATOS_ADMIN_URL", "http://127.0.0.1:4434").strip()
    thresholds = sorted(
        {int(t) for t in os.environ.get("DORMANCY_THRESHOLDS", "7,14,30").split(",") if t.strip()}
    )
    now = time.time()

    if not nb_url or not nb_token:
        print("dormancy-report: NETBIRD_MGMT_URL / NETBIRD_API_TOKEN unset", file=sys.stderr)
        atomic_write(out_path, render_prom(env, 0, {}, {}, 0, 0))
        return 1

    try:
        peers = fetch_peers(nb_url, nb_token)
    except Exception as e:  # noqa: BLE001
        print(f"dormancy-report: peer fetch failed: {e}", file=sys.stderr)
        atomic_write(out_path, render_prom(env, 0, {}, {}, 0, 0))
        return 1

    # PEER axis. days-quiet per peer (connected ⇒ active; never-seen ⇒ inf).
    peer_quiet: list[tuple[str, float]] = []
    for p in peers:
        if p.get("connected"):
            continue
        ts = parse_ts(str(p.get("last_seen") or ""))
        days = (now - ts) / 86400 if ts is not None else float("inf")
        peer_quiet.append((str(p.get("name", "?")), days))
    peers_by_thr = {thr: sum(1 for _, d in peer_quiet if d > thr) for thr in thresholds}

    # IDENTITY axis (login recency; never-logged-in split out).
    sessions_up, idents_by_thr, never, ident_quiet = 0, {}, 0, []
    try:
        idents = fetch_identities(kratos)
    except Exception as e:  # noqa: BLE001
        print(f"dormancy-report: identities fetch failed: {e}", file=sys.stderr)
        idents = []
    if idents:
        sessions_up, latest = fetch_latest_sessions(kratos)
        if sessions_up:
            for i in idents:
                if i.get("state") != "active":
                    continue
                email = str((i.get("traits") or {}).get("email") or i.get("id"))
                ts = latest.get(str(i.get("id")))
                if ts is None:
                    never += 1
                else:
                    ident_quiet.append((email, (now - ts) / 86400))
            idents_by_thr = {
                thr: sum(1 for _, d in ident_quiet if d > thr) for thr in thresholds
            }

    expiry_rows = expirable_device_peers(peers)
    atomic_write(
        out_path,
        render_prom(
            env, 1, peers_by_thr, idents_by_thr, never, sessions_up,
            expiry_counts(expiry_rows),
        ),
    )

    # Human report → journald. Names live here, never in the .prom.
    main_thr = thresholds[len(thresholds) // 2] if thresholds else 14
    print(
        f"dormancy-report[{env}]: peers dormant>{main_thr}d: {peers_by_thr.get(main_thr, 0)}"
        f"/{len(peers)}; identities login-dormant>{main_thr}d: {idents_by_thr.get(main_thr, 0)}"
        f"; never-logged-in: {never}; sessions_check={'ok' if sessions_up else 'DEGRADED'}"
    )
    for name, days in sorted(peer_quiet, key=lambda x: -x[1]):
        if days > main_thr:
            d = "never" if days == float("inf") else f"{int(days)}d"
            print(f"  DORMANT-PEER {name} quiet={d}")
    for email, days in sorted(ident_quiet, key=lambda x: -x[1]):
        if days > main_thr:
            print(f"  DORMANT-LOGIN {email} last-login={int(days)}d ago")
    for name, cls, kind in sorted(expiry_rows):
        print(f"  EXPIRABLE-DEVICE {name} class={cls} expiry={kind}")
    print(
        "dormancy-report: REPORT-ONLY — nothing moved. The shed leg (goat admin members "
        "move --filter last_seen>Nd --to PROBATION-<role>) stays behind probation_enforce "
        "until these counts are calibrated (ADR 1089)."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
