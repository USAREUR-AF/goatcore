#!/usr/bin/env python3
"""test_gen_roster_derive_kind.py — ADR 1151 step 3, the Python half.

`affiliation.kind` is no longer REQUIRED in the manifest schema, so a manifest
may declare only the target block (`sector`, and `gov_branch` when the sector is
state). Two things must then hold, and both are asserted here:

  1. the roster derives the same `kind` the console derives — the table lives on
     both sides of the language boundary (ADR 0112), so it can drift, and this is
     the test that fails when it does. The sibling is
     `ops/goat-id-console/internal/coalition/derive.go`, whose own test file
     carries the identical case table;

  2. a LEGACY manifest derives the kind it already carries. If that ever stops
     being true the migration window is not additive: an org would read
     `organic` today and `partner` tomorrow with nobody having touched the data.

Run directly (no pytest on the operator laptop): `python3 <this file>`.
"""
import importlib.util
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent

spec = importlib.util.spec_from_file_location("gen_roster", HERE / "gen-roster.py")
gen_roster = importlib.util.module_from_spec(spec)
sys.modules["gen_roster"] = gen_roster
spec.loader.exec_module(gen_roster)

derive_kind = gen_roster.derive_kind

failures: list[str] = []


def check(label: str, got, want) -> None:
    if got == want:
        print(f"  ok  {label}")
    else:
        failures.append(f"{label}: got {got!r}, want {want!r}")
        print(f"  FAIL {label}: got {got!r}, want {want!r}")


print("target sector set:")
check("commercial is vendor, whatever the nation", derive_kind("commercial", "EST", "USA"), "vendor")
check("a state-owned market producer is still vendor", derive_kind("commercial", "USA", "USA"), "vendor")
check("civil-society is civil", derive_kind("civil-society", "USA", "USA"), "civil")
check("state + own sovereign is organic", derive_kind("state", "USA", "USA"), "organic")
check("state + foreign is partner", derive_kind("state", "EST", "USA"), "partner")
check("nation case/space is normalised", derive_kind("state", " usa ", "USA"), "organic")

print("legacy ADR 1077 sector values derive what they already carry:")
for legacy in ("military", "law-enforcement", "intelligence", "civil-government"):
    check(f"{legacy} + own sovereign -> organic", derive_kind(legacy, "USA", "USA"), "organic")
    check(f"{legacy} + foreign -> partner", derive_kind(legacy, "EST", "USA"), "partner")
for legacy in ("ngo", "media", "academic"):
    check(f"{legacy} -> civil", derive_kind(legacy, "USA", "USA"), "civil")

print("cannot-decide cases return None, so the caller keeps what the manifest carries:")
check("state with no operating sovereign", derive_kind("state", "EST", ""), None)
check("state with no nation", derive_kind("state", "", "USA"), None)
check("no sector at all", derive_kind(None, "USA", "USA"), None)
check("empty sector", derive_kind("", "USA", "USA"), None)
check("sector outside the vocabulary", derive_kind("banana", "USA", "USA"), None)

# An alliance body is nobody's own sovereign, so it must not come out organic.
check("NATO under a US-operated goatnet is partner", derive_kind("state", "NATO", "USA"), "partner")

print("alpha-2 is folded before the comparison (both spellings validate in the schema):")
check("US on a USA goatnet is the goatnet's own nation", derive_kind("state", "US", "USA"), "organic")
check("USA on a US-spelled goatnet is too", derive_kind("state", "USA", "US"), "organic")
check("CA on a USA goatnet stays partner", derive_kind("state", "CA", "USA"), "partner")
check("NATO is not a country and passes through", derive_kind("state", "NATO", "USA"), "partner")

print("normalize_nation folds what it knows and passes through what it does not:")
check("lowercase alpha-2 folds", gen_roster.normalize_nation("us"), "USA")
check("surrounding space is trimmed", gen_roster.normalize_nation(" de "), "DEU")
check("NATO is untouched", gen_roster.normalize_nation("NATO"), "NATO")
check("EU is untouched", gen_roster.normalize_nation("EU"), "EU")
check("None is empty, not a crash", gen_roster.normalize_nation(None), "")

print()
if failures:
    print(f"FAILED: {len(failures)} assertion(s)")
    for f in failures:
        print("  -", f)
    sys.exit(1)
print("all ADR 1151 derive_kind checks passed")

