#!/usr/bin/env python3
"""Our own surfaces for phase 14, read from the core and edge declarations.

acceptance.sh reads our core's REST surface, our edge router's log and our
SPIRE server. A drill names them (FED_OURS_REST, FED_EDGE_LOG_OURS,
FED_SPIRE_OURS); a deployment declares them already, in the inventory and the
group_vars the plays read:

  FED_OURS_REST      goat_zenoh_rest_host and goat_zenoh_rest_port of the core
                     group (roles/goat-zenoh), on the core host's inventory address.
                     A REST surface bound to loopback is read on the core itself,
                     so FED_SSH_OURS is derived as well (the core host's address,
                     ansible_user and ansible_port), unless it is set.
  FED_EDGE_LOG_OURS  `docker logs <goat_zenoh_edge_stack>` (roles/goat-zenoh-edge;
                     goat-zenoh-edge-<partner> when unset, as the role names it),
                     run on the edge host over ssh (EDGE_LOG_VIA).
  FED_SPIRE_OURS     `docker exec <peering_spire_container> <peering_spire_bin>`
                     (roles/goat-peering-spire), run on peering_spire_server_host
                     over ssh (SPIRE_VIA), only when the edge group sets
                     peering_spire_enabled: true.

The hosts are read from the inventories the plays run with: the peering's own
ops/ansible/inventory/peering-<edge group>.ini (rendered by `goatbuilder federate
allocate` when not every peering declares hosts:), then peering-edges.ini.

A value set in the environment is never replaced; it is an override. A value
that cannot be derived is left unset, and stderr names what was looked for and
where, so acceptance's own BLOCKED or SKIP row stands with that reason. A value
declared with a `{{ }}` template resolves only inside a play and is not guessed.

Prints shell assignments for acceptance.sh to eval, one per line, and the notes
on stderr. Exit 0 always, unless the arguments are wrong (2).
"""
from __future__ import annotations

import argparse
import os
import shlex
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import preflight_live as live  # noqa: E402

LOOPBACK = {"127.0.0.1", "localhost", "::1"}
WILDCARD = {"0.0.0.0", "::", ""}


def inventory_hosts(inv_text, group):
    """{host: {var: value}} for the hosts listed directly under [group], with the
    variables on each host's line."""
    want, hosts = False, {}
    for line in inv_text.splitlines():
        line = line.strip()
        if not line or line.startswith(("#", ";")):
            continue
        if line.startswith("["):
            want = line[1:-1].strip() == group
            continue
        if want:
            words = shlex.split(line, comments=True)
            if not words:
                continue
            hosts[words[0]] = dict(w.split("=", 1) for w in words[1:] if "=" in w)
    return hosts


def find_host(inv_text, name):
    """The variables of one inventory host, wherever it is listed."""
    for line in inv_text.splitlines():
        words = shlex.split(line.strip(), comments=True) if line.strip() and not line.strip().startswith(("#", ";", "[")) else []
        if words and words[0] == name:
            return dict(w.split("=", 1) for w in words[1:] if "=" in w)
    return None


def declared(repo, groups, role, var):
    """(value, where) from the first of `groups` that sets `var` in group_vars,
    then the role's default; (None, why) when unset or templated."""
    for g in groups:
        v = live._group_var_raw(repo, g, var)
        if v is live.TEMPLATED:
            return None, f"group_vars/{g}: {var} is templated and resolves only in a play"
        if v is not None:
            return v, f"group_vars/{g}: {var}"
    if role:
        v = live.role_default(repo, role, var)
        if v is not None:
            return v, f"roles/{role} default {var}"
    return None, f"{var} in group_vars/{groups[0]}/"


def ssh_target(repo, groups, host, hostvars):
    """ssh://[user@]address[:port] for an inventory host, from its line's
    ansible_host / ansible_user / ansible_port, then its groups' group_vars."""
    def var(name):
        if name in hostvars:
            return hostvars[name]
        v, _ = declared(repo, groups, None, name)
        return v
    addr = var("ansible_host") or host
    user = var("ansible_user")
    port = var("ansible_port")
    t = "ssh://" + (f"{user}@" if user else "") + (f"[{addr}]" if ":" in str(addr) else str(addr))
    return t + (f":{port}" if port else "")


def become(repo, groups, hostvars):
    v = hostvars.get("ansible_become")
    if v is None:
        v, _ = declared(repo, groups, None, "ansible_become")
    return str(v).lower() in ("true", "yes", "1")


