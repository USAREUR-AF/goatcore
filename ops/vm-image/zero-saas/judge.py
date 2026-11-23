#!/usr/bin/env python3
"""judge.py — the zero-SaaS lab run's pass rule, read from the packets themselves.

The lab (ops/vm-image/zero-saas/lab.sh) records every frame on every network interface
of both machines with qemu's filter-dump, so neither guest can leave a packet out of the
record. This reads those pcap files and judges the clock's window:

  - a packet sent to an address outside the lab's networks is an outside attempt. The
    lab has no route out, so the attempt never reached anything; it is still a failure,
    because it names a service the standup reaches for.
  - a DNS question for a name outside the lab's zones is an outside name, whoever was
    asked: it is the standup looking for a service it does not run. It is listed with
    the machine that asked.

The run passes when both lists are empty. Multicast and link-local frames (neighbour
discovery, mDNS, DHCP broadcast) belong to the segment, not to a service, and are not
counted. Only the standard library: the lab's runners and laptops have no packet library.

  judge.py --lab NET[,NET...] --zone ZONE [--zone ZONE...] [--from EPOCH] [--to EPOCH]
           [--out verdict.json] PCAP...

Exit 0 pass, 1 fail, 2 usage or unreadable input.
"""
import argparse
import ipaddress
import json
import struct
import sys

PROTO = {1: "icmp", 6: "tcp", 17: "udp", 58: "icmp6"}


def frames(path):
    """(epoch seconds, frame bytes) for every record of a classic pcap file."""
    with open(path, "rb") as f:
        hdr = f.read(24)
        if len(hdr) < 24:
            return
        magic = struct.unpack("<I", hdr[:4])[0]
        if magic in (0xA1B2C3D4, 0xA1B23C4D):
            endian = "<"
        elif magic in (0xD4C3B2A1, 0x4D3CB2A1):
            endian = ">"
        else:
            raise ValueError(f"{path}: not a pcap file")
        nano = magic in (0xA1B23C4D, 0x4D3CB2A1)
        while True:
            ph = f.read(16)
            if len(ph) < 16:
                return
            sec, frac, incl, _ = struct.unpack(endian + "IIII", ph)
            data = f.read(incl)
            if len(data) < incl:
                return
            yield sec + frac / (1e9 if nano else 1e6), data


def qname(msg, off):
    """The first question's name of a DNS message, or None."""
    labels = []
    for _ in range(128):
        if off >= len(msg):
            return None
        n = msg[off]
        if n == 0:
            return ".".join(labels).lower() + "." if labels else "."
        if n & 0xC0:
            return None
        part = msg[off + 1:off + 1 + n]
        if len(part) < n:
            return None
        labels.append(part.decode("ascii", "replace"))
        off += 1 + n
    return None


def decode(frame):
    """(proto, src, dst, sport, dport, payload) of an IPv4 or IPv6 frame, else None."""
    if len(frame) < 14:
        return None
    et = struct.unpack(">H", frame[12:14])[0]
    p = frame[14:]
    if et == 0x8100 and len(p) >= 4:
        et = struct.unpack(">H", p[2:4])[0]
        p = p[4:]
    if et == 0x0800 and len(p) >= 20:
        ihl = (p[0] & 0x0F) * 4
        proto = p[9]
        src, dst = ipaddress.ip_address(p[12:16]), ipaddress.ip_address(p[16:20])
        l4 = p[ihl:]
    elif et == 0x86DD and len(p) >= 40:
        proto = p[6]
        src, dst = ipaddress.ip_address(p[8:24]), ipaddress.ip_address(p[24:40])
        l4 = p[40:]
    else:
        return None
    sport = dport = None
    payload = b""
    if proto == 17 and len(l4) >= 8:
        sport, dport = struct.unpack(">HH", l4[:4])
        payload = l4[8:]
    elif proto == 6 and len(l4) >= 20:
        sport, dport = struct.unpack(">HH", l4[:4])
        payload = l4[(l4[12] >> 4) * 4:]
    return proto, src, dst, sport, dport, payload


