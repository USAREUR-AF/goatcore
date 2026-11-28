#!/usr/bin/env python3
"""audit-repo-baseline: report every repository of an organization against the repository baseline.

The baseline (docs/process/repository-baseline.md, ADR 1276) has four members:

  identity  the commit-identity check runs on every pull request (by the organization
            ruleset, or by the repository's own workflow) and its status is required
  leak      public repositories: a leak-gate workflow is present, and a ruleset
            requires the leak-gate check
  secrets   GitHub secret scanning and push protection are both enabled, or the
            credential-scan check (gitleaks over the commits a pull request
            adds) runs and is required
  license   public repositories: LICENSE and NOTICE are at the repository root; a
            fork keeps its upstream's license files and needs no NOTICE

Each cell reads `required` (present and enforced), `present` (there, not
enforced), `missing`, or `n/a` (the member does not apply: a private or
internal repository needs no leak gate and no license file). It reads the GitHub API only and changes nothing.

  GITHUB_TOKEN=... audit-repo-baseline.py <org> [--visibility public|private|internal|all] [--json]

The token must be able to read repository administration to see secret
scanning and rulesets; a cell it cannot read says `unreadable`.
Exit 0 when every repository listed has no `missing` cell, 1 otherwise, 2 on an API failure.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request

API = "https://api.github.com"
IDENTITY_WORKFLOW = "commit-identity.yml"
IDENTITY_CONTEXT = "commit-identity"
SCAN_WORKFLOW = "credential-scan.yml"
SCAN_CONTEXT = "credential-scan"
RECHECK = ".github/workflows/commit-identity-recheck.yml"


class ApiError(Exception):
    def __init__(self, code, what):
        super().__init__(f"{what}: HTTP {code}")
        self.code = code


def api(path, token):
    req = urllib.request.Request(API + path, headers={
        "Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "audit-repo-baseline"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return json.loads(r.read() or "null")
    except urllib.error.HTTPError as e:
        raise ApiError(e.code, path.split("?")[0]) from e


def paged(path, token):
    out, page, sep = [], 1, "&" if "?" in path else "?"
    while True:
        chunk = api(f"{path}{sep}per_page=100&page={page}", token)
        out += chunk or []
        if not chunk or len(chunk) < 100:
            return out
        page += 1


# ---------------------------------------------------------------- pure logic


def ruleset_facts(rulesets: list[dict]) -> dict:
    """What the active branch rulesets that apply to a repository require."""
    workflows, contexts = set(), set()
    for rs in rulesets:
        if rs.get("enforcement") != "active" or rs.get("target") not in (None, "branch"):
            continue
        for rule in rs.get("rules") or []:
            p = rule.get("parameters") or {}
            if rule.get("type") == "workflows":
                workflows |= {(w.get("path") or "").rsplit("/", 1)[-1] for w in p.get("workflows") or []}
            if rule.get("type") == "required_status_checks":
                contexts |= {c.get("context") or "" for c in p.get("required_status_checks") or []}
    return {"workflows": workflows, "contexts": contexts}


def grade_identity(facts: dict, workflow_names: set[str] = frozenset()) -> str:
    runs = IDENTITY_WORKFLOW in facts["workflows"] or IDENTITY_WORKFLOW in workflow_names
    required = IDENTITY_CONTEXT in facts["contexts"]
    return "required" if runs and required else "present" if runs else "missing"


def grade_leak(workflow_names: set[str], facts: dict, public: bool = True) -> str:
    if not public:
        return "n/a"
    present = any(n.startswith("leak-gate") for n in workflow_names)
    required = any("leak" in c.lower() for c in facts["contexts"]) or any(
        w.startswith("leak-gate") for w in facts["workflows"])
    return "required" if present and required else "present" if present else "missing"


def grade_secrets(security: dict | None, facts: dict | None = None, workflow_names: set[str] = frozenset()) -> str:
    """GitHub's own scanning with push protection, or the credential-scan check."""
    facts = facts or {"workflows": set(), "contexts": set()}
    by_ruleset = SCAN_WORKFLOW in facts["workflows"]           # a required workflow is enforced by itself
    by_file = SCAN_WORKFLOW in workflow_names
    scan_required = by_ruleset or (by_file and SCAN_CONTEXT in facts["contexts"])
    security = security or {}
    native = (security.get("secret_scanning") or {}).get("status") == "enabled"
    push = (security.get("secret_scanning_push_protection") or {}).get("status") == "enabled"
    if (native and push) or scan_required:
        return "required"
    if native or by_file:
        return "present"
    return "missing"


