#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""check-ci-scripts-wired.py — a guard script that nothing runs is not a guard.

THE FAILURE THIS EXISTS FOR. On 2026-09-08 a review of ops/ci found 15 files,
3,013 lines, that no workflow, git hook, session hook, Makefile target,
ops/scripts entry or Ansible role invoked. Two were test suites whose own
docstrings said they ran on every PR. One was the soundness test for the
pre-commit live-secret guard. Each had been written for a finding, landed
green, and then never executed again, because nothing in the tree noticed an
unreferenced file. This is the shrink-only ratchet that notices.

WHAT IT CHECKS. Every `ops/ci/check-*`, `ops/ci/test-*` and `ops/ci/test_*`
file must have its basename referenced somewhere that can run it:
.github/workflows/, ops/git-hooks/, .claude/hooks/, .claude/settings.json,
Makefile, ops/scripts/, ops/ansible/roles/, or tests/ (ci.yml runs
`bats tests/` and pytest over tests/, so a suite there that calls a script
runs it). A reference inside ops/ci
itself does not count (a script mentioning a sibling in a comment is how the
2026-09-08 set stayed invisible).

THE ALLOWLIST. ops/ci/ci-scripts-wired-allowlist.txt names the files that are
known-unwired today, grouped by why: a guard that needs a live host, an API
token or a goatnet's environment repository; a guard whose subject is not in
this repository; a guard that fails on main and is fixed before it is wired.
It is shrink-only: a file on the list that becomes wired makes its line stale
and this guard refuses until the line is removed, and a new unwired file fails
on its own PR. docs/design/ci-discipline.md has the rule.

Usage:
  ops/ci/check-ci-scripts-wired.py            # verdict, exit 1 on failure
  ops/ci/check-ci-scripts-wired.py --report   # print every file and its wiring
  ops/ci/check-ci-scripts-wired.py --self-test
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile

ALLOWLIST = "ops/ci/ci-scripts-wired-allowlist.txt"
CALLER_DIRS = [".github/workflows", "ops/git-hooks", ".claude/hooks", "ops/scripts", "ops/ansible/roles", "tests"]
CALLER_FILES = ["Makefile", ".claude/settings.json"]
PATTERN = re.compile(r"^(check-.*|test-.*|test_.*)$")


def repo_root() -> str:
    try:
        return subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip()
    except Exception:
        return os.getcwd()


def candidates(root: str) -> list[str]:
    out = []
    for name in sorted(os.listdir(os.path.join(root, "ops/ci"))):
        if PATTERN.match(name) and os.path.isfile(os.path.join(root, "ops/ci", name)):
            out.append(name)
    return out


def caller_text(root: str) -> str:
    chunks = []
    for d in CALLER_DIRS:
        for dirpath, _, files in os.walk(os.path.join(root, d)):
            for f in files:
                p = os.path.join(dirpath, f)
                try:
                    with open(p, "rb") as fh:
                        chunks.append(fh.read().decode("utf-8", "replace"))
                except OSError:
                    pass  # unreadable file: skip
    for f in CALLER_FILES:
        p = os.path.join(root, f)
        if os.path.isfile(p):
            with open(p, "rb") as fh:
                chunks.append(fh.read().decode("utf-8", "replace"))
    return "\n".join(chunks)


def read_allowlist(root: str) -> list[str]:
    p = os.path.join(root, ALLOWLIST)
    if not os.path.isfile(p):
        return []
    with open(p) as fh:
        return [ln.strip() for ln in fh if ln.strip() and not ln.startswith("#")]


