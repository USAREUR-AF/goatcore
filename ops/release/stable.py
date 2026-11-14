#!/usr/bin/env python3
"""stable: decide whether the `stable` branch may move to a whole-tree release (ADR 1231).

A whole-tree release is a tag `goatcore/vX.Y.Z`. A maintainer cuts it, signed, at
the commit of the goatbuilder release with the same version, after that commit
has run in a standing environment. `.github/workflows/stable.yml` runs this
script on the tag and, when it passes, fast-forwards `stable` to the tagged
commit. Nothing else moves `stable`.

  stable.py check <tag>    exit 0 when `stable` may move to <tag>; exit 1 with each reason when not
  stable.py table <tag>    the versions that go together at <tag>, as Markdown

`check` refuses a tag when any of these holds:
  - the tag is not `goatcore/vX.Y.Z`
  - the tagged commit is not on `main`
  - no tag `goatbuilder/vX.Y.Z` with the same version points at the same commit
  - `stable` exists and is not an ancestor of the tagged commit (the move would not be a fast-forward)
"""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

TAG = re.compile(r"^goatcore/v(\d+\.\d+\.\d+)$")
# Tag families built from this repository, with the name each is known by.
COMPONENTS = [
    ("goatbuilder", "goatbuilder/v"),
    ("goat (the operator CLI)", "goat-cli-v"),
    ("goat-relay", "goat-relay/v"),
    ("snitch", "snitch-v"),
    ("grant-crypto SDK", "grant-crypto-sdk-v"),
]
WORKS_WITH = Path(__file__).with_name("works-with.md")


def git(repo: str, *args: str) -> tuple[int, str]:
    p = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True)
    return p.returncode, p.stdout.strip()


def commit_of(repo: str, ref: str) -> str | None:
    rc, out = git(repo, "rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}")
    return out if rc == 0 and out else None


def is_ancestor(repo: str, older: str, newer: str) -> bool:
    return git(repo, "merge-base", "--is-ancestor", older, newer)[0] == 0


def refusals(repo: str, tag: str, main: str = "origin/main", stable: str = "origin/stable") -> list[str]:
    m = TAG.match(tag)
    if not m:
        return [f"{tag} is not a whole-tree release tag (goatcore/vX.Y.Z)"]
    version = m.group(1)
    commit = commit_of(repo, f"refs/tags/{tag}")
    if commit is None:
        return [f"the tag {tag} does not exist in this clone"]
    why = []
    if not is_ancestor(repo, commit, main):
        why.append(f"the commit {tag} names ({commit[:9]}) is not on main")
    builder = commit_of(repo, f"refs/tags/goatbuilder/v{version}")
    if builder is None:
        why.append(f"there is no tag goatbuilder/v{version}; a whole-tree release is cut at a goatbuilder release")
    elif builder != commit:
        why.append(f"goatbuilder/v{version} names {builder[:9]}, and {tag} names {commit[:9]}; they must be one commit")
    current = commit_of(repo, stable)
    if current is not None and not is_ancestor(repo, current, commit):
        why.append(f"stable ({current[:9]}) is not an ancestor of {commit[:9]}; stable only moves forward")
    return why


def versions(repo: str, commit: str) -> list[tuple[str, str]]:
    """The newest tag of each family that is reachable from the commit."""
    out = []
    for name, prefix in COMPONENTS:
        rc, tag = git(repo, "describe", "--tags", "--abbrev=0", "--match", f"{prefix}*", commit)
        if rc == 0 and tag:
            out.append((name, tag))
    return out


def table(repo: str, tag: str) -> str:
    commit = commit_of(repo, f"refs/tags/{tag}") or ""
    lines = [
        f"`{tag}` is a release of the whole tree, and the commit `stable` points at.",
        "Build on this tag, or on the `stable` branch, when you want a fixed reference.",
        "",
        "## Versions that go together",
        "",
        f"Each of these was released from commit `{commit[:12]}` or from an earlier commit that it contains.",
        "",
        "| Component | Release |",
        "|---|---|",
    ]
    lines += [f"| {name} | `{t}` |" for name, t in versions(repo, commit)]
    if WORKS_WITH.exists():
        lines += ["", WORKS_WITH.read_text().strip()]
    return "\n".join(lines) + "\n"


def main(argv: list[str]) -> int:
    if len(argv) != 3 or argv[1] not in ("check", "table"):
        print(__doc__)
        return 2
    repo, tag = ".", argv[2]
    if argv[1] == "table":
        print(table(repo, tag), end="")
        return 0
    why = refusals(repo, tag)
    for w in why:
        print(f"refused: {w}")
    if not why:
        print(f"ok: stable may move to {tag} ({commit_of(repo, 'refs/tags/' + tag)[:9]})")
    return 1 if why else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
