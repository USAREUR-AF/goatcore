#!/usr/bin/env python3
"""commit-identity: refuse a pull request once when it would publish a personal mailbox.

Runs from the default branch under pull_request_target, issue_comment and
merge_group (see workflow.yml beside this file). It never checks out, builds
or runs the pull request's code: it reads commit metadata and comments from
the GitHub API and nothing else. ADR 1276.

What it reads, for every commit the pull request adds: the author address,
the committer address, and every `Signed-off-by:` and `Co-authored-by:`
trailer. An address is acceptable when it is a GitHub no-reply address, an
address GitHub itself writes, an address under a domain reserved for
examples, or a line of allowed-addresses.txt beside this file. Every other
address is a personal mailbox.

A personal mailbox is cleared in one of three ways:

  fixed         the commit no longer carries it (the author rewrote the branch)
  acknowledged  the person it belongs to has commented `/email-ok` on a pull
                request in this organization, once; it holds for every later
                pull request of theirs in every repository, and deleting the
                comment takes it back
  overridden    an owner of this repository (admin or maintain) has commented
                `/email-ok-maintainer` on this pull request; it holds for
                this pull request only

Whose mailbox is it? The author address belongs to the account GitHub maps
the commit's author to, and the committer address to the committer's
account. A trailer belongs to the commit's author when its address is the
author's (or to the committer when it is the committer's), or when it is a
`Signed-off-by:` whose name is the author's name or login. Any other trailer
has no known owner: nobody can acknowledge it for someone else, so it is
fixed or overridden.

The verdict is a commit status named `commit-identity` on the pull request's
head commit, so a comment can turn it green without a new push. The job
itself exits 0 whatever the verdict; it exits 2, and posts the status
`error`, only when it could not decide (an API failure).

No mailbox is stored anywhere and none is printed in full: findings show a
masked form (`d***@g***.com`).
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

API = "https://api.github.com"
CONTEXT = "commit-identity"
MARK = "<!-- commit-identity -->"
ACK = "/email-ok"
OVERRIDE = "/email-ok-maintainer"
OVERRIDE_PERMISSIONS = {"admin", "maintain"}
GITHUB_OWN_ADDRESSES = {"noreply@github.com", "support@github.com"}  # oss-allow: the two addresses GitHub writes itself
RESERVED_TLDS = {"invalid", "example", "test", "localhost"}
RESERVED_DOMAINS = {"example.com", "example.org", "example.net"}
TRAILER = re.compile(r"^(signed-off-by|co-authored-by):\s*(.*?)\s*<([^<>\s]+)>\s*$", re.I)
GUIDE_DEFAULT = "https://github.com/USAREUR-AF/goatcore/blob/main/docs/process/commit-identity.md"


class ApiError(Exception):
    pass


# ---------------------------------------------------------------- pure logic


def load_allowed(text: str) -> set[str]:
    """One entry per line: a whole address, or `@domain` for every address under it."""
    return {ln.strip().lower() for ln in text.splitlines() if ln.strip() and not ln.lstrip().startswith("#")}


def is_acceptable(address: str, allowed: set[str] = frozenset()) -> bool:
    """True when publishing this address tells nobody a person's own mailbox."""
    a = address.strip().lower()
    if a.count("@") != 1:
        return True  # not a mailbox at all
    local, domain = a.split("@")
    if not local or not domain:
        return True
    if domain == "users.noreply.github.com":
        return True
    if a in GITHUB_OWN_ADDRESSES:
        return True
    labels = domain.split(".")
    if labels[-1] in RESERVED_TLDS:
        return True
    if any(domain == d or domain.endswith("." + d) for d in RESERVED_DOMAINS):
        return True
    if a in allowed or ("@" + domain) in allowed:
        return True
    return False


def mask(address: str) -> str:
    """`d***@g***.com`: enough to recognise your own, not enough to read another's."""
    a = address.strip()
    if a.count("@") != 1:
        return "***"
    local, domain = a.split("@")
    labels = domain.split(".")
    tld = labels[-1] if len(labels) > 1 else ""
    return f"{local[:1]}***@{domain[:1]}***" + (f".{tld}" if tld else "")