def dns_question(proto, dport, payload):
    if dport != 53:
        return None
    if proto == 17 and len(payload) > 12:
        return qname(payload, 12)
    if proto == 6 and len(payload) > 14:  # DNS over TCP: a two-byte length first
        return qname(payload, 14)
    return None


def in_zone(name, zones):
    return any(name == z or name.endswith("." + z) for z in zones)


def reverse_zones(nets):
    """The reverse-lookup zones of the lab's networks, so a PTR query for a lab address
    is a lab name."""
    out = []
    for n in nets:
        if n.version == 4:
            octets = n.network_address.exploded.split(".")[: n.prefixlen // 8]
            out.append(".".join(reversed(octets)) + ".in-addr.arpa.")
        else:
            nibbles = n.network_address.exploded.replace(":", "")[: n.prefixlen // 4]
            out.append(".".join(reversed(nibbles)) + ".ip6.arpa.")
    return out


def judge(pcaps, lab, zones, t_from=None, t_to=None):
    zones = [z.lower().rstrip(".") + "." for z in zones] + reverse_zones(lab)
    outside, names, lab_names, counted = {}, {}, 0, 0
    for path in pcaps:
        for ts, fr in frames(path):
            if (t_from is not None and ts < t_from) or (t_to is not None and ts > t_to):
                continue
            d = decode(fr)
            if not d:
                continue
            proto, src, dst, _, dport, payload = d
            if dst.is_multicast or dst.is_link_local or dst == ipaddress.ip_address("255.255.255.255"):
                continue
            counted += 1
            q = dns_question(proto, dport, payload)
            if q:
                if in_zone(q, zones):
                    lab_names += 1
                else:
                    key = f"{q} from {src}"
                    names[key] = names.get(key, 0) + 1
            if not any(dst in n for n in lab):
                key = f"{PROTO.get(proto, proto)} {src} -> {dst}" + (f":{dport}" if dport is not None else "")
                outside[key] = outside.get(key, 0) + 1
    return {
        "pass": not outside and not names,
        "window": {"from": t_from, "to": t_to},
        "lab_networks": [str(n) for n in lab],
        "lab_zones": zones,
        "packets_judged": counted,
        "lab_dns_questions": lab_names,
        "outside_packets": dict(sorted(outside.items(), key=lambda kv: -kv[1])),
        "outside_names": dict(sorted(names.items(), key=lambda kv: -kv[1])),
        "pcaps": list(pcaps),
    }


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--lab", required=True, help="the lab's networks, comma-separated")
    ap.add_argument("--zone", action="append", default=[], help="a lab DNS zone (repeatable)")
    ap.add_argument("--from", dest="t_from", type=float)
    ap.add_argument("--to", dest="t_to", type=float)
    ap.add_argument("--out")
    ap.add_argument("pcaps", nargs="+")
    a = ap.parse_args(argv)
    try:
        lab = [ipaddress.ip_network(n.strip()) for n in a.lab.split(",") if n.strip()]
        v = judge(a.pcaps, lab, a.zone, a.t_from, a.t_to)
    except (OSError, ValueError) as e:
        print(f"judge: {e}", file=sys.stderr)
        return 2
    text = json.dumps(v, indent=2)
    if a.out:
        with open(a.out, "w") as f:
            f.write(text + "\n")
    print(text)
    n_p, n_n = sum(v["outside_packets"].values()), sum(v["outside_names"].values())
    print(f"judge: {'PASS' if v['pass'] else 'FAIL'}: {n_p} outside packet(s) to "
          f"{len(v['outside_packets'])} destination(s), {n_n} outside DNS question(s) for "
          f"{len(v['outside_names'])} name(s), {v['packets_judged']} packet(s) judged", file=sys.stderr)
    return 0 if v["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
