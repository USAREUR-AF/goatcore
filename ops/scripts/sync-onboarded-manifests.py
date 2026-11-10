#!/usr/bin/env python3
"""Materialize runner-minted manifests into the declared operator registry.

The onboard-runner mints from its OWN repo checkout on the mgmt host, so the
manifest it writes never lands in the operator's manifest store — the declared
roster (docs/operators/roster.json, the Data-Access COP's declared half) then
doesn't know the person, and the COP misreports them (a freshly onboarded,
connected admin renders `cut_off` off the ghost of a previously revoked identity
sharing their email).

The portal database is the authoritative record of every runner mint: each
onboarded proposal carries the exact manifest the runner materialized
(`manifest_json`). This tool closes the split-brain by syncing that record
into the operator manifest store:

    ssh -N -L 18088:127.0.0.1:8088 operator@<mgmt-host> &
    ops/scripts/sync-onboarded-manifests.py --env site-a --url http://127.0.0.1:18088 \
        [--operators-dir ops/operators] [--dry-run]

reads onboarded proposals from the ID-ADMIN-CONSOLE, which owns the proposal
store (the portal served this route before the onboarding extraction and now
answers 404). Base from --url / $ID_CONSOLE_URL, bearer from
$<ENV>_PORTAL_OPERATOR_API_KEY / $PORTAL_OPERATOR_API_KEY, <ENV> being --env in
capitals with `-` as `_` (site-a reads $SITE_A_PORTAL_OPERATOR_API_KEY). Writes each as
ops/operators/<env>/<handle>.yml, then the normal `gen-roster.py` regen +
roster PR + portal deploy picks them up.

Rules:
  - never clobbers an existing manifest file (hand-authored wins; --force to
    override a specific handle);
  - de-dupes proposals by handle (newest wins);
  - validates every written manifest with ops/operators/validate.py and
    reports (but keeps) failures so drift is visible, not silent.

Run it after any runner-minted onboarding, and before any roster regen.
"""

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

import yaml


def env_prefix(env: str) -> str:
    """--env as the prefix of the goatnet's own variables: site-a -> SITE_A."""
    return env.upper().replace("-", "_")


def console_base(env: str) -> str:
    """Where the onboarding proposals actually live.

    THE PROPOSAL STORE MOVED. This tool was written when the portal served
    `/api/onboard/proposals`; onboarding was since extracted into the
    id-admin-console (onboarding-extraction-plan.md), which owns the proposal
    store — the portal now serves no such route and answers **404**. The tool
    therefore failed for every caller, silently in the sense that a 404 reads
    like "nothing to sync" to anyone not watching the traceback, and the
    declared roster drifted further behind live onboarding the whole time.

    Reaching the console: its public URL is behind oauth2-proxy and answers a
    Bearer call with a 302 to the login page, so a server-to-server caller must
    reach the APP port — `127.0.0.1:8088` on the mgmt host, or a port-forward of
    it. Pass `--url`, or set `ID_CONSOLE_URL` / `<ENV>_ID_CONSOLE_URL`.
    """
    for var in ("ID_CONSOLE_URL", f"{env_prefix(env)}_ID_CONSOLE_URL"):
        v = os.environ.get(var, "").strip()
        if v:
            return v.rstrip("/")
    return "http://127.0.0.1:8088"


def console_key(env: str) -> str:
    """A key the console's onboard-API gate accepts.

    The gate resolves the presented bearer against its own operator-key map and
    then, failing that, delegates to the portal's whoami — so a portal operator
    key works too, which is what most callers have to hand.
    """
    own = f"{env_prefix(env)}_PORTAL_OPERATOR_API_KEY"
    for var in (own, "PORTAL_OPERATOR_API_KEY"):
        v = os.environ.get(var, "").strip()
        if v:
            return v
    sys.exit(f"no operator key in ${own} / $PORTAL_OPERATOR_API_KEY")


def fetch_live_emails(admin_url: str) -> set[str]:
    """Emails of every identity LIVE on this goatnet's Kratos.

    ADR 1172: the live identity in an environment is authoritative FOR that
    environment. A proposal still reading `enrolled` while the person holds a
    live Kratos identity is a lag in the proposal store, not evidence they were
    never onboarded — and because this tool only ever read `status=onboarded`,
    every such person stays out of the declared half indefinitely, even with a
    proposal carrying a complete manifest_json, stuck at `enrolled` or
    `offboarded`.
    """
    req = urllib.request.Request(admin_url.rstrip("/") + "/admin/identities?per_page=1000")
    import ssl
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    with urllib.request.urlopen(req, timeout=30, context=ctx) as r:
        body = json.load(r)
    out = set()
    for i in body if isinstance(body, list) else []:
        em = ((i.get("traits") or {}).get("email") or "").strip().lower()
        if em:
            out.add(em)
    return out


def fetch_proposals(base: str, key: str, status: str | None) -> list[dict]:
    q = f"?status={status}" if status else ""
    return _get_proposals(base + "/api/onboard/proposals" + q, key)


