#!/usr/bin/env python3
"""Negative tests for check-gear-acting-group-scope.py.

A guard that cannot fail proves nothing. The load-bearing cases rebuild the
real defect it was written for: NETWORK-JUMP declared only at goatnet altitude
while a rule aims it at NETWORK gear in a scope. They use the real taxonomy
and a small rule set for the goatnet that hosts its scope, because this
repository's own intents hold only the hello-galaxy example, which hosts no
scopes.
"""

import copy
import subprocess
import sys
import tempfile
from pathlib import Path

import yaml

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
GUARD = HERE / "check-gear-acting-group-scope.py"
TAXONOMY = "ops/ansible/netbird_acl_taxonomy.yml"
INTENTS = "ops/warrant/rule-intents/netbird-acl.yaml"

FAILURES = []


def check(name, cond, detail=""):
    if cond:
        print(f"ok    {name}")
    else:
        print(f"FAIL  {name}{(' — ' + detail) if detail else ''}")
        FAILURES.append(name)


def run(root):
    p = subprocess.run(
        [sys.executable, str(GUARD), str(root)],
        capture_output=True, text=True,
    )
    return p.returncode, p.stdout + p.stderr


def stage(tax_text, intents_text):
    """Write a tree the guard can read, reusing the real repo for everything else."""
    d = Path(tempfile.mkdtemp())
    for rel, text in ((TAXONOMY, tax_text), (INTENTS, intents_text)):
        dst = d / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        dst.write_text(text)
    return d


# --- 1. the repo as it stands -------------------------------------------------
rc, out = run(ROOT)
check("the tree as committed passes", rc == 0, out.strip().splitlines()[-1] if out else "")

# --- 2. the regression shape, on the real taxonomy ---------------------------
# The guard was written for a real defect: NETWORK-JUMP declared only at goatnet
# altitude while its rule aimed at NETWORK gear enrolled in scopes. The commit
# that fixed it predates this repository's history, so the shape is rebuilt
# here: the real taxonomy (whose ex-demo scope is hosted on zagros) with a
# zagros rule from NETWORK-JUMP to NETWORK. This repository's own intents hold
# only the hello-galaxy example, which hosts no scopes, so case 1 judges no
# scope-hosting goatnet; these cases are what make the guard bite.
tax = yaml.safe_load((ROOT / TAXONOMY).read_text())
host = sorted({s["host_site"] for s in tax["netbird_exercises"].values()})[0]


def intents_with(*rules):
    return yaml.safe_dump({"netbird_acl_matrix": {host: {"rules": list(rules)}}})


JUMP_TO_NETWORK = {
    "name": "network-jump -> network (shell)",
    "sources": ["NETWORK-JUMP"],
    "destinations": ["NETWORK"],
    "protocol": "tcp",
    "action": "accept",
}

rc, out = run(stage(yaml.safe_dump(tax), intents_with(JUMP_TO_NETWORK)))
check("the fixed shape passes: NETWORK-JUMP is in the scope template", rc == 0, out.strip())

# --- 3. pull NETWORK-JUMP out of the scope template: the defect returns -------
mutated = copy.deepcopy(tax)
mutated["netbird_exercise_template"]["groups"] = [
    g for g in mutated["netbird_exercise_template"]["groups"]
    if g["suffix"] != "NETWORK-JUMP"
]
rc, out = run(stage(yaml.safe_dump(mutated), intents_with(JUMP_TO_NETWORK)))
check(
    "removing NETWORK-JUMP from the scope template fails the guard",
    rc == 1 and "NETWORK-JUMP" in out,
    f"rc={rc}",
)
check(
    "names the gear class it cannot reach",
    "aimed at gear class: NETWORK" in out and "scope template" in out,
)

# --- 4. a qualified cross-scope source is not a violation ---------------------
gear = sorted(g["suffix"] for g in tax["netbird_exercise_gear_template"]["groups"])[0]
qualified = {
    "name": "synthetic qualified source",
    "sources": ["HOST:INFRA"],
    "destinations": [gear],
    "protocol": "tcp",
    "action": "accept",
}
rc, out = run(stage(yaml.safe_dump(mutated), intents_with(qualified)))
check("an ADR 1124 qualified source (HOST:INFRA) is not flagged", rc == 0, out.strip())

intents_text = (ROOT / INTENTS).read_text()

# --- 5. malformed input is refused, not traced back ---------------------------
rc, out = run(stage("netbird_acl_taxonomy: [", intents_text))
check(
    "malformed taxonomy is refused with a message, not a traceback",
    rc == 2 and "REFUSING" in out and "Traceback" not in out,
    f"rc={rc}",
)

print()
if FAILURES:
    print(f"{len(FAILURES)} failure(s)")
    sys.exit(1)
print("gear-acting group scope guard: all negative tests pass")
