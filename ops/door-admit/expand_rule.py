#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Expand an external service's `admit:` rule into the concrete lists door-admit checks (ADR 1254).

  expand_rule.py --registry ops/identity/orgs/<env>.yaml --admit '<json>' --service <name> [--emails-file F] > rules.json

`admit` (from the declaration):
  orgs:    [org id, ...]   the organization and every descendant (by `parent`) in the registry
  nations: [ISO 3166 alpha-3, ...]   every organization whose `nation` is one of these, and their descendants
  groups:  [group name, ...]   passed through
  emails_ref: a secret-store reference; the play resolves it and passes --emails-file

Output aff_ids are canonical organization ids plus their aliases (an identity's `aff.id`
may still carry an alias until the console's reconciliation folds it in). Fails when a
named organization or nation matches nothing in the registry, so a typo cannot silently
admit no one. Needs PyYAML (present wherever ansible runs).
"""
import argparse
import json
import sys

import yaml


def expand(orgs_doc, admit):
    orgs = [o for o in orgs_doc.get("orgs", []) if o.get("status", "active") == "active"]
    by_id = {o["id"]: o for o in orgs}
    children = {}
    for o in orgs:
        if o.get("parent"):
            children.setdefault(o["parent"], []).append(o["id"])

    def descend(root):
        out, stack = set(), [root]
        while stack:
            cur = stack.pop()
            if cur in out:
                continue
            out.add(cur)
            stack.extend(children.get(cur, []))
        return out

    ids, problems, matched = set(), [], {}
    for name in admit.get("orgs", []) or []:
        if name not in by_id:
            problems.append(f"organization {name!r} is not in the registry")
            continue
        got = descend(name)
        ids |= got
        matched[f"org:{name}"] = sorted(got)
    for nation in admit.get("nations", []) or []:
        roots = [o["id"] for o in orgs if (o.get("nation") or "").upper() == nation.upper()]
        if not roots:
            problems.append(f"no organization in the registry has nation {nation!r}")
            continue
        got = set()
        for r in roots:
            got |= descend(r)
        ids |= got
        matched[f"nation:{nation}"] = sorted(got)
    if problems:
        raise ValueError("; ".join(problems))
    with_aliases = set(ids)
    for i in ids:
        with_aliases |= set(by_id[i].get("aliases", []) or [])
    return sorted(with_aliases), matched


def rule_text(admit):
    parts = []
    if admit.get("orgs"):
        parts.append("members of " + ", ".join(admit["orgs"]) + " and its sub-organizations")
    if admit.get("nations"):
        parts.append("members of any organization from " + ", ".join(admit["nations"]))
    if admit.get("groups"):
        parts.append("the groups " + ", ".join(admit["groups"]))
    if admit.get("emails_ref"):
        parts.append("people on this service's allowlist")
    return "; ".join(parts) or "nobody (the rule is empty)"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--registry", required=True)
    ap.add_argument("--admit", required=True, help="the declaration's admit block, as JSON")
    ap.add_argument("--service", required=True)
    ap.add_argument("--contact", default="")
    ap.add_argument("--emails-file", default=None, help="one e-mail per line")
    a = ap.parse_args()
    admit = json.loads(a.admit)
    with open(a.registry) as f:
        doc = yaml.safe_load(f)
    try:
        aff_ids, matched = expand(doc, admit)
    except ValueError as e:
        sys.exit(f"expand_rule: {e}")
    emails = []
    if a.emails_file:
        with open(a.emails_file) as f:
            emails = sorted({l.strip().lower() for l in f if l.strip() and not l.startswith("#")})
    json.dump({"service": a.service, "rule_text": rule_text(admit), "contact": a.contact,
               "admit": {"aff_ids": aff_ids, "groups": sorted(admit.get("groups", []) or []), "emails": emails},
               "matched": matched}, sys.stdout, indent=1)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
