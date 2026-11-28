#!/usr/bin/env python3
"""standup-relay-health-record.py — phase 8: record the mesh address the goatnet's
relay-health box got.

mesh-enroll-host.yml enrols the box (group <site>_relay_health in
inventory/<site>-relay-health.ini) and writes {inventory host: mesh IP} on the
operator machine. This reads it and writes the address in the two places that
name it:

  1. the box's own line in inventory/<site>-relay-health.ini: mesh_ip_a=<IP>
     (what the box's services bind, and what a heartbeat URL is matched to);
  2. group_vars/<site>_hosts/vars.yml: relay_health_mesh_ip, which the site's
     obs hosts read for the address of the off-site dead-man watcher.

Idempotent. The files it writes are recorded as the stand-up's own
(standup_written.py), so the next play of the run is not refused for them.

usage: standup-relay-health-record.py <site> --ips <file> [--repo <dir>]
exit:  0 recorded, or nothing to record (no box declared) · 1 refused
"""
import argparse
import ipaddress
import json
import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True
DEFAULT_REPO = Path(__file__).resolve().parents[2]
CGNAT = ipaddress.ip_network("100.64.0.0/10")


def set_host_var(text, group, host, key, value):
    """Set key=value on host's line in [group]. Returns (text, found)."""
    out, in_group, found = [], False, False
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("["):
            in_group = s == f"[{group}]"
        elif in_group and s and not s.startswith(("#", ";")) and s.split()[0] == host:
            found = True
            if re.search(rf"\b{re.escape(key)}=\S+", line):
                line = re.sub(rf"\b{re.escape(key)}=\S+", f"{key}={value}", line)
            else:
                line = f"{line.rstrip()} {key}={value}"
        out.append(line)
    return "\n".join(out) + ("\n" if text.endswith("\n") else ""), found


def set_gv(text, key, value, note):
    pat = re.compile(rf'^{re.escape(key)}:[ \t]*(.*?)[ \t]*(#.*)?$', re.M)
    line = f'{key}: "{value}"   # {note}'
    if pat.search(text):
        return pat.sub(lambda _m: line, text, count=1)
    return text.rstrip("\n") + f"\n\n{line}\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("site")
    ap.add_argument("--ips", required=True)
    ap.add_argument("--repo", default=str(DEFAULT_REPO))
    a = ap.parse_args()
    repo = Path(a.repo)
    group = a.site.replace("-", "_") + "_relay_health"
    inv = repo / "ops/ansible/inventory" / f"{a.site}-relay-health.ini"
    if not inv.is_file():
        print(f"standup-relay-health-record: no {inv.relative_to(repo)}: this goatnet declares no relay-health box; nothing to record")
        return 0
    try:
        ips = json.loads(Path(a.ips).read_text())
    except (OSError, ValueError) as e:
        print(f"standup-relay-health-record: cannot read {a.ips}: {e}", file=sys.stderr)
        return 1
    if not isinstance(ips, dict) or not ips:
        print(f"standup-relay-health-record: {a.ips} names no host; refusing", file=sys.stderr)
        return 1
    text, wrote = inv.read_text(), []
    first = None
    for host, ip in ips.items():
        try:
            ok = ipaddress.ip_address(ip) in CGNAT
        except ValueError:
            ok = False
        if not ok:
            print(f"standup-relay-health-record: {host}: {ip!r} is not a mesh address; refusing", file=sys.stderr)
            return 1
        text, found = set_host_var(text, group, host, "mesh_ip_a", ip)
        if not found:
            print(f"standup-relay-health-record: {host} has no line in [{group}] of {inv.name}; refusing", file=sys.stderr)
            return 1
        first = first or ip
        print(f"  {host}: mesh_ip_a={ip} in {inv.name}")
    if text != inv.read_text():
        inv.write_text(text)
        wrote.append(str(inv.relative_to(repo)))
    gv = repo / "ops/ansible/group_vars" / (a.site.replace("-", "_") + "_hosts") / "vars.yml"
    if gv.is_file():
        new = set_gv(gv.read_text(), "relay_health_mesh_ip", first,
                     "the goatnet's relay-health box on the mesh, recorded by standup phase 8")
        if new != gv.read_text():
            gv.write_text(new)
            wrote.append(str(gv.relative_to(repo)))
        print(f"  relay_health_mesh_ip: {first} in {gv.relative_to(repo)}")
    if wrote:
        sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
        try:
            import standup_written
            standup_written.record(str(repo), wrote)
        except Exception as e:  # noqa: BLE001 - the record is a convenience; say so and go on
            print(f"  note: could not record the writes as the stand-up's own ({e})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
