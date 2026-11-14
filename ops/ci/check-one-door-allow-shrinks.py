#!/usr/bin/env python3
"""check-one-door-allow-shrinks.py: the one-door allow-list only shrinks.

goatbuilder starts every child process through crates/goatbuilder/src/runner.rs
(docs/design/goatbuilder-legibility.md, step 2). The files that still start
processes themselves are listed, with a count each, in
crates/goatbuilder/tests/legibility-one-door-allow.txt, while their call sites
move. The test tests/legibility_one_door.rs holds the tree to the list; this gate
holds the list to the base branch: no file may gain a line or a higher count than
the base gives it. A file the base does not list, or a count above the base's,
fails. Lowering or removing a line is the point.

Usage:
  check-one-door-allow-shrinks.py --base <git ref>   compare with the list at <ref>
  check-one-door-allow-shrinks.py --self-test        prove red, then green

Exit 0 = every count is at or below the base's. Exit 1 = one grew or appeared.
Exit 2 = usage, a base that is not a commit, or a list that could not be read.
"""
import os
import subprocess
import sys

PATH = "crates/goatbuilder/tests/legibility-one-door-allow.txt"


def parse(text):
    out = {}
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) != 2 or not parts[1].isdigit():
            raise ValueError(f"not '<file> <count>': {line!r}")
        out[parts[0]] = int(parts[1])
    return out


def grown(base, head):
    """Lines of `head` that are new or higher than in `base`."""
    problems = []
    for f, n in sorted(head.items()):
        if f not in base:
            problems.append(f"{f} {n}: not on the base's list; start its processes through crate::runner::Cmd")
        elif n > base[f]:
            problems.append(f"{f} {n}: the base allows {base[f]}; start the new ones through crate::runner::Cmd")
    return problems


def self_test():
    base = parse("# c\na.rs 3\nb.rs 1\n")
    assert grown(base, parse("a.rs 3\nb.rs 1\n")) == [], "the same list passes"
    assert grown(base, parse("a.rs 2\n")) == [], "lowering and removing pass"
    assert len(grown(base, parse("a.rs 4\nb.rs 1\n"))) == 1, "a higher count fails"
    assert len(grown(base, parse("a.rs 3\nb.rs 1\nc.rs 1\n"))) == 1, "a new file fails"
    # The base is checked, not assumed: a ref that is not a commit fails closed.
    assert against("no-such-ref", {}) == 2, "a ref that does not exist"
    assert against("0" * 40, {}) == 2, "an all-zero sha"
    # A base commit with no list: the empty tree, committed in place. This writes
    # one loose commit object into the checkout's object store, and nothing else.
    # It needs an identity (a CI runner has none) and must not try to sign.
    empty_tree = subprocess.run(["git", "hash-object", "-t", "tree", os.devnull], capture_output=True, text=True, check=True).stdout.strip()
    ident = {
        "GIT_AUTHOR_NAME": "self-test",
        "GIT_AUTHOR_EMAIL": "self-test@example.org",
        "GIT_COMMITTER_NAME": "self-test",
        "GIT_COMMITTER_EMAIL": "self-test@example.org",
    }
    made = subprocess.run(
        ["git", "-c", "commit.gpgsign=false", "commit-tree", empty_tree, "-m", "a base with no list"],
        capture_output=True,
        text=True,
        env={**os.environ, **ident},
    )
    if made.returncode != 0:
        # git's own words, not a bare assertion.
        raise SystemExit(f"self-test: git commit-tree failed: {made.stderr.strip()}")
    before_list = made.stdout.strip()
    assert before_list and against(before_list, {}) == 0, "a base with no list: this change introduces it"
    print("ok: self-test (a higher count and a new file fail; the same, lower and removed pass; a base that is not a commit fails closed; a base with no list passes)")
    return 0


def main(argv):
    top = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True)
    if top.returncode != 0:
        print("not inside a git checkout", file=sys.stderr)
        return 2
    os.chdir(top.stdout.strip())
    if argv[1:] == ["--self-test"]:
        return self_test()
    if len(argv) != 3 or argv[1] != "--base":
        print(__doc__.strip().split("\n\n")[2], file=sys.stderr)
        return 2
    base_ref = argv[2]
    try:
        head = parse(open(PATH, encoding="utf-8").read())
    except (OSError, ValueError) as e:
        print(f"cannot read {PATH}: {e}", file=sys.stderr)
        return 2
    return against(base_ref, head)


def against(base_ref, head):
    # A base that is not a commit is a broken gate, never a pass.
    if subprocess.run(["git", "cat-file", "-e", f"{base_ref}^{{commit}}"], capture_output=True).returncode != 0:
        print(f"cannot compare: {base_ref!r} is not a commit here (fetch the base first)", file=sys.stderr)
        return 2
    listed = subprocess.run(["git", "ls-tree", "--name-only", base_ref, "--", PATH], capture_output=True, text=True)
    if listed.returncode != 0:
        print(f"cannot list {PATH} at {base_ref}: {listed.stderr.strip()}", file=sys.stderr)
        return 2
    if not listed.stdout.strip():
        # The base predates the list: the list this change brings is the first one.
        print(f"ok: {base_ref} has no {PATH}; this change introduces it")
        return 0
    got = subprocess.run(["git", "show", f"{base_ref}:{PATH}"], capture_output=True, text=True)
    if got.returncode != 0:
        print(f"cannot read {PATH} at {base_ref}: {got.stderr.strip()}", file=sys.stderr)
        return 2
    try:
        base = parse(got.stdout)
    except ValueError as e:
        print(f"cannot read {PATH} at {base_ref}: {e}", file=sys.stderr)
        return 2
    problems = grown(base, head)
    if problems:
        print(f"refused: {PATH} grew against {base_ref}:", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1
    print(f"ok: {PATH} is at or below {base_ref} ({sum(head.values())} calls in {len(head)} files)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
