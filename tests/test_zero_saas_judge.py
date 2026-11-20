#!/usr/bin/env python3
"""The zero-SaaS lab run's pass rule (ops/vm-image/zero-saas/judge.py), on pcap files
written here frame by frame.

Run: python3 -m pytest tests/test_zero_saas_judge.py -v
"""
import importlib.util
import ipaddress
import struct
import tempfile
import unittest
from pathlib import Path

JUDGE = Path(__file__).resolve().parents[1] / "ops/vm-image/zero-saas/judge.py"
spec = importlib.util.spec_from_file_location("zero_saas_judge", JUDGE)
judge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(judge)

LAB = "10.88.0.0/24,fd88::/64,10.0.2.0/24,fec0::/64"
NETS = [ipaddress.ip_network(n) for n in LAB.split(",")]
ETH = b"\x52\x54\x00\x00\x01\x02" + b"\x52\x54\x00\x00\x02\x02"


def dns_query(name, qid=0x1234):
    q = struct.pack(">HHHHHH", qid, 0x0100, 1, 0, 0, 0)
    for label in name.rstrip(".").split("."):
        q += bytes([len(label)]) + label.encode()
    return q + b"\x00" + struct.pack(">HH", 1, 1)


def l4(proto, sport, dport, payload=b""):
    if proto == 17:
        return struct.pack(">HHHH", sport, dport, 8 + len(payload), 0) + payload
    if proto == 6:
        return struct.pack(">HHIIBBHHH", sport, dport, 1, 0, 5 << 4, 0x02, 65535, 0, 0) + payload
    return b"\x08\x00\x00\x00\x00\x01\x00\x01"


def frame(src, dst, proto=6, sport=40000, dport=443, payload=b""):
    s, d = ipaddress.ip_address(src), ipaddress.ip_address(dst)
    body = l4(proto, sport, dport, payload)
    if s.version == 4:
        ip = struct.pack(">BBHHHBBH4s4s", 0x45, 0, 20 + len(body), 0, 0, 64, proto, 0, s.packed, d.packed)
        return ETH + b"\x08\x00" + ip + body
    ip = struct.pack(">IHBB16s16s", 6 << 28, len(body), proto, 64, s.packed, d.packed)
    return ETH + b"\x86\xdd" + ip + body


def write_pcap(path, records):
    with open(path, "wb") as f:
        f.write(struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1))
        for ts, fr in records:
            f.write(struct.pack("<IIII", int(ts), int((ts % 1) * 1e6), len(fr), len(fr)) + fr)


