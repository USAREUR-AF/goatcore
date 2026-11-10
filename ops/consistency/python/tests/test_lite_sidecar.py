#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_lite_sidecar.py — the lite sidecar's merge rules (ADR 1213).
#
# ops/pins/lite-pins.yaml is read beside fleet-pins.yaml and merged into ONE
# table: goatnets union (a name in both is an error), deviations union per
# component (a scope in both is an error), component definitions refused. A
# goatnet declared in the sidecar carries the sidecar's hash in its intent
# `sources`; a goatnet declared in fleet-pins.yaml carries exactly what it did
# before the sidecar existed, so no committed intent moves.
#
# Plain python3 + asserts, no pytest — the ops/operators/test_*.py convention.
# Exit codes: 0 all pass, 1 one or more failed.

import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.normpath(os.path.join(HERE, "..")))

from goat_consistency import SchemaError, resolve  # noqa: E402
from goat_consistency import intent as intent_mod  # noqa: E402
from goat_consistency.paths import CAPS_FILE, rel  # noqa: E402

D = "@sha256:" + "0" * 64
FLEET = """\
goatnets:
  zagros:
    members: [zagros_a]
components:
  alpha:
    var: alpha_image
    fleet: "a%s"
    deviations:
      zagros:
        value: "a:tag"
        reason: rc
  beta:
    var: beta_image
    fleet: "b%s"
""" % (D, D)

FAILURES = []


def check(what, cond, detail=""):
    if not cond:
        FAILURES.append("%s %s" % (what, detail))


def tree(lite):
    d = tempfile.mkdtemp(prefix="lite-sidecar-")
    with open(os.path.join(d, "fleet-pins.yaml"), "w") as f:
        f.write(FLEET)
    if lite is not None:
        with open(os.path.join(d, "lite-pins.yaml"), "w") as f:
            f.write(lite)
    return d


def load(lite):
    d = tree(lite)
    try:
        return resolve.load_tables(os.path.join(d, "fleet-pins.yaml")), d
    finally:
        pass


def refuses(what, lite, needle):
    d = tree(lite)
    try:
        resolve.load_tables(os.path.join(d, "fleet-pins.yaml"))
    except SchemaError as e:
        check(what, needle in str(e), "refused, but not for the right reason: %s" % e)
    else:
        FAILURES.append("%s: was accepted, must be refused" % what)
    finally:
        shutil.rmtree(d)


def lite_only_checkout():
    """An open-source export declares no goatnet in fleet-pins.yaml and one in the
    sidecar: the merged table carries the lite goatnet; both empty is refused."""
    d = tempfile.mkdtemp(prefix="lite-only-")
    try:
        empty = FLEET.replace("goatnets:\n  zagros:\n    members: [zagros_a]\n", "goatnets: {}\n")
        empty = empty.replace("    deviations:\n      zagros:\n        value: \"a:tag\"\n        reason: rc\n", "")
        with open(os.path.join(d, "fleet-pins.yaml"), "w") as f:
            f.write(empty)
        with open(os.path.join(d, "lite-pins.yaml"), "w") as f:
            f.write("goatnets:\n  pasture:\n    members: [pasture_hosts]\ncomponents: {}\n")
        t = resolve.load_tables(os.path.join(d, "fleet-pins.yaml"))
        check("lite-only: the sidecar's goatnet is the table's", list(t.goatnets) == ["pasture"], str(list(t.goatnets)))
        check("lite-only: it is marked lite", "pasture" in t.lite_goatnets)
        with open(os.path.join(d, "lite-pins.yaml"), "w") as f:
            f.write("goatnets: {}\ncomponents: {}\n")
        try:
            resolve.load_tables(os.path.join(d, "fleet-pins.yaml"))
            check("both tables empty is refused", False, "was accepted")
        except (SystemExit, SchemaError, Exception) as e:  # die() exits
            check("both tables empty names the cause", "no goatnet is declared" in str(e) or isinstance(e, SystemExit), str(e))
    finally:
        shutil.rmtree(d, ignore_errors=True)