def derive(repo, peering, environ, edge_group="", core_group=""):
    """([(name, value)], [note]) for the surfaces the environment does not set."""
    edge_group = edge_group or f"{peering}_edge"
    core_group = core_group or f"{peering}_core"
    edge_groups = [edge_group, "peering_edges", "all"]
    core_groups = [core_group, "peering_cores", "all"]
    # The inventories the plays run with: the peering's own rendered file
    # (`goatbuilder federate allocate`, when not every peering declares hosts:)
    # beside the peering inventory, as the driver passes them with -i.
    files = [f for f in (f"ops/ansible/inventory/peering-{edge_group}.ini",
                         "ops/ansible/inventory/peering-edges.ini") if (Path(repo) / f).is_file()]
    inv_text = "\n".join((Path(repo) / f).read_text() for f in files)
    where = " and ".join(files) or "ops/ansible/inventory/peering-edges.ini"
    out, notes = [], []

    def one_host(group):
        hosts = inventory_hosts(inv_text, group)
        if len(hosts) == 1:
            return next(iter(hosts.items())), None
        if not hosts:
            return None, f"no host under [{group}] in {where}"
        return None, f"{len(hosts)} hosts under [{group}] in {where}; one is read"

    def sudo(groups, hostvars):
        return "sudo -n " if become(repo, groups, hostvars) else ""

    if not environ.get("FED_OURS_REST"):
        core, why = one_host(core_group)
        if core is None:
            notes.append(f"FED_OURS_REST not derived: {why}")
        else:
            name, hv = core
            port, pwhere = declared(repo, core_groups, "goat-zenoh", "goat_zenoh_rest_port")
            host, hwhere = declared(repo, core_groups, "goat-zenoh", "goat_zenoh_rest_host")
            if port is None or host is None:
                notes.append(f"FED_OURS_REST not derived: {pwhere if port is None else hwhere}")
            else:
                host = str(host)
                via_ssh = bool(environ.get("FED_SSH_OURS"))
                if host in LOOPBACK and not via_ssh:
                    out.append(("FED_SSH_OURS", ssh_target(repo, core_groups, name, hv)))
                    notes.append(f"FED_SSH_OURS derived from [{core_group}] host {name}: the REST surface is bound to {host} ({hwhere}), so it is read on the core")
                    via_ssh = True
                if via_ssh and host in LOOPBACK | WILDCARD:
                    addr = "127.0.0.1"
                elif host in WILDCARD:
                    addr = str(hv.get("ansible_host") or name)
                else:
                    addr = host
                url = f"http://{'[' + addr + ']' if ':' in addr else addr}:{port}"
                out.append(("FED_OURS_REST", url))
                notes.append(f"FED_OURS_REST derived: {url} ({hwhere}, {pwhere}, [{core_group}] host {name})")

    if not environ.get("FED_EDGE_LOG_OURS"):
        edge, why = one_host(edge_group)
        if edge is None:
            notes.append(f"FED_EDGE_LOG_OURS not derived: {why}")
        else:
            name, hv = edge
            stack, swhere = declared(repo, edge_groups, None, "goat_zenoh_edge_stack")
            if stack is None and "templated" in swhere:
                notes.append(f"FED_EDGE_LOG_OURS not derived: {swhere}")
            else:
                if stack is None:
                    stack, swhere = f"goat-zenoh-edge-{peering}", "roles/goat-zenoh-edge default goat_zenoh_edge_stack"
                out.append(("FED_EDGE_LOG_OURS", f"{sudo(edge_groups, hv)}docker logs {shlex.quote(str(stack))}"))
                out.append(("EDGE_LOG_VIA", ssh_target(repo, edge_groups, name, hv)))
                notes.append(f"FED_EDGE_LOG_OURS derived: the logs of {stack} ({swhere}) on [{edge_group}] host {name}")

    if not environ.get("FED_SPIRE_OURS"):
        enabled, ewhere = declared(repo, edge_groups, "goat-peering-spire", "peering_spire_enabled")
        server, vwhere = declared(repo, edge_groups, "goat-peering-spire", "peering_spire_server_host")
        if str(enabled).lower() not in ("true", "yes", "1"):
            notes.append(f"FED_SPIRE_OURS not derived: peering_spire_enabled is not true ({ewhere})")
        elif not server:
            notes.append(f"FED_SPIRE_OURS not derived: {vwhere if server is None else 'peering_spire_server_host is empty (' + vwhere + ')'}")
        else:
            hv = find_host(inv_text, str(server))
            container, cwhere = declared(repo, edge_groups, "goat-peering-spire", "peering_spire_container")
            binary, bwhere = declared(repo, edge_groups, "goat-peering-spire", "peering_spire_bin")
            if hv is None:
                notes.append(f"FED_SPIRE_OURS not derived: peering_spire_server_host {server} ({vwhere}) is not a host in {where}")
            elif container is None or binary is None:
                notes.append(f"FED_SPIRE_OURS not derived: {cwhere if container is None else bwhere}")
            else:
                out.append(("FED_SPIRE_OURS", f"{sudo(edge_groups, hv)}docker exec {shlex.quote(str(container))} {shlex.quote(str(binary))}"))
                out.append(("SPIRE_VIA", ssh_target(repo, edge_groups, str(server), hv)))
                notes.append(f"FED_SPIRE_OURS derived: {binary} in {container} on {server} ({vwhere})")
    return out, notes


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--peering", required=True, help="the partner id; groups default to <peering>_edge and <peering>_core")
    ap.add_argument("--edge-group", default=os.environ.get("FED_EDGE_GROUP", ""))
    ap.add_argument("--core-group", default=os.environ.get("FED_CORE_GROUP", ""))
    ap.add_argument("--repo", default=str(Path(__file__).resolve().parents[3]))
    a = ap.parse_args(argv)
    out, notes = derive(a.repo, a.peering, os.environ, a.edge_group, a.core_group)
    for n in notes:
        print(f"acceptance: {n}", file=sys.stderr)
    for k, v in out:
        print(f"{k}={shlex.quote(v)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