def trailers(message: str) -> list[tuple[str, str, str]]:
    """(kind, name, address) for every sign-off and co-author line in a commit message."""
    out = []
    for line in message.splitlines():
        m = TRAILER.match(line.strip())
        if m:
            out.append((m.group(1).lower(), m.group(2), m.group(3)))
    return out


@dataclass
class Commit:
    sha: str
    author_email: str
    author_name: str
    author_login: str | None
    committer_email: str
    committer_name: str
    committer_login: str | None
    message: str


@dataclass
class Finding:
    sha: str
    where: str          # "author", "committer", "Signed-off-by", "Co-authored-by"
    address: str        # never printed; use mask()
    owner: str | None   # the login that may acknowledge it, or None

    @property
    def masked(self) -> str:
        return mask(self.address)


def _same(a: str | None, b: str | None) -> bool:
    return bool(a) and bool(b) and a.strip().casefold() == b.strip().casefold()


def findings_of(commits: list[Commit], allowed: set[str] = frozenset()) -> list[Finding]:
    out: list[Finding] = []
    for c in commits:
        if not is_acceptable(c.author_email, allowed):
            out.append(Finding(c.sha, "author", c.author_email, c.author_login))
        # The same address in both fields is one finding, the author's.
        if not is_acceptable(c.committer_email, allowed) and not _same(c.committer_email, c.author_email):
            out.append(Finding(c.sha, "committer", c.committer_email, c.committer_login))
        seen: set[tuple[str, str]] = set()
        for kind, name, address in trailers(c.message):
            if is_acceptable(address, allowed):
                continue
            label = "Signed-off-by" if kind == "signed-off-by" else "Co-authored-by"
            if (label, address.lower()) in seen:
                continue
            seen.add((label, address.lower()))
            owner = None
            if _same(address, c.author_email):
                owner = c.author_login
            elif _same(address, c.committer_email):
                owner = c.committer_login
            elif kind == "signed-off-by" and c.author_login and (
                    _same(name, c.author_login) or _same(name, c.author_name)):
                owner = c.author_login
            out.append(Finding(c.sha, label, address, owner))
    return out


def is_phrase(body: str, phrase: str) -> bool:
    """The comment's first non-empty line is exactly the phrase."""
    for line in (body or "").splitlines():
        if line.strip():
            return line.strip() == phrase
    return False


@dataclass
class Verdict:
    passed: bool
    open: list[Finding] = field(default_factory=list)       # still block
    cleared: list[Finding] = field(default_factory=list)    # acknowledged or overridden
    overridden: bool = False


def decide(findings: list[Finding], acknowledged: set[str], overridden: bool) -> Verdict:
    """acknowledged: logins (casefolded) whose own `/email-ok` stands."""
    if overridden:
        return Verdict(True, [], list(findings), True)
    still, cleared = [], []
    for f in findings:
        (cleared if f.owner and f.owner.casefold() in acknowledged else still).append(f)
    return Verdict(not still, still, cleared, False)


def noreply_address(user_id: int | str | None, login: str) -> str:
    return f"{user_id}+{login}@users.noreply.github.com" if user_id else f"{login}@users.noreply.github.com"


