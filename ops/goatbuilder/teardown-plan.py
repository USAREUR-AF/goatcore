#!/usr/bin/env python3
"""teardown-plan.py — a site's teardown, from its standup plan and
ops/goatbuilder/teardown.yaml, for ops/scripts/goat-teardown.sh (the scripts
avenue of `goatbuilder teardown`; the parity gate compares the two plans).

usage: teardown-plan.py <site> [--profile lan|internet] [--from N] [--to N]
                        [--services] [--plan-file FILE] [--commands]

Prints, in reverse standup order, one block per applied step:
    === Teardown of phase N [A] <label> ===
        <action>: <why or what>
        <command>                       (runnable steps only)
and a closing verdict line. --commands prints only the commands to run, one
per line, in order (empty when the teardown is refused). Exit 0 runnable,
3 refused (a reached step destroys data, is not built, or is undeclared).
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    sys.exit("teardown-plan: needs PyYAML (pip3 install pyyaml)")

REPO = Path(__file__).resolve().parents[2]
TAGS = ("[internet] ", "[lan] ", "[joiner] ")


def strip_tags(label):
    for t in TAGS:
        if label.startswith(t):
            label = label[len(t):]
    return label


def declaration(decls, label):
    l = strip_tags(label)
    for d in decls:
        if l.startswith(d["label"]):
            return d
    return None


def step_group(site, d):
    """The inventory group a step runs against: the core group, or the obs
    group for a step declared `target: obs`."""
    base = site.replace("-", "_")
    return f"{base}_obs_hosts" if d.get("target") == "obs" else f"{base}_hosts"


def step_cmd(prefix, site, d):
    """The command that takes a runnable step down, or None for keep/refused/designed."""
    group = step_group(site, d)
    act = d["action"]
    if act == "compose-down":
        dirs = json.dumps([{"var": x["var"], "default": x["default"].replace("<site>", site)} for x in d.get("dirs", [])], separators=(",", ":"))
        return f"{prefix} teardown-compose.yml -e target={group} -e teardown_component={d['component']} -e 'teardown_dirs={dirs}'"
    if act == "service-stop":
        svcs = json.dumps(d.get("services", []), separators=(",", ":"))
        return f"{prefix} teardown-service-stop.yml -e target={group} -e teardown_component={d['component']} -e teardown_project={d['project']} -e 'teardown_services={svcs}'"
    if act == "systemd-stop":
        units = json.dumps(d.get("units", []), separators=(",", ":"))
        return f"{prefix} teardown-systemd.yml -e target={group} -e teardown_component={d['component']} -e 'teardown_units={units}'"
    if act == "command":
        return d["run"].replace("{repo}", str(REPO)).replace("{site}", site)
    return None


SERVICE_ACTIONS = ("compose-down", "service-stop", "systemd-stop")
WHAT = {"compose-down": "stop {c}, volumes kept",
        "service-stop": "stop {c} in its shared compose project, containers and volumes kept",
        "systemd-stop": "stop and disable {c}, unit files kept",
        "command": "{c} on the operator machine"}
# The background observability legs, and the phase each starts with.
LEG_PHASE = {"host": 2, "stack": 9}


def phase_arg(v):
    """A standup phase number 1..14. A lettered id is from the numbering before
    2026-10-29; refuse it with the phase it became (goatbuilder's own words)."""
    moved = {"0": "phases start at 1 now; the gates are phase 1, the host baseline phase 2",
             "0b": "phases start at 1 now; the gates are phase 1, the host baseline phase 2",
             "6b": "phase 6b is now part of phase 5", "7a": "phase 7a is now part of phase 6",
             "7b": "phase 7b is now part of phase 6", "7c": "phase 7c is now part of phase 6",
             "9b": "phase 9b is now part of phase 7", "9c": "phase 9c is now part of phase 7",
             "12m": "the first-login step is now phase 11"}
    if v in moved:
        raise argparse.ArgumentTypeError(moved[v])
    if not v.isdigit() or not 1 <= int(v) <= 14:
        raise argparse.ArgumentTypeError('want a phase number 1..14 (docs/operations/goatnet-standup.md §"Phase numbers")')
    return int(v)


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("site")
    ap.add_argument("--profile")
    ap.add_argument("--from", dest="frm", type=phase_arg, default=1)
    ap.add_argument("--to", type=phase_arg, default=14)
    ap.add_argument("--services", action="store_true")
    ap.add_argument("--plan-file")
    ap.add_argument("--commands", action="store_true")
    a = ap.parse_args(argv)

    if a.plan_file:
        text = Path(a.plan_file).read_text()
    else:
        cmd = [str(REPO / "ops/scripts/goat-standup.sh"), a.site, "--plan"] + (["--profile", a.profile] if a.profile else [])
        text = subprocess.run(cmd, capture_output=True, text=True).stdout
    decls = (yaml.safe_load((REPO / "ops/goatbuilder/teardown.yaml").read_text()) or {}).get("steps") or []

    m = re.search(r"(cd '[^']+/ops/ansible' && .*?ansible-preflight\.sh -i inventory/mgmt-hosts\.ini -i inventory/mgmt-hosts-mesh\.ini)", text)
    prefix = m.group(1) if m else f"cd '{REPO}/ops/ansible' && ansible-playbook -i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini"

    # Phases ([A] only) and the observability legs, in plan order.
    applied = []
    for h in re.finditer(r'^=== (?:Phase (\d+) \[A\]|Observability leg "(\w+)" \[A ∥\]) (.*?) ===\n(.*)$', text, re.M):
        if h.group(4).strip().startswith("SKIPPED"):
            continue   # a rescue class the site does not declare: never applied, nothing to take down
        n = int(h.group(1)) if h.group(1) else LEG_PHASE.get(h.group(2), 2)
        head = f"phase {n} [A]" if h.group(1) else f'observability leg "{h.group(2)}" [A ∥]'
        if a.frm <= n <= a.to:
            applied.append((n, head, h.group(3)))

    out, cmds, blocking = [], [], []
    for n, head, label in reversed(applied):
        d = declaration(decls, label)
        out.append(f"=== Teardown of {head} {label} ===")
        tag = f"{n} {label}"
        if d is None:
            out.append("    undeclared: no line in ops/goatbuilder/teardown.yaml says how this step comes down")
            blocking.append(f"{tag} (undeclared)")
            continue
        act = d["action"]
        c = step_cmd(prefix, a.site, d)
        if c is not None:
            out.append(f"    {act}: " + WHAT[act].format(c=d["component"]) + (f" ({d['why']})" if d.get("why") else ""))
            out.append(f"    {c}")
            if act in SERVICE_ACTIONS or not a.services:
                cmds.append(c)
            else:
                blocking.append(f"{tag} ({act})")
        else:
            out.append(f"    {act}: {d.get('why', '')}")
            if act in ("refused", "designed"):
                blocking.append(f"{tag} ({act})")

    if blocking and not a.services:
        verdict = (f"refused: {len(blocking)} step(s) cannot come down by this teardown (data, not built, or undeclared): "
                   + "; ".join(blocking) + ". --services stops the services only and leaves those in place.")
        rc = 3
        cmds = []
    else:
        left = f"; {len(blocking)} step(s) left in place" if blocking else ""
        verdict = f"{len(cmds)} step(s) come down, volumes kept{left}. Records go to teardown.jsonl; deploy-history.jsonl is untouched."
        rc = 0
    out.append(f"=== Teardown verdict: {verdict} ===")

    if a.commands:
        for c in cmds:
            print(c)
    else:
        print("\n".join(out))
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
