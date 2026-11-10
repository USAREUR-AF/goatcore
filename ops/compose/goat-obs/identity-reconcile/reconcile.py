#!/usr/bin/env python3
"""identity-reconcile — detect live identities with no declared record (strays).

The durable backstop for the missing-manifest gap (2026-07): a person can be live
on the mesh + logged in yet have no manifest / roster entry, because no path
enforces "every live identity is declared." The access COP already CLASSIFIES
this (a person node reconciles to `out_of_band`), but nothing WATCHED that
signal — so a stray sat invisible for a month until a hand census found it.

This is the watcher. It reads the portal's already-computed reconciliation
(`GET /api/access/roster`, operator-scoped) and emits a Prometheus textfile
metric so the obs stack alerts when an unexplained person-stray appears. It adds
NO reconciliation logic of its own — the graph is the single source of the
verdict (data-access-cop.md); this only surfaces + counts it.

Metrics (node-exporter textfile collector; NO PII in labels — mesh-internal but
still aggregate-only, per the operator-action-audit PII discipline):
    goat_identity_strays{env,scope="all|unallowlisted"}     gauge
    goat_identity_strays_by_reason{env,recon_reason}        gauge
    goat_identity_reconcile_last_run_timestamp_seconds      gauge (staleness canary)
    goat_identity_reconcile_up{env}         gauge (1 ok / 0 blind: fetch failed
                                            OR a layer feeding the verdict is dark)
    goat_identity_reconcile_degraded_layers{env}            gauge (count, why up=0)

The identities themselves (WHO) are printed to stdout → journald (operator reads
`journalctl -t identity-reconcile` for the names), never written to the .prom.

An allowlist (identity-reconcile-allowlist.txt, one email or `*@domain` glob per
line) exempts legitimately-manifest-less identities — test/drill accounts and
machine/service bridges. A GENUINE person-stray (the pawel case) is NOT
allowlisted and IS what the alert fires on: the fix is to backfill their
manifest, not to allowlist them.

A roster the portal served while one of its own layers was dark reads exactly
like a healthy one (`strays=0`), so `_up` also goes to 0 when the payload's
`notes[]` says a layer feeding the verdict is unavailable — see degrading_notes.

Usage: reconcile.py <output-dir>
  env: PORTAL_URL, PORTAL_OPERATOR_API_KEY, GOAT_ENV (default unknown; obs.yml sets it),
       IDENTITY_RECONCILE_ALLOWLIST (default <script-dir>/identity-reconcile-allowlist.txt)
Exit: 0 clean, 2 unallowlisted strays found, 1 fetch/config error. Degradation
is reported on `_up`, NOT on the exit code — the deploy legs in obs.yml treat
rc=1 as a task failure, and a portal layer being down is not a failed deploy.
"""

import fnmatch
import json
import os
import pathlib
import socket
import ssl
import sys
import time
import urllib.parse
import urllib.request

HERE = pathlib.Path(__file__).parent


def install_resolve_override(host: str, ip: str) -> None:
    """`curl --resolve` equivalent: connect to `ip` but keep `host` for the TLS
    SNI + Host header. The portal is mesh-internal and the mgmt HOST resolver
    doesn't carry the mesh DNS name (only containers do), yet traefik on the Thor
    serves it on 127.0.0.1:443 keyed by SNI. Patching getaddrinfo (not /etc/hosts)
    keeps this self-contained and leaves urllib to set SNI/Host from the URL host.
    """
    real = socket.getaddrinfo

    def patched(h, port, *a, **k):
        return real(ip if h == host else h, port, *a, **k)

    socket.getaddrinfo = patched


def load_allowlist(path: pathlib.Path) -> list[str]:
    if not path.exists():
        return []
    out = []
    for raw in path.read_text().splitlines():
        line = raw.split("#", 1)[0].strip().lower()
        if line:
            out.append(line)
    return out


def is_allowlisted(email: str, patterns: list[str]) -> bool:
    e = email.strip().lower()
    return any(fnmatch.fnmatch(e, p) for p in patterns)


def fetch_roster(url: str, key: str) -> dict:
    # Mesh-internal portal with an internally-signed cert; verification is not
    # the trust boundary here (the mesh is), so tolerate the self-signed chain —
    # same posture the ansible revoke/onboard legs take against portal/kratos.
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    req = urllib.request.Request(
        url.rstrip("/") + "/api/access/roster",
        headers={"Authorization": f"Bearer {key}"},
    )
    with urllib.request.urlopen(req, context=ctx, timeout=30) as r:
        return json.loads(r.read().decode())


# The failure vocabulary the portal actually emits into `notes[]`:
# "<layer> unavailable: <err>", "<layer> ... error: <err>" (api/access.go
# collectAccessSources) and netbird's "slow/unreachable" (netbirdpoll.go
# PeersForAccess). Matched as substrings so an unrecognized future note in the
# same vocabulary still counts — blind-but-loud beats blind-and-quiet.
DEGRADED_MARKERS = ("unavailable", "unreachable", "error:")


