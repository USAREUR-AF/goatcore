#!/usr/bin/env python3
"""repo-policy-token-coverage — can this GitHub token grant every repo the policy names?

The identity console grants repository access with a fine-grained token, which
sees only the repositories chosen when it was set up. A repository added to the
repo-access policy later answers 404 to that token, and every grant to it fails
for everyone onboarded after (#2107). The id-admin-console role
runs this before it deploys, with the deploying admin's token, and refuses the
deploy when any policy repository is not covered.

    repo-policy-token-coverage.py --policy <repo-access.yaml>   token from $GITHUB_ONBOARD_TOKEN

One read-only GET /repos/{owner}/{repo} per repository. Exit 0: every repository
answers 200. Exit 1: at least one does not; each is named with its HTTP status.
Exit 2: usage, no token, or an unreadable policy. The token is never printed.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request

import yaml


def policy_repos(text: str) -> list[str]:
    """Every owner/repo the policy grants, in first-seen order."""
    doc = yaml.safe_load(text) or {}
    owner = str(doc.get("default_owner") or "").strip()
    seen: list[str] = []
    for rule in doc.get("rules") or []:
        for g in rule.get("grant") or rule.get("repos") or []:
            name = g if isinstance(g, str) else (g.get("repo") or g.get("name") or "")
            name = str(name).strip()
            if not name:
                continue
            full = name if "/" in name else f"{owner}/{name}"
            if full not in seen:
                seen.append(full)
    return seen


def status_of(api: str, token: str, repo: str) -> int:
    req = urllib.request.Request(
        f"{api.rstrip('/')}/repos/{repo}",
        headers={"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            return resp.status
    except urllib.error.HTTPError as e:
        return e.code
    except (urllib.error.URLError, TimeoutError):
        return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--policy", required=True, help="the repo-access policy YAML, or - for stdin")
    ap.add_argument("--api", default="https://api.github.com")
    ap.add_argument("--json", action="store_true", help="print one JSON object instead of lines")
    a = ap.parse_args(argv)

    token = os.environ.get("GITHUB_ONBOARD_TOKEN", "").strip()
    if not token:
        print("repo-policy-token-coverage: GITHUB_ONBOARD_TOKEN is empty", file=sys.stderr)
        return 2
    try:
        if a.policy == "-":
            text = sys.stdin.read()
        else:
            with open(a.policy, encoding="utf-8") as f:
                text = f.read()
        repos = policy_repos(text)
    except (OSError, yaml.YAMLError) as e:
        print(f"repo-policy-token-coverage: cannot read {a.policy}: {e}", file=sys.stderr)
        return 2

    results = {r: status_of(a.api, token, r) for r in repos}
    missing = {r: s for r, s in results.items() if s != 200}
    if a.json:
        print(json.dumps({"repos": len(repos), "covered": len(repos) - len(missing),
                          "not_covered": [{"repo": r, "status": s} for r, s in missing.items()]}))
    else:
        for r, s in results.items():
            print(f"{'ok  ' if s == 200 else 'MISS'} {s or 'unreachable'} {r}")
        print(f"{len(repos) - len(missing)} of {len(repos)} policy repositories covered by the token")
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
