#!/usr/bin/env python3
"""check-changelog-base-current.py: a pull request that changes goatbuilder's
CHANGELOG.md is on a base that holds every goatbuilder release.

goatbuilder's changelog is one file per entry in crates/goatbuilder/changelog.d/
(ops/release/changelog.py). A release folds the entries into a dated section of
crates/goatbuilder/CHANGELOG.md and removes their files; the Unreleased section is
one fixed pointer line, never a render. So a pull request that only adds an entry
file merges correctly whenever it was cut: a release made since then did not know
its file, which stays and goes into the next release. What can still go wrong is a
change to CHANGELOG.md itself (a release, or release notes) on a base older than a
release: a three-way merge would mix the two releases' sections. So when a pull
request changes crates/goatbuilder/CHANGELOG.md against its merge base with the base
branch, every `## X.Y.Z` section the base's CHANGELOG.md has must also be in the pull
request's: otherwise it is rebased first.

Usage:
  check-changelog-base-current.py --base <ref> --head <ref> [--repo DIR]
  python3 ops/ci/test_check_changelog_base_current.py     its tests

Exit 0 = the pull request does not change CHANGELOG.md (entry files alone are fine at
any base), or its base holds no release it lacks. Exit 1 = it changes CHANGELOG.md and
the base has a release it lacks.
Exit 2 = a ref that is not a commit, or no merge base (a shallow checkout: CI fetches
the whole history for this check); never a silent pass.
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path

CHANGELOG = "crates/goatbuilder/CHANGELOG.md"
RELEASE = re.compile(r"^## (\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)(?:\s|$)")


def git(repo, *args):
    r = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else None


def releases(repo, ref):
    """The `## X.Y.Z` versions in CHANGELOG.md at ref, in order (none when absent)."""
    text = git(repo, "show", f"{ref}:{CHANGELOG}") or ""
    return [m.group(1) for m in map(RELEASE.match, text.split("\n")) if m]


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--repo", default=str(Path(__file__).resolve().parents[2]))
    ap.add_argument("--base", required=True, help="the base branch's commit, as the pull request merges into it")
    ap.add_argument("--head", required=True, help="the pull request's own head commit (not the merge commit)")
    a = ap.parse_args(argv)
    for name, ref in (("base", a.base), ("head", a.head)):
        if git(a.repo, "rev-parse", "--verify", "-q", f"{ref}^{{commit}}") is None:
            print(f"check-changelog-base-current: --{name} {ref} is not a commit here", file=sys.stderr)
            return 2
    mb = git(a.repo, "merge-base", a.base, a.head)
    if not mb:
        shallow = (git(a.repo, "rev-parse", "--is-shallow-repository") or "").strip() == "true"
        print(f"check-changelog-base-current: no merge base between {a.base} and {a.head}"
              + (" (a shallow checkout: fetch the whole history, fetch-depth: 0)" if shallow else "")
              + "; cannot tell what the pull request changes", file=sys.stderr)
        return 2
    touched = (git(a.repo, "diff", "--name-only", mb.strip(), a.head, "--", CHANGELOG) or "").split()
    if not touched:
        print(f"check-changelog-base-current: ok, the pull request does not change {CHANGELOG} "
              f"(entry files alone merge correctly at any base)")
        return 0
    have = set(releases(a.repo, a.head))
    missing = [v for v in releases(a.repo, a.base) if v not in have]
    if missing:
        print(f"check-changelog-base-current: the pull request changes {', '.join(touched)}, and its base has "
              f"goatbuilder {', '.join(missing)}, released after this branch was cut. Merged as it is, the "
              f"two changelogs would mix: rebase onto the base branch first. (An entry belongs in its own file "
              f"in crates/goatbuilder/changelog.d/; take the base branch's {CHANGELOG} if that was the only "
              f"change to it.)", file=sys.stderr)
        return 1
    print(f"check-changelog-base-current: ok, the pull request changes {', '.join(touched)} on a base whose "
          f"releases it holds")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