class Judge(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.TemporaryDirectory()
        self.addCleanup(self.d.cleanup)
        self.n = 0

    def pcap(self, *frames_, ts=100.0):
        self.n += 1
        p = str(Path(self.d.name, f"{self.n}.pcap"))
        write_pcap(p, [(ts + i, f) for i, f in enumerate(frames_)])
        return p

    def run_judge(self, *pcaps, **kw):
        return judge.judge(pcaps, NETS, ["pasture.lan"], kw.get("t_from"), kw.get("t_to"))

    def test_lab_traffic_alone_passes(self):
        p = self.pcap(frame("10.88.0.1", "10.88.0.2", dport=22),
                      frame("10.88.0.2", "10.88.0.1", sport=22, dport=40000),
                      frame("fd88::1", "fd88::2", dport=443))
        v = self.run_judge(p)
        self.assertTrue(v["pass"], v)
        self.assertEqual(v["packets_judged"], 3)

    def test_an_outside_address_fails_and_is_named(self):
        p = self.pcap(frame("10.88.0.2", "1.1.1.1", dport=443), frame("10.88.0.2", "1.1.1.1", dport=443))
        v = self.run_judge(p)
        self.assertFalse(v["pass"])
        self.assertEqual(v["outside_packets"], {"tcp 10.88.0.2 -> 1.1.1.1:443": 2})

    def test_an_outside_ipv6_address_fails(self):
        v = self.run_judge(self.pcap(frame("fec0::15", "2001:db8::53", proto=17, dport=123)))
        self.assertEqual(v["outside_packets"], {"udp fec0::15 -> 2001:db8::53:123": 1})

    def test_a_question_for_an_outside_name_fails_even_when_a_lab_resolver_is_asked(self):
        p = self.pcap(frame("10.88.0.1", "10.88.0.2", proto=17, dport=53, payload=dns_query("ntp.ubuntu.com")))
        v = self.run_judge(p)
        self.assertFalse(v["pass"])
        self.assertEqual(v["outside_packets"], {})
        self.assertEqual(v["outside_names"], {"ntp.ubuntu.com. from 10.88.0.1": 1})

    def test_lab_zone_and_lab_reverse_names_pass(self):
        p = self.pcap(
            frame("10.88.0.1", "10.88.0.2", proto=17, dport=53, payload=dns_query("dashboard.pasture.pasture.lan")),
            frame("10.88.0.1", "10.88.0.2", proto=17, dport=53, payload=dns_query("PASTURE.lan")),
            frame("10.88.0.1", "10.88.0.2", proto=17, dport=53, payload=dns_query("2.0.88.10.in-addr.arpa")))
        v = self.run_judge(p)
        self.assertTrue(v["pass"], v)
        self.assertEqual(v["lab_dns_questions"], 3)

    def test_a_name_that_only_ends_like_the_zone_is_outside(self):
        p = self.pcap(frame("10.88.0.1", "10.88.0.2", proto=17, dport=53, payload=dns_query("evilpasture.lan")))
        self.assertEqual(self.run_judge(p)["outside_names"], {"evilpasture.lan. from 10.88.0.1": 1})

    def test_dns_over_tcp_is_read(self):
        q = dns_query("registry-1.docker.io")
        p = self.pcap(frame("10.88.0.2", "10.88.0.1", proto=6, dport=53, payload=struct.pack(">H", len(q)) + q))
        self.assertEqual(self.run_judge(p)["outside_names"], {"registry-1.docker.io. from 10.88.0.2": 1})

    def test_multicast_and_broadcast_are_the_segment_not_a_service(self):
        p = self.pcap(frame("10.88.0.2", "224.0.0.251", proto=17, dport=5353, payload=dns_query("x.local")),
                      frame("0.0.0.0", "255.255.255.255", proto=17, dport=67),
                      frame("fe80::1", "ff02::2", proto=58, dport=None))
        v = self.run_judge(p)
        self.assertTrue(v["pass"], v)
        self.assertEqual(v["packets_judged"], 0)

    def test_only_the_clock_window_is_judged(self):
        p = self.pcap(frame("10.88.0.2", "1.1.1.1"), frame("10.88.0.1", "10.88.0.2"), frame("10.88.0.2", "8.8.8.8"), ts=100.0)
        v = self.run_judge(p, t_from=100.5, t_to=101.5)
        self.assertTrue(v["pass"], v)
        v = self.run_judge(p, t_from=100.5)
        self.assertEqual(list(v["outside_packets"]), ["tcp 10.88.0.2 -> 8.8.8.8:443"])

    def test_several_pcaps_are_one_record(self):
        a = self.pcap(frame("10.88.0.1", "10.88.0.2"))
        b = self.pcap(frame("10.0.2.15", "192.0.2.80", dport=80))
        self.assertEqual(self.run_judge(a, b)["outside_packets"], {"tcp 10.0.2.15 -> 192.0.2.80:80": 1})

    def test_the_command_exits_by_the_verdict(self):
        ok = self.pcap(frame("10.88.0.1", "10.88.0.2"))
        bad = self.pcap(frame("10.88.0.2", "1.1.1.1"))
        out = str(Path(self.d.name, "v.json"))
        self.assertEqual(judge.main(["--lab", LAB, "--zone", "pasture.lan", ok]), 0)
        self.assertEqual(judge.main(["--lab", LAB, "--zone", "pasture.lan", "--out", out, bad]), 1)
        self.assertIn("1.1.1.1", Path(out).read_text())
        junk = Path(self.d.name, "junk.pcap")
        junk.write_bytes(b"not a pcap file at all, just bytes")
        self.assertEqual(judge.main(["--lab", LAB, "--zone", "pasture.lan", str(junk)]), 2)


if __name__ == "__main__":
    unittest.main()
