#!/usr/bin/env python3
"""check-golden-base-current.py: a pull request that changes goatbuilder's full-plans record
is on a base holding every change on the base branch to what the record is made from.

crates/goatbuilder/tests/full_plans_unchanged.rs holds every full fixture site's standup
plan to crates/goatbuilder/tests/fixtures/full-plans.golden.json. A pull request that
records the file is green on the base it was tested on. If the base branch has since
changed what the plans are made from (crates/goatbuilder/src/standup.rs, the fixture sites
under ops/goatbuilder/fixtures/, or the record itself), merging it turns the base branch
red: on 14 Nov a record cut before a phase 12 title change merged after it. So when a pull
request changes the record, the newest base-branch commit that touched any of those paths
must be in the pull request's history; otherwise it is rebased and the record regenerated.
The same shape as ops/ci/check-changelog-base-current.py.

Usage:
  check-golden-base-current.py --base <ref> --head <ref> [--repo DIR]
  python3 ops/ci/test_check_golden_base_current.py     its tests

Exit 0 = the pull request does not change the record, or its history holds every such
commit. Exit 1 = it changes the record and the base has such a commit it lacks. Exit 2 =
a ref that is not a commit, or no merge base (a shallow checkout: CI fetches the whole
history for this check); never a silent pass.
"""
import argparse
import subprocess
import sys
from pathlib import Path

GOLDEN = "crates/goatbuilder/tests/fixtures/full-plans.golden.json"
# What the recorded plans are made from, and the record itself.
SOURCES = ("crates/goatbuilder/src/standup.rs", "ops/goatbuilder/fixtures", GOLDEN)
REGENERATE = "GOATBUILDER_GOLDEN=overwrite cargo test -p goatbuilder --test full_plans_unchanged"


def git(repo, *args):
    r = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else None


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--repo", default=str(Path(__file__).resolve().parents[2]))
    ap.add_argument("--base", required=True, help="the base branch's commit, as the pull request merges into it")
    ap.add_argument("--head", required=True, help="the pull request's own head commit (not the merge commit)")
    a = ap.parse_args(argv)
    for name, ref in (("base", a.base), ("head", a.head)):
        if git(a.repo, "rev-parse", "--verify", "-q", f"{ref}^{{commit}}") is None:
            print(f"check-golden-base-current: --{name} {ref} is not a commit here", file=sys.stderr)
            return 2
    mb = git(a.repo, "merge-base", a.base, a.head)
    if not mb:
        shallow = (git(a.repo, "rev-parse", "--is-shallow-repository") or "").strip() == "true"
        print(f"check-golden-base-current: no merge base between {a.base} and {a.head}"
              + (" (a shallow checkout: fetch the whole history, fetch-depth: 0)" if shallow else "")
              + "; cannot tell what the pull request changes", file=sys.stderr)
        return 2
    if not (git(a.repo, "diff", "--name-only", mb.strip(), a.head, "--", GOLDEN) or "").strip():
        print(f"check-golden-base-current: ok, the pull request does not change {GOLDEN}")
        return 0
    newest = (git(a.repo, "log", "-1", "--format=%H %s", a.base, "--", *SOURCES) or "").strip()
    if not newest:
        print(f"check-golden-base-current: ok, the base has no commit touching {', '.join(SOURCES)}")
        return 0
    sha, _, subject = newest.partition(" ")
    held = subprocess.run(["git", "-C", str(a.repo), "merge-base", "--is-ancestor", sha, a.head]).returncode
    if held == 0:
        print(f"check-golden-base-current: ok, the pull request changes {GOLDEN} on a base holding {sha[:12]} "
              f"({subject}), the newest change to what it records")
        return 0
    if held != 1:
        print(f"check-golden-base-current: cannot tell whether {a.head} holds {sha[:12]}", file=sys.stderr)
        return 2
    print(f"check-golden-base-current: the pull request changes {GOLDEN}, and its base has {sha[:12]} "
          f"({subject}), which changed what the record is made from ({', '.join(SOURCES)}) after this branch "
          f"was cut. Merged as it is, the record would not match the plans on the base. Rebase onto the base "
          f"branch, then regenerate the record: {REGENERATE}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