def fetch_onboarded(base: str, key: str) -> list[dict]:
    return _get_proposals(base + "/api/onboard/proposals?status=onboarded", key)


def _get_proposals(url: str, key: str) -> list[dict]:
    req = urllib.request.Request(url, headers={"Authorization": "Bearer " + key})
    import ssl

    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE  # mesh-internal, cert pinned elsewhere
    try:
        with urllib.request.urlopen(req, timeout=30, context=ctx) as r:
            body = json.load(r)
    except urllib.error.HTTPError as e:
        # Name the two failures that actually happen, because both look like
        # "no proposals" to a caller reading only the exit code.
        if e.code == 404:
            sys.exit(f"{url} → 404. The proposal store lives on the "
                     "id-admin-console, not the portal — pass --url pointing at "
                     "the console's APP port (127.0.0.1:8088 on the mgmt host).")
        if e.code in (301, 302, 401, 403):
            sys.exit(f"{url} → {e.code}. The public console URL is behind "
                     "oauth2-proxy; a server-to-server call must reach the app "
                     "port directly (port-forward 8088 from the mgmt host).")
        raise
    return body if isinstance(body, list) else body.get("proposals", [])


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--env", required=True, help="goatnet env (manifest subdir), e.g. zagros")
    ap.add_argument("--operators-dir", default="ops/operators", help="operator manifest store root")
    ap.add_argument(
        "--kratos-admin-url", default="",
        help="this goatnet's OWN Kratos admin API. When given, a proposal in ANY "
             "status is materialized if its person holds a live identity here — "
             "the live plane is authoritative for this environment (ADR 1172), "
             "and a proposal stuck at `enrolled` while the person is live is a "
             "lag in the record, not evidence they were never onboarded")
    ap.add_argument("--force", action="append", default=[], help="handle whose existing file may be overwritten")
    ap.add_argument("--url", default="", help="id-admin-console base (app port), e.g. http://127.0.0.1:8088")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    outdir = Path(args.operators_dir) / args.env
    outdir.mkdir(parents=True, exist_ok=True)
    validate = Path(args.operators_dir) / "validate.py"

    base = args.url.rstrip('/') if args.url else console_base(args.env)
    key = console_key(args.env)
    proposals = fetch_onboarded(base, key)
    print(f'read {len(proposals)} onboarded proposal(s) from {base}', file=sys.stderr)

    if args.kratos_admin_url:
        live = fetch_live_emails(args.kratos_admin_url)
        seen = {(p.get("email") or "").strip().lower() for p in proposals}
        extra = []
        for p in fetch_proposals(base, key, None):
            em = (p.get("email") or "").strip().lower()
            if em and em in live and em not in seen and p.get("manifest_json"):
                extra.append(p)
                seen.add(em)
        proposals += extra
        print(
            f"live-authoritative: {len(live)} identity(ies) on this goatnet's Kratos; "
            f"{len(extra)} proposal(s) in a non-onboarded status belong to somebody "
            f"who is live here and were included",
            file=sys.stderr,
        )
    # De-dupe by handle, newest created_at wins.
    by_handle: dict[str, dict] = {}
    for p in proposals:
        if p.get("env") != args.env or not p.get("handle"):
            continue
        cur = by_handle.get(p["handle"])
        if cur is None or p.get("created_at", 0) >= cur.get("created_at", 0):
            by_handle[p["handle"]] = p

    written, skipped, invalid = [], [], []
    for handle, p in sorted(by_handle.items()):
        path = outdir / f"{handle}.yml"
        if path.exists() and handle not in args.force:
            skipped.append(handle)
            continue
        try:
            manifest = json.loads(p.get("manifest_json") or "{}")
        except json.JSONDecodeError:
            invalid.append((handle, "unparseable manifest_json"))
            continue
        if not manifest.get("identity"):
            invalid.append((handle, "no identity block (incomplete draft?)"))
            continue
        header = (
            f"# {args.env} — synced from portal proposal {p.get('id','?')} "
            f"(runner-minted; sponsor {p.get('sponsor','?')}).\n"
            f"# Written by ops/scripts/sync-onboarded-manifests.py — the portal DB is the\n"
            f"# record of runner mints; this file makes the declared roster agree with it.\n"
        )
        body = yaml.safe_dump(manifest, sort_keys=False, allow_unicode=True)
        if args.dry_run:
            written.append(handle + " (dry-run)")
            continue
        path.write_text(header + body)
        os.chmod(path, 0o600)
        if validate.exists():
            r = subprocess.run([sys.executable, str(validate), str(path)], capture_output=True, text=True)
            if r.returncode != 0:
                invalid.append((handle, r.stdout.strip().splitlines()[-1] if r.stdout else "validate failed"))
        written.append(handle)

    print(f"synced {len(written)} manifest(s) into {outdir}: {', '.join(written) or '-'}")
    if skipped:
        print(f"kept existing (not clobbered): {', '.join(skipped)}")
    if invalid:
        print("VALIDATION ISSUES (files written, fix before roster regen):")
        for h, why in invalid:
            print(f"  - {h}: {why}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
