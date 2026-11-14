#!/usr/bin/env python3
"""standup-mesh-record.py — phase 8, second step: record the mesh IPs the hosts
got and re-mint the router certificate against them.

mesh-enroll-host.yml (phase 8, first step) enrols each of a site's mgmt hosts
and writes one JSON object {inventory host: mesh IP} on the operator machine.
This reads it and, for the site's hosts in inventory order:

  1. group_vars/<site>_hosts/vars.yml: mesh_ip_a = the first host's IP,
     mesh_ip_b = the second's (a pair). The cert manifest's mesh-IP SAN
     placeholders resolve from these (mint-site-certs.sh, F-544).
  2. inventory/mgmt-hosts-mesh.ini: each host's line in [<site>_hosts] gets
     ansible_host=<mesh IP>. A host with no line there keeps its current
     transport; the step says so and adds none (which door a run uses is the
     site's choice, not this step's).
  3. the cert manifest's router leaf: a literal SAN equal to the previous
     mesh_ip_a / mesh_ip_b is replaced with the new one.
  4. when anything above changed, or the router pair is missing: the router
     pair is deleted from the leaves directory and re-minted with
     `mint-site-certs.sh <site> --only router`.

Idempotent: a second run with the same IPs changes nothing and mints nothing.

mesh_ip_a and mesh_ip_b follow each host's place in the site's inventory, not
its place in the IP file, so recording one node that joins a site already
running (join-node phase 10) never moves another host's address. With
--group <group>, a node of another class (obs, breakglass, network-jump) at the
site, only its line in the mesh inventory is set: mesh_ip_a/b and the router
leaf belong to the site's core pair. With --no-mint and no cert manifest,
nothing about the certificate is done, and that is not a refusal.

usage: standup-mesh-record.py <site> --ips <file> [--group <group>] [--no-mint] [--repo <dir>]
exit:  0 recorded · 1 refused (an IP file that names no site host, or a mint failure)
"""
import argparse
import ipaddress
import json
import os
import re
import subprocess
import sys
from pathlib import Path

DEFAULT_REPO = Path(__file__).resolve().parents[2]
CGNAT = ipaddress.ip_network("100.64.0.0/10")


def site_hosts(repo, group):
    """The group's hosts in inventory order (ops/scripts/inventory-hosts.py)."""
    out = subprocess.run([sys.executable, str(repo / "ops/scripts/inventory-hosts.py"), group],
                         capture_output=True, text=True).stdout
    return [h.strip() for h in out.splitlines() if h.strip()]


def set_gv(text, key, value, note):
    """Set `key: "value"` in a group_vars file's text: replace the first
    top-level line for key, else append one. Returns (text, previous value)."""
    pat = re.compile(rf'^{re.escape(key)}:[ \t]*(.*?)[ \t]*(#.*)?$', re.M)
    m = pat.search(text)
    line = f'{key}: "{value}"   # {note}'
    if m:
        prev = m.group(1).strip().strip('"\'')
        if prev == value:
            return text, prev
        return text[:m.start()] + line + text[m.end():], prev
    sep = "" if text.endswith("\n") or not text else "\n"
    return text + sep + line + "\n", ""


def set_mesh_inventory(text, group, host, ip):
    """Point host's line in [group] at ip. Returns (text, status) where status
    is 'set', 'same' or 'absent'."""
    out, in_group, status = [], False, "absent"
    for line in text.splitlines(keepends=True):
        s = line.strip()
        if s.startswith("["):
            in_group = s == f"[{group}]"
        elif in_group and s and not s.startswith(("#", ";")) and s.split()[0] == host:
            if re.search(r'\bansible_host=\S+', line):
                new = re.sub(r'\bansible_host=\S+', f"ansible_host={ip}", line, count=1)
            else:
                new = line.rstrip("\n") + f" ansible_host={ip}" + ("\n" if line.endswith("\n") else "")
            status = "same" if new == line else "set"
            line = new
        out.append(line)
    return "".join(out), status


def replace_router_sans(text, old_to_new):
    """In the manifest's router leaf only, replace a SAN list item equal to an
    old mesh IP with the new one. Returns (text, n replaced)."""
    out, in_router, n = [], False, 0
    for line in text.splitlines(keepends=True):
        m = re.match(r'^\s*- name:\s*(\S+)', line)
        if m:
            in_router = m.group(1) == "router"
        if in_router:
            for old, new in old_to_new.items():
                if old and old != new:
                    pat = re.compile(rf'^(\s*-\s*){re.escape(old)}(\s|$)')
                    if pat.match(line):
                        line = pat.sub(lambda mm: f"{mm.group(1)}{new}{mm.group(2)}", line, count=1)
                        n += 1
        out.append(line)
    return "".join(out), n