def verdict(root: str, report: bool = False) -> int:
    text = caller_text(root)
    allow = read_allowlist(root)
    unwired = [n for n in candidates(root) if n not in text]
    if report:
        for n in candidates(root):
            print(f"  {'unwired' if n in unwired else 'wired  '}  {n}")
    new = [n for n in unwired if n not in allow]
    stale = [n for n in allow if n not in unwired]
    rc = 0
    if new:
        rc = 1
        print(f"REFUSED: {len(new)} ops/ci script(s) are invoked by nothing:")
        for n in new:
            print(f"  ops/ci/{n}")
        print("Wire each one into a workflow, hook, Makefile target or role in this PR,")
        print(f"or delete it. Listing it in {ALLOWLIST} is for a guard that cannot run in this")
        print("repository's CI, with the reason; see docs/design/ci-discipline.md.")
    if stale:
        rc = 1
        present = set(candidates(root))
        print(f"REFUSED: {len(stale)} allowlist entr{'y is' if len(stale)==1 else 'ies are'} stale (remove the line):")
        for n in stale:
            print(f"  {n}  ({'now wired' if n in present else 'no such file in ops/ci'})")
    if rc == 0:
        print(f"ok: {len(candidates(root)) - len(unwired)} ops/ci scripts wired; {len(unwired)} on the shrink-only allowlist.")
    return rc


def self_test() -> int:
    with tempfile.TemporaryDirectory() as t:
        os.makedirs(os.path.join(t, "ops/ci"))
        os.makedirs(os.path.join(t, ".github/workflows"))
        for d in CALLER_DIRS:
            os.makedirs(os.path.join(t, d), exist_ok=True)
        with open(os.path.join(t, "ops/ci/check-wired.sh"), "w") as source_file:
            source_file.write("#!/bin/sh\n")
        with open(os.path.join(t, "ops/ci/check-planted.sh"), "w") as source_file:
            source_file.write("#!/bin/sh\n")
        with open(os.path.join(t, "ops/ci/check-listed.sh"), "w") as source_file:
            source_file.write("#!/bin/sh\n")
        with open(os.path.join(t, ".github/workflows/x.yml"), "w") as source_file:
            source_file.write("run: ops/ci/check-wired.sh\n")
        with open(os.path.join(t, "ops/ci/check-from-tests.sh"), "w") as source_file:
            source_file.write("#!/bin/sh\n")
        with open(os.path.join(t, "tests/x.bats"), "w") as source_file:
            source_file.write('run "$REPO/ops/ci/check-from-tests.sh"\n')
        with open(os.path.join(t, ALLOWLIST), "w") as source_file:
            source_file.write("check-listed.sh\n")
        fails = 0
        import io, contextlib
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = verdict(t)
        out = buf.getvalue()
        if rc == 1 and "check-planted.sh" in out and "check-listed.sh" not in out.split("stale")[0]:
            print("  ok    refuses a planted unwired script and accepts the listed one")
        else:
            fails += 1; print("  FAIL  did not refuse the planted unwired script\n" + out)
        if "check-from-tests.sh" not in out:
            print("  ok    a script a tests/ suite calls counts as wired")
        else:
            fails += 1; print("  FAIL  a script called from tests/ was reported unwired\n" + out)
        # stale entry: list a wired script
        with open(os.path.join(t, ALLOWLIST), "w") as source_file:
            source_file.write("check-listed.sh\ncheck-wired.sh\n")
        os.remove(os.path.join(t, "ops/ci/check-planted.sh"))
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = verdict(t)
        if rc == 1 and "check-wired.sh  (now wired)" in buf.getvalue():
            print("  ok    refuses a stale allowlist entry")
        else:
            fails += 1; print("  FAIL  accepted a stale allowlist entry\n" + buf.getvalue())
        with open(os.path.join(t, ALLOWLIST), "w") as source_file:
            source_file.write("check-listed.sh\ncheck-gone.sh\n")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = verdict(t)
        if rc == 1 and "check-gone.sh  (no such file in ops/ci)" in buf.getvalue():
            print("  ok    names a listed file that no longer exists")
        else:
            fails += 1; print("  FAIL  a listed missing file was not named as missing\n" + buf.getvalue())
        print(f"  {4 - fails} passed, {fails} failed")
        return 1 if fails else 0


if __name__ == "__main__":
    if "--self-test" in sys.argv:
        sys.exit(self_test())
    sys.exit(verdict(repo_root(), report="--report" in sys.argv))
