#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_hotfix_shadow.py — the ladder names the member deviations that keep a
# member off its goatnet's value (ADR 1256).
#
# graduate.py resolves a rung at goatnet level (goatnet deviation -> fleet) and
# leaves member deviations to node parity. A `hotfix` therefore reported a goatnet
# on the hotfix ref while a member host-group with its own older deviation stayed
# on the old image. `shadowing_members` names those members. `hotfix` prints them
# with the two ways out, and `status` flags them under its table.
#
# Synthetic goatnet `zagros` only. Plain python3 + asserts, the convention of
# test_readiness.py. Exit codes: 0 all pass, 1 one or more failed.

import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RELEASE = os.path.normpath(os.path.join(HERE, ".."))
spec = importlib.util.spec_from_file_location("graduate", os.path.join(RELEASE, "graduate.py"))
g = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g)

OLD = "ghcr.io/example/app@sha256:" + "a" * 64
FIX = "ghcr.io/example/app@sha256:" + "b" * 64
PIN = "ghcr.io/example/app@sha256:" + "c" * 64
GOATNETS = {"zagros": {"members": ["zagros_a_hosts", "zagros_b_hosts"]}}

FAILURES = []


def check(name, cond, detail=""):
    if cond:
        print("ok   %s" % name)
    else:
        FAILURES.append(name)
        print("FAIL %s %s" % (name, detail))


def comp(**devs):
    return {"var": "app_image", "fleet": OLD, "deviations": devs}


def main():
    have = hasattr(g, "shadowing_members")
    check("graduate.py defines shadowing_members", have)
    if not have:
        return
    c = comp(zagros={"value": FIX, "reason": "HOTFIX"},
             zagros_b_hosts={"value": PIN, "over": OLD, "reason": "held back"})
    got = g.shadowing_members(c, "zagros", GOATNETS)
    check("a member pinning another value shadows its goatnet", got == [("zagros_b_hosts", PIN)], got)

    got = g.shadowing_members(c, "zagros", GOATNETS, against=FIX)
    check("against a hotfix ref not yet in the table, the member still shadows",
          got == [("zagros_b_hosts", PIN)], got)

    c2 = comp(zagros_b_hosts={"value": FIX, "over": OLD, "reason": "already on it"})
    check("a member already on the hotfix ref does not shadow it",
          g.shadowing_members(c2, "zagros", GOATNETS, against=FIX) == [])

    c3 = comp(zagros_b_hosts={"absent": True, "reason": "not run here"})
    check("an absent member shadows nothing", g.shadowing_members(c3, "zagros", GOATNETS) == [])

    check("a goatnet with no member deviation has no shadow",
          g.shadowing_members(comp(zagros={"value": FIX, "reason": "h"}), "zagros", GOATNETS) == [])

    lines = g.shadow_advice("app", "zagros", FIX, [("zagros_b_hosts", PIN)])
    text = "\n".join(lines)
    check("the hotfix advice names the member, both ways out and the line to paste",
          "zagros_b_hosts" in text and "delete" in text.lower() and 'over: "%s"' % FIX in text, text)


if __name__ == "__main__":
    main()
    sys.exit(1 if FAILURES else 0)
