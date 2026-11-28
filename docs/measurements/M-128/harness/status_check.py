#!/usr/bin/env python3
"""Exit 0 and print peer-b's NetBird address when both peers see each other
connected through the relay; exit 1 with the reason otherwise.

    status_check.py <peer-a status.json> <peer-b status.json>
"""
import json
import sys


def load(path):
    with open(path) as f:
        return json.load(f)


def relayed_peers(status):
    details = (status.get("peers") or {}).get("details") or []
    return [p for p in details
            if p.get("status") == "Connected" and p.get("connectionType") == "Relayed"]


def main():
    try:
        a = load(sys.argv[1])
        b = load(sys.argv[2])
    except (OSError, ValueError) as e:
        print(f"status not readable: {e}", file=sys.stderr)
        return 1
    b_ip = (b.get("netbirdIp") or "").split("/")[0]
    a_ip = (a.get("netbirdIp") or "").split("/")[0]
    if not a_ip or not b_ip:
        print("a peer has no NetBird address yet", file=sys.stderr)
        return 1
    a_sees = [p for p in relayed_peers(a) if p.get("netbirdIp", "").split("/")[0] == b_ip]
    b_sees = [p for p in relayed_peers(b) if p.get("netbirdIp", "").split("/")[0] == a_ip]
    if not a_sees or not b_sees:
        print(f"not both relayed yet: a->b {len(a_sees)}, b->a {len(b_sees)}", file=sys.stderr)
        return 1
    print(b_ip)
    return 0


if __name__ == "__main__":
    sys.exit(main())
