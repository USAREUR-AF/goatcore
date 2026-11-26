#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_one_host.py — `one_host_var:` names the group variable that names the one
# host of each member group a component runs on (resolve.check_one_host;
# goatbuilder's pins.rs check_one_host refuses in the same words).
#
# Runs under pytest (CI) and as a plain script (exit 0 = all pass).

import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.normpath(os.path.join(HERE, "..")))

from goat_consistency import SchemaError, resolve  # noqa: E402


def refused(spec):
    try:
        resolve.check_one_host("map", spec)
    except SchemaError as e:
        return str(e)
    return ""


def test_a_variable_name_passes():
    assert refused({"one_host_var": "map_host"}) == ""
    assert refused({}) == ""


def test_anything_else_is_refused():
    for bad in ("Map-Host", 3, ""):
        assert ("component 'map': `one_host_var:` is the name of a group variable that names "
                "one host, such as example_host") in refused({"one_host_var": bad}), bad


def test_not_beside_a_single_homed_role():
    assert ("component 'map' declares both `one_host_var:` and `single_homed_role:`; a component "
            "runs on one host of each group by one of them") in refused(
        {"one_host_var": "map_host", "single_homed_role": "A"})


if __name__ == "__main__":
    failed = 0
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            try:
                fn()
                print("ok   " + name)
            except AssertionError as e:
                failed += 1
                print("FAIL %s: %s" % (name, e))
    sys.exit(1 if failed else 0)
