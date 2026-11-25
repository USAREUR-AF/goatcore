#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""What each source port sends, item by item: compare a working feed with one a receiver will not display.

    sudo tcpdump -ni <interface> -U -G 30 -W 1 -w /tmp/feed.pcap udp   # stops by itself; AppArmor blocks a timeout signal
    python3 asterix_inspect.py /tmp/feed.pcap

For each UDP source port: datagrams, records per datagram, ASTERIX categories, the SAC/SIC seen, and
the sets of data items present (CAT034, CAT048, CAT062 by item number). Reads classic pcap with an
Ethernet, Linux cooked or raw IP link layer. Nothing is sent anywhere."""
import collections
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import simple_bn_sender as s  # noqa: E402

UAP = {   # FRN -> data item, for the categories this node sends
    34: {1: "010", 2: "000", 3: "030", 4: "020", 5: "041", 6: "050", 7: "060", 8: "070", 9: "100", 10: "110",
         11: "120", 12: "090"},
    48: {1: "010", 2: "140", 3: "020", 4: "040", 5: "070", 6: "090", 7: "130", 8: "220", 9: "240", 10: "250",
         11: "161", 12: "042", 13: "200", 14: "170", 15: "210", 16: "030", 17: "080", 18: "100", 19: "110",
         20: "120", 21: "230"},
    62: {1: "010", 3: "015", 4: "070", 5: "105", 6: "100", 7: "185", 8: "210", 9: "060", 10: "245", 11: "380",
         12: "040", 13: "080", 14: "290", 15: "200", 16: "295", 17: "136", 18: "130", 19: "135", 20: "220",
         21: "390", 22: "270", 23: "300", 24: "110", 25: "120", 26: "510", 27: "500", 28: "340"},
}


def udp_payloads(path):
    d = open(path, "rb").read()
    magic = d[:4]
    end = "<" if magic in (b"\xd4\xc3\xb2\xa1", b"\x4d\x3c\xb2\xa1") else ">"
    link = struct.unpack(end + "I", d[20:24])[0]
    o = 24
    while o + 16 <= len(d):
        _, _, cl, _ = struct.unpack(end + "IIII", d[o:o + 16])
        p = d[o + 16:o + 16 + cl]
        o += 16 + cl
        off = {1: 14, 113: 16, 101: 0, 12: 0, 228: 0}.get(link)
        if off is None or len(p) < off + 28 or p[off] >> 4 != 4 or p[off + 9] != 17:
            continue
        ihl = (p[off] & 15) * 4
        sport = struct.unpack(">H", p[off + ihl:off + ihl + 2])[0]
        yield sport, p[off + ihl + 8:]


def records(data):
    """(cat, [FRN], sac, sic) for every record; CAT062 parsed fully, CAT034/048 by their first items."""
    if data and data[0] == 62:
        return [(62, sorted(r), r[1][0] if 1 in r else None, r[1][1] if 1 in r else None) for r in s.cat062_records(data)]
    out = []
    for cat, sac, sic in s.source_ids(data):
        i = 3
        bits, _ = s._fspec_at(data, i)
        out.append((cat, [k + 1 for k, on in enumerate(bits) if on], sac, sic))
    return out


def main(path):
    per = collections.defaultdict(lambda: {"datagrams": 0, "records": 0, "ids": collections.Counter(),
                                           "items": collections.Counter(), "errors": 0})
    for sport, payload in udp_payloads(path):
        st = per[sport]
        st["datagrams"] += 1
        try:
            for cat, frns, sac, sic in records(payload):
                st["records"] += 1
                st["ids"][(cat, sac, sic)] += 1
                st["items"][(cat, tuple("I%03d/%s" % (cat, UAP.get(cat, {}).get(f, "FRN%d" % f)) for f in frns))] += 1
        except (ValueError, IndexError) as e:
            st["errors"] += 1
            st["last_error"] = str(e)
    for sport in sorted(per):
        st = per[sport]
        print(f"source port {sport}: {st['datagrams']} datagrams, {st['records']} records"
              + (f", {st['errors']} undecodable ({st.get('last_error')})" if st["errors"] else ""))
        for (cat, sac, sic), n in st["ids"].most_common():
            print(f"  CAT{cat:03d} SAC {sac} / SIC {sic}: {n} records")
        for (cat, items), n in st["items"].most_common(3):
            print(f"  {n} x {' '.join(items)}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
