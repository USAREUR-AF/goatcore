#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# c3-agreement.py: gate row C3, "status against a standing goatnet agrees with
# verify and diff for every machine", judged from the three verbs' own reports.
#
#   goatbuilder status --env <e> -o json            > status.json
#   goatbuilder diff --env <e> -o json              > diff.json
#   goatbuilder verify <component> --env <e> -o json > verify-<component>.json   (one per component)
#   c3-agreement.py --status status.json --diff diff.json --verify <component>=verify-<component>.json ...
#
# Each row is graded into one of three verdicts per (machine, component):
#   healthy      the machine's newest ledger row names the declared ref, deploy finished
#   drifted      the machine was read and is not at the declared ref, for any reason
#   unreachable  the machine was not read: unreachable, or silent where the inventory names it
# Exit 0 when the three verbs name the same machines and the same verdict for
# every (machine, component); exit 3 with each disagreement printed otherwise;
# exit 2 when a report has a row this script cannot grade. On agreement it
# prints the transport diff read over and how many machines any read reached.
#
# The same rules are the Rust test crates/goatbuilder/src/inspect.rs
# (status_agreement_tests), which runs this script on its fixture.

import argparse
import json
import sys

HEALTHY, DRIFTED, UNREACHABLE = "healthy", "drifted", "unreachable"


def load(path):
    with open(path) as fh:
        return json.load(fh)


def grade_row(detail):
    """A diff or status row's verdict, or None when the row names no machine state."""
    d = detail
    if d.startswith("same: ") or d.startswith("converged on "):
        return HEALTHY
    if d.startswith("unknown: host unreachable") or d.startswith("unknown: no answer") \
            or d.endswith(": host unreachable") or ": no answer from this host" in d:
        return UNREACHABLE
    if d.startswith("DRIFT") or d.startswith("pending: ") or d.startswith("unknown: "):
        return DRIFTED
    if d.startswith("not provisioned") or d.startswith("no ledger coverage"):
        return None
    raise ValueError(d)


def from_rows(report):
    out = {}
    for c in report.get("checks", []):
        parts = c["name"].split("/")
        if len(parts) != 3 or parts[2] == "inventory":
            continue
        v = grade_row(c["detail"])
        if v is not None:
            out[(parts[1], parts[2])] = v
    return out


def from_verify(report, component):
    out = {}
    for c in report.get("checks", []):
        parts = c["name"].split("/")
        d = c["detail"]
        if len(parts) != 2:
            continue
        if d.startswith("converged on"):
            v = HEALTHY
        elif d.startswith("unknown — host unreachable") or "no ledger row came back" in d:
            v = UNREACHABLE
        elif d.startswith("NOT converged"):
            v = DRIFTED
        else:
            continue  # preflight, transport, playbook, a group row, the verdict line
        out[(parts[1], component)] = v
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0] if __doc__ else None)
    ap.add_argument("--status", required=True)
    ap.add_argument("--diff", required=True)
    ap.add_argument("--verify", action="append", default=[], metavar="COMPONENT=FILE")
    a = ap.parse_args()
    try:
        status = from_rows(load(a.status))
        diff = from_rows(load(a.diff))
        verify = {}
        for spec in a.verify:
            comp, _, path = spec.partition("=")
            verify.update(from_verify(load(path), comp))
    except ValueError as e:
        print(f"c3: a row with no verdict: {e}", file=sys.stderr)
        return 2
    comps = {c for (_, c) in verify}
    # verify is read per component: compare only the components it was run for.
    status_v = {k: v for k, v in status.items() if k[1] in comps}
    diff_v = {k: v for k, v in diff.items() if k[1] in comps}
    missing = sorted({c for (_, c) in status} - comps)
    bad = []
    for key in sorted(set(status_v) | set(diff_v) | set(verify)):
        row = (status_v.get(key, "absent"), verify.get(key, "absent"), diff_v.get(key, "absent"))
        if len(set(row)) != 1:
            bad.append((key, row))
    machines = sorted({h for (h, _) in status_v})
    if missing:
        print(f"c3: status names component(s) verify was not run for: {' '.join(missing)}")
    for (h, c), (s, v, d) in bad:
        print(f"c3: DISAGREE {h}/{c}: status {s}, verify {v}, diff {d}")
    if bad or missing:
        return 3
    counts = {x: sum(1 for v in status_v.values() if v == x) for x in (HEALTHY, DRIFTED, UNREACHABLE)}
    # Reached: a machine at least one read got an answer from. The rest are unknown
    # under all three verbs (a transport that reaches only part of a goatnet).
    reached = sorted({h for (h, _), v in status_v.items() if v != UNREACHABLE})
    print(f"c3: transport: {transport(load(a.diff))}")
    print(f"c3: agree on {len(machines)} machine(s), {len(comps)} component(s): "
          f"{counts[HEALTHY]} healthy, {counts[DRIFTED]} drifted, {counts[UNREACHABLE]} unreachable; "
          f"reached {len(reached)} of {len(machines)} machine(s)")
    return 0


def transport(report):
    """How diff read the hosts: its `<goatnet>/transport` row."""
    for c in report.get("checks", []):
        if c["name"].endswith("/transport"):
            return c["detail"]
    return "not recorded"


if __name__ == "__main__":
    sys.exit(main())
