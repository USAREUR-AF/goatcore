#!/usr/bin/env python3
"""The reload watcher (files/slot-fragment-reload.py) tells a revocation from
an addition by the (subject, leaf CN) pairs it finds. Checks that parsing,
without docker, on fragments the portal renders: single-CN slot and copub
subjects, multi-CN carry-<anchor> subjects (ADR 1251), and the unassigned CN,
which is no leaf.

    python3 ops/ansible/roles/goat-zenoh/tests/slot-fragment-reload-cns-check.py
"""
import importlib.util
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("reload", HERE.parent / "files" / "slot-fragment-reload.py")
rl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rl)
sys.path.insert(0, str(HERE / "slot-fragment"))
import slotfragment as sf  # noqa: E402

A, B, C = "0" * 32, "1" * 32, "2" * 32
fails = 0


def check(cond, msg):
    global fails
    print(("  PASS: " if cond else "  FAIL: ") + msg)
    if not cond:
        fails += 1


two = sf.render([(A, A + ".g1", [], [B + ".g1", C + ".g1"]), (B, B + ".g1"), (C, C + ".g1")])
one = sf.render([(A, A + ".g1", [], [C + ".g1"]), (B, B + ".g1"), (C, C + ".g1")])
none = sf.render([(A, A + ".g1"), (B, B + ".g1"), (C, C + ".g1")])
pairs = rl.leaf_cns(two)
check(("carry-" + A, B + ".g1") in pairs and ("carry-" + A, C + ".g1") in pairs, "both carriers of a two-CN carry list are read")
check(("slot-" + A, A + ".g1") in pairs, "single-CN slot subjects are still read")
check(not any(cn.endswith(".unassigned") for _, cn in rl.leaf_cns(none)), "the unassigned CN is no leaf")
check(rl.leaf_cns(two) - rl.leaf_cns(one) == {("carry-" + A, B + ".g1")},
      "a carrier taken off a two-CN list is a removal (a prompt reload)")
check(not (rl.leaf_cns(one) - rl.leaf_cns(two)) and rl.leaf_cns(two) - rl.leaf_cns(one),
      "adding a carrier is an addition only (batched)")
check(rl.leaf_cns(one) - rl.leaf_cns(none) == {("carry-" + A, C + ".g1")}, "the last carrier leaving is a removal")
check(rl.digest_ok(two), "the rendered fragment passes the watcher's digest check")
print()
print("RELOAD CN CHECKS PASSED" if fails == 0 else f"{fails} CHECK(S) FAILED")
sys.exit(1 if fails else 0)
