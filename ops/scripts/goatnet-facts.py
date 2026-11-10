#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Print one fact a goatnet's own configuration declares, for an operator script.

The operator scripts that act on one goatnet (the dormancy report, the Rosenpass
roster and revocation, the live canary) take the goatnet's name and read where
its management host is and what its management API is called from the files
that goatnet already carries, so no script names a deployment:

  goatnet-facts.py mgmt-ssh <goatnet>      user@mesh_ip of the goatnet's first
                                           management host in the door registry,
                                           ops/ansible/inventory/doors.yaml: the
                                           entry whose gv_group is <goatnet>_hosts
                                           and that the door lands on, else the
                                           one with router_role A, else the first
  goatnet-facts.py mgmt-url <goatnet>      https://<public_mgmt_fqdn> from
                                           ops/ansible/group_vars/<goatnet>_hosts/
  goatnet-facts.py var <goatnet> <name>    one variable from those group_vars; a
                                           list prints one item per line
  goatnet-facts.py dev-goatnet             the goatnet of the release ladder's
                                           rung named `dev` (ops/release/ladder.yaml)
  goatnet-facts.py deploy <key>            one value of this deployment's deploy
                                           block, ops/goatbuilder/project.yml
                                           (aws_profile, ssh_user, state_dir;
                                           state_dir with ~ expanded)

<goatnet> is written either way (site-a or site_a). A fact that is not declared
prints nothing and exits 1, with the file it looked in on stderr, so a caller
names its own fallback or refuses.
"""
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def load(path):
    import yaml
    try:
        return yaml.safe_load(path.read_text()) or {}
    except FileNotFoundError:
        return {}


def group_vars(goatnet):
    """Every top-level variable in group_vars/<goatnet>_hosts/ (a directory of
    files, or one <goatnet>_hosts.yml), later files overriding earlier ones as
    ansible loads them (lexical order)."""
    base = REPO / "ops/ansible/group_vars"
    name = goatnet.replace("-", "_") + "_hosts"
    out = {}
    d = base / name
    files = sorted(d.glob("*.yml")) + sorted(d.glob("*.yaml")) if d.is_dir() else []
    files += [p for p in (base / f"{name}.yml", base / f"{name}.yaml") if p.is_file()]
    for p in files:
        data = load(p)
        if isinstance(data, dict):
            out.update(data)
    return out, base / name


def mgmt_ssh(goatnet):
    reg = REPO / "ops/ansible/inventory/doors.yaml"
    doors = (load(reg).get("doors") or {})
    want = goatnet.replace("-", "_") + "_hosts"
    rows = []
    for key, door in doors.items():
        door = door or {}
        for m in door.get("mgmt") or []:
            if m.get("gv_group") == want or (not m.get("gv_group") and key.replace("-", "_") + "_hosts" == want):
                rows.append((door, m))
    if not rows:
        print(f"goatnet-facts: {reg} lists no management host with gv_group {want}", file=sys.stderr)
        return 1
    pick = (next((r for r in rows if r[1].get("landed")), None)
            or next((r for r in rows if r[1].get("router_role") == "A"), None)
            or rows[0])
    door, m = pick
    addr = m.get("mesh_ip") or m.get("host") or door.get("host")
    user = m.get("user") or door.get("user")
    if not addr or not user:
        print(f"goatnet-facts: {reg}: {m.get('name')} has no mesh_ip/host or its door no user", file=sys.stderr)
        return 1
    print(f"{user}@{addr}")
    return 0


def mgmt_url(goatnet):
    gv, where = group_vars(goatnet)
    fqdn = gv.get("public_mgmt_fqdn")
    if not isinstance(fqdn, str) or not fqdn or "{{" in fqdn:
        print(f"goatnet-facts: {where} declares no plain public_mgmt_fqdn", file=sys.stderr)
        return 1
    print(f"https://{fqdn}")
    return 0


def var(goatnet, name):
    gv, where = group_vars(goatnet)
    if name not in gv or gv[name] in (None, "", []):
        print(f"goatnet-facts: {where} declares no {name}", file=sys.stderr)
        return 1
    value = gv[name]
    for item in (value if isinstance(value, list) else [value]):
        print(item)
    return 0


def dev_goatnet():
    ladder = REPO / "ops/release/ladder.yaml"
    for rung in load(ladder).get("rungs") or []:
        if rung.get("name") == "dev" and rung.get("goatnet"):
            print(rung["goatnet"])
            return 0
    print(f"goatnet-facts: {ladder} has no rung named dev with a goatnet", file=sys.stderr)
    return 1


def deploy(key):
    proj = REPO / "ops/goatbuilder/project.yml"
    value = (load(proj).get("deploy") or {}).get(key)
    if value in (None, ""):
        print(f"goatnet-facts: {proj} declares no deploy.{key}", file=sys.stderr)
        return 1
    if key.endswith("_dir"):
        import os
        value = os.path.expanduser(str(value))
    print(value)
    return 0


def main(argv):
    if len(argv) == 2 and argv[0] == "mgmt-ssh":
        return mgmt_ssh(argv[1])
    if len(argv) == 2 and argv[0] == "mgmt-url":
        return mgmt_url(argv[1])
    if len(argv) == 3 and argv[0] == "var":
        return var(argv[1], argv[2])
    if argv == ["dev-goatnet"]:
        return dev_goatnet()
    if len(argv) == 2 and argv[0] == "deploy":
        return deploy(argv[1])
    print(__doc__.split("\n\n")[1], file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
