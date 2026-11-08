#!/usr/bin/env python3
"""contributor-trust: decide whether a pull request needs a recorded security review.

Runs from the base branch under pull_request_target (see
.github/workflows/contributor-trust.yml). It never checks out, builds or runs
the pull request's code: it reads the author and the list of changed paths
from the GitHub API and nothing else. ADR 1221.

The author is placed in one tier:

  maintainer  an owner, organisation member or repository collaborator
  automation  a bot this repository runs (Dependabot)
  known       registered in a community environment (a collaborator of the
              roster repository named by CONTRIBUTOR_ROSTER_REPO), or the
              author of at least one merged pull request in the organisation
  unknown     neither

A pull request needs a security review when its author is unknown, or when
its author is known and it touches a sensitive path (listed one glob per line
in sensitive-paths.txt beside this file). A review is recorded by an owner
(admin or maintain permission) applying the label `security-reviewed`; the
workflow removes the label whenever new commits are pushed, so the review
always covers the code that merges.

Exit 0: no review needed, or the review is recorded. Exit 1: a review is
needed and not recorded; the job summary says what to do. Exit 2: the check
could not decide (an API failure); it fails closed.
"""
from __future__ import annotations

import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

LABEL = "security-reviewed"
MAINTAINER_ASSOCIATIONS = {"OWNER", "MEMBER", "COLLABORATOR"}
AUTOMATION_LOGINS = {"dependabot[bot]"}
ATTESTING_PERMISSIONS = {"admin", "maintain"}
MAINTAINER_PERMISSIONS = {"admin", "maintain", "write"}
API = "https://api.github.com"


class ApiError(Exception):
    pass


# ---------------------------------------------------------------- pure logic


def glob_to_regex(pattern: str) -> re.Pattern[str]:
    """`**` matches across directories, `*` and `?` within one path segment.

    A pattern with no slash matches a file of that name in any directory,
    the way .gitignore does.
    """
    pattern = pattern.strip()
    if "/" not in pattern:
        pattern = "**/" + pattern
    out = []
    i = 0
    while i < len(pattern):
        if pattern.startswith("**/", i):
            out.append("(?:.*/)?")
            i += 3
        elif pattern.startswith("**", i):
            out.append(".*")
            i += 2
        elif pattern[i] == "*":
            out.append("[^/]*")
            i += 1
        elif pattern[i] == "?":
            out.append("[^/]")
            i += 1
        else:
            out.append(re.escape(pattern[i]))
            i += 1
    return re.compile("^" + "".join(out) + "$")


def load_patterns(text: str) -> list[str]:
    return [ln.strip() for ln in text.splitlines() if ln.strip() and not ln.lstrip().startswith("#")]


def sensitive_matches(paths: list[str], patterns: list[str]) -> list[str]:
    compiled = [glob_to_regex(p) for p in patterns]
    return [p for p in paths if any(rx.match(p) for rx in compiled)]


@dataclass
class Facts:
    author: str
    association: str
    registered: bool | None  # None: the roster could not be read (not configured)
    merged_prs: int
    changed: list[str]
    label_present: bool
    label_actor: str | None = None
    label_actor_permission: str | None = None
    repo_permission: str | None = None  # the author's own permission on this repository


@dataclass
class Verdict:
    tier: str
    needs_review: bool
    reasons: list[str] = field(default_factory=list)
    sensitive: list[str] = field(default_factory=list)
    passed: bool = False
    detail: str = ""


def tier_of(f: Facts) -> tuple[str, list[str]]:
    if f.author in AUTOMATION_LOGINS:
        return "automation", [f"{f.author} is automation this repository runs"]
    if f.association in MAINTAINER_ASSOCIATIONS:
        return "maintainer", [f"GitHub reports the author as {f.association}"]
    if f.repo_permission in MAINTAINER_PERMISSIONS:
        return "maintainer", [f"the author has {f.repo_permission} permission on this repository"]
    reasons = []
    if f.registered:
        reasons.append("registered in a community environment")
    if f.merged_prs > 0:
        reasons.append(f"{f.merged_prs} merged pull request(s) in this organisation")
    if reasons:
        return "known", reasons
    why = ["not registered in a community environment" if f.registered is False
           else "registration could not be checked (roster not configured)",
           "no merged pull request in this organisation"]
    return "unknown", why


