#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Refuse ADR or finding IDs claimed by another open pull request.

The GitHub query is deliberately a single request. GitHub caps both the open
PR connection and each PR's files connection at 100 nodes, so incomplete data
is not evidence of uniqueness: confirmed collisions fail, while an incomplete
or unavailable answer emits a warning and passes.
"""

from __future__ import annotations

import json
import os
import re
import sys
import urllib.error
import urllib.request


GRAPHQL_URL = "https://api.github.com/graphql"
QUERY = r"""
query CrossPrIds($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    current: pullRequest(number: $number) {
      number
      url
      changedFiles
      files(first: 100) {
        pageInfo { hasNextPage }
        nodes { path changeType }
      }
    }
    pullRequests(first: 100, states: OPEN, orderBy: {field: CREATED_AT, direction: ASC}) {
      totalCount
      pageInfo { hasNextPage }
      nodes {
        number
        url
        changedFiles
        files(first: 100) {
          pageInfo { hasNextPage }
          nodes { path changeType }
        }
      }
    }
  }
}
"""

ID_PATTERNS = (
    ("ADR", re.compile(r"^docs/adr/(\d{4})-[^/]+\.md$")),
    ("FND", re.compile(r"^docs/findings/(F-\d{3})-[^/]+\.md$")),
)
CLAIM_CHANGE_TYPES = {"ADDED", "RENAMED"}


def warning(message: str) -> None:
    message = " ".join(str(message).splitlines())
    print(f"::warning title=Cross-PR ID check incomplete::{message}")


def claimed_id(path: str, change_type: str) -> str | None:
    if change_type not in CLAIM_CHANGE_TYPES:
        return None
    for kind, pattern in ID_PATTERNS:
        match = pattern.fullmatch(path)
        if match:
            return f"{kind} {match.group(1)}"
    return None


def inspect_response(payload: object, current_pr: int) -> tuple[list[dict], list[str]]:
    """Return confirmed collisions and reasons the response is incomplete."""
    incomplete: list[str] = []
    if not isinstance(payload, dict):
        raise ValueError("GitHub response is not a JSON object")
    if payload.get("errors"):
        raise ValueError("GitHub GraphQL response contains errors")

    try:
        repository = payload["data"]["repository"]
        current = repository["current"]
        connection = repository["pullRequests"]
        nodes = connection["nodes"]
    except (KeyError, TypeError) as exc:
        raise ValueError("GitHub response is missing pull-request data") from exc
    if not isinstance(nodes, list):
        raise ValueError("GitHub pull-request nodes are not a list")

    if current is None:
        incomplete.append(f"current PR #{current_pr} was not returned by GitHub")
        return [], incomplete
    if not isinstance(current, dict) or int(current.get("number", -1)) != current_pr:
        raise ValueError("GitHub response contains invalid current-PR data")

    if connection.get("pageInfo", {}).get("hasNextPage"):
        incomplete.append(
            f"only the first 100 of {connection.get('totalCount', 'unknown')} open PRs were inspected"
        )

    claims: dict[int, tuple[str, list[tuple[str, str]]]] = {}
    for pr in [current, *nodes]:
        try:
            number = int(pr["number"])
            url = str(pr["url"])
            files = pr["files"]
            file_nodes = files["nodes"]
        except (KeyError, TypeError, ValueError) as exc:
            raise ValueError("GitHub response contains an invalid pull request") from exc
        if not isinstance(file_nodes, list):
            raise ValueError(f"PR #{number} file nodes are not a list")

        if files.get("pageInfo", {}).get("hasNextPage") or int(
            pr.get("changedFiles", len(file_nodes))
        ) > len(file_nodes):
            incomplete.append(
                f"PR #{number} has {pr.get('changedFiles', 'more than 100')} files; only its first 100 were inspected"
            )

        pr_claims: list[tuple[str, str]] = []
        for changed in file_nodes:
            if not isinstance(changed, dict):
                raise ValueError(f"PR #{number} contains an invalid file entry")
            claim = claimed_id(str(changed.get("path", "")), str(changed.get("changeType", "")))
            if claim:
                pr_claims.append((claim, str(changed["path"])))
        claims[number] = (url, pr_claims)

    current_claims = claims[current_pr][1]
    collisions: list[dict] = []
    seen: set[tuple[str, int, str]] = set()
    for claim, current_path in current_claims:
        for number, (url, other_claims) in claims.items():
            if number == current_pr:
                continue
            for other_claim, other_path in other_claims:
                key = (claim, number, other_path)
                if claim == other_claim and key not in seen:
                    seen.add(key)
                    collisions.append(
                        {
                            "id": claim,
                            "current_path": current_path,
                            "other_pr": number,
                            "other_url": url,
                            "other_path": other_path,
                        }
                    )
    collisions.sort(key=lambda row: (row["id"], row["other_pr"], row["other_path"]))
    return collisions, incomplete


def fetch_payload(token: str, repository: str, current_pr: int, opener=None) -> object:
    owner, name = repository.split("/", 1)
    body = json.dumps(
        {
            "query": QUERY,
            "variables": {"owner": owner, "name": name, "number": current_pr},
        }
    ).encode()
    request = urllib.request.Request(
        os.environ.get("GITHUB_GRAPHQL_URL", GRAPHQL_URL),
        data=body,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "User-Agent": "goat-cross-pr-id-guard",
        },
        method="POST",
    )
    open_request = opener or urllib.request.urlopen
    with open_request(request, timeout=20) as response:
        return json.loads(response.read())


def run(environ=None, opener=None) -> int:
    env = environ if environ is not None else os.environ
    token = env.get("GITHUB_TOKEN", "")
    repository = env.get("GITHUB_REPOSITORY", "")
    pr_number = env.get("PR_NUMBER", "")
    if not token or not repository or not pr_number:
        warning("GITHUB_TOKEN, GITHUB_REPOSITORY, or PR_NUMBER is unavailable; skipping")
        return 0
    if repository.count("/") != 1:
        warning("GITHUB_REPOSITORY is invalid; skipping")
        return 0
    try:
        current_pr = int(pr_number)
    except ValueError:
        warning("PR_NUMBER is invalid; skipping")
        return 0

    try:
        payload = fetch_payload(token, repository, current_pr, opener=opener)
        collisions, incomplete = inspect_response(payload, current_pr)
    except (
        AttributeError,
        KeyError,
        OSError,
        TimeoutError,
        TypeError,
        urllib.error.URLError,
        json.JSONDecodeError,
        ValueError,
    ) as exc:
        warning(f"GitHub API result could not be verified ({type(exc).__name__}); skipping")
        return 0

    for reason in incomplete:
        warning(reason)

    if collisions:
        print(f"cross-pr-id-uniqueness: FAIL — {len(collisions)} confirmed collision(s)", file=sys.stderr)
        for row in collisions:
            print(
                f"  {row['id']}: {row['current_path']} is also claimed by "
                f"PR #{row['other_pr']} {row['other_url']} as {row['other_path']}",
                file=sys.stderr,
            )
        print("Renumber this PR's claim before merge.", file=sys.stderr)
        return 1

    if incomplete:
        print("cross-pr-id-uniqueness: WARNING — no collision confirmed from incomplete data")
    else:
        print("cross-pr-id-uniqueness: OK — no open PR claims the same new ADR or finding ID")
    return 0


if __name__ == "__main__":
    sys.exit(run())
