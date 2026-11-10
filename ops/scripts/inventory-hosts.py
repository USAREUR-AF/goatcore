#!/usr/bin/env python3
"""Print the hosts in one ansible inventory group, one per line.

Small on purpose. It exists so a driver's --plan output stays readable: an
inline heredoc in a phase command turns the plan — which is the thing a reviewer
reads before running anything — into a wall of Python.

Prints nothing and exits 0 when the group does not exist, because "this site has
no hosts yet" is the normal state before a standup's first phase, not an error.
"""
import argparse
import getpass
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def default_user() -> str:
    """The account this project reaches a machine as: deploy.ssh_user in
    ops/goatbuilder/goatbuilder.yaml, else the login name, which is what ssh
    itself uses."""
    try:
        lines = (REPO / "ops/goatbuilder/goatbuilder.yaml").read_text().splitlines()
    except OSError:
        lines = []
    inside = False
    for line in lines:
        if line[:1] not in ("", " ", "\t", "#"):
            inside = line.rstrip() == "deploy:"
        elif inside and line.strip().startswith("ssh_user:"):
            value = line.split(":", 1)[1].split("#", 1)[0].strip().strip("'\"")
            if value:
                return value
    return getpass.getuser()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("group", nargs="+", help="one or more inventory groups; hosts across all are printed")
    # A group is not tied to one file. `zagros_hosts` lives in mgmt-hosts.ini,
    # peering groups in peering-edges.ini, and several groups appear in more than
    # one (a mesh-override inventory is loaded AFTER the SSM one on purpose).
    # Assuming a single file is how this returns nothing and reads as "no hosts".
    ap.add_argument("--inventory", action="append", default=None,
                    help="an inventory file; repeatable. Default: every *.ini in "
                         "ops/ansible/inventory/")
    ap.add_argument("--ssh-target", action="store_true",
                    help="print `user@ansible_host` per host instead of the name "
                         "(user from ansible_user=, else --user); a host with no "
                         "ansible_host= is skipped. For tools that need a reachable "
                         "address, e.g. the Forgejo bot mint in goat-standup.sh")
    ap.add_argument("--user", default=default_user(),
                    help="ssh user when the inventory line carries no ansible_user=")
    a = ap.parse_args()

    files = ([Path(x) for x in a.inventory] if a.inventory
             else sorted((REPO / "ops/ansible/inventory").glob("*.ini")))

    # Hand-rolled rather than configparser: ansible inventories carry
    # `name key=value` lines that configparser reads as a key with a value, and
    # `[group:children]` sections it has no notion of.
    out = []
    group_user = None
    for f in files:
        if not f.is_file():
            continue
        want = False
        gvars = False
        for line in f.read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if line.startswith("["):
                # `[g:children]` and `[g:vars]` list groups and variables, not
                # hosts; only the bare section is a host list. The `:vars`
                # section is read for ONE key, ansible_user, because a group
                # commonly sets its ssh user there rather than per host line.
                sec = line[1:-1].strip()
                want = sec in a.group
                gvars = sec in {f"{g}:vars" for g in a.group}
                continue
            if gvars and line.startswith("ansible_user="):
                group_user = line.split("=", 1)[1].strip().strip('"')
                continue
            if want:
                parts = line.split()
                host = parts[0]
                if not re.fullmatch(r"[A-Za-z0-9_.:-]+", host):
                    continue
                if a.ssh_target:
                    kv = dict(p.split("=", 1) for p in parts[1:] if "=" in p)
                    addr = kv.get("ansible_host", "").strip('"')
                    if not addr or addr.startswith("TBD_"):
                        continue
                    # A later inventory file (the mesh override) wins, as it does
                    # for ansible: replace an earlier target for the same host.
                    tgt = f"{kv.get('ansible_user', group_user or a.user)}@{addr}"
                    out = [o for o in out if not o.startswith(f"{host} ")]
                    out.append(f"{host} {tgt}")
                elif host not in out:
                    out.append(host)
    if a.ssh_target:
        out = [o.split(" ", 1)[1] for o in out]
    print("\n".join(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
