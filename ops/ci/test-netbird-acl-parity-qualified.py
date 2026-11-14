#!/usr/bin/env python3
"""Negative tests for the ADR 1124 checks in check-netbird-acl-parity.py.

A guard that passes but never fails is worthless, and every check added for
ADR 1124 refuses something that would otherwise apply cleanly and silently
widen — or silently sever — a cross-scope grant. This mutates a COPY of the
real taxonomy/matrix one defect at a time and asserts the guard rejects each.

The real files carry no exercise rule templates and no gear-bearing scope, so
the copy gains a small synthetic set first: a base and a gear rules template
whose rules reach the local INFRA and `HOST:INFRA` on a port (plus one gear
rule that reaches no substrate), and a
`site-one` scope (`gear: true`) hosted on the real `zagros`. The unmutated
copy must pass the guard; each mutation below must then be refused.

Standalone (no pytest — the repo's CI runs bats and plain python); exits
non-zero if any mutation slips through. Run:

    python3 ops/ci/test-netbird-acl-parity-qualified.py

The positive case (the real files must PASS) is the guard's own CI job; this
file only proves the failures fire.
"""
from __future__ import annotations

import copy
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    import yaml
except ImportError:
    print("FAIL: PyYAML not available", file=sys.stderr)
    sys.exit(2)

REPO = Path(__file__).resolve().parents[2]
GUARD = REPO / "ops" / "ci" / "check-netbird-acl-parity.py"
ANSIBLE = REPO / "ops" / "ansible"

BASE_MAT = yaml.safe_load((ANSIBLE / "netbird_acl_matrix.yml").read_text())
BASE_TAX = yaml.safe_load((ANSIBLE / "netbird_acl_taxonomy.yml").read_text())

GEAR_T = "netbird_exercise_gear_rules_template"
BASE_T = "netbird_exercise_rules_template"
HOST = "zagros"  # the real taxonomy's scope host

BASE_MAT[BASE_T] = {"rules": [
    {"name": "site admin -> site infra (web)", "sources": ["PLATFORM-ADMIN"],
     "destinations": ["INFRA"], "protocol": "tcp", "ports": ["443"],
     "action": "accept", "bidirectional": False},
    {"name": "site infra -> host infra (web)", "sources": ["INFRA"],
     "destinations": ["HOST:INFRA"], "protocol": "tcp", "ports": ["443"],
     "action": "accept", "bidirectional": False},
]}
BASE_MAT[GEAR_T] = {"rules": [
    {"name": "gear -> host infra (web)", "sources": ["KID"],
     "destinations": ["HOST:INFRA"], "protocol": "tcp", "ports": ["443"],
     "action": "accept", "bidirectional": False},
    # Reaches no substrate: what is left when every substrate path is cut.
    {"name": "gear -> control (web)", "sources": ["KID"],
     "destinations": ["CONTROL"], "protocol": "tcp", "ports": ["443"],
     "action": "accept", "bidirectional": False},
]}
BASE_TAX["netbird_exercises"]["site-one"] = {
    "site_prefix": "SITE-ONE", "host_site": HOST, "kind": "operational", "gear": True,
}

failures: list[str] = []


def run_case(tmp: str, label: str, expect: str, mutate_mat=None, mutate_tax=None) -> None:
    mat, tax = copy.deepcopy(BASE_MAT), copy.deepcopy(BASE_TAX)
    if mutate_mat:
        mutate_mat(mat)
    if mutate_tax:
        mutate_tax(tax)
    (Path(tmp) / "netbird_acl_matrix.yml").write_text(yaml.safe_dump(mat))
    (Path(tmp) / "netbird_acl_taxonomy.yml").write_text(yaml.safe_dump(tax))
    proc = subprocess.run(
        [sys.executable, str(GUARD), tmp], capture_output=True, text=True, check=False
    )
    out = proc.stdout + proc.stderr
    if proc.returncode != 0 and expect.lower() in out.lower():
        print(f"  ok    {label}")
        return
    failures.append(label)
    print(f"  MISS  {label}", file=sys.stderr)
    print(f"        wanted non-zero exit containing {expect!r}; rc={proc.returncode}", file=sys.stderr)


# --- mutations: each is one defect the guard must refuse -------------------
def m_bidirectional(mat):
    """A bidirectional qualified rule silently creates the reverse grant."""
    for rule in mat[GEAR_T]["rules"]:
        if "HOST:INFRA" in (rule.get("destinations") or []):
            rule["bidirectional"] = True
            return