def decide(f: Facts, patterns: list[str]) -> Verdict:
    tier, reasons = tier_of(f)
    sensitive = sensitive_matches(f.changed, patterns)
    if tier in ("maintainer", "automation"):
        needs = False
    elif tier == "known":
        needs = bool(sensitive)
    else:
        needs = True
    v = Verdict(tier=tier, needs_review=needs, reasons=reasons, sensitive=sensitive)
    if not needs:
        v.passed = True
        v.detail = "No security review needed."
        return v
    if not f.label_present:
        v.detail = (f"A security review is needed. An owner reviews the change and applies the "
                    f"`{LABEL}` label; this check then passes.")
        return v
    if f.label_actor_permission in ATTESTING_PERMISSIONS:
        v.passed = True
        v.detail = f"Security review recorded by {f.label_actor} (`{LABEL}`)."
        return v
    v.detail = (f"`{LABEL}` was applied by {f.label_actor or 'an unknown account'}, whose permission "
                f"is {f.label_actor_permission or 'unknown'}; only an owner (admin or maintain) can record "
                f"the review.")
    return v


def render(v: Verdict, f: Facts) -> str:
    lines = [
        "## contributor-trust",
        "",
        f"**{'PASS' if v.passed else 'REVIEW NEEDED'}** · author `{f.author}` · tier **{v.tier}**",
        "",
        "Why this tier: " + "; ".join(v.reasons) + ".",
        "",
    ]
    if v.sensitive:
        lines.append(f"Sensitive paths changed ({len(v.sensitive)}):")
        lines.append("")
        lines += [f"- `{p}`" for p in v.sensitive[:50]]
        if len(v.sensitive) > 50:
            lines.append(f"- … and {len(v.sensitive) - 50} more")
        lines.append("")
    lines.append(v.detail)
    if v.needs_review and not v.passed:
        lines += [
            "",
            "The review, before applying the label (CONTRIBUTING.md, \"Outside contributions\"):",
            "the change does only what its title says; no new dependency, network call, binary or",
            "obfuscated code; every workflow change read line by line; tests exercise the change;",
            "auth, access and crypto changes get a second, independent reviewer.",
        ]
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------- GitHub API


def api(path: str, token: str | None, *, method: str = "GET", ok: tuple[int, ...] = (200,)) -> tuple[int, object]:
    req = urllib.request.Request(API + path, method=method)
    req.add_header("Accept", "application/vnd.github+json")
    req.add_header("X-GitHub-Api-Version", "2022-11-28")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            body = r.read()
            return r.status, (json.loads(body) if body else None)
    except urllib.error.HTTPError as e:
        if e.code in ok:
            return e.code, None
        raise ApiError(f"{method} {path}: HTTP {e.code}") from None
    except urllib.error.URLError as e:
        raise ApiError(f"{method} {path}: {e.reason}") from None


def changed_paths(repo: str, pr: int, token: str) -> list[str]:
    paths: list[str] = []
    for page in range(1, 31):  # the API returns at most 3,000 files
        _, items = api(f"/repos/{repo}/pulls/{pr}/files?per_page=100&page={page}", token)
        if not items:
            break
        for it in items:
            paths.append(it["filename"])
            if it.get("previous_filename"):
                paths.append(it["previous_filename"])
        if len(items) < 100:
            break
    return paths


def is_registered(roster_repo: str | None, roster_token: str | None, author: str) -> bool | None:
    if not roster_repo or not roster_token:
        return None
    status, _ = api(f"/repos/{roster_repo}/collaborators/{urllib.parse.quote(author)}",
                    roster_token, ok=(204, 404))
    return status == 204