def comment_body(v: Verdict, *, pr_author: str, pr_author_id, base_ref: str, guide: str) -> str:
    if v.passed:
        how = ("An owner of this repository accepted the addresses for this pull request."
               if v.overridden else
               "Nothing in this pull request's commits needs an answer now.")
        return f"{MARK}\n**commit-identity: clear.** {how}"
    lines = [MARK,
             "**commit-identity: this pull request would publish a personal mailbox.**",
             "",
             "Commit history is public and permanent. These commits carry an address that is not a "
             "GitHub no-reply address (shown masked):",
             "",
             "| Commit | Where | Address | Who can accept it |",
             "|---|---|---|---|"]
    for f in v.open:
        who = f"@{f.owner}" if f.owner else "nobody on their behalf: fix it, or an owner overrides"
        lines.append(f"| `{f.sha[:10]}` | {f.where} | `{f.masked}` | {who} |")
    owners = sorted({f.owner for f in v.open if f.owner}, key=str.casefold)
    lines += ["",
              "This check stops a pull request **once**. Choose one:",
              "",
              f"**1. Keep the address out.** Use your no-reply address and rewrite this branch "
              f"([how, with the GitHub settings that prevent it next time]({guide})):",
              "",
              "```sh",
              f'git config user.email "{noreply_address(pr_author_id, pr_author)}"',
              "git rebase --exec 'git log -1 --format=%B | grep -v \"^Signed-off-by:\" "
              f"| git commit --amend --reset-author -S -s -F -' origin/{base_ref}",
              "git push --force-with-lease",
              "```",
              "",
              "The rebase line re-makes each commit as you, signed, with a fresh sign-off; use it only "
              "on a branch whose commits are all yours.",
              "",
              f"**2. Accept it.** If you know the address is public in this history and you do not mind, "
              f"comment exactly `{ACK}` on this pull request. It is remembered for your own address on "
              "every later pull request in every repository of this organization; delete the comment "
              "to take it back."]
    if owners:
        lines.append("")
        lines.append("Only the person an address belongs to can accept it: "
                     + ", ".join(f"@{o}" for o in owners) + ".")
    if any(f.owner is None for f in v.open):
        lines += ["",
                  "An address with no known owner (a co-author line, or a commit GitHub maps to no "
                  f"account) cannot be accepted by someone else. Fix it, or an owner of this repository "
                  f"comments `{OVERRIDE}` for this pull request alone."]
    return "\n".join(lines)


def status_of(v: Verdict) -> tuple[str, str]:
    if v.passed:
        if v.overridden:
            return "success", "an owner accepted the addresses for this pull request"
        if v.cleared:
            return "success", "personal mailbox accepted by its owner"
        return "success", "no personal mailbox in these commits"
    n = len(v.open)
    return "failure", f"{n} personal mailbox entr{'y' if n == 1 else 'ies'}: fix, or comment {ACK} (see the comment)"


# ---------------------------------------------------------------- GitHub API


