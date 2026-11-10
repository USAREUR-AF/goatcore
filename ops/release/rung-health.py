#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""rung-health.py — how a component behaved on a rung since its last deploy.

Canon: docs/design/release-graduation.md §4 (the runtime-health gate, ADR 1120).
The acceptance rows prove each service ANSWERS; this reads how its containers
BEHAVED over the soak, from the host itself, read-only:

  restarts      docker RestartCount, per container, since it was created
  oom_kills     kernel OOM kills charged to the container's cgroup (the
                `oom-kill:` lines in the kernel journal, across reboots, since
                the container was created), falling back to the cgroup's own
                oom_kill counter when the journal cannot be read and the
                container never restarted; Docker's State.OOMKilled counts too
  peak memory   cgroup v2 memory.peak (v1 memory.max_usage_in_bytes), else the
                current usage (named as such), against the container's limit
                (HostConfig.Memory, else the cgroup's memory.max)

The clock starts at the newest container's Created time: a deploy recreates
the container, a restart or a reboot does not.

Hosts come from the component's connection map, the `hosts:` block of
ops/ansible/roles/<component>/deploy-history.yml (the map rollback.sh reads),
narrowed to `site: <goatnet>`. Each host is reached the way the acceptance rows
reach it: direct ssh as the map's ssh_user, through GOAT_ROWS_SSH_JUMP when a
door is open (ops/validation/probes.py), else — for a one-host map — through
the goatnet's door (ops/scripts/door-open.sh), as rollback.sh does. On the host
it runs one read-only python3 under `sudo -n` (the host-shell user is not in
the docker group): docker inspect, cgroup files, journalctl. Nothing is written
on the host.

usage:
  rung-health.py <component> --goatnet <site> [--rung NAME]
                 [--container NAME ...] [--host user@address]
                 [--write-record ops/release/records/<component>/<file>.yaml]
                 [--max-memory-fraction F] [--min-days N] [--json]

Prints the runtime_health block as JSON on stdout and a one-line verdict on
stderr. --write-record puts the block in the record under
`runtime_health.<rung>`, the field the runtime-health gate grades. Exit 0 pass,
1 fail, 2 a container could not be read (unknown), 3 usage or reach error.
"""

import argparse
import datetime as dt
import importlib.util
import json
import os
import pathlib
import re
import shlex
import subprocess
import sys

import yaml

REPO = pathlib.Path(__file__).resolve().parents[2]
SCHEMA = "rung-health-v1"


def _load(name, rel):
    spec = importlib.util.spec_from_file_location(name, REPO / rel)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


graduate = _load("graduate", "ops/release/graduate.py")

# The probe that runs on the host. Read-only: docker inspect, cgroup files,
# the kernel journal. Prints one JSON document; argv = container names.
REMOTE = r'''
import datetime, json, os, subprocess, sys
def rd(p):
    try:
        with open(p) as f:
            return f.read().strip()
    except OSError:
        return None
def num(v):
    try:
        return int(v)
    except (TypeError, ValueError):
        return None
out = []
for name in sys.argv[1:]:
    r = {"container": name}
    p = subprocess.run(["docker", "inspect", name], capture_output=True, text=True)
    if p.returncode != 0:
        r["error"] = (p.stderr.strip() or "docker inspect failed")[:300]
        out.append(r)
        continue
    d = json.loads(p.stdout)[0]
    cid = d.get("Id", "")
    st = d.get("State") or {}
    r.update(id=cid, created=d.get("Created"), started=st.get("StartedAt"),
             running=st.get("Running"), restart_count=d.get("RestartCount"),
             oom_killed_flag=st.get("OOMKilled"),
             memory_limit=(d.get("HostConfig") or {}).get("Memory") or 0,
             image=(d.get("Config") or {}).get("Image"), image_id=d.get("Image"))
    q = subprocess.run(["docker", "image", "inspect", "--format", "{{json .RepoDigests}}",
                        d.get("Image", "")], capture_output=True, text=True)
    try:
        r["repo_digests"] = json.loads(q.stdout) if q.returncode == 0 else []
    except ValueError:
        r["repo_digests"] = []
    for base, ver in (("/sys/fs/cgroup/system.slice/docker-%s.scope" % cid, 2),
                      ("/sys/fs/cgroup/docker/%s" % cid, 2),
                      ("/sys/fs/cgroup/memory/docker/%s" % cid, 1),
                      ("/sys/fs/cgroup/memory/system.slice/docker-%s.scope" % cid, 1)):
        if not os.path.isdir(base):
            continue
        r["cgroup"] = base
        if ver == 2:
            peak = num(rd(base + "/memory.peak"))
            r["memory_peak"] = peak
            r["memory_current"] = num(rd(base + "/memory.current"))
            r["cgroup_memory_max"] = num(rd(base + "/memory.max"))
            ev = rd(base + "/memory.events") or ""
        else:
            r["memory_peak"] = num(rd(base + "/memory.max_usage_in_bytes"))
            r["memory_current"] = num(rd(base + "/memory.usage_in_bytes"))
            lim = num(rd(base + "/memory.limit_in_bytes"))
            r["cgroup_memory_max"] = lim if lim is not None and lim < (1 << 60) else None
            ev = rd(base + "/memory.oom_control") or ""
        for line in ev.splitlines():
            k, _, v = line.partition(" ")
            if k == "oom_kill":
                r["cgroup_oom_kill"] = num(v)
        break
    created = (r.get("created") or "")[:19]
    try:
        since = int(datetime.datetime.strptime(created, "%Y-%m-%dT%H:%M:%S")
                    .replace(tzinfo=datetime.timezone.utc).timestamp())
        j = subprocess.run(["journalctl", "_TRANSPORT=kernel", "--no-pager", "-q", "-o", "cat",
                            "--since", "@%d" % since], capture_output=True, text=True, timeout=120)
        if j.returncode == 0:
            r["oom_kills_kernel"] = sum(1 for l in j.stdout.splitlines()
                                        if "oom-kill:" in l and cid and cid in l)
        else:
            r["kernel_log_error"] = (j.stderr.strip() or "journalctl failed")[:300]
    except Exception as e:  # noqa: BLE001 — one container's unreadable log is an unknown, not a crash
        r["kernel_log_error"] = str(e)[:300]
    out.append(r)
print(json.dumps(out))
'''


def norm(name):
    return str(name or "").replace("_", "-")


def ladder_rung(goatnet):
    """The ladder rung whose goatnet is this one (fleet-pins keys use `_`,
    connection maps use `-`; both spellings match)."""
    try:
        ladder = yaml.safe_load(open(graduate.LADDER)) or {}
    except OSError:
        return None, {}
    for r in ladder.get("rungs") or []:
        if norm(r.get("goatnet")) == norm(goatnet):
            return r.get("name"), ladder
    return None, ladder


def gate_settings(ladder, rung):
    """The runtime-health gate on the hop whose lower rung this is, or the
    defaults when the ladder carries none."""
    for h in ladder.get("hops") or []:
        if h.get("from") != rung:
            continue
        for g in h.get("gates") or []:
            if graduate.gate_id(g) == "runtime-health":
                return graduate.runtime_health_settings(h, g)
    return graduate.runtime_health_settings({"gates": []}, {})


def connection_map(component, goatnet):
    path = REPO / "ops/ansible/roles" / component / "deploy-history.yml"
    try:
        doc = yaml.safe_load(open(path)) or {}
    except OSError:
        return None, f"no connection map at {path.relative_to(REPO)}"
    hosts = [h for h in (doc.get("hosts") or []) if isinstance(h, dict)
             and norm(h.get("site")) == norm(goatnet)]
    if not hosts:
        return None, (f"{path.relative_to(REPO)} has no hosts: entry with site: {goatnet} "
                      f"— pass --host and --container")
    return hosts, None


def ssh_base(port=None):
    probes = _load("probes", "ops/validation/probes.py")
    cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=6",
           "-o", "StrictHostKeyChecking=accept-new"]
    jump = probes._ssh_jump().strip()
    if jump and port is None:
        # _ssh_jump returns "-o ProxyCommand=<quoted>"; split it as the shell would.
        cmd += shlex.split(jump)
    if port is not None:
        cmd += ["-p", str(port), "-o", "StrictHostKeyChecking=no",
                "-o", "UserKnownHostsFile=/dev/null"]
    return cmd


def door_port(goatnet):
    """Open (or reuse) the goatnet's door, as rollback.sh does; its local port or None."""
    door = REPO / "ops/scripts/door-open.sh"
    if not door.exists():
        return None
    if subprocess.run(["bash", str(door), goatnet], capture_output=True).returncode != 0:
        return None
    st = subprocess.run(["bash", str(door), goatnet, "--status"], capture_output=True, text=True)
    m = re.search(r"\(127\.0\.0\.1:(\d+)\)", st.stdout)
    return int(m.group(1)) if m else None


def run_remote(target, containers, port=None):
    remote = "sudo -n python3 -c " + shlex.quote(REMOTE) + " " + \
        " ".join(shlex.quote(c) for c in containers)
    dest = target if port is None else "%s@127.0.0.1" % target.split("@", 1)[0]
    p = subprocess.run(ssh_base(port) + [dest, remote], capture_output=True, text=True, timeout=300)
    if p.returncode != 0:
        return None, (p.stderr.strip() or f"ssh exited {p.returncode}")[:300]
    try:
        return json.loads(p.stdout.strip().splitlines()[-1]), None
    except (ValueError, IndexError):
        return None, f"unreadable probe output: {p.stdout[:200]!r}"


def iso(value):
    """Docker's nanosecond RFC 3339 → seconds, UTC, `Z`; None when unreadable."""
    try:
        t = dt.datetime.strptime(str(value)[:19], "%Y-%m-%dT%H:%M:%S")
    except (TypeError, ValueError):
        return None
    return t.strftime("%Y-%m-%dT%H:%M:%SZ")


def digests_of(c):
    out = set()
    for s in [c.get("image") or ""] + list(c.get("repo_digests") or []):
        m = graduate.DIGEST_RE.search(str(s))
        if m:
            out.add(m.group(0)[1:])
    return sorted(out)


def container_facts(host, c):
    """One container's row: the probe's raw reading folded into the four facts."""
    row = {"host": host, "container": c.get("container")}
    if c.get("error"):
        row["error"] = c["error"]
        return row
    restarts = c.get("restart_count")
    kernel = c.get("oom_kills_kernel")
    if kernel is not None:
        oom = kernel
        oom_source = "kernel journal"
    elif restarts == 0 and c.get("cgroup_oom_kill") is not None:
        # The cgroup is recreated on every start; with no restart it covers
        # the whole window since the container was created.
        oom, oom_source = c["cgroup_oom_kill"], "cgroup oom_kill counter"
    else:
        oom, oom_source = None, "unreadable (" + (c.get("kernel_log_error") or "no journal, restarted") + ")"
    if oom is not None and c.get("oom_killed_flag") and oom == 0:
        oom, oom_source = 1, oom_source + " + State.OOMKilled"
    peak, peak_source = c.get("memory_peak"), "memory.peak"
    if peak is None and c.get("memory_current") is not None:
        peak, peak_source = c["memory_current"], "current usage (no peak counter on this host)"
    if peak is not None and restarts:
        peak_source += ", since the last restart"
    limit = c.get("memory_limit") or c.get("cgroup_memory_max") or None
    frac = round(peak / limit, 4) if (peak is not None and limit) else None
    row.update({
        "id": (c.get("id") or "")[:12], "created": iso(c.get("created")),
        "started": iso(c.get("started")), "running": c.get("running"),
        "image": c.get("image"), "digests": digests_of(c),
        "restarts": restarts, "oom_kills": oom, "oom_source": oom_source,
        "memory_peak_bytes": peak, "memory_peak_source": peak_source,
        "memory_limit_bytes": limit, "peak_memory_fraction": frac,
    })
    return row


def summarise(component, goatnet, rung, rows, now):
    """The runtime_health block from the container rows. A count any container
    could not give is None for the whole block — an unknown, never a zero."""
    def total(key, agg):
        vals = [r.get(key) for r in rows]
        if not vals or any(v is None for v in vals):
            return None
        return agg(vals)
    created = [r["created"] for r in rows if r.get("created")]
    return {
        "schema": SCHEMA, "component": component, "goatnet": goatnet, "rung": rung,
        "captured": now.strftime("%Y-%m-%dT%H:%M:%SZ"),
        "deployed": max(created) if created and len(created) == len(rows) else None,
        "digests": sorted({d for r in rows for d in r.get("digests") or []}),
        "restarts": total("restarts", sum),
        "oom_kills": total("oom_kills", sum),
        "peak_memory_fraction": total("peak_memory_fraction", max),
        "containers": rows,
    }


def verdict(block, rung, candidate_ref, settings):
    max_fraction, min_days, require_limit = settings
    fails = graduate.grade_runtime_health(block, rung, candidate_ref, max_fraction,
                                          min_days, require_limit)
    if any(r.get("error") for r in block["containers"]):
        return "unknown", fails
    return ("pass" if not fails else "fail"), fails


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[1])
    ap.add_argument("component")
    ap.add_argument("--goatnet", required=True, help="the goatnet (connection-map site:) of the rung")
    ap.add_argument("--rung", default=None, help="ladder rung name (default: the rung whose goatnet this is)")
    ap.add_argument("--container", action="append", default=[], help="container name (default: the map's container_name)")
    ap.add_argument("--host", default=None, help="user@address, instead of the connection map")
    ap.add_argument("--write-record", dest="write_record", default=None,
                    help="candidate record to write runtime_health.<rung> into")
    ap.add_argument("--max-memory-fraction", dest="max_fraction", type=float, default=None)
    ap.add_argument("--min-days", dest="min_days", type=int, default=None)
    ap.add_argument("--json", action="store_true", help="JSON only, no verdict line")
    args = ap.parse_args(argv)

    rung, ladder = ladder_rung(args.goatnet)
    rung = args.rung or rung
    if args.write_record and not rung:
        print(f"rung-health: no ladder rung has goatnet {args.goatnet}; pass --rung", file=sys.stderr)
        return 3
    rung = rung or norm(args.goatnet)
    mf, md, rl = gate_settings(ladder, rung)
    settings = (args.max_fraction if args.max_fraction is not None else mf,
                args.min_days if args.min_days is not None else md, rl)

    if args.host:
        if not args.container:
            print("rung-health: --host needs --container", file=sys.stderr)
            return 3
        targets = [(args.host, args.container)]
    else:
        hosts, err = connection_map(args.component, args.goatnet)
        if err:
            print(f"rung-health: {err}", file=sys.stderr)
            return 3
        targets = [(f"{h.get('ssh_user')}@{h.get('host')}",
                    args.container or [h.get("container_name") or args.component]) for h in hosts]

    rows = []
    for target, containers in targets:
        data, err = run_remote(target, containers)
        if data is None and len(targets) == 1 and not args.host:
            port = door_port(args.goatnet)
            if port:
                print(f"rung-health: {target} not reachable directly; using the {args.goatnet} "
                      f"door (127.0.0.1:{port})", file=sys.stderr)
                data, err = run_remote(target, containers, port=port)
        if data is None:
            rows += [{"host": target.split("@", 1)[-1], "container": c, "error": err} for c in containers]
            continue
        rows += [container_facts(target.split("@", 1)[-1], c) for c in data]

    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0)
    block = summarise(args.component, args.goatnet, rung, rows, now)
    candidate_ref = None
    rec = None
    if args.write_record:
        path = pathlib.Path(args.write_record)
        if not path.is_absolute():
            path = REPO / path
        rec = yaml.safe_load(open(path)) or {}
        candidate_ref = rec.get("candidate_ref")
    state, fails = verdict(block, rung, candidate_ref, settings)
    block["verdict"] = state
    block["reasons"] = fails
    block["thresholds"] = {"max_memory_fraction": settings[0], "min_days": settings[1],
                           "require_memory_limit": settings[2]}
    if rec is not None:
        if not isinstance(rec.get("runtime_health"), dict):
            rec["runtime_health"] = {}
        rec["runtime_health"][rung] = block
        path.write_text(yaml.safe_dump(rec, sort_keys=False))
    print(json.dumps(block, indent=2))
    if not args.json:
        line = (f"rung-health: {args.component} on {rung} ({args.goatnet}): {state.upper()} — "
                f"restarts {block['restarts']}, oom_kills {block['oom_kills']}, "
                f"peak {block['peak_memory_fraction']} of limit (ceiling {settings[0]})")
        if fails:
            line += "; " + "; ".join(fails)
        if rec is not None:
            line += f"; written to runtime_health.{rung} in {args.write_record}"
        print(line, file=sys.stderr)
    return {"pass": 0, "fail": 1}.get(state, 2)


if __name__ == "__main__":
    sys.exit(main())