def main():
    lite_only_checkout()
    # 1. no sidecar / an empty sidecar: the fleet table alone, nothing declared lite
    for lite in (None, "goatnets: {}\ncomponents: {}\n", "---\n# header only\ngoatnets: {}\ncomponents: {}\n"):
        t, d = load(lite)
        check("empty sidecar: goatnets", set(t.goatnets) == {"zagros"})
        check("empty sidecar: deviations untouched", set(t.components["alpha"]["deviations"]) == {"zagros"})
        check("empty sidecar: no lite goatnets", t.lite_goatnets == frozenset())
        check("empty sidecar: two-tuple API", resolve.load_table(os.path.join(d, "fleet-pins.yaml"))[0] is not None)
        shutil.rmtree(d)

    # 2. a lite goatnet with deviations keyed by the goatnet and by its member
    lite = """\
goatnets:
  pasture:
    members: [pasture_hosts]
    ca_id_var: goat_ca_id
components:
  alpha:
    deviations:
      pasture:
        absent: true
        reason: tier ceiling
  beta:
    deviations:
      pasture_hosts:
        value: "b:tag"
        reason: dev rung
"""
    t, d = load(lite)
    check("merge: goatnets union", set(t.goatnets) == {"zagros", "pasture"})
    check("merge: lite set", t.lite_goatnets == frozenset({"pasture"}))
    check("merge: alpha deviations union", set(t.components["alpha"]["deviations"]) == {"zagros", "pasture"})
    check("merge: beta gained a deviations map", t.components["beta"]["deviations"]["pasture_hosts"]["value"] == "b:tag")
    check("merge: fleet value kept", t.components["beta"]["fleet"] == "b" + D)
    v, dev = resolve.resolve(t.components["alpha"], "pasture_hosts", "pasture")
    check("merge: resolve sees the lite deviation", v is None and dev["absent"] is True)
    # sources: a fleet goatnet carries fleet + caps only; a lite goatnet adds the sidecar
    s_fleet = intent_mod.sources_of("zagros", t.lite_goatnets, t.table, t.lite_table)
    s_lite = intent_mod.sources_of("pasture", t.lite_goatnets, t.table, t.lite_table)
    check("sources: fleet goatnet unchanged", set(s_fleet) == {rel(t.table), rel(CAPS_FILE)}, str(s_fleet))
    check("sources: lite goatnet carries the sidecar", set(s_lite) == {rel(t.table), rel(t.lite_table), rel(CAPS_FILE)}, str(s_lite))
    check("sources: hashes well-formed", all(v.startswith("sha256:") and len(v) == 71 for v in s_lite.values()))
    shutil.rmtree(d)

    # 3. the refusals
    refuses("goatnet in both tables", "goatnets:\n  zagros:\n    members: [x]\ncomponents: {}\n", "one table")
    refuses("component not in fleet", "goatnets:\n  p:\n    members: [p_hosts]\ncomponents:\n  gamma:\n    deviations:\n      p:\n        absent: true\n        reason: r\n",
            "component definitions live in fleet-pins.yaml")
    refuses("component definition in sidecar", "goatnets:\n  p:\n    members: [p_hosts]\ncomponents:\n  alpha:\n    var: alpha_image\n    deviations:\n      p:\n        absent: true\n        reason: r\n",
            "component definitions live in fleet-pins.yaml")
    refuses("deviation for a foreign scope", "goatnets:\n  p:\n    members: [p_hosts]\ncomponents:\n  alpha:\n    deviations:\n      zagros_a:\n        absent: true\n        reason: r\n",
            "neither a goatnet declared in this sidecar nor one of its members")
    refuses("deviation scope in both tables", "goatnets:\n  p:\n    members: [zagros]\ncomponents:\n  alpha:\n    deviations:\n      zagros:\n        absent: true\n        reason: r\n",
            "is also declared in")
    refuses("unknown top-level key", "goatnets: {}\ncomponents: {}\nextra: 1\n", "unknown top-level key")
    refuses("goatnets not a mapping", "goatnets: [a]\ncomponents: {}\n", "`goatnets:` must be a mapping")

    if FAILURES:
        print("FAIL: %d lite-sidecar test(s)" % len(FAILURES))
        for f in FAILURES:
            print("  - " + f)
        return 1
    print("ok: lite sidecar — goatnets union, deviations union, definitions refused, "
          "fleet goatnets' sources unchanged, lite goatnets carry the sidecar hash")
    return 0


if __name__ == "__main__":
    sys.exit(main())