def merged_pr_count(org: str, author: str, token: str) -> int:
    q = urllib.parse.quote(f"is:pr is:merged author:{author} org:{org}")
    # 422: GitHub will not search this account (it does not exist, or is
    # hidden from search). Nothing it wrote can have merged, so count none.
    status, body = api(f"/search/issues?q={q}&per_page=1", token, ok=(200, 422))
    return 0 if status == 422 else int(body.get("total_count", 0))


def label_state(repo: str, pr: int, token: str) -> tuple[bool, str | None, str | None]:
    _, issue = api(f"/repos/{repo}/issues/{pr}", token)
    if not any(lb["name"] == LABEL for lb in issue.get("labels", [])):
        return False, None, None
    actor = None
    for page in range(1, 11):
        _, events = api(f"/repos/{repo}/issues/{pr}/events?per_page=100&page={page}", token)
        if not events:
            break
        for ev in events:
            if ev.get("event") == "labeled" and ev.get("label", {}).get("name") == LABEL:
                actor = (ev.get("actor") or {}).get("login")
        if len(events) < 100:
            break
    perm = None
    if actor:
        _, body = api(f"/repos/{repo}/collaborators/{urllib.parse.quote(actor)}/permission", token)
        perm = body.get("role_name") or body.get("permission")
    return True, actor, perm


def repo_permission(repo: str, login: str, token: str) -> str | None:
    """The author's permission here. An owner whose organisation membership is
    private can reach the check reported as CONTRIBUTOR; their permission still
    says what they are. Not fatal: on an error the other tiers decide."""
    try:
        _, body = api(f"/repos/{repo}/collaborators/{urllib.parse.quote(login)}/permission", token)
    except ApiError:
        return None
    return body.get("role_name") or body.get("permission")


def main() -> int:
    env = os.environ
    repo = env["GITHUB_REPOSITORY"]
    pr = int(env["PR_NUMBER"])
    author = env["PR_AUTHOR"]
    association = env.get("PR_AUTHOR_ASSOCIATION", "NONE")
    token = env["GITHUB_TOKEN"]
    roster_repo = env.get("CONTRIBUTOR_ROSTER_REPO") or None
    roster_token = env.get("CONTRIBUTOR_ROSTER_TOKEN") or None
    patterns_file = Path(env.get("SENSITIVE_PATHS", Path(__file__).with_name("sensitive-paths.txt")))
    summary = env.get("GITHUB_STEP_SUMMARY")

    try:
        patterns = load_patterns(patterns_file.read_text()) if patterns_file.exists() else []
        perm = None
        if author not in AUTOMATION_LOGINS and association not in MAINTAINER_ASSOCIATIONS:
            perm = repo_permission(repo, author, token)
        quick = (author in AUTOMATION_LOGINS or association in MAINTAINER_ASSOCIATIONS
                 or perm in MAINTAINER_PERMISSIONS)
        facts = Facts(
            author=author,
            association=association,
            repo_permission=perm,
            registered=None if quick else is_registered(roster_repo, roster_token, author),
            merged_prs=0 if quick else merged_pr_count(repo.split("/")[0], author, token),
            changed=changed_paths(repo, pr, token),
            label_present=False,
        )
        verdict = decide(facts, patterns)
        if verdict.needs_review:
            facts.label_present, facts.label_actor, facts.label_actor_permission = label_state(repo, pr, token)
            verdict = decide(facts, patterns)
    except ApiError as e:
        msg = f"## contributor-trust\n\n**COULD NOT DECIDE** — {e}. The check fails closed; re-run it.\n"
        print(msg)
        if summary:
            Path(summary).write_text(msg)
        return 2

    text = render(verdict, facts)
    print(text)
    if summary:
        with open(summary, "a") as fh:
            fh.write(text)
    if not verdict.passed:
        print(f"::error title=contributor-trust::{verdict.detail}")
    return 0 if verdict.passed else 1


if __name__ == "__main__":
    sys.exit(main())
