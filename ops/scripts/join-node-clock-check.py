#!/usr/bin/env python3
"""join-node-clock-check.py: is a joining node's clock fit to join a goatnet?

    join-node-clock-check.py <node> <user>@<address>

goatbuilder join-node runs this in phase 3, before anything on the node
changes (ADR 1232, docs/design/peer-fault-handling.md). A Zenoh router refuses
a timestamp more than 500 ms ahead of its own clock, and nothing tells the
machine's owner, so a node with a wrong clock is stopped before it connects.

It reads the node's own time service over one ssh session, read-only:
`chronyc -c tracking`, and `timedatectl show -p NTPSynchronized -p TimeUSec`
for when chronyd does not answer. Nothing is installed, no port is opened and
nothing is written on the node. The readings are parsed and judged by the
federation preflight's clock code (ops/federation/standup/preflight_live.py,
check `clock`, GB-268), so the two checks hold one rule: synchronized to a
reference that is not the node's own clock and, where chrony reports an
offset, an absolute offset within the budget. The rule is written once, in
docs/design/federation-preflight.md ("Clocks").

The budget is 250 ms, half the router's limit; JOIN_NODE_CLOCK_BUDGET_MS sets
it, a whole number from 1 to 500.

Exit 0: the clock passed (the offset is printed), or the time service could
not be read: the run goes on and the line says the clock was NOT verified and
which commands were tried, as join-node says of any fact it could not read.
Exit 1: refused, GB-297: not synchronized, or over the budget.
Exit 2: the budget is not usable, or the node was not reachable over ssh.
"""
from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BUDGET_VAR = "JOIN_NODE_CLOCK_BUDGET_MS"
MARK = "@@goat-clock"


def _live():
    """The federation preflight's clock code: one parser, one rule."""
    path = ROOT / "ops/federation/standup/preflight_live.py"
    spec = importlib.util.spec_from_file_location("preflight_live", path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = mod  # its dataclasses look their module up here
    spec.loader.exec_module(mod)
    return mod


def remote_command(commands):
    """One shell line for the node: each read with its exit status after a
    marker line. stderr is dropped: a missing command reads as no answer."""
    parts = ["LC_ALL=C", "export LC_ALL"]
    for name, cmd in zip(("chrony", "timedatectl"), commands):
        parts.append(f"{cmd} 2>/dev/null; echo \"{MARK} {name} rc=$?\"")
    return "; ".join(parts)


def facts_of(out):
    """The probe's output -> the facts preflight_live.read_clock reads, or None
    when the output holds no marker (the remote shell never ran the reads)."""
    facts, buf, seen = {}, [], False
    for line in out.splitlines():
        if line.startswith(MARK + " "):
            name, _, rc = line[len(MARK) + 1:].partition(" rc=")
            facts[name] = "\n".join(buf)
            facts[name + "_ok"] = rc.strip() == "0"
            buf, seen = [], True
        else:
            buf.append(line)
    return facts if seen else None


def budget(live, environ):
    return live.clock_budget({"FED_CLOCK_BUDGET_MS": environ.get(BUDGET_VAR, "")})


def judge(live, node, reading, budget_ms):
    """(exit code, lines) for one node's reading (None: not read)."""
    limit = live.CLOCK_ROUTER_LIMIT_MS
    tried = " and ".join(live.CLOCK_COMMANDS)
    if reading is None:
        return 0, [f">> clock: {node}: NOT verified, not read: no time service answered (tried {tried}); "
                   f"the clock check is left out, and the node joins unchecked. Make chronyd or systemd "
                   f"answer on {node} and run phase 3 again to check it"]
    if not reading["synchronized"]:
        measured = f"not synchronized ({reading['why']})"
    elif reading["offset_ms"] is not None and reading["offset_ms"] > budget_ms:
        measured = f"offset {reading['offset_ms']:.1f} ms from UTC, over the budget"
    else:
        if reading["offset_ms"] is None:
            how = f"synchronized ({reading['method']}; it reports no offset)"
        else:
            how = (f"offset {reading['offset_ms']:.1f} ms ({reading['method']}, stratum {reading['stratum']}, "
                   f"reference {reading['reference']})")
        return 0, [f">> clock: {node} {how}; budget {budget_ms} ms of the {limit} ms router limit"]
    return 1, [
        f"GB-297: {node}'s clock is {measured}; budget {budget_ms} ms, router limit {limit} ms "
        f"(a Zenoh router refuses a timestamp more than {limit} ms ahead of its own clock, and nothing "
        f"tells the node's owner). Nothing on {node} has changed.",
        f"fix: repair time synchronization on {node}: its time service must follow an outside reference "
        f"and hold the clock within {budget_ms} ms of it. Check with `chronyc tracking` or `timedatectl show` "
        f"on {node}, then resume with --from 3",
    ]


def main(argv, environ=os.environ, run=subprocess.run):
    if len(argv) != 2:
        print("usage: join-node-clock-check.py <node> <user>@<address>", file=sys.stderr)
        return 2
    node, target = argv
    live = _live()
    budget_ms, bad = budget(live, environ)
    if bad:
        print(f"!! {BUDGET_VAR}: {bad}")
        return 2
    try:
        p = run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", target,
                 remote_command(live.CLOCK_COMMANDS)], capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.SubprocessError) as exc:
        print(f"!! clock: could not reach {node} over ssh ({target}): {exc}")
        return 2
    facts = facts_of(p.stdout or "")
    if facts is None and p.returncode != 0:
        last = (p.stderr or "").strip().splitlines()[-1:] or [f"exit {p.returncode}"]
        print(f"!! clock: could not reach {node} over ssh ({target}): {last[0]}")
        return 2
    rc, lines = judge(live, node, live.read_clock(facts or {}), budget_ms)
    for line in lines:
        print(line)
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
