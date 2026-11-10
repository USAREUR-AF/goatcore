#!/usr/bin/env python3
"""A management host is addressed at the mesh IP the door registry declares; a bad-day address is used nowhere else.

Two declarations decide the question, and this gate reads both:

  * ops/ansible/inventory/doors.yaml, `doors.<goatnet>.mgmt[].mesh_ip` — the ONE address a management
    host has on its goatnet's mesh. Every inventory row that names the host must carry it (or 127.0.0.1,
    a door landing; or a LAN address, a bootstrap row).
  * ops/ansible/group_vars/<group>/vars.yml, `operator_ssh_scopes.breakglass-path[].ssh_host` — the
    bad-day addresses (ADR 1114: a node's Tailscale or LAN side, for when the mesh is down). A bad-day
    address appearing ANYWHERE else in ops/ is the defect of 2026-10-27: nine files addressed the
    zagros-dev Thor at its Tailscale address, so every deployer not on that one operator's tailnet merged
    a pin and reached no host.

Comments are ignored (history may name any address); recorded evidence under ops/validation/evidence is
not scanned. Exit 0 ok; 1 with every offender printed; 2 when the declarations cannot be read.

usage: check-goatnet-addresses.py [--repo DIR] [--self-test]
"""
import argparse, glob, ipaddress, os, re, sys, tempfile
from pathlib import Path

try:
    import yaml
except ImportError:
    print("check-goatnet-addresses: PyYAML missing (pip install pyyaml)", file=sys.stderr)
    sys.exit(2)

ADDR = re.compile(r"(?<![\d.])(\d{1,3}(?:\.\d{1,3}){3})(?![\d.])")
ROW = re.compile(r"^(mgmt-[a-z0-9-]+-\d{2}-[a-z0-9]+)\b.*\bansible_host=([^\s\"']+)")
SCAN_DIRS = ("ops/ansible", "ops/validation", "ops/scripts", "ops/goatnets", "ops/serve", "ops/release", "ops/dns")
SKIP_DIRS = ("ops/validation/evidence",)
EXT = (".yml", ".yaml", ".ini", ".sh", ".py", ".json", ".toml", ".cfg", ".txt")
PRIVATE = [ipaddress.ip_network(n) for n in ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "127.0.0.0/8")]


def strip_comment(line):
    """Drop a trailing `# ...` (ini, yaml, sh, py). Good enough: no address this gate cares about sits in a string with a hash."""
    i = line.find("#")
    return line if i < 0 else line[:i]


def declared_mesh(repo):
    with open(os.path.join(repo, "ops/ansible/inventory/doors.yaml"), encoding="utf-8") as f:
        doc = yaml.safe_load(f) or {}
    out = {}
    for door in (doc.get("doors") or {}).values():
        for row in door.get("mgmt") or []:
            name, ip = row.get("name"), row.get("mesh_ip")
            if name and ip and not str(ip).startswith("TBD"):
                out[name] = str(ip)
    return out


def declared_bad_day(repo):
    """breakglass-path ssh_hosts across every group's vars.yml, with the file:line they are declared on."""
    out = {}
    for path in sorted(glob.glob(os.path.join(repo, "ops/ansible/group_vars/*/vars.yml"))):
        try:
            with open(path, encoding="utf-8") as f:
                doc = yaml.safe_load(f) or {}
        except (yaml.YAMLError, OSError):
            continue
        scopes = (doc.get("operator_ssh_scopes") or {}) if isinstance(doc, dict) else {}
        for entry in scopes.get("breakglass-path") or []:
            host = str((entry or {}).get("ssh_host", ""))
            if not ADDR.fullmatch(host):
                continue
            # A site's own LAN is also a bad-day road, and it is the site's network: bootstrap rows and
            # on-site runs use it by design. The address this gate hunts is the one on somebody ELSE's
            # network (a personal tailnet), which no deployer but its owner can reach.
            if any(ipaddress.ip_address(host) in p for p in PRIVATE):
                continue
            out.setdefault(host, os.path.relpath(path, repo))
    return out


def in_breakglass_block(lines, n):
    """True when line n (1-based) sits under a `breakglass-path:` key, before the next sibling key."""
    for i in range(n - 1, -1, -1):
        s = lines[i].strip()
        if s.startswith("breakglass-path:"):
            return True
        if s and not s.startswith("-") and not s.startswith("#") and re.match(r"^[A-Za-z_][\w-]*:\s*(\[\]|$)", s):
            return False
    return False


