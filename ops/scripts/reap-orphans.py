#!/usr/bin/env python3
"""Batch orphan reaper — enumerate half-created accounts, classify, emit the cut plan.

The offboard *cut* is a solved one-command operation (`goat admin onboard
<handle> --revoke [--purge]`, ADR 1031). What this tool adds is the missing
glue (PR #1911 backlog item 1): a reconciliation over the *join-bearing* rows
so orphans are enumerated authoritatively instead of guessed.

Cross-plane join model (no single key spans all planes — F-266):

  onboard_proposals row  — carries BOTH handle and email; the only reliable join
  portal slots row       — keyed slot_id (uuid), aliased by handle (assigned_to)
  Kratos identity        — keyed by email; never stores the handle
  NetBird peers/keys     — name-prefixed `peer-<handle>-...`

Classification (read-only; this tool NEVER cuts):

  ORPHAN        slot assigned + unrevoked, proposal exists, NO Kratos identity
                → the F-266 crash-orphan shape. Disposition repair vs purge by
                email: test-domain emails purge, real emails repair
                (revoke → re-onboard, proven procedure).
  RESIDUE       latest-status `failed` proposal with NO live artifact on any
                plane (no slot, no Kratos, no NetBird peer/key) → inert; the
                proposal row is audit trail, leave it.
  TEST-IDENT    active Kratos identity on a test domain (drill-*@example.test …)
                → report for declare-or-purge decision; never auto-classified
                as cut (they may be deliberate RBAC-drill fixtures).
  PRE-CONSOLE   slot with no proposal row at all (pre-console cohort, synth
                vendors, system slots) → report count only; triage belongs to
                `goat access strays` + the probation list (ADR 1068).
  UNDECLARED    active Kratos identity whose email_hash is absent from the
                declared roster → NOT an orphan; the fix is the declared-roster
                sync (sync-onboarded-manifests.py + gen-roster.py), pointer
                emitted.

Run on the mgmt host (all stores local), e.g.:

    sudo python3 ops/scripts/reap-orphans.py --env site-a \
        --onboard-db /var/lib/goat-id-console/onboard.db \
        --registry-db /opt/goat-zenoh-portal/registry.db \
        --kratos-admin-url http://127.0.0.1:4434 \
        [--nb-url https://netbird.example.net]   # token: $SITE_A_NB_API_TOKEN
        [--roster docs/operators/site-a/roster.json] [--json] [--cohort purge.txt]

On a goatnet whose portal runs on Postgres (portal_store: postgres), replace
--registry-db with --registry-store postgres: the slots are read through
`docker exec postgres-a psql` (portal_registry_pg.py). The onboard store is the
console's and keeps its own flag.

--env names the goatnet (default $GOATNET; neither set is refused). The NetBird
token is read from <GOATNET>_NB_API_TOKEN (site-a: SITE_A_NB_API_TOKEN), else
GOAT_NB_API_TOKEN.

Test-residue e-mail suffixes: the public list below, plus one suffix per line
(# comments allowed) from every ops/scripts/reap-orphans.d/*.test-domains file a
deployment adds, plus --test-domain.

Every data source is optional-degradable EXCEPT the two DBs: a missing
Kratos/NetBird/roster source drops the dependent buckets and says so loudly

"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
import ssl
import sys
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import portal_registry_pg  # noqa: E402

TEST_DOMAIN_SUFFIXES = (
    "@example.com",
    ".invite.local",
    "@zagros.test",
    ".test",
    "@notional.zagros",
)


def _deployment_test_domains() -> tuple[str, ...]:
    """The suffixes a deployment adds in ops/scripts/reap-orphans.d/*.test-domains."""
    out: list[str] = []
    for p in sorted((Path(__file__).resolve().parent / "reap-orphans.d").glob("*.test-domains")):
        for line in p.read_text(encoding="utf-8").splitlines():
            line = line.split("#", 1)[0].strip().lower()
            if line and line not in TEST_DOMAIN_SUFFIXES and line not in out:
                out.append(line)
    return tuple(out)


TEST_DOMAIN_SUFFIXES = TEST_DOMAIN_SUFFIXES + _deployment_test_domains()


def is_test_email(email: str, extra: tuple[str, ...] = ()) -> bool:
    e = (email or "").strip().lower()
    return any(e.endswith(s) for s in TEST_DOMAIN_SUFFIXES + tuple(extra))


def email_hash(email: str) -> str:
    return hashlib.sha256(email.strip().lower().encode()).hexdigest()


# ---------------------------------------------------------------- data pulls

def read_proposals(db_path: str) -> list[dict]:
    """Latest proposal per handle from the id-console store (read-only).

    Carries `short_tag` (from manifest_json's vendor block) because the
    peer-name match must mirror goat's `revoke_peer_matches` anchoring, and
    legacy peer names embed the tag: `peer-<handle>-<tag>-<env>-...`.
    """
    db = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    cur = db.execute(
        "SELECT id, handle, email, status, env, created_at, manifest_json FROM onboard_proposals"
    )
    cols = [d[0] for d in cur.description]
    rows = [dict(zip(cols, r)) for r in cur]
    db.close()
    latest: dict[str, dict] = {}
    for p in rows:
        h = p["handle"]
        if h and (h not in latest or p["created_at"] >= latest[h]["created_at"]):
            latest[h] = p
    for p in latest.values():
        try:
            manifest = json.loads(p.get("manifest_json") or "{}")
        except json.JSONDecodeError:
            manifest = {}
        p["short_tag"] = ((manifest.get("vendor") or {}).get("short_tag") or "").strip()
        p.pop("manifest_json", None)
    return sorted(latest.values(), key=lambda p: p["handle"])


SLOTS_SQL = (
    "SELECT slot_id, assigned_to, assigned_at, cert_minted_at FROM slots "
    "WHERE assigned_to IS NOT NULL AND assigned_to != '' AND revoked_at IS NULL"
)


def read_slots_pg(conn: dict[str, str]) -> list[dict]:
    """read_slots() against the Postgres registry."""
    return portal_registry_pg.query(SLOTS_SQL, **conn)


def read_slots(db_path: str) -> list[dict]:
    """Assigned, unrevoked slots from the portal registry (read-only)."""
    db = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    cur = db.execute(SLOTS_SQL)
    cols = [d[0] for d in cur.description]
    rows = [dict(zip(cols, r)) for r in cur]
    db.close()
    return rows


def _http_json(url: str, headers: dict[str, str]) -> object:
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE  # mesh-internal, cert pinned elsewhere
    req = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(req, timeout=30, context=ctx) as r:
        return json.load(r)


def read_kratos_identities(admin_url: str) -> list[dict]:
    """All Kratos identities via the admin API.

    Deliberately a single un-paged call at the max page size: Kratos's legacy
    `page` parameter is 0-indexed on some versions and token-based on others —
    `page=1` can return an EMPTY list, which would silently classify every
    healthy slot-holder as an orphan. A hard failure beats a
    silently-empty identity set, so an at-cap result raises.
    """
    batch = _http_json(f"{admin_url.rstrip('/')}/admin/identities?per_page=1000", {})
    if not isinstance(batch, list):
        raise RuntimeError(f"Kratos /admin/identities returned non-list: {type(batch).__name__}")
    if len(batch) >= 1000:
        raise RuntimeError(
            "Kratos identity count at the 1000 pagination cap — coverage would be "
            "silently incomplete; teach this tool token-based pagination first"
        )
    return batch


def read_netbird(nb_url: str, token: str) -> tuple[list[dict], list[dict]]:
    """(peers, setup_keys) from the NetBird mgmt API."""
    hdr = {"Authorization": f"Token {token}"}
    peers = _http_json(f"{nb_url.rstrip('/')}/api/peers", hdr)
    keys = _http_json(f"{nb_url.rstrip('/')}/api/setup-keys", hdr)
    return (peers if isinstance(peers, list) else []), (
        keys if isinstance(keys, list) else []
    )


def kratos_emails(identities: list[dict]) -> dict[str, str]:
    """email(lower) -> state, from identity traits."""
    out: dict[str, str] = {}
    for i in identities:
        traits = i.get("traits") or {}
        email = traits.get("email") or (traits.get("emails") or [None])[0]
        if email:
            out[email.strip().lower()] = i.get("state", "?")
    return out


# ------------------------------------------------------------------ classify

def classify(
    proposals: list[dict],
    slots: list[dict],
    kratos: dict[str, str] | None,
    nb_peers: list[dict] | None,
    nb_keys: list[dict] | None,
    roster_hashes: set[str] | None,
    extra_test_domains: tuple[str, ...] = (),
    env: str = "zagros",
) -> dict:
    """Pure classification over the pulled snapshots. Returns the plan dict."""
    by_handle = {p["handle"]: p for p in proposals}
    slot_handles = {s["assigned_to"]: s for s in slots}

    def name_matches(name: str, handle: str, tag: str) -> bool:
        """Exact mirror of goat's `revoke_peer_matches` anchoring: only
        `peer-<handle>-<env>-…` and (when the manifest carries a short_tag)
        `peer-<handle>-<tag>-<env>-…`. A loose `peer-<handle>-` prefix falsely
        attributes `peer-pat-lee-…` to handle `pat` — the same bug the revoke
        matcher guards against."""
        if name.startswith(f"peer-{handle}-{env}-"):
            return True
        t = (tag or "").strip()
        return bool(t) and name.startswith(f"peer-{handle}-{t}-{env}-")

    def nb_artifacts(handle: str, tag: str = "") -> list[str]:
        if nb_peers is None:
            return []
        found = [
            f"peer:{p['name']}"
            for p in nb_peers
            if name_matches(p.get("name", ""), handle, tag)
        ]
        found += [
            f"setup-key:{k['name']}"
            for k in (nb_keys or [])
            if name_matches(k.get("name", ""), handle, tag) and not k.get("revoked")
        ]
        return found

    orphans, residue, pre_console = [], [], []
    for s in slots:
        h = s["assigned_to"]
        p = by_handle.get(h)
        if p is None:
            pre_console.append({"handle": h, "slot": s["slot_id"]})
            continue
        if kratos is not None and p["email"].strip().lower() not in kratos:
            disposition = "purge" if is_test_email(p["email"], extra_test_domains) else "repair"
            orphans.append(
                {
                    "handle": h,
                    "email": p["email"],
                    "slot": s["slot_id"],
                    "proposal": p["id"],
                    "proposal_status": p["status"],
                    "disposition": disposition,
                    "netbird": nb_artifacts(h, p.get("short_tag", "")),
                }
            )

    for p in proposals:
        if p["status"] != "failed":
            continue
        h = p["handle"]
        if h in slot_handles:
            continue  # already surfaced as an orphan above
        has_kratos = kratos is not None and p["email"].strip().lower() in kratos
        nb = nb_artifacts(h, p.get("short_tag", ""))
        if not has_kratos and not nb:
            residue.append({"handle": h, "email": p["email"], "proposal": p["id"]})
        else:
            kratos_state = (
                "unknown" if kratos is None else ("present" if has_kratos else "absent")
            )
            disposition = "purge" if is_test_email(p["email"], extra_test_domains) else "repair"
            orphans.append(
                {
                    "handle": h,
                    "email": p["email"],
                    "slot": None,
                    "proposal": p["id"],
                    "proposal_status": p["status"],
                    "disposition": disposition,
                    "netbird": nb,
                    "kratos": kratos_state,
                }
            )

    test_idents = (
        sorted(
            e
            for e, state in (kratos or {}).items()
            if state == "active" and is_test_email(e, extra_test_domains)
        )
        if kratos is not None
        else None
    )

    undeclared = None
    if kratos is not None and roster_hashes is not None:
        undeclared = sorted(
            e
            for e, state in kratos.items()
            if state == "active"
            and email_hash(e) not in roster_hashes
            and not is_test_email(e, extra_test_domains)
        )

    return {
        "orphans": sorted(orphans, key=lambda o: o["handle"]),
        "residue": sorted(residue, key=lambda r: r["handle"]),
        "test_identities": test_idents,
        "pre_console_slots": sorted(pre_console, key=lambda s: s["handle"]),
        "undeclared": undeclared,
    }


# -------------------------------------------------------------------- render

def render_plan(plan: dict, env: str, degraded: list[str]) -> str:
    lines: list[str] = []
    add = lines.append
    for d in degraded:
        add(f"!! DEGRADED: {d}")
    orphans = plan["orphans"]
    add(f"ORPHANS ({len(orphans)}) — half-created; cut with the audited revoke:")
    for o in orphans:
        add(
            f"  {o['handle']:30s} {o['email']:40s} slot={str(o['slot'])[:8] if o['slot'] else '-':8s} "
            f"proposal={o['proposal_status']} → {o['disposition'].upper()}"
        )
        for a in o["netbird"]:
            add(f"    live netbird artifact: {a}")
        if o["disposition"] == "repair":
            add(
                f"    goat admin onboard {o['handle']} --revoke --env {env}   # then re-onboard"
            )
        else:
            add(f"    goat admin onboard {o['handle']} --revoke --purge --env {env}")
    residue = plan["residue"]
    add(f"INERT RESIDUE ({len(residue)}) — failed proposals, zero live artifacts; rows are audit trail, no action:")
    for r in residue:
        add(f"  {r['handle']:30s} {r['email']:40s} {r['proposal']}")
    if plan["test_identities"] is not None:
        ti = plan["test_identities"]
        add(f"TEST IDENTITIES ({len(ti)}) — active Kratos on test domains; decide declare-vs-purge (may be deliberate fixtures):")
        for e in ti:
            add(f"  {e}")
    if plan["undeclared"] is not None:
        ud = plan["undeclared"]
        add(
            f"UNDECLARED ({len(ud)}) — live + legitimate but missing from the declared roster; "
            "NOT orphans. Fix = sync-onboarded-manifests.py + gen-roster.py regen from the full manifest store:"
        )
        for e in ud:
            add(f"  {e}")
    pc = plan["pre_console_slots"]
    add(
        f"PRE-CONSOLE SLOTS ({len(pc)}) — no proposal row; triage via `goat access strays` + probation (ADR 1068), not this tool."
    )
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--onboard-db", required=True, help="id-console onboard.db path")
    ap.add_argument("--registry-db", default="", help="portal registry.db path (sqlite store)")
    portal_registry_pg.add_arguments(ap, "--registry-db")
    ap.add_argument("--kratos-admin-url", default="http://127.0.0.1:4434")
    ap.add_argument("--no-kratos", action="store_true", help="skip the Kratos pull (drops orphan/undeclared detection)")
    ap.add_argument("--nb-url", default="", help="NetBird mgmt URL; token from $<GOATNET>_NB_API_TOKEN / $GOAT_NB_API_TOKEN")
    ap.add_argument(
        "--roster", default="",
        help="THIS GOATNET's declared roster for the UNDECLARED bucket — "
             "docs/operators/<--env>/roster.json. The controller-side union at "
             "docs/operators/roster.json is refused (ADR 1172)")
    ap.add_argument("--env", default=os.environ.get("GOATNET", ""),
                    help="the goatnet (site-a): its peer names, its token, the emitted commands (default $GOATNET)")
    ap.add_argument("--test-domain", action="append", default=[], help="extra email suffix classified as test residue")
    ap.add_argument("--cohort", default="", help="write the PURGE cohort handles to FILE (for --revoke --from FILE)")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()
    if not args.env:
        ap.error("--env is required (or set GOATNET): the goatnet whose accounts are swept")
    token_var = args.env.upper().replace("-", "_") + "_NB_API_TOKEN"
    if args.registry_store == "sqlite" and not args.registry_db:
        ap.error("--registry-db is required with --registry-store sqlite")

    degraded: list[str] = []
    proposals = read_proposals(args.onboard_db)
    if args.registry_store == "postgres":
        slots = read_slots_pg(portal_registry_pg.kwargs(args))
    else:
        slots = read_slots(args.registry_db)

    kratos = None
    if not args.no_kratos:
        try:
            kratos = kratos_emails(read_kratos_identities(args.kratos_admin_url))
        except Exception as e:  # noqa: BLE001 — degrade loudly, keep sweeping
            degraded.append(f"Kratos pull failed ({e}) — ORPHAN/UNDECLARED buckets dropped")
    else:
        degraded.append("--no-kratos: ORPHAN/UNDECLARED buckets dropped")

    nb_peers = nb_keys = None
    if args.nb_url:
        token = os.environ.get(token_var) or os.environ.get("GOAT_NB_API_TOKEN") or ""
        if not token:
            degraded.append(f"NetBird token unset (${token_var}) — live-artifact check dropped")
        else:
            try:
                nb_peers, nb_keys = read_netbird(args.nb_url, token)
            except Exception as e:  # noqa: BLE001
                degraded.append(f"NetBird pull failed ({e}) — live-artifact check dropped")
    else:
        degraded.append("--nb-url unset — NetBird live-artifact check dropped")

    roster_hashes = None
    if args.roster:
        # REFUSE THE UNION. The roster answers one question here: is this live
        # Kratos identity DECLARED on this goatnet? The controller-side union
        # spans every environment, so a person declared only on one goatnet would
        # read as declared on another and the reaper would leave a real orphan in
        # place. Under-reaping is the failure direction that matters, and it is
        # silent — the identity simply never appears in a bucket. ADR 1172.
        rp = os.path.abspath(args.roster)
        if os.path.basename(os.path.dirname(rp)) == "operators":
            sys.exit(
                f"REFUSING --roster {args.roster}: that is the controller-side "
                f"union across every environment, not {args.env}'s declared half. "
                f"Pass docs/operators/{args.env}/roster.json (build it with "
                f"ops/operators/gen-roster.py --env {args.env})."
            )
        with open(args.roster, encoding="utf-8") as f:
            roster = json.load(f)
        roster_hashes = {m.get("email_hash") for m in roster.get("members", [])}

    plan = classify(
        proposals, slots, kratos, nb_peers, nb_keys, roster_hashes,
        tuple(args.test_domain), env=args.env,
    )

    if args.cohort:
        purge = [o["handle"] for o in plan["orphans"] if o["disposition"] == "purge"]
        with open(args.cohort, "w", encoding="utf-8") as f:
            f.write("\n".join(purge) + ("\n" if purge else ""))
        print(f"wrote {len(purge)} purge handle(s) to {args.cohort}", file=sys.stderr)

    if args.json:
        print(json.dumps({"degraded": degraded, **plan}, indent=2))
    else:
        print(render_plan(plan, args.env, degraded))
    return 0


if __name__ == "__main__":
    sys.exit(main())
