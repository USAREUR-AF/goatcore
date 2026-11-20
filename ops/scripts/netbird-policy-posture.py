#!/usr/bin/env python3
"""netbird-policy-posture.py — does a goatnet's NetBird account carry only the
declared ACL, and never an All->All? (ADR 1185)

Two modes:

  --declared SITE
      Offline. The site has an entry in ops/ansible/netbird_acl_matrix.yml and
      declares cutover.default_policy_action: delete. goat-standup.sh runs this
      at phase 0 for a founder, so a goatnet with no declared ACL is refused
      before anything is built.

  SITE --url https://<public_mgmt_fqdn> [--token-env VAR]
      Live, read-only (two GETs). FAILS when the account holds:
        - an enabled policy with an enabled accept rule whose sources or
          destinations include the `All` group (a Default All->All, whatever
          it is called or however it got there — F-551 arrived by raw SQL
          with no audit event);
        - an enabled policy whose name is not a rule name the matrix declares
          for SITE or for an exercise scope hosted on SITE. The declared ACL
          is the whole permission surface; anything else is drift.
      The token env var defaults to <SITE>_NB_API_TOKEN.

Exit 0 = clean, 1 = violation, 2 = could not decide (bad input, API error).
Reads the matrix and taxonomy from the checkout it runs in.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.request
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parents[2]
MATRIX = REPO / "ops/ansible/netbird_acl_matrix.yml"
TAXONOMY = REPO / "ops/ansible/netbird_acl_taxonomy.yml"


def load():
    m = yaml.safe_load(MATRIX.read_text())
    t = yaml.safe_load(TAXONOMY.read_text())
    return m, t


def declared_names(site: str, m: dict, t: dict) -> set[str]:
    names = {r["name"] for r in m["netbird_acl_matrix"][site].get("rules", [])}
    ex_rules = [r["name"] for r in m.get("netbird_exercise_rules_template", {}).get("rules", [])]
    gear_rules = [r["name"] for r in m.get("netbird_exercise_gear_rules_template", {}).get("rules", [])]
    for ex in (t.get("netbird_exercises") or {}).values():
        if ex.get("host_site") != site:
            continue
        rules = ex_rules + (gear_rules if ex.get("gear") else [])
        names |= {f"{ex['site_prefix']} · {n}" for n in rules}
    return names


def check_declared(site: str, m: dict) -> int:
    entry = (m.get("netbird_acl_matrix") or {}).get(site)
    if entry is None:
        print(f"FAIL: '{site}' has no entry in {MATRIX.relative_to(REPO)}. A goatnet with no "
              "declared ACL has no reach at all once NetBird's Default is off (ADR 1185): "
              "declare its rules in ops/warrant/rule-intents/ and regenerate.")
        return 1
    action = (entry.get("cutover") or {}).get("default_policy_action")
    if action != "delete":
        print(f"FAIL: '{site}' declares cutover.default_policy_action: {action!r}; ADR 1185 "
              "allows only 'delete' — no goatnet keeps a Default All->All.")
        return 1
    print(f"OK: '{site}' declares its ACL ({len(entry.get('rules', []))} rules) and "
          "default_policy_action: delete.")
    return 0


def _tls_context(site: str):
    """Verify against the goatnet's own CA when its root certificate is in the
    repository; a goatnet behind a public CA verifies with the system store.
    Verification is never turned off."""
    import ssl
    ca = REPO / "ops/enrollment/ca" / site / "root-cert.pem"
    # A goatnet holding its own CA can still serve its mgmt API under a public ACME
    # certificate (internet profile): NETBIRD_MGMT_CA=public keeps the system store.
    if ca.is_file() and os.environ.get("NETBIRD_MGMT_CA", "") != "public":
        return ssl.create_default_context(cafile=str(ca))
    return ssl.create_default_context()


def get(url: str, token: str, site: str = ""):
    req = urllib.request.Request(url, headers={"Authorization": f"Token {token}",
                                               "Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=30, context=_tls_context(site)) as r:
        return json.load(r)


def check_live(site: str, base: str, token_env: str, m: dict, t: dict) -> int:
    if site not in (m.get("netbird_acl_matrix") or {}):
        print(f"UNDECIDED: '{site}' has no matrix entry to compare against.")
        return 2
    token = os.environ.get(token_env, "")
    if not token:
        print(f"UNDECIDED: ${token_env} is empty.")
        return 2
    base = base.rstrip("/")
    try:
        groups = get(f"{base}/api/groups", token, site)
        policies = get(f"{base}/api/policies", token, site)
    except Exception as e:  # noqa: BLE001 — any read failure is undecided, not clean
        print(f"UNDECIDED: reading {base}: {e}")
        return 2
    gname = {g["id"]: g["name"] for g in groups}

    def names(xs):
        return [x["name"] if isinstance(x, dict) else gname.get(x, x) for x in (xs or [])]

    declared = declared_names(site, m, t)
    all_to_all, undeclared = [], []
    for p in policies:
        if not p.get("enabled"):
            continue
        for r in p.get("rules", []):
            if r.get("enabled") and r.get("action") == "accept" and (
                    "All" in names(r.get("sources")) or "All" in names(r.get("destinations"))):
                all_to_all.append(p)
                break
        if p["name"] not in declared:
            undeclared.append(p)

    print(f"{site}: {len(policies)} policies at {base} "
          f"({sum(1 for p in policies if p.get('enabled'))} enabled), "
          f"{len(declared)} declared rule names")
    for p in all_to_all:
        print(f"  FAIL All-group rule: {p['id']} '{p['name']}' — {(p.get('description') or '')[:100]}")
    for p in undeclared:
        if p in all_to_all:
            continue
        print(f"  FAIL undeclared:     {p['id']} '{p['name']}'")
    if all_to_all or undeclared:
        print("Remediate: disable (do not delete) an All-group policy, then find what wrote it; "
              "declare or remove an undeclared one through ops/warrant/ (ADR 1185).")
        return 1
    print("OK: no All-group rule; every enabled policy is declared.")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("site")
    ap.add_argument("--declared", action="store_true", help="offline declaration check only")
    ap.add_argument("--url", help="mgmt API base, https://<public_mgmt_fqdn>")
    ap.add_argument("--token-env", help="env var holding the NetBird PAT "
                    "(default <SITE>_NB_API_TOKEN)")
    a = ap.parse_args()
    m, t = load()
    if a.declared:
        return check_declared(a.site, m)
    if not a.url:
        ap.error("--url is required for the live check")
    env = a.token_env or a.site.upper().replace("-", "_") + "_NB_API_TOKEN"
    return check_live(a.site, a.url, env, m, t)


if __name__ == "__main__":
    sys.exit(main())