def degrading_notes(notes: list) -> list[str]:
    """The subset of the roster's `notes[]` meaning a layer feeding the person-
    stray verdict went dark.

    `notes[]` is NOT a pure warning channel. A perfectly healthy portal always
    returns standing doctrinal text there — the write-anchor rule and the flat
    community-read caveat are appended unconditionally. So "notes is non-empty"
    cannot mean degraded: it would pin _up=0 forever and train operators to
    ignore the alert. Degradation is matched on the failure vocabulary instead.

    Exempt: the mesh/netbird notes. That layer supplies the `on_mesh` display
    column, not the verdict — a person node is out_of_band when a live Kratos
    identity has no declared roster entry (access/graph.go), which a stale peer
    snapshot does not change. Calling that "blind" would be its own false alarm.
    """
    out: list[str] = []
    for note in notes:
        text = str(note)
        low = text.lower()
        if not any(m in low for m in DEGRADED_MARKERS):
            continue
        if "netbird mgmt" in low or low.startswith("mesh layer unavailable"):
            continue
        out.append(text)
    return out


def fetch_offvocab(kratos_url: str, sot: set[str]) -> tuple[int, int, list[str]]:
    """(check_up, count, emails) of identities carrying traits groups outside the
    Ory SoT vocabulary. data.* grammar values (ADR 1074) ride the same claim by
    design and are never off-vocab. Mesh-role names appearing here are the
    F-213 conflation this check exists to catch."""
    try:
        req = urllib.request.Request(kratos_url.rstrip("/") + "/admin/identities?per_page=500")
        with urllib.request.urlopen(req, timeout=30) as r:
            ids = json.loads(r.read().decode())
    except Exception as e:  # noqa: BLE001 — degrade to check_up=0, never crash the cron
        print(f"identity-reconcile: kratos off-vocab fetch failed: {e}", file=sys.stderr)
        return 0, 0, []
    bad: list[str] = []
    for i in ids:
        # Login groups live in admin-managed metadata_public.
        # A groups key there, even null, marks the identity migrated: traits are
        # never read for it. Only a not-yet-backfilled identity falls back.
        meta = i.get("metadata_public") or {}
        groups = (meta.get("groups") or []) if "groups" in meta else ((i.get("traits") or {}).get("groups") or [])
        if any(g not in sot and not str(g).startswith("data.") for g in groups):
            bad.append(str((i.get("traits") or {}).get("email") or i.get("id")))
    return 1, len(bad), bad


def atomic_write(out_path: pathlib.Path, body: str) -> None:
    tmp = out_path.with_suffix(out_path.suffix + f".tmp.{os.getpid()}")
    tmp.write_text(body)
    tmp.chmod(0o644)
    tmp.replace(out_path)