def scan(repo, mesh, bad):
    offenders = []
    for d in SCAN_DIRS:
        for root, _dirs, files in os.walk(os.path.join(repo, d)):
            rel_root = os.path.relpath(root, repo)
            if any(rel_root.startswith(s) for s in SKIP_DIRS):
                continue
            for fn in sorted(files):
                if not fn.endswith(EXT):
                    continue
                path = os.path.join(root, fn)
                rel = os.path.relpath(path, repo)
                try:
                    with open(path, encoding="utf-8", errors="replace") as source:
                        lines = source.read().splitlines()
                except OSError:
                    continue
                bad_day_file = "breakglass" in fn  # the bad-day path's own inventories name its addresses by design
                for n, raw in enumerate(lines, 1):
                    line = strip_comment(raw)
                    if not line.strip():
                        continue
                    # 1. A bad-day address anywhere but its declaration and the bad-day inventories.
                    #    A rescue node's OWN inventory row at that address is the node's declared reach
                    #    (ADR 1114: a BREAKGLASS node carries Tailscale); the defect is a MANAGEMENT host there.
                    own_row = re.match(r"^(\S+)\s.*\bansible_host=", line.strip())
                    if own_row and own_row.group(1) not in mesh:
                        continue
                    for a in ADDR.findall(line):
                        if a in bad and not bad_day_file and not (rel == bad[a] and in_breakglass_block(lines, n - 1)):
                            offenders.append(f"{rel}:{n}: {a} is a bad-day address (declared under breakglass-path in {bad[a]}); it is not a deploy, validation or onboarding address")
                    # 2. An inventory row for a management host at an address that is neither its mesh IP, a door landing nor a LAN bootstrap address.
                    m = ROW.match(line.strip())
                    if m and m.group(1) in mesh:
                        host, addr = m.group(1), m.group(2)
                        try:
                            ip = ipaddress.ip_address(addr)
                        except ValueError:
                            continue  # TBD_… placeholders are the door generator's concern
                        if addr != mesh[host] and not any(ip in p for p in PRIVATE):
                            offenders.append(f"{rel}:{n}: {host} at {addr}; its mesh address is {mesh[host]} (doors.yaml) — neither that, a door landing (127.0.0.1) nor a LAN bootstrap address")
    return offenders


def self_test():
    """A planted copy must fail on both rules; the clean copy must pass."""
    with tempfile.TemporaryDirectory() as t:
        os.makedirs(os.path.join(t, "ops/ansible/inventory"))
        os.makedirs(os.path.join(t, "ops/ansible/group_vars/x_hosts"))
        os.makedirs(os.path.join(t, "ops/validation"))
        Path(t, "ops/ansible/inventory/doors.yaml").write_text(
            "doors:\n  x:\n    type: ssm\n    mgmt:\n      - {name: mgmt-x-01-thor, mesh_ip: 100.127.0.41}\n")
        Path(t, "ops/ansible/group_vars/x_hosts/vars.yml").write_text(
            "operator_ssh_scopes:\n  host-nodes:\n    - { ssh_host: \"100.127.0.41\", ssh_user: u }\n"
            "  breakglass-path:\n    - { ssh_host: \"100.127.0.252\", ssh_user: u }   # bad-day\n")
        Path(t, "ops/ansible/inventory/mgmt-hosts.ini").write_text(
            "[x_hosts]\nmgmt-x-01-thor ansible_host=100.127.0.41 ansible_user=u\n")
        Path(t, "ops/validation/values-x.yaml").write_text("orin_a: \"u@100.127.0.41\"\n")
        mesh, bad = declared_mesh(t), declared_bad_day(t)
        assert scan(t, mesh, bad) == [], scan(t, mesh, bad)
        Path(t, "ops/validation/values-x.yaml").write_text("orin_a: \"u@100.127.0.252\"\n")
        with open(os.path.join(t, "ops/ansible/inventory/mgmt-hosts.ini"), "a") as fixture:
            fixture.write("[x_other]\nmgmt-x-01-thor ansible_host=100.127.0.248 ansible_user=u\n")
        got = scan(t, mesh, bad)
        assert any("bad-day" in o and o.startswith("ops/validation/values-x.yaml") for o in got), got
        assert any("100.127.0.248" in o for o in got), got
        assert not any(o.startswith("ops/ansible/group_vars") for o in got), got
        assert len(got) == 2, got
    print("self-test ok: clean tree passes; a bad-day address in a value and a foreign inventory address are both refused")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default=os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..")))
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    try:
        mesh, bad = declared_mesh(args.repo), declared_bad_day(args.repo)
    except (OSError, yaml.YAMLError) as e:
        print(f"check-goatnet-addresses: cannot read the declarations: {e}", file=sys.stderr)
        return 2
    if not mesh:
        print("check-goatnet-addresses: no management hosts declared in doors.yaml", file=sys.stderr)
        return 2
    offenders = scan(args.repo, mesh, bad)
    if offenders:
        print("check-goatnet-addresses: FAIL")
        for o in offenders:
            print("  " + o)
        print("fix: address the host at its doors.yaml mesh_ip (declare a new one there and run "
              "ops/scripts/generate-door-inventory.py --all); a bad-day address belongs only under "
              "operator_ssh_scopes.breakglass-path")
        return 1
    print(f"ok: {len(mesh)} management hosts addressed at their declared mesh IP; "
          f"{len(bad)} bad-day address(es) used nowhere but their declaration")
    return 0


if __name__ == "__main__":
    sys.exit(main())