def api(path: str, token: str, method: str = "GET", body: dict | None = None):
    url = path if path.startswith("http") else API + path
    req = urllib.request.Request(url, method=method,
                                 data=json.dumps(body).encode() if body is not None else None,
                                 headers={"Authorization": f"Bearer {token}",
                                          "Accept": "application/vnd.github+json",
                                          "X-GitHub-Api-Version": "2022-11-28",
                                          "User-Agent": "commit-identity"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read()
            return json.loads(raw) if raw else None
    except urllib.error.HTTPError as e:
        raise ApiError(f"{method} {url.split('?')[0]}: HTTP {e.code}") from e
    except (urllib.error.URLError, TimeoutError) as e:
        raise ApiError(f"{method} {url.split('?')[0]}: {e}") from e


def paged(path: str, token: str, limit: int = 1000) -> list:
    out, page = [], 1
    sep = "&" if "?" in path else "?"
    while len(out) < limit:
        chunk = api(f"{path}{sep}per_page=100&page={page}", token)
        if not chunk:
            break
        out.extend(chunk)
        if len(chunk) < 100:
            break
        page += 1
    return out


def read_commits(repo: str, pr: int, token: str) -> list[Commit]:
    out = []
    for c in paged(f"/repos/{repo}/pulls/{pr}/commits", token, limit=250):
        g = c.get("commit") or {}
        ga, gc = g.get("author") or {}, g.get("committer") or {}
        out.append(Commit(sha=c["sha"],
                          author_email=ga.get("email") or "", author_name=ga.get("name") or "",
                          author_login=(c.get("author") or {}).get("login"),
                          committer_email=gc.get("email") or "", committer_name=gc.get("name") or "",
                          committer_login=(c.get("committer") or {}).get("login"),
                          message=g.get("message") or ""))
    return out


def own_ack_in(comments: list[dict], login: str) -> bool:
    return any((c.get("user") or {}).get("type") == "User"
               and _same((c.get("user") or {}).get("login"), login)
               and is_phrase(c.get("body") or "", ACK) for c in comments)


def acknowledged_elsewhere(org: str, login: str, token: str) -> bool:
    """Has this person an `/email-ok` of their own on any pull request in the organization?

    The search finds candidates; each is confirmed by reading the pull
    request's comments, so a comment that only mentions the phrase, or one
    that was deleted since, does not count.
    """
    q = urllib.parse.quote(f'"{ACK}" org:{org} commenter:{login} is:pr in:comments')
    found = api(f"/search/issues?q={q}&per_page=20&sort=updated&order=desc", token)
    for item in (found or {}).get("items", []):
        url = item.get("comments_url")
        if url and own_ack_in(paged(url, token, limit=500), login):
            return True
    return False


def main() -> int:
    token = os.environ["GITHUB_TOKEN"]
    repo = os.environ["GITHUB_REPOSITORY"]
    org = repo.split("/")[0]
    guide = os.environ.get("GUIDE_URL") or GUIDE_DEFAULT
    summary = Path(os.environ.get("GITHUB_STEP_SUMMARY", os.devnull))

    if os.environ.get("EVENT_NAME") == "merge_group":
        # The queue re-tests commits whose pull request this check already passed.
        sha = os.environ["MERGE_GROUP_SHA"]
        api(f"/repos/{repo}/statuses/{sha}", token, "POST",
            {"state": "success", "context": CONTEXT, "description": "decided on the pull request"})
        return 0

    pr_number = int(os.environ["PR_NUMBER"])
    head_sha = None
    try:
        pr = api(f"/repos/{repo}/pulls/{pr_number}", token)
        head_sha = pr["head"]["sha"]
        author = pr["user"]["login"]
        allowed_file = Path(__file__).with_name("allowed-addresses.txt")
        allowed = load_allowed(allowed_file.read_text()) if allowed_file.exists() else set()

        if pr["user"].get("type") == "Bot":
            findings: list[Finding] = []
        else:
            findings = findings_of(read_commits(repo, pr_number, token), allowed)

        comments = paged(f"/repos/{repo}/issues/{pr_number}/comments", token, limit=1000)
        overridden = False
        if findings:
            for c in comments:
                u = c.get("user") or {}
                if u.get("type") == "User" and is_phrase(c.get("body") or "", OVERRIDE):
                    perm = api(f"/repos/{repo}/collaborators/{urllib.parse.quote(u['login'])}/permission", token)
                    role = (perm or {}).get("role_name") or (perm or {}).get("permission")
                    if role in OVERRIDE_PERMISSIONS:
                        overridden = True
                        break
        acknowledged: set[str] = set()
        if findings and not overridden:
            for login in sorted({f.owner for f in findings if f.owner}):
                if own_ack_in(comments, login):
                    acknowledged.add(login.casefold())
                    continue
                try:
                    if acknowledged_elsewhere(org, login, token):
                        acknowledged.add(login.casefold())
                except ApiError as e:
                    # The search is a convenience across repositories; without it the
                    # person is asked again here, which is the safe direction.
                    print(f"::warning::could not look for an earlier {ACK} by {login}: {e}")

        v = decide(findings, acknowledged, overridden)
        state, description = status_of(v)
        api(f"/repos/{repo}/statuses/{head_sha}", token, "POST",
            {"state": state, "context": CONTEXT, "description": description[:140]})

        mine = next((c for c in comments if (c.get("body") or "").startswith(MARK)), None)
        if not v.passed or mine:
            body = comment_body(v, pr_author=author, pr_author_id=pr["user"].get("id"),
                                base_ref=pr["base"]["ref"], guide=guide)
            if mine and mine.get("body") != body:
                api(f"/repos/{repo}/issues/comments/{mine['id']}", token, "PATCH", {"body": body})
            elif not mine:
                api(f"/repos/{repo}/issues/{pr_number}/comments", token, "POST", {"body": body})

        with summary.open("a") as fh:
            fh.write(f"### commit-identity: {state}\n\n{description}\n\n")
            for f in v.open:
                fh.write(f"- `{f.sha[:10]}` {f.where} `{f.masked}`\n")
        print(f"commit-identity: {state}: {description}")
        return 0
    except ApiError as e:
        print(f"::error::commit-identity could not decide: {e}")
        if head_sha:
            try:
                api(f"/repos/{repo}/statuses/{head_sha}", token, "POST",
                    {"state": "error", "context": CONTEXT, "description": "could not decide; re-run the job"})
            except ApiError:
                pass
        return 2


if __name__ == "__main__":
    sys.exit(main())