def render_prom(env: str, up: int, total: int, unallow: int, by_reason: dict[str, int], offvocab_up: int = 0, offvocab: int = 0, degraded: int = 0) -> str:
    lines = [
        "# HELP goat_identity_reconcile_up 1 if the roster fetch succeeded AND every layer feeding the stray verdict was live, else 0 (the counts below are then not trustworthy).",
        "# TYPE goat_identity_reconcile_up gauge",
        f'goat_identity_reconcile_up{{env="{env}"}} {up}',
        "# HELP goat_identity_reconcile_degraded_layers Portal-reported layers that were unavailable this run (0 with up=0 means the fetch itself failed). Texts go to journald.",
        "# TYPE goat_identity_reconcile_degraded_layers gauge",
        f'goat_identity_reconcile_degraded_layers{{env="{env}"}} {degraded}',
        "# HELP goat_identity_strays Live person identities reconciling to out_of_band (no declared record).",
        "# TYPE goat_identity_strays gauge",
        f'goat_identity_strays{{env="{env}",scope="all"}} {total}',
        f'goat_identity_strays{{env="{env}",scope="unallowlisted"}} {unallow}',
        "# HELP goat_identity_strays_by_reason Unallowlisted person-strays by reconcile reason.",
        "# TYPE goat_identity_strays_by_reason gauge",
    ]
    for reason, n in sorted(by_reason.items()):
        lines.append(f'goat_identity_strays_by_reason{{env="{env}",recon_reason="{reason}"}} {n}')
    lines.append("# HELP goat_identity_offvocab_check_up 1 if the Kratos off-vocab groups check ran, else 0.")
    lines.append("# TYPE goat_identity_offvocab_check_up gauge")
    lines.append(f'goat_identity_offvocab_check_up{{env="{env}"}} {offvocab_up}')
    lines.append("# HELP goat_identity_offvocab_groups Identities whose Ory traits carry groups outside the group-permissions SoT vocabulary.")
    lines.append("# TYPE goat_identity_offvocab_groups gauge")
    lines.append(f'goat_identity_offvocab_groups{{env="{env}"}} {offvocab}')
    lines.append("# HELP goat_identity_reconcile_last_run_timestamp_seconds Unix time of the last run.")
    lines.append("# TYPE goat_identity_reconcile_last_run_timestamp_seconds gauge")
    lines.append(f"goat_identity_reconcile_last_run_timestamp_seconds {int(time.time())}")
    return "\n".join(lines) + "\n"


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: reconcile.py <output-dir>", file=sys.stderr)
        return 1
    outdir = pathlib.Path(sys.argv[1])
    outdir.mkdir(parents=True, exist_ok=True)
    out_path = outdir / "goat_identity_reconcile.prom"

    env = os.environ.get("GOAT_ENV", "unknown")
    url = os.environ.get("PORTAL_URL", "").strip()
    key = os.environ.get("PORTAL_OPERATOR_API_KEY", "").strip()
    allow_path = pathlib.Path(
        os.environ.get("IDENTITY_RECONCILE_ALLOWLIST", HERE / "identity-reconcile-allowlist.txt")
    )
    allow = load_allowlist(allow_path)

    if not url or not key:
        print("identity-reconcile: PORTAL_URL / PORTAL_OPERATOR_API_KEY unset", file=sys.stderr)
        atomic_write(out_path, render_prom(env, 0, 0, 0, {}))
        return 1

    # Mesh-internal portal, mgmt-host resolver blind to the mesh name: connect to
    # PORTAL_RESOLVE_IP (default 127.0.0.1 — traefik on the Thor) but keep the
    # hostname for SNI/Host. Set PORTAL_RESOLVE_IP="" to disable (real DNS).
    resolve_ip = os.environ.get("PORTAL_RESOLVE_IP", "127.0.0.1").strip()
    if resolve_ip:
        install_resolve_override(urllib.parse.urlparse(url).hostname or "", resolve_ip)

    # Off-vocab groups check: Ory traits vs the SoT vocabulary,
    # rendered into ORY_SOT_GROUPS at deploy time from group-permissions.yml.
    sot = {g.strip() for g in os.environ.get("ORY_SOT_GROUPS", "").split(",") if g.strip()}
    kratos_url = os.environ.get("KRATOS_ADMIN_URL", "http://127.0.0.1:4434").strip()
    offvocab_up, offvocab_n, offvocab_who = (0, 0, [])
    if sot and kratos_url:
        offvocab_up, offvocab_n, offvocab_who = fetch_offvocab(kratos_url, sot)

    try:
        data = fetch_roster(url, key)
    except Exception as e:  # noqa: BLE001 — a fetch failure must degrade to up=0, not crash the cron
        print(f"identity-reconcile: roster fetch failed: {e}", file=sys.stderr)
        atomic_write(out_path, render_prom(env, 0, 0, 0, {}, offvocab_up, offvocab_n))
        return 1

    rows = data.get("rows") or (data if isinstance(data, list) else [])
    strays = [r for r in rows if r.get("kind") == "person" and r.get("state") == "out_of_band"]
    unallowlisted = [r for r in strays if not is_allowlisted(str(r.get("label", "")), allow)]

    by_reason: dict[str, int] = {}
    for r in unallowlisted:
        reason = str(r.get("recon_reason") or "unknown")
        by_reason[reason] = by_reason.get(reason, 0) + 1

    # A 200 is not health: the portal serves a well-formed roster with the dark
    # layers simply missing from it, so `strays=0` off a degraded roster means
    # "did not look", not "looked and found none". The counts are still emitted
    # (they are data) — up=0 marks them untrustworthy.
    degraded = degrading_notes(data.get("notes") or [])
    up = 0 if degraded else 1
    atomic_write(out_path, render_prom(env, up, len(strays), len(unallowlisted), by_reason, offvocab_up, offvocab_n, len(degraded)))

    # Human report → stdout (journald). Names live here, never in the .prom.
    fresh = data.get("freshness") or {}
    print(
        f"identity-reconcile[{env}]: {len(strays)} person-stray(s), "
        f"{len(unallowlisted)} unallowlisted. roster generated_at={fresh.get('generated_at','?')}"
    )
    for note in degraded:
        print(f"  DEGRADED {note}", file=sys.stderr)
    if degraded:
        print(
            f"identity-reconcile: BLIND — {len(degraded)} portal layer(s) unavailable, so the "
            "stray counts above are a floor, not a verdict (reconcile_up=0). Restore the named "
            "layer and re-run; do not read a zero here as all-clear.",
            file=sys.stderr,
        )
    for r in unallowlisted:
        print(
            f"  STRAY {r.get('label','?')} reason={r.get('recon_reason','?')} "
            f"on_mesh={r.get('on_mesh')} — needs a manifest (backfill) or offboard"
        )
    for email in offvocab_who:
        print(f"  OFF-VOCAB {email} — Ory traits carry a group outside the SoT vocabulary; "
              f"mesh-role names belong in NetBird groups, not Ory traits")
    if unallowlisted:
        print(
            "identity-reconcile: ACTION — each unallowlisted stray is a live identity with no "
            "declared record. Author its manifest or revoke it. "
            "If it's a legitimate test/service account, add it to the allowlist."
        )
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
