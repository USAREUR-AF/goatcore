#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_opt_in.py — an opt-in component (`opt_in: true`) resolves only for a
# goatnet whose row names it under `opt_in:` (resolve.apply_opt_in, opted_out;
# goatbuilder's pins.rs holds the same rules, and
# ops/goatbuilder/parity-pins-opt-in.sh holds the two avenues equal).
#
# Runs under pytest (CI) and as a plain script (exit 0 = all pass).

import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.normpath(os.path.join(HERE, "..")))

from goat_consistency import SchemaError, resolve  # noqa: E402

D = "@sha256:" + "0" * 64


def table(opt_in=None, map_spec=None):
    goatnets = {"g": {"members": ["m"]}, "h": {"members": ["n"]}}
    if opt_in is not None:
        goatnets["g"]["opt_in"] = opt_in
    comps = {"map": dict({"var": "map_image", "opt_in": True, "fleet": "map" + D}, **(map_spec or {})),
             "plain": {"var": "plain_image", "fleet": "plain" + D}}
    return goatnets, comps


def refused(goatnets, comps):
    try:
        resolve.apply_opt_in(goatnets, comps)
    except SchemaError as e:
        return str(e)
    return None


def test_only_a_goatnet_that_opts_in_resolves_it():
    goatnets, comps = table(opt_in=["map"])
    resolve.apply_opt_in(goatnets, comps)
    assert resolve.resolve(comps["map"], "m", "g") == ("map" + D, None)
    assert resolve.resolve(comps["map"], "n", "h") == (None, None)
    assert resolve.opted_out(comps["map"], "h") and not resolve.opted_out(comps["map"], "g")
    assert resolve.resolve(comps["plain"], "n", "h") == ("plain" + D, None)


def test_no_goatnet_opts_in():
    goatnets, comps = table()
    resolve.apply_opt_in(goatnets, comps)
    assert resolve.resolve(comps["map"], "m", "g") == (None, None)


def test_a_member_of_an_opted_in_goatnet_may_be_absent():
    goatnets, comps = table(opt_in=["map"], map_spec={"deviations": {"m": {"absent": True, "reason": "r"}}})
    resolve.apply_opt_in(goatnets, comps)
    value, dev = resolve.resolve(comps["map"], "m", "g")
    assert value is None and dev["absent"] is True


def test_its_var_is_governed_only_where_a_goatnet_opted_in():
    # A goatnet that did not opt in may set the var by hand (it runs the play by
    # hand): that is not a hand pin shadowing a generated one.
    goatnets, comps = table(opt_in=["map"])
    resolve.apply_opt_in(goatnets, comps)
    assert resolve.governed_vars_for(comps, "g") == {"map_image", "plain_image"}
    assert resolve.governed_vars_for(comps, "h") == {"plain_image"}


def test_the_list_names_only_opt_in_components():
    assert "which is not an opt-in component" in refused(*table(opt_in=["plain"]))
    assert "which is not a component" in refused(*table(opt_in=["nope"]))
    assert "must be a list of component names" in refused(*table(opt_in="map"))


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
