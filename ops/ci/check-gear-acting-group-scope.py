#!/usr/bin/env python3
"""Refuse a gear-acting group that exists only at goatnet altitude.

WHY THIS EXISTS
---------------
Gear is enrolled PER SCOPE, not per goatnet. A group that acts on gear must
therefore be declared in the scope-local template, or it lands in a scope that
holds no instance of it and cannot reach the gear it was created to serve.
NetBird groups only UNION reach, so nothing rescues it after the fact, and ADR
1124 cross-scope does not either: the `HOST:` qualifier carries a FORWARD
reference from a site toward the host substrate, never the reverse.

This fault class has two shapes:

  * A group added to the goatnet-wide group sets and not the scope-local template,
    on the reasoning that a jump host is not an exercise group. Right for
    exercises, incomplete for permanent sites: gear is enrolled PER SCOPE, so
    a goatnet-scoped egress peer cannot reach gear that lives in a scope.
  * An empty scope-local INFRA group makes the scope-local `infra -> gear`
    rule permit nothing. Same shape: the rule is expressible and correctly
    written, and the membership is at the wrong altitude.

check-scope-self-sufficiency.py covers the second (an EMPTY scope group, read
off the live plane). This guard covers the first, which is static and needs no
API call: a group DECLARED at the wrong altitude in the first place.

WHAT IT CHECKS
--------------
For every goatnet that hosts scopes, for every rule whose DESTINATION includes
a gear-bearing class: each unqualified SOURCE must be declared in a scope-local
group template. A source that exists ONLY in that goatnet's own group set is a
failure -- it can never reach a scope's gear.

Validated against the real regression (NETWORK-JUMP -> NETWORK, fixed before
this repository's history begins): ops/ci/test-gear-acting-group-scope.py
rebuilds that shape on the real taxonomy and requires this guard to name it.

Exit 1 on a violation. Advisory findings are printed and do NOT fail.
"""

import re
import sys
from pathlib import Path

import yaml

TAXONOMY = "ops/ansible/netbird_acl_taxonomy.yml"
INTENTS = "ops/warrant/rule-intents/netbird-acl.yaml"

# Gear-bearing classes are the ADR 1116 gear template PLUS these. NETWORK is
# not in the gear template because it predates it (ADR 1113), but it is where
# routers, switches and radios are enrolled -- it is the class the NETWORK-JUMP
# regression was actually about. Adding a class to the gear template picks it
# up here automatically; only genuinely-outside classes belong in this list.
#
# EDGE-SERVER (ADR 1216) joins it for the same reason: it is enrolled per site,
# beside the edge devices it supports, so a source aimed at it must exist in
# the scope the server lives in.
GEAR_BEARING_EXTRAS = {"NETWORK", "EDGE-SERVER"}

# Descriptions that suggest a class holds physical gear. Advisory only: the
# prose is not a machine surface, and it has known noise in both directions --
# NETWORK-JUMP says "bearer gear" because it REACHES gear, and SENSOR/EFFECTOR
# are RESERVED AND EMPTY so they do not carry the phrase at all.
GEAR_PROSE = re.compile(r"gear class|bearer gear", re.I)


def suffixes(groups):
    return {g["suffix"] for g in groups}


def base(name):
    """Strip an ADR 1124 scope qualifier: 'HOST:INFRA' -> 'INFRA'."""
    return name.split(":", 1)[1] if ":" in name else name


def load(root):
    tax = yaml.safe_load((root / TAXONOMY).read_text())
    intents = yaml.safe_load((root / INTENTS).read_text())
    return tax, intents


def analyse(tax, intents):
    scope_groups = tax["netbird_exercise_template"]["groups"]
    gear_groups = tax["netbird_exercise_gear_template"]["groups"]

    gear_bearing = suffixes(gear_groups) | GEAR_BEARING_EXTRAS
    scope_local = suffixes(scope_groups) | suffixes(gear_groups)
    per_goatnet = {
        name: suffixes(spec["groups"])
        for name, spec in tax["netbird_acl_taxonomy"].items()
    }

    # A goatnet with no scopes under it (a flat mesh) cannot have
    # this defect -- there is no second altitude for a group to be missing from.
    hosting = {s["host_site"] for s in tax["netbird_exercises"].values()}

    violations = {}
    for goatnet, section in intents["netbird_acl_matrix"].items():
        if goatnet not in hosting:
            continue
        declared = per_goatnet.get(goatnet, set())
        for rule in section.get("rules", []):
            hit = {base(d) for d in rule.get("destinations", [])} & gear_bearing
            if not hit:
                continue
            for src in rule.get("sources", []):
                if ":" in src:
                    continue  # qualified cross-scope reference, ADR 1124
                if src in scope_local:
                    continue  # scopes instantiate it -- correct altitude
                if src not in declared:
                    continue  # not this goatnet's group at all
                key = (goatnet, src)
                violations.setdefault(key, {"dests": set(), "rules": []})
                violations[key]["dests"] |= hit
                violations[key]["rules"].append(rule.get("name", "<unnamed>"))

    # Advisory: prose says gear, derivation disagrees.
    marked = set()
    for spec in tax["netbird_acl_taxonomy"].values():
        for g in spec["groups"]:
            if GEAR_PROSE.search(g.get("description", "")):
                marked.add(g["suffix"])
    for g in scope_groups + gear_groups:
        if GEAR_PROSE.search(g.get("description", "")):
            marked.add(g["suffix"])

    return violations, sorted(marked - gear_bearing), gear_bearing


def main(argv):
    root = Path(argv[1]) if len(argv) > 1 else Path(".")
    if not (root / TAXONOMY).exists():
        print(f"cannot find {TAXONOMY} under {root}", file=sys.stderr)
        return 2

    try:
        tax, intents = load(root)
    except yaml.YAMLError as exc:
        print(f"REFUSING: {TAXONOMY} / {INTENTS} did not parse — {exc}", file=sys.stderr)
        return 2

    violations, unmarked, gear_bearing = analyse(tax, intents)

    if unmarked:
        print(
            "ADVISORY — description mentions gear, class is not treated as "
            "gear-bearing. Not a failure; confirm each is a class that REACHES "
            "gear rather than one that IS gear:"
        )
        for s in unmarked:
            print(f"  {s}")
        print()

    if violations:
        # NOT "cannot reach any gear" — that overstates it, and the NETWORK-JUMP
        # case proved why: gear enrolled at goatnet altitude IS reachable from a
        # goatnet-scoped source. The true claim is narrower.
        print(
            "GEAR-ACTING GROUP AT THE WRONG ALTITUDE — this cannot reach gear\n"
            "enrolled IN A SCOPE.\n"
        )
        for (goatnet, src), info in sorted(violations.items()):
            dests = ", ".join(sorted(info["dests"]))
            print(f"  {src}  (declared in {goatnet} only, not the scope template)")
            print(f"      aimed at gear class: {dests}")
            for name in info["rules"][:3]:
                print(f"      via rule: {name}")
            print()
        print(
            "Gear is enrolled PER SCOPE. A source declared only at goatnet\n"
            "altitude has no instance inside the scope where the gear lives, and\n"
            "ADR 1124 does not rescue it — HOST: carries a FORWARD reference from\n"
            "a site toward the host substrate, never the reverse.\n\n"
            "Fix: declare the group in netbird_exercise_template.groups and give\n"
            "it its rules in netbird_exercise_rules_template, so every scope\n"
            "inherits it."
        )
        return 1

    n = len(gear_bearing)
    print(
        f"gear-acting group scope placement: PASS "
        f"({n} gear-bearing classes; every source that aims at one is "
        f"declared scope-locally)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
