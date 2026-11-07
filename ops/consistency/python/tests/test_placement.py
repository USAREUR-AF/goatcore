#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_placement.py — the two placement rules of goatnet-deployment-doctrine.md
# P13, enforced by resolve.check_placement (called from resolve.validate):
#
#   1. a deviation whose reason claims single-homing is refused unless the
#      component declares `single_homed_role:`;
#   2. a component declared `read_only: true` may not declare
#      `single_homed_role:`.
#
# Runs under pytest (CI) and as a plain script (exit 0 = all pass).

import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.normpath(os.path.join(HERE, "..")))

from goat_consistency import SchemaError, resolve  # noqa: E402

D = "@sha256:" + "0" * 64


def comp(**extra):
    spec = {"var": "x_image", "fleet": "x" + D}
    spec.update(extra)
    return spec


def absent(reason):
    return {"site_b": {"absent": True, "reason": reason}}


def refused(components):
    try:
        resolve.check_placement(components)
    except SchemaError as e:
        return str(e)
    return None


def test_single_homing_reason_without_role_is_refused():
    msg = refused({"viewer": comp(deviations=absent(
        "Inspection surface, single-homed on the founder (ADR 1084 §D3)."))})
    assert msg and "single_homed_role" in msg and "viewer" in msg, msg


def test_d3_citation_without_role_is_refused():
    msg = refused({"viewer": comp(deviations=absent("Stays on node A per D3."))})
    assert msg and "single_homed_role" in msg, msg


def test_spelling_variants_are_caught():
    for reason in ("single homed on A", "Single-Homed on A", "singlehomed on A"):
        assert refused({"viewer": comp(deviations=absent(reason))}), reason


def test_single_homing_reason_with_role_is_accepted():
    assert refused({"portal": comp(single_homed_role="A", deviations=absent(
        "Admin write surface, single-homed on the founder (ADR 1084 §D3)."))}) is None


def test_other_reason_without_role_is_accepted():
    assert refused({"viewer": comp(deviations=absent(
        "Dormant standby: a served route stays absent until promotion "
        "(ADR 1111, route trigger)."))}) is None


def test_value_deviation_reason_is_checked_too():
    devs = {"site_b": {"value": "x:tag", "reason": "single-homed copy"}}
    assert refused({"viewer": comp(deviations=devs)})


def test_read_only_with_role_is_refused():
    msg = refused({"viewer": comp(read_only=True, single_homed_role="A")})
    assert msg and "read_only" in msg and "never single-homed" in msg, msg


def test_read_only_alone_is_accepted():
    assert refused({"viewer": comp(read_only=True)}) is None


def test_read_only_must_be_boolean():
    assert refused({"viewer": comp(read_only="yes")})


def test_empty_role_is_refused():
    assert refused({"viewer": comp(single_homed_role="")})


def test_validate_runs_the_rules():
    # validate() needs member group_vars directories on disk, so the wiring is
    # asserted on its source: the rules run on every validate().
    src = open(resolve.__file__).read()
    body = src[src.index("def validate(goatnets, components):"):]
    body = body[:body.index("\n    return member_of")]
    assert "check_placement(components)" in body


if __name__ == "__main__":
    failed = 0
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            try:
                fn()
                print("ok   %s" % name)
            except AssertionError as e:
                failed += 1
                print("FAIL %s: %s" % (name, e))
    sys.exit(1 if failed else 0)
