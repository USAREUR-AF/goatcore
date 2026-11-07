#!/usr/bin/env python3
"""check-alertmanager-goatnets.py: every goatnet's rendered Alertmanager config
loads, every route reaches a receiver, and the routing tree sends each tier
where its comment says.

check-alertmanager-renders.py renders the templates with minimal made-up vars
and judges the PagerDuty severity value. This one renders BOTH templates
(roles/observability and roles/obs-host) with each mgmt host's real group_vars,
through ansible itself (render-alertmanager.yml, local, no host contact), so
the whitespace control that ansible applies (trim_blocks=True) is the one that
produced the file. Ansible's trim_blocks once pulled a route's `- matchers:`
line up into the comment above it; a route without matchers matches every
alert, so that shape would send every alert, warnings included, to whatever
receiver it named.

Per rendered file:
  - `amtool check-config` accepts it (the same amtool as the fleet's image);
  - every child route has non-empty matchers and names a defined receiver;
  - `amtool config routes test`:
      severity=warning   never reaches the critical pager (critical-fanout);
      alertname=Watchdog never reaches the critical pager (it fires forever);
      severity=critical  reaches critical-fanout wherever that receiver exists.

  check-alertmanager-goatnets.py              render every mgmt host and judge
  check-alertmanager-goatnets.py FILE...      judge already-rendered files
  check-alertmanager-goatnets.py --self-test  prove RED and GREEN on fixtures

amtool comes from $AMTOOL, else PATH. Without it the check refuses (exit 2)
rather than pass on a partial read.
"""
import os
import shutil
import subprocess
import sys
import tempfile

import yaml

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
ANSIBLE = os.path.join(ROOT, "ops", "ansible")
CRITICAL = "critical-fanout"


def amtool():
    path = os.environ.get("AMTOOL") or shutil.which("amtool")
    if not path:
        print("check-alertmanager-goatnets: amtool not found ($AMTOOL or PATH); refusing to judge without it",
              file=sys.stderr)
        sys.exit(2)
    return path


def route_for(am, f, labels):
    r = subprocess.run([am, "config", "routes", "test", f"--config.file={f}"] + labels,
                       capture_output=True, text=True)
    return r.stdout.strip().splitlines()[0] if r.returncode == 0 and r.stdout.strip() else None


def problems(f, am):
    out = []
    r = subprocess.run([am, "check-config", f], capture_output=True, text=True)
    if r.returncode != 0:
        return [f"amtool check-config refuses it: {(r.stdout + r.stderr).strip().splitlines()[-1]}"]
    with open(f) as fh:
        doc = yaml.safe_load(fh) or {}
    names = {x.get("name") for x in doc.get("receivers") or []}
    for i, sub in enumerate((doc.get("route") or {}).get("routes") or []):
        if not sub.get("matchers") and not sub.get("match") and not sub.get("match_re"):
            out.append(f"route {i} has no matchers, so it matches every alert (receiver {sub.get('receiver')})")
        if "receiver" in sub and sub["receiver"] not in names:
            out.append(f"route {i} names receiver {sub['receiver']}, which is not defined")
    warn = route_for(am, f, ["severity=warning", "alertname=CheckWarning"])
    if warn == CRITICAL:
        out.append("severity=warning routes to critical-fanout: warnings would page the critical pager")
    dog = route_for(am, f, ["alertname=Watchdog", "severity=none"])
    if dog == CRITICAL:
        out.append("Watchdog routes to critical-fanout: the always-firing heartbeat would page forever")
    if CRITICAL in names:
        crit = route_for(am, f, ["severity=critical", "alertname=CheckCritical"])
        if crit != CRITICAL:
            out.append(f"severity=critical routes to {crit}, not critical-fanout")
    return out


def judge(files):
    am = amtool()
    bad = 0
    for f in files:
        for p in problems(f, am):
            print(f"FAIL {os.path.basename(f)}: {p}")
            bad += 1
    if bad:
        print(f"{bad} problem(s). Fix the template (roles/observability or roles/obs-host "
              "templates/alertmanager.yml.j2); a comment ending in a block tag eats the next line.")
        return 1
    print(f"ok: {len(files)} rendered Alertmanager config(s) load, every route has a receiver, "
          "warnings and the Watchdog never reach the critical pager")
    return 0


def render(out):
    cmd = ["ansible-playbook", "-i", "inventory/mgmt-hosts.ini", "render-alertmanager.yml",
           "-e", "target=all", "-e", f"out={out}"]
    r = subprocess.run(cmd, cwd=ANSIBLE, capture_output=True, text=True)
    if r.returncode != 0:
        sys.stdout.write(r.stdout[-4000:])
        sys.stderr.write(r.stderr[-4000:])
        print("FAIL could not render alertmanager.yml.j2 for every mgmt host")
        sys.exit(1)
    files = sorted(os.path.join(out, f) for f in os.listdir(out) if f.endswith(".yml"))
    if not files:
        print("FAIL rendered nothing; refusing to report clean on an empty read")
        sys.exit(2)
    return files


BASE = """
route:
  receiver: critical-fanout
  routes:
{routes}
receivers:
  - name: webhook-noop
    webhook_configs: [{{url: "http://localhost:9999/noop"}}]
  - name: critical-fanout
    webhook_configs: [{{url: "http://localhost:9999/crit"}}]
  - name: warning-fanout
    webhook_configs: [{{url: "http://localhost:9999/warn"}}]
  - name: deadman
    webhook_configs: [{{url: "http://localhost:9999/dead"}}]
"""
DEADMAN = '    - matchers: [\'alertname = "Watchdog"\']\n      receiver: deadman\n'
CRIT = '    - matchers: [\'severity = "critical"\']\n      receiver: critical-fanout\n'
WARN = '    - matchers: [\'severity = "warning"\']\n      receiver: warning-fanout\n'
FIXTURES = {
    # GREEN: the shape every goatnet renders today.
    "green": (DEADMAN + CRIT + WARN, 0),
    # The trim_blocks shape: the warning route lost its matchers line.
    "swallowed-matchers": (DEADMAN + CRIT + "    - receiver: warning-fanout\n", 1),
    # No warning route: warnings fall to the top-level critical-fanout.
    "warning-falls-through": (DEADMAN + CRIT, 1),
    # No deadman route: the Watchdog pages critical every repeat_interval.
    "watchdog-unrouted": (CRIT + WARN, 1),
    # A route naming a receiver nobody defined.
    "undefined-receiver": (DEADMAN + CRIT + WARN.replace("warning-fanout", "slack"), 1),
}


def self_test():
    am = amtool()
    with tempfile.TemporaryDirectory() as d:
        for name, (routes, want_bad) in FIXTURES.items():
            f = os.path.join(d, f"{name}.yml")
            with open(f, "w") as fh:
                fh.write(BASE.format(routes=routes))
            got = problems(f, am)
            assert bool(got) == bool(want_bad), (name, got)
            print(f"self-test {name}: {'RED' if got else 'GREEN'} {got[:1]}")
    print("self-test ok: four broken shapes fail, the shipped shape passes")
    return 0


if __name__ == "__main__":
    args = sys.argv[1:]
    if args == ["--self-test"]:
        sys.exit(self_test())
    if args:
        sys.exit(judge(args))
    with tempfile.TemporaryDirectory() as d:
        sys.exit(judge(render(d)))
