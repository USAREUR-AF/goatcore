#!/usr/bin/env python3
"""parity-line-diff.py: say where two normalised parity plans differ, line and column.

The parity gates (parity-standup.sh, parity-teardown.sh) compare the script's
plan with goatbuilder's, one normalised line per step. They used to print the
`diff` of the two cut at 220 characters, so two lines that differed further
along printed the same, and the reader had to guess. This prints, for each
line that differs on both sides, its line number, the 1-based column of the
first different character, both lengths, and a window of text around that
column on each side. A line on only one side is printed as such.

usage: parity-line-diff.py LEFT_FILE RIGHT_FILE [LEFT_NAME RIGHT_NAME]
       (prints at most --max differences, 20 by default; exits 0)
"""
import argparse
import difflib
import sys

WINDOW = 60


def first_difference(a, b):
    """The 0-based index of the first character where a and b differ."""
    n = min(len(a), len(b))
    return next((i for i in range(n) if a[i] != b[i]), n)


def window(s, i):
    lo, hi = max(0, i - WINDOW), min(len(s), i + WINDOW)
    return ("…" if lo else "") + s[lo:hi] + ("…" if hi < len(s) else "")


def report(left, right, lname, rname, limit):
    out = []
    width = max(len(lname), len(rname)) + 1
    sm = difflib.SequenceMatcher(a=left, b=right, autojunk=False)
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            continue
        pairs = list(zip(range(i1, i2), range(j1, j2))) if tag == "replace" else []
        for i, j in pairs:
            a, b = left[i], right[j]
            c = first_difference(a, b)
            out.append(f"line {i + 1}, column {c + 1} ({len(a)} and {len(b)} characters):")
            out.append(f"  {(lname + ':').ljust(width)} {window(a, c)}")
            out.append(f"  {(rname + ':').ljust(width)} {window(b, c)}")
        for i in range(i1 + len(pairs), i2):
            out.append(f"line {i + 1} only in {lname}: {window(left[i], 0)}")
        for j in range(j1 + len(pairs), j2):
            out.append(f"line {j + 1} only in {rname}: {window(right[j], 0)}")
    shown, n = [], 0
    for line in out:
        if not line.startswith("  "):
            n += 1
            if n > limit:
                shown.append(f"... and more differences (first {limit} shown)")
                break
        shown.append(line)
    return shown


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("left")
    ap.add_argument("right")
    ap.add_argument("lname", nargs="?", default="left")
    ap.add_argument("rname", nargs="?", default="right")
    ap.add_argument("--max", type=int, default=20)
    a = ap.parse_args()
    read = lambda p: open(p, encoding="utf-8", errors="replace").read().splitlines()
    for line in report(read(a.left), read(a.right), a.lname, a.rname, a.max):
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
