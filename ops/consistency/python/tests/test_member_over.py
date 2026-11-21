#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_member_over.py — a member deviation names the goatnet value it
# overrides (ADR 1256), enforced by resolve.check_member_over (called from
# resolve.validate):
#
#   GB-358  a member `value:` deviation with no `over:`, or an `over:` where it
#           means nothing (a goatnet-scope or an `absent: true` deviation);
#   GB-357  an `over:` that no longer names what the goatnet resolves to: the
#           goatnet value changed after the member deviation was written, so
#           the member would silently miss the change (a hotfix among them).
#
# and resolve.redundant_member_deviations (a warning: the member pins what
# its goatnet already resolves to).
#
# Synthetic goatnet `zagros` only. Runs under pytest (CI) and as a plain
# script (exit 0 = all pass).

import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.normpath(os.path.join(HERE, "..")))

from goat_consistency import SchemaError, resolve  # noqa: E402

OLD = "ghcr.io/example/app@sha256:" + "a" * 64
FIX = "ghcr.io/example/app@sha256:" + "b" * 64
PIN = "ghcr.io/example/app@sha256:" + "c" * 64

GOATNETS = {"zagros": {"members": ["zagros_a_hosts", "zagros_b_hosts"]}}
MEMBER_OF = {"zagros_a_hosts": "zagros", "zagros_b_hosts": "zagros"}


def comp(fleet=OLD, **devs):
    return {"app": {"var": "app_image", "fleet": fleet, "deviations": devs}}


def refused(components):
    try:
        resolve.check_member_over(GOATNETS, components, MEMBER_OF)
    except SchemaError as e:
        return str(e)
    return None


def test_the_measured_shadow_is_refused():
    # A hotfix moves the goatnet; a member deviation written before it still
    # names the old goatnet value. Before ADR 1256 this generated silently and
    # zagros_b_hosts kept PIN while the ladder reported the hotfix live.
    msg = refused(comp(
        zagros={"value": FIX, "reason": "HOTFIX (2026-11-19, ack x): fix — record r.yaml"},
        zagros_b_hosts={"value": PIN, "over": OLD, "reason": "held back for a test"}))
    assert msg and msg.startswith("GB-357"), msg
    assert "zagros_b_hosts" in msg and FIX in msg and OLD in msg, msg
    assert 'over: "%s"' % FIX in msg, "the refusal must carry the line to paste: %s" % msg


def test_a_fleet_move_after_the_member_deviation_is_refused():
    msg = refused(comp(fleet=FIX, zagros_b_hosts={"value": PIN, "over": OLD, "reason": "r"}))
    assert msg and msg.startswith("GB-357") and FIX in msg, msg


def test_a_member_value_without_over_is_refused_with_the_line_to_paste():
    msg = refused(comp(zagros_b_hosts={"value": PIN, "reason": "r"}))
    assert msg and msg.startswith("GB-358"), msg
    assert 'over: "%s"' % OLD in msg and "zagros_b_hosts" in msg, msg


def test_over_naming_the_current_goatnet_value_passes():
    assert refused(comp(
        zagros={"value": FIX, "reason": "hotfix"},
        zagros_b_hosts={"value": PIN, "over": FIX, "reason": "kept deliberately"})) is None
    assert refused(comp(zagros_b_hosts={"value": PIN, "over": OLD, "reason": "r"})) is None


def test_deleting_the_member_deviation_passes():
    assert refused(comp(zagros={"value": FIX, "reason": "hotfix"})) is None


def test_an_absent_member_is_exempt():
    assert refused(comp(zagros={"value": FIX, "reason": "hotfix"},
                        zagros_b_hosts={"absent": True, "reason": "no app here"})) is None


def test_over_on_an_absent_member_is_refused():
    msg = refused(comp(zagros_b_hosts={"absent": True, "over": OLD, "reason": "r"}))
    assert msg and msg.startswith("GB-358") and "absent" in msg, msg


def test_a_goatnet_deviation_needs_no_over_and_may_not_carry_one():
    assert refused(comp(zagros={"value": FIX, "reason": "hotfix"})) is None
    msg = refused(comp(zagros={"value": FIX, "over": OLD, "reason": "hotfix"}))
    assert msg and msg.startswith("GB-358") and "goatnet" in msg, msg


def test_over_absent_when_the_goatnet_declares_the_component_absent():
    ok = comp(zagros={"absent": True, "reason": "not run"},
              zagros_a_hosts={"value": PIN, "over": "absent", "reason": "runs here only"})
    assert refused(ok) is None
    msg = refused(comp(zagros={"absent": True, "reason": "not run"},
                       zagros_a_hosts={"value": PIN, "over": OLD, "reason": "r"}))
    assert msg and msg.startswith("GB-357") and "over: absent" in msg, msg


def test_over_none_when_fleet_is_null_and_the_goatnet_does_not_deviate():
    assert refused(comp(fleet=None,
                        zagros_a_hosts={"value": PIN, "over": "none", "reason": "r"},
                        zagros_b_hosts={"value": PIN, "over": "none", "reason": "r"})) is None
    msg = refused(comp(fleet=None, zagros_a_hosts={"value": PIN, "reason": "r"},
                       zagros_b_hosts={"value": PIN, "over": "none", "reason": "r"}))
    assert msg and msg.startswith("GB-358") and "over: none" in msg, msg


def test_a_redundant_member_deviation_is_a_warning_not_a_refusal():
    c = comp(zagros_b_hosts={"value": OLD, "over": OLD, "reason": "same as the goatnet"})
    assert refused(c) is None
    warns = resolve.redundant_member_deviations(GOATNETS, c, MEMBER_OF)
    assert len(warns) == 1 and "zagros_b_hosts" in warns[0] and "app" in warns[0], warns
    assert resolve.redundant_member_deviations(
        GOATNETS, comp(zagros_b_hosts={"value": PIN, "over": OLD, "reason": "r"}), MEMBER_OF) == []


def test_validate_runs_the_rule():
    with open(resolve.__file__) as source:
        src = source.read()
    body = src[src.index("def validate(goatnets, components):"):]
    body = body[:body.index("\n    return member_of")]
    assert "check_member_over(goatnets, components, member_of)" in body


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
            except AttributeError as e:
                failed += 1
                print("FAIL %s: %s" % (name, e))
    sys.exit(1 if failed else 0)
