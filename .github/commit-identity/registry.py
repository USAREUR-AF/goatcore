#!/usr/bin/env python3
"""commit-identity registry: an index of where each person's `/email-ok` is.

Runs on a timer in the organization's `.github` repository (see
registry-workflow.yml beside this file) with that repository's own token. It
reads the pull-request comments of every public repository of the
organization, and keeps one comment on one issue of the `.github` repository
(label `commit-identity-registry`) holding

    {"scanned_until": "<time>", "people": {"<login>": "<API URL of their comment>"}}

The index is a convenience for check.py in other repositories, which cannot
search across the organization reliably. It is a list of pointers and holds
no mailbox. check.py trusts none of it: it follows the pointer and reads the
comment. A pointer whose comment was deleted, or no longer says the phrase,
is dropped here on the next run. ADR 1276.

Exit 0 on success; 2 when the API could not be read (nothing is written).
"""
from __future__ import annotations

import datetime as dt
import importlib.util
import json
import os
import sys
from pathlib import Path

_spec = importlib.util.spec_from_file_location("commit_identity_check", Path(__file__).with_name("check.py"))
ck = importlib.util.module_from_spec(_spec)
sys.modules["commit_identity_check"] = ck
_spec.loader.exec_module(ck)

TITLE = "commit-identity: index of acknowledgments (kept by a workflow)"
INTRO = ("This issue is kept by a workflow. It lists where each person's `/email-ok` comment is, so the "
         "`commit-identity` check in every repository can find it. It holds no address. To take your "
         "acknowledgment back, delete your `/email-ok` comment; the entry goes on the next run.")
OVERLAP = dt.timedelta(minutes=10)
EPOCH = "2026-11-01T00:00:00Z"


def merge(people: dict, comments: list[dict]) -> dict:
    """Add the first standing `/email-ok` of each person not already listed."""
    out = dict(people)
    have = {k.casefold() for k in out}
    for c in sorted(comments, key=lambda c: c.get("created_at") or ""):
        user = c.get("user") or {}
        login = user.get("login") or ""
        if (user.get("type") == "User" and login and login.casefold() not in have
                and "/pull/" in (c.get("html_url") or "") and ck.is_phrase(c.get("body") or "", ck.ACK)):
            out[login] = c["url"]
            have.add(login.casefold())
    return out


def render(scanned_until: str, people: dict) -> str:
    data = {"scanned_until": scanned_until, "people": dict(sorted(people.items(), key=lambda kv: kv[0].casefold()))}
    return f"{INTRO}\n\n{ck.REGISTRY_OPEN}\n{json.dumps(data, indent=1)}\n{ck.REGISTRY_CLOSE}\n\n{len(people)} listed."


def main() -> int:
    token = os.environ["GITHUB_TOKEN"]
    org = os.environ["GITHUB_REPOSITORY"].split("/")[0]
    home = f"{org}/.github"
    try:
        issues = ck.api(f"/repos/{home}/issues?labels={ck.REGISTRY_LABEL}&state=all&per_page=5", token) or []
        issue = issues[0] if issues else ck.api(f"/repos/{home}/issues", token, "POST", {
            "title": TITLE, "body": INTRO, "labels": [ck.REGISTRY_LABEL]})
        mine, data = None, {}
        for c in ck.paged(issue["comments_url"], token, limit=300):
            if ck.REGISTRY_OPEN in (c.get("body") or ""):
                mine, data = c, ck.parse_registry(c["body"])
                break
        people = dict(data.get("people") or {})
        since = data.get("scanned_until") or EPOCH
        try:
            since = (dt.datetime.fromisoformat(since.replace("Z", "+00:00")) - OVERLAP).strftime("%Y-%m-%dT%H:%M:%SZ")
        except ValueError:
            since = EPOCH
        now = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

        # Drop pointers whose comment is gone or changed.
        for login, url in list(people.items()):
            try:
                ok = ck.comment_is_own_ack(ck.api(url, token), login)
            except ck.ApiError as e:
                if "HTTP 404" not in str(e) and "HTTP 410" not in str(e):
                    raise
                ok = False
            if not ok:
                del people[login]

        found: list[dict] = []
        for r in ck.paged(f"/orgs/{org}/repos?type=public", token, limit=1000):
            if r.get("archived"):
                continue
            found += ck.paged(f"/repos/{r['full_name']}/issues/comments?since={since}&sort=created&direction=asc",
                              token, limit=5000)
        people = merge(people, found)

        body = render(now, people)
        if mine:
            ck.api(f"/repos/{home}/issues/comments/{mine['id']}", token, "PATCH", {"body": body})
        else:
            ck.api(f"/repos/{home}/issues/{issue['number']}/comments", token, "POST", {"body": body})
        print(f"commit-identity registry: {len(people)} listed; comments read since {since}")
        return 0
    except ck.ApiError as e:
        print(f"::error::commit-identity registry could not be updated: {e}")
        return 2


if __name__ == "__main__":
    sys.exit(main())
