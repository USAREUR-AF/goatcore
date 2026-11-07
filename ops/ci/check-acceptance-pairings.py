#!/usr/bin/env python3
"""Every `must_refuse` acceptance row must name a paired positive that exists.

WHY THIS IS A CI GUARD AND NOT A CONVENTION. A negative row asserts that
something is DENIED, and a denial is indistinguishable from a path that was never
live. The runner therefore refuses to count a negative whose paired positive did
not pass — but a negative whose `paired_positive` names a row that does not exist
at all degrades to UNKNOWN forever, which is a check that can never fail sitting
in the suite looking like coverage.

Caught exactly that on 2026-09-07: a row paired with `zenoh-router-answers`,
which had never been written.
"""
import sys
from pathlib import Path

import yaml

MANIFEST = Path(__file__).resolve().parents[1] / "validation" / "acceptance-tests.yaml"


def main() -> int:
    if not MANIFEST.is_file():
        print(f"FAIL: no acceptance manifest at {MANIFEST}")
        return 1
    tests = yaml.safe_load(MANIFEST.read_text()).get("tests", [])
    ids = {t.get("id") for t in tests}
    problems = []
    negatives = 0
    for t in tests:
        if not t.get("must_refuse"):
            continue
        negatives += 1
        tid, pair = t.get("id"), t.get("paired_positive")
        if not pair:
            problems.append(f"{tid}: must_refuse with no paired_positive")
        elif pair not in ids:
            problems.append(f"{tid}: paired_positive '{pair}' is not a row in this manifest")
        elif next(p for p in tests if p.get("id") == pair).get("must_refuse"):
            problems.append(f"{tid}: paired_positive '{pair}' is itself a must_refuse row")
    if problems:
        print("FAIL: acceptance pairings")
        for p in problems:
            print(f"  {p}")
        print("\n  A negative row asserts a refusal. Without a positive proving the path was")
        print("  live, it passes against an outage and reports a boundary that is not there.")
        return 1
    print(f"ok: acceptance pairings — {negatives} negative row(s), each paired with a real positive.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