def m_dest_outside_allowlist(mat):
    """Reaching another scope's hidden backend tier."""
    mat[GEAR_T]["rules"][0]["destinations"] = ["HOST:BACKEND-INFRA"]


def m_concrete_scope_in_template(mat):
    """A shared template cannot name one scope — each has a different host."""
    mat[GEAR_T]["rules"][0]["destinations"] = [f"{HOST}:INFRA"]


def m_any_port_upward(mat):
    """Reaching out of a scope is service reach, never administration."""
    for rule in mat[GEAR_T]["rules"]:
        if "HOST:INFRA" in (rule.get("destinations") or []):
            rule["protocol"] = "all"
            rule.pop("ports", None)
            return


def m_no_substrate_path(mat):
    """A gear scope that severs every machine arriving at it."""
    for key in (BASE_T, GEAR_T):
        mat[key]["rules"] = [
            r
            for r in mat[key]["rules"]
            if not any(d == "INFRA" or ":" in d for d in (r.get("destinations") or []))
        ]


def m_site_admin_any_port_upward(mat):
    """The boundary a tactical-site scope exists to hold."""
    mat[BASE_T]["rules"].append(
        {
            "name": "platform-admin -> host-site infra (any port)",
            "description": "must be refused",
            "sources": ["PLATFORM-ADMIN"],
            "destinations": ["HOST:INFRA"],
            "protocol": "all",
            "action": "accept",
            "bidirectional": False,
        }
    )


def m_dangling_host_site(tax):
    """A HOST: reference that cannot resolve must fail at PR time, not mid-standup."""
    tax["netbird_exercises"]["site-one"]["host_site"] = "nosuchsite"


def m_colon_in_suffix(tax):
    """':' is reserved for the qualifier namespace."""
    tax["netbird_acl_taxonomy"][HOST]["groups"].append(
        {"suffix": "BAD:NAME", "description": "must be refused"}
    )


def m_bad_default_policy_action(mat):
    """A typo'd action value silently makes the site un-enforceable."""
    mat["netbird_acl_matrix"][HOST]["cutover"]["default_policy_action"] = "dsable"


CASES = [
    ("bidirectional + qualified is refused", "bidirectional: false", m_bidirectional, None),
    ("typo'd cutover.default_policy_action", "must be one of", m_bad_default_policy_action, None),
    ("qualified destination outside {INFRA}", "not a permitted qualified DESTINATION", m_dest_outside_allowlist, None),
    ("concrete scope name inside a template", "may only use the HOST: alias", m_concrete_scope_in_template, None),
    ("protocol: all on an upward rule", "explicit port list", m_any_port_upward, None),
    ("gear scope with no substrate path", "no rule reaches a substrate", m_no_substrate_path, None),
    ("site admin any-port UPWARD to the host substrate", "explicit port list", m_site_admin_any_port_upward, None),
    ("dangling host_site", "not a declared taxonomy site", None, m_dangling_host_site),
    ("taxonomy suffix containing ':'", "containing ':'", None, m_colon_in_suffix),
]

print("ADR 1124 guard negative tests — each mutation must be REFUSED:")
with tempfile.TemporaryDirectory() as tmp:
    # The unmutated fixture must pass, or every refusal below proves nothing.
    (Path(tmp) / "netbird_acl_matrix.yml").write_text(yaml.safe_dump(BASE_MAT))
    (Path(tmp) / "netbird_acl_taxonomy.yml").write_text(yaml.safe_dump(BASE_TAX))
    base = subprocess.run([sys.executable, str(GUARD), tmp], capture_output=True, text=True, check=False)
    if base.returncode != 0:
        failures.append("the unmutated fixture passes the guard")
        print("  MISS  the unmutated fixture passes the guard", file=sys.stderr)
        print("        " + (base.stdout + base.stderr).strip().replace("\n", "\n        "), file=sys.stderr)
    else:
        print("  ok    the unmutated fixture passes the guard")
    for label, expect, mm, mt in CASES:
        run_case(tmp, label, expect, mm, mt)

if failures:
    print(f"\nFAIL — {len(failures)} mutation(s) slipped through the guard:", file=sys.stderr)
    for f in failures:
        print(f"  - {f}", file=sys.stderr)
    sys.exit(1)
print(f"\nok: all {len(CASES)} ADR 1124 guard checks fire")