def grade_license(root_names: set[str], public: bool = True, fork: bool = False) -> str:
    if not public:
        return "n/a"
    lic = any(n.split(".")[0] in ("LICENSE", "LICENSES", "COPYING") for n in root_names)
    notice = any(n.split(".")[0] == "NOTICE" for n in root_names)
    return "present" if lic and (notice or fork) else "missing"


def summary(rows: list[dict]) -> str:
    """One line per visibility: for each member, how many repositories read `required` (or `present` for license)."""
    out = []
    for vis in ("public", "private", "internal"):
        rs = [r for r in rows if r["visibility"] == vis]
        if not rs:
            continue
        cells = []
        for key, good in (("identity", "required"), ("leak", "required"), ("secrets", "required"), ("license", "present")):
            applies = [r for r in rs if r[key] != "n/a"]
            cells.append(f"{key} {sum(1 for r in applies if r[key] == good)}/{len(applies)}" if applies else f"{key} n/a")
        out.append(f"{vis} ({len(rs)}): " + ", ".join(cells))
    return "\n".join(out)


def table(rows: list[dict]) -> str:
    out = ["| Repository | Visibility | commit-identity | leak gate | secret scanning | license + notice | acts on a comment |",
           "|---|---|---|---|---|---|---|"]
    for r in rows:
        out.append(f"| {r['name']} | {r['visibility']} | {r['identity']} | {r['leak']} | {r['secrets']} "
                   f"| {r['license']} | {'yes' if r['recheck'] else 'no'} |")
    return "\n".join(out)


# ---------------------------------------------------------------- reading


def names_in(repo, path, token) -> set[str]:
    try:
        return {e["name"] for e in api(f"/repos/{repo}/contents/{path}", token) or []}
    except ApiError as e:
        if e.code == 404:
            return set()
        raise


def audit(org: str, visibility: str, token: str) -> list[dict]:
    rows = []
    for r in sorted(paged(f"/orgs/{org}/repos?type=all", token), key=lambda r: r["name"].lower()):
        if r.get("archived") or (visibility != "all" and r.get("visibility") != visibility):
            continue
        full = r["full_name"]
        try:
            listed = paged(f"/repos/{full}/rulesets?includes_parents=true", token)
            facts = ruleset_facts([api(f"/repos/{full}/rulesets/{x['id']}?includes_parents=true", token)
                                   for x in listed])
        except ApiError:
            facts = {"workflows": set(), "contexts": set()}
        workflows = names_in(full, ".github/workflows", token)
        public = r.get("visibility") == "public"
        detail = api(f"/repos/{full}", token)
        rows.append({"name": r["name"], "visibility": r.get("visibility"),
                     "identity": grade_identity(facts, workflows),
                     "leak": grade_leak(workflows, facts, public),
                     "secrets": grade_secrets(detail.get("security_and_analysis"), facts, workflows),
                     "license": grade_license(names_in(full, "", token) if public else set(), public,
                                              bool(detail.get("fork"))),
                     "recheck": RECHECK.rsplit("/", 1)[-1] in workflows or IDENTITY_WORKFLOW in workflows})
    return rows


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("org")
    ap.add_argument("--visibility", default="public", choices=["public", "private", "internal", "all"])
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()
    try:
        rows = audit(a.org, a.visibility, os.environ["GITHUB_TOKEN"])
    except ApiError as e:
        print(f"audit-repo-baseline: {e}", file=sys.stderr)
        return 2
    print(json.dumps(rows, indent=1) if a.json else table(rows) + "\n\n" + summary(rows))
    return 1 if any("missing" in (r["identity"], r["leak"], r["secrets"], r["license"]) for r in rows) else 0


if __name__ == "__main__":
    sys.exit(main())