def state_dir(repo):
    """Where this machine keeps the project's local state: `deploy.state_dir` in
    ops/goatbuilder/goatbuilder.yaml (the key goatbuilder and mint-site-certs.sh
    read), else ~/.goat/state. Read without a YAML library, block or flow style."""
    try:
        text = (Path(repo) / "ops/goatbuilder/goatbuilder.yaml").read_text()
    except OSError:
        return "~/.goat/state"
    # line by line: inside the `deploy:` block (block style) or on its own line (flow style)
    inside = False
    for line in text.splitlines():
        if re.match(r'deploy:', line):
            flow = re.search(r'\bstate_dir:[ \t]*([^,}\s]+)', line)
            if flow:
                return flow.group(1).strip('"\'')
            inside = True
            continue
        if inside:
            if line.strip() and not line[0] in " \t":
                break
            key = re.match(r'[ \t]+state_dir:[ \t]*([^\s#]+)', line)
            if key:
                return key.group(1).strip('"\'')
    return "~/.goat/state"


def leaves_dir(manifest_text, site, repo=DEFAULT_REPO):
    m = re.search(r'^out_dir:\s*(\S+)', manifest_text, re.M)
    d = m.group(1) if m else f"{state_dir(repo)}/{site}-leaves"
    return Path(os.path.expanduser(d.strip('"\'')))


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("site")
    ap.add_argument("--ips", required=True)
    ap.add_argument("--no-mint", action="store_true")
    ap.add_argument("--group", help="the node's group, when it is not the site's core group")
    ap.add_argument("--repo", default=str(DEFAULT_REPO))
    a = ap.parse_args(argv)
    repo = Path(a.repo)
    core = a.site.replace("-", "_") + "_hosts"
    group = a.group or core

    ips = json.loads(Path(a.ips).read_text())
    for h, ip in ips.items():
        if ipaddress.ip_address(ip) not in CGNAT:
            print(f"standup-mesh-record: {h} reported {ip}, which is not a mesh address (100.64.0.0/10); refusing", file=sys.stderr)
            return 1
    every = site_hosts(repo, group)
    hosts = [h for h in every if h in ips]
    if not hosts:
        print(f"standup-mesh-record: {a.ips} names no host of [{group}]; refusing", file=sys.stderr)
        return 1
    changed = False

    old = {}
    if group == core:
        gv = repo / "ops/ansible/group_vars" / group / "vars.yml"
        text = gv.read_text() if gv.exists() else ""
        for host in hosts:
            place = every.index(host)
            if place > 1:
                print(f"  note: {host} is host {place + 1} of [{group}]; mesh_ip_a / mesh_ip_b name the first two")
                continue
            key = ("mesh_ip_a", "mesh_ip_b")[place]
            text, prev = set_gv(text, key, ips[host], f"{host} wt0, recorded by standup phase 8")
            old[prev] = ips[host]
            print(f"  {key} = {ips[host]} ({host}){'' if prev != ips[host] else ' — unchanged'}")
            changed |= prev != ips[host]
        gv.parent.mkdir(parents=True, exist_ok=True)
        gv.write_text(text)

    inv = repo / "ops/ansible/inventory/mgmt-hosts-mesh.ini"
    if inv.exists():
        itext = inv.read_text()
        for host in hosts:
            itext, st = set_mesh_inventory(itext, group, host, ips[host])
            if st == "absent":
                print(f"  {host}: no line in [{group}] of inventory/mgmt-hosts-mesh.ini; its transport is unchanged")
            else:
                print(f"  {host}: mgmt-hosts-mesh.ini ansible_host={ips[host]}{' — unchanged' if st == 'same' else ''}")
            changed |= st == "set"
        inv.write_text(itext)

    if group != core:
        print(f"  [{group}] is not the site's core group: its router leaf is not this node's")
        return 0
    manifest = repo / "ops/enrollment/manifests" / f"{a.site}-certs.yaml"
    if not manifest.exists():
        if a.no_mint:
            print(f"  --no-mint: no cert manifest {manifest.relative_to(repo)}; the certificate is left alone")
            return 0
        print(f"standup-mesh-record: no cert manifest {manifest.relative_to(repo)}; nothing to re-mint", file=sys.stderr)
        return 1
    mtext, n = replace_router_sans(manifest.read_text(), old)
    if n:
        manifest.write_text(mtext)
        print(f"  router leaf: {n} SAN(s) moved to the new mesh IP")
        changed = True
    out = leaves_dir(mtext, a.site, repo)
    pair = [out / "router-cert.pem", out / "router-key.pem"]
    if a.no_mint:
        print("  --no-mint: router certificate left as it is")
        return 0
    if not changed and all(p.exists() for p in pair):
        print("  router certificate already matches the recorded mesh IPs — nothing to mint")
        return 0
    for p in pair:
        if p.exists():
            p.unlink()
    rc = subprocess.run([str(repo / "ops/scripts/mint-site-certs.sh"), a.site, "--only", "router"]).returncode
    if rc:
        print(f"standup-mesh-record: mint-site-certs.sh {a.site} --only router failed (exit {rc})", file=sys.stderr)
        return 1
    print(f"  router certificate re-minted in {out}; phase 9 deploys it")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
