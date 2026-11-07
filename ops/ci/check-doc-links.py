#!/usr/bin/env python3
"""check-doc-links.py — relative .md links in tracked docs must resolve.

A RATCHET, not a clean-room gate. The repo carried 293 broken links when this
was written; 199 were repaired mechanically and the rest point at documents that
were cited but never written (ADR 0008/0009/0610/0710 among them). Deciding what
those should say is authoring work, not link-fixing, so the gate freezes the
remaining count and fails on any INCREASE.

That makes the debt visible without blocking unrelated work, and makes it
impossible to quietly add more. Lower BASELINE whenever you fix some — the gate
tells you the new number.

Skips, each for a reason:
  - untracked files (node_modules and other local clutter are not the repo's)
  - http/mailto/protocol-relative targets: not ours to resolve
  - templates and placeholders containing '<' or '%': deliberately unresolvable
  - anchors: only the path before '#' is checked

Usage:  python3 ops/ci/check-doc-links.py [--list]
"""
import os
import re
import subprocess
import sys

# The count of broken links accepted at the time of writing. Only ever lower it.
BASELINE = 0

LINK = re.compile(r'\[[^\]]*\]\(([^)]+)\)')


def tracked_markdown():
    out = subprocess.run(["git", "ls-files", "docs"], capture_output=True, text=True).stdout
    return sorted(p for p in out.split() if p.endswith(".md"))


def broken_links():
    found = []
    for f in tracked_markdown():
        base = os.path.dirname(f)
        try:
            with open(f, encoding="utf-8") as source_file:
                body = source_file.read()
        except (OSError, UnicodeDecodeError):
            continue
        for m in LINK.finditer(body):
            target = m.group(1).split("#")[0].strip()
            if not target or target.endswith("/"):
                continue
            if target.startswith(("http://", "https://", "mailto:", "//")):
                continue
            if not target.endswith(".md"):
                continue
            if "<" in target or "%" in target:
                continue
            if not os.path.exists(os.path.normpath(os.path.join(base, target))):
                found.append((f, target))
    return found


def main():
    found = broken_links()
    n = len(found)

    # Listed on any failure whether or not --list was passed: CI is where you
    # least want to be told to re-run something locally to see what broke.
    if "--list" in sys.argv or n != BASELINE:
        for f, t in found:
            print(f"  {f} -> {t}")
        print()

    if n > BASELINE:
        print(f"FAIL: doc links — {n} broken, baseline {BASELINE}. "
              f"{n - BASELINE} new one(s).", file=sys.stderr)
        print("The list above is every broken link, not only the new ones — diff it "
              "against your branch. Fix the new link, or if you deliberately removed "
              "a document, update everything that pointed at it.", file=sys.stderr)
        return 1

    if n < BASELINE:
        print(f"FAIL: doc links — {n} broken, better than the {BASELINE} baseline. "
              f"Lower BASELINE to {n} in this script to lock the improvement in.",
              file=sys.stderr)
        return 1

    print(f"ok: doc links — {n} broken, at the {BASELINE} baseline (known debt, "
          f"mostly ADRs cited but never written).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
