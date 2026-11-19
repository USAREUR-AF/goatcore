#!/usr/bin/env python3
"""joinsite_lab.py — the two-site lab of the join-site-hosts unit: a founder and a
single-host joining site, laid out from the tree under test's fixture.

    joinsite_lab.py layout <fixture> <dest> <founder-address> <joiner-address>
    joinsite_lab.py reach  <plan.json> <name>=<address>...

layout copies ops/goatbuilder/fixtures/join-node-third-site to <dest> and makes it a goatnet
of two sites: the founder alpha (its node named mgmt-alpha-01, the name standup's plays
are limited to) and the single-host joiner gamma (gamma-core-1), each at its lab address. beta and the phone site delta are taken out of
the inventory and group_vars, and alpha, which federated signal with beta, declares signal
federation off with its reason, as a founder with no second federating site does.
Nothing else of the fixture changes.

reach reads a goatbuilder -o json plan (standup or join-node --dry-run) and fails when a
command names an address of the documentation range the fixture was written with
(192.0.2.0/24), or an ansible_host, ssh target or `-l` limit that is not one of the lab's
names or addresses: every command of the plan must reach only the lab.
"""
import json
import re
import shlex
import shutil
import sys
from pathlib import Path

# standup limits the founder's plays to `mgmt-<site>-01*` (standup.rs), so the lab names the
# founder's node that way; the fixture's alpha-core-1 would match none of them.
FOUNDER, JOINER = ("alpha", "mgmt-alpha-01"), ("gamma", "gamma-core-1")
FIXTURE_FOUNDER_NODE = "alpha-core-1"
DROPPED_GROUPS = ("beta_hosts", "delta_hosts", "class_phone")
ALPHA_FEDERATION_KEYS = (
    "federation_peers_for_a", "signal_federation_proxy_enabled", "signal_federation_ingress_enabled",
    "federation_public_relays", "signal_federation_token", "signal_federation_ingress_fqdn",
    "federation_peer_mesh_addr", "federation_peer_public_sni",
)
FIXTURE_NET = re.compile(r"\b192\.0\.2\.\d{1,3}\b")
USER_AT_HOST = re.compile(r"[A-Za-z0-9_<>.-]+@([A-Za-z0-9_.<>-]+)")


def words(cmd):
    """The command's words, with a quoted argument (bash -c '...') split into its own words."""
    try:
        parts = shlex.split(cmd)
    except ValueError:
        parts = cmd.split()
    out = []
    for p in parts:
        out.extend(words(p) if any(c.isspace() for c in p) else [p])
    return out


def ssh_targets(cmd):
    """The host of each ssh's first user@host word. A word scan, not a nested regex."""
    ws = words(cmd)
    hosts = []
    for i, w in enumerate(ws):
        if w.rsplit("/", 1)[-1] != "ssh":
            continue
        for nxt in ws[i + 1:]:
            m = USER_AT_HOST.fullmatch(nxt)
            if m:
                hosts.append(m.group(1))
                break
    return hosts


def layout(fixture, dest, founder_addr, joiner_addr):
    fixture, dest = Path(fixture), Path(dest)
    if dest.exists():
        shutil.rmtree(dest)
    shutil.copytree(fixture, dest, symlinks=True)
    (dest / "README.md").unlink(missing_ok=True)
    inv = dest / "ops/ansible/inventory/mgmt-hosts.ini"
    inv.write_text(
        "# The join-site-hosts lab: the founder and one single-host joining site.\n"
        f"[{FOUNDER[0]}_hosts]\n{FOUNDER[1]} ansible_host={founder_addr}\n\n"
        f"[{JOINER[0]}_hosts]\n{JOINER[1]} ansible_host={joiner_addr}\n"
    )
    gv = dest / "ops/ansible/group_vars"
    for g in DROPPED_GROUPS:
        shutil.rmtree(gv / g, ignore_errors=True)
        (gv / f"{g}.yml").unlink(missing_ok=True)
    mesh = dest / "ops/ansible/inventory/mgmt-hosts-mesh.ini"
    if mesh.exists():
        keep, section = [], None
        for line in mesh.read_text().splitlines():
            m = re.match(r"\[([^\]]+)\]", line)
            if m:
                section = m[1]
            if section not in DROPPED_GROUPS:
                keep.append(re.sub(rf"^{FIXTURE_FOUNDER_NODE}\b", FOUNDER[1], line))
        mesh.write_text("\n".join(keep).rstrip("\n") + "\n")
    alpha = gv / f"{FOUNDER[0]}_hosts/vars.yml"
    lines, skip = [], False
    for line in alpha.read_text().splitlines():
        key = line.split(":", 1)[0] if line and not line[0].isspace() and not line.startswith("#") else None
        if key is not None:
            skip = key in ALPHA_FEDERATION_KEYS
        elif line.startswith("#"):
            skip = False
        if not skip:
            lines.append(line)
    lines.append("signal_federation_absent_reason: the lab's founder has no second site that federates signal with it")
    alpha.write_text("\n".join(lines) + "\n")
    print(f"join-site lab: {FOUNDER[1]} at {founder_addr}, {JOINER[1]} at {joiner_addr}, under {dest}")


def reach(plan_path, pairs):
    names = {}
    for p in pairs:
        n, _, a = p.partition("=")
        names[n] = a
    allowed = set(names) | set(names.values())
    with open(plan_path, encoding="utf-8") as fh:
        plan = json.load(fh)
    cmds = [c.get("detail") or "" for c in plan.get("checks") or [] if c.get("name") == "cmd"]
    bad = []
    for cmd in cmds:
        for hit in FIXTURE_NET.findall(cmd):
            bad.append(f"the fixture's address {hit}: {cmd[:160]}")
        for target in re.findall(r"(?:^|\s)-l\s+'?([A-Za-z0-9_.*:-]+)", cmd) + \
                re.findall(r"(?:^|\s)--limit\s+'?([A-Za-z0-9_.*:-]+)", cmd) + \
                ssh_targets(cmd):
            if target.strip("'*") not in allowed and not target.startswith("<"):
                bad.append(f"a target outside the lab, {target}: {cmd[:160]}")
    for b in bad:
        print(f"OUTSIDE  {b}")
    print(f"{len(cmds)} commands; {len(bad)} reach outside the lab ({', '.join(sorted(allowed))})")
    return 1 if bad or not cmds else 0


def main(argv):
    if len(argv) >= 6 and argv[1] == "layout":
        layout(*argv[2:6])
        return 0
    if len(argv) >= 4 and argv[1] == "reach":
        return reach(argv[2], argv[3:])
    print(__doc__.strip().splitlines()[3].strip(), file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
