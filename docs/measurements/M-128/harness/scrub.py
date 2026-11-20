#!/usr/bin/env python3
"""Replace machine addresses in raw output with role names before it is
published.

    scrub.py <hosts.env> <dir>

Every address named in hosts.env (the SSH targets and the *_ADDR values) is
replaced with the role it belongs to, for example "relay-addr". NetBird mesh
addresses (100.64.0.0/10) are replaced with documentation-range mesh
addresses (100.127.0.x), one per distinct address, the same in every file.
Files are rewritten in place; binary files are left alone.
"""
import os
import re
import sys

MESH = re.compile(r"(?<![0-9.])100\.(6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\.\d{1,3}\.\d{1,3}(?![0-9])")


def read_env(path):
    vals = {}
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            k, v = line.split("=", 1)
            vals[k.strip()] = v.strip().strip('"').strip("'")
    return vals


def address_map(env):
    names = {
        "CONTROL_TARGET": "control-ssh", "RELAY_TARGET": "relay-ssh",
        "PEER_A_TARGET": "peer-a-ssh", "PEER_B_TARGET": "peer-b-ssh",
        "CONTROL_ADDR": "control-addr", "RELAY_ADDR": "relay-addr",
        "RELAY_BIND": "relay-addr", "PEER_A_ADDR": "peer-a-addr", "PEER_B_ADDR": "peer-b-addr",
    }
    out = {}
    for k, label in names.items():
        v = env.get(k)
        if not v:
            continue
        host = v.split("@", 1)[-1]
        if re.fullmatch(r"[0-9.]+|[0-9a-fA-F:]+|[A-Za-z0-9.-]+", host) and len(host) >= 3:
            out.setdefault(host, label)
    return out


def main():
    env = read_env(sys.argv[1])
    root = sys.argv[2]
    amap = address_map(env)
    # Longest first so 10.0.1.100 is not replaced through 10.0.1.10.
    pats = [(re.compile(r"(?<![0-9A-Za-z.-])" + re.escape(a) + r"(?![0-9A-Za-z-]|\.[0-9])"), l)
            for a, l in sorted(amap.items(), key=lambda kv: -len(kv[0]))]
    mesh = {}

    def mesh_sub(m):
        a = m.group(0)
        if a not in mesh:
            mesh[a] = f"100.127.0.{len(mesh) + 1}"
        return mesh[a]

    changed = 0
    for dirpath, _dirs, files in os.walk(root):
        for name in sorted(files):
            p = os.path.join(dirpath, name)
            try:
                with open(p, encoding="utf-8") as f:
                    text = f.read()
            except (UnicodeDecodeError, OSError):
                continue
            new = text
            for pat, label in pats:
                new = pat.sub(label, new)
            new = MESH.sub(mesh_sub, new)
            if new != text:
                with open(p, "w", encoding="utf-8") as f:
                    f.write(new)
                changed += 1
    print(f"scrubbed {changed} files; {len(amap)} host addresses, {len(mesh)} mesh addresses")


if __name__ == "__main__":
    main()
