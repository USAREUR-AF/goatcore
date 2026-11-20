#!/usr/bin/env python3
"""Mesh CoreDNS with no forwarders answers NXDOMAIN, not SERVFAIL (roles/mesh-coredns).

With an empty `mesh_coredns_forwarders`, this resolver is the only source of answers. The
hosts plugin answers a name it does not hold with SERVFAIL (a hosts file has no SOA), and a
resolver that gets SERVFAIL retries or tries another server. So the hosts file falls through
to a zone file for the root that holds only an SOA and an NS: every name the hosts file and
the decoy stanzas do not answer gets NXDOMAIN, with a short negative TTL.

1. With forwarders set, the rendered Corefile and docker-compose.yml are byte for byte what
   they were before this change (tests/fixtures/mesh-coredns/, recorded from that template).
2. With an empty list, the hosts plugin falls through to the root zone file, nothing is
   forwarded, and the decoy stanzas still come before the hosts file.
3. The root zone file holds one SOA and one NS for the root, its negative TTL the role's
   `mesh_coredns_negative_ttl` (30 seconds by default).
4. The compose file mounts it, and the role renders it, only with an empty list.
5. With Docker (REQUIRE_DOCKER=1 makes it required): on the role's pinned image, with an
   empty list, a hosts name, a decoy name and an extra name outside the site's zone answer
   NOERROR; a missing name in the zone, a name under the extra name and an outside name
   answer NXDOMAIN with the root SOA, its TTL the negative TTL; CoreDNS logs no error.

Run: python3 -m pytest tests/test_mesh_coredns_nxdomain.py -v
"""
import os
import random
import re
import shutil
import socket
import struct
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

import jinja2
import yaml

REPO = Path(__file__).resolve().parents[1]
ROLE = REPO / "ops/ansible/roles/mesh-coredns"
COREFILE = ROLE / "templates/Corefile.j2"
COMPOSE = ROLE / "templates/docker-compose.yml.j2"
ROOT_ZONE = ROLE / "templates/mesh-root.zone.j2"
DEFAULTS = ROLE / "defaults/main.yml"
TASKS = ROLE / "tasks/main.yml"
FIXTURES = REPO / "tests/fixtures/mesh-coredns"


def defaults():
    return yaml.safe_load(DEFAULTS.read_text())


def env():
    e = jinja2.Environment(autoescape=jinja2.select_autoescape(default_for_string=False),
                           keep_trailing_newline=True, trim_blocks=True,  # as Ansible's template module
                           undefined=jinja2.StrictUndefined)
    e.filters["regex_escape"] = re.escape  # Ansible's filter, used by the decoy stanza
    return e


def values(**over):
    d = defaults()
    v = {
        "mesh_coredns_port": 5353,
        "mesh_coredns_bind_ip": "100.127.0.1",
        "mesh_coredns_wg_cp0_bind_ip": "",
        "mesh_coredns_decoy_zone": "",
        "mesh_coredns_image": d["mesh_coredns_image"],
        "mesh_coredns_forwarders": d["mesh_coredns_forwarders"],
        "mesh_coredns_negative_ttl": d.get("mesh_coredns_negative_ttl", 30),
    }
    v.update(over)
    return v


def render(path, **over):
    return env().from_string(path.read_text()).render(**values(**over))


def directives(text):
    out = []
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            out.append(line)
    return out


class WithForwarders(unittest.TestCase):
    def test_the_corefile_is_unchanged(self):
        cases = {
            "Corefile.default": {},
            "Corefile.supplied": {"mesh_coredns_forwarders": ["192.0.2.53", "198.51.100.53:5300"]},
            "Corefile.decoy": {"mesh_coredns_decoy_zone": "site.example.internal",
                               "mesh_coredns_decoy_answer_ip": "100.127.0.1"},
        }
        for name, over in cases.items():
            self.assertEqual(render(COREFILE, **over), (FIXTURES / name).read_text(), name)

    def test_the_compose_file_is_unchanged(self):
        self.assertEqual(render(COMPOSE), (FIXTURES / "docker-compose.default.golden").read_text())


class WithNoForwarders(unittest.TestCase):
    def test_the_hosts_file_falls_through_to_the_root_zone_and_nothing_is_forwarded(self):
        lines = directives(render(COREFILE, mesh_coredns_forwarders=[]))
        self.assertFalse([ln for ln in lines if ln.startswith("forward")], lines)
        hosts = lines.index("hosts /etc/mesh-hosts {")
        self.assertEqual(lines[hosts + 1:hosts + 4], ["reload 5s", "no_reverse", "fallthrough"], lines)
        self.assertIn("file /etc/mesh-root.zone .", lines)
        self.assertLess(hosts, lines.index("file /etc/mesh-root.zone ."))

    def test_the_decoy_stanzas_still_come_first(self):
        lines = directives(render(COREFILE, mesh_coredns_forwarders=[], mesh_coredns_decoy_zone="site.example.internal",
                                  mesh_coredns_decoy_answer_ip="100.127.0.1"))
        first_template = next(i for i, ln in enumerate(lines) if ln.startswith("template "))
        self.assertLess(first_template, lines.index("hosts /etc/mesh-hosts {"))
        self.assertEqual(lines.count("fallthrough"), 3, lines)

    def test_the_root_zone_holds_one_soa_and_one_ns_with_the_negative_ttl(self):
        self.assertEqual(defaults()["mesh_coredns_negative_ttl"], 30)
        records = [ln.split() for ln in render(ROOT_ZONE).splitlines() if ln.strip() and not ln.lstrip().startswith(";")]
        self.assertEqual([r[3] for r in records], ["SOA", "NS"], records)
        soa = records[0]
        self.assertEqual((soa[0], soa[1], soa[2]), (".", "30", "IN"))
        self.assertEqual(soa[-1], "30", "the SOA minimum is the negative TTL")
        ttl = render(ROOT_ZONE, mesh_coredns_negative_ttl=10).split()
        self.assertIn("10", ttl)

    def test_the_compose_file_mounts_the_root_zone_only_with_an_empty_list(self):
        self.assertIn("./mesh-root.zone:/etc/mesh-root.zone:ro", render(COMPOSE, mesh_coredns_forwarders=[]))
        self.assertNotIn("mesh-root.zone", render(COMPOSE))

    def test_the_role_renders_the_root_zone_only_with_an_empty_list(self):
        tasks = yaml.safe_load(TASKS.read_text())
        t = [t for t in tasks if (t.get("ansible.builtin.template") or {}).get("src") == "mesh-root.zone.j2"]
        self.assertEqual(len(t), 1, "one task renders the root zone")
        self.assertIn("mesh_coredns_forwarders | length == 0", str(t[0].get("when")))
        self.assertEqual(t[0].get("notify"), "Restart coredns")


def query(port, name, qtype=1):
    """One UDP query to 127.0.0.1:port: (rcode, answers, authority records, first authority TTL)."""
    qid = random.randint(0, 0xFFFF)
    q = struct.pack(">HHHHHH", qid, 0x0100, 1, 0, 0, 0)
    q += b"".join(bytes([len(p)]) + p.encode() for p in name.rstrip(".").split(".")) + b"\x00"
    q += struct.pack(">HH", qtype, 1)
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.settimeout(2)
    try:
        s.sendto(q, ("127.0.0.1", port))
        data, _ = s.recvfrom(4096)
    finally:
        s.close()
    rid, flags, qd, an, ns, _ = struct.unpack(">HHHHHH", data[:12])
    assert rid == qid
    off = 12

    def skip_name(o):
        while True:
            n = data[o]
            if n & 0xC0 == 0xC0:
                return o + 2
            if n == 0:
                return o + 1
            o += n + 1

    for _ in range(qd):
        off = skip_name(off) + 4
    for _ in range(an):
        off = skip_name(off)
        rdlen = struct.unpack(">H", data[off + 8:off + 10])[0]
        off += 10 + rdlen
    ttl = None
    if ns:
        off = skip_name(off)
        ttl = struct.unpack(">I", data[off + 4:off + 8])[0]
    return flags & 0x000F, an, ns, ttl


NOERROR, NXDOMAIN = 0, 3
ZONE = "gamma.zagros.example"


@unittest.skipUnless(shutil.which("docker") or os.environ.get("REQUIRE_DOCKER") == "1", "needs Docker")
class PinnedCoreDNS(unittest.TestCase):
    def test_an_empty_list_answers_nxdomain_for_every_name_it_does_not_hold(self):
        image = defaults()["mesh_coredns_image"]
        port = random.randint(20000, 40000)
        with tempfile.TemporaryDirectory() as d:
            corefile = render(COREFILE, mesh_coredns_forwarders=[], mesh_coredns_bind_ip="", mesh_coredns_port=53,
                              mesh_coredns_decoy_zone=ZONE, mesh_coredns_decoy_answer_ip="192.0.2.99")
            # In the container: listen on all of its addresses, so the published port reaches it.
            Path(d, "Corefile").write_text(corefile.replace("bind 127.0.0.1", "bind 0.0.0.0"))
            Path(d, "mesh-hosts").write_text(f"192.0.2.10 mgmt.{ZONE}\n192.0.2.20 chat.partner.example\n")
            Path(d, "mesh-root.zone").write_text(render(ROOT_ZONE))
            name = f"mesh-coredns-nxdomain-test-{os.getpid()}"
            run = subprocess.run(
                ["docker", "run", "-d", "--rm", "--name", name, "-p", f"127.0.0.1:{port}:53/udp",
                 "-v", f"{d}/Corefile:/Corefile:ro", "-v", f"{d}/mesh-hosts:/etc/mesh-hosts:ro",
                 "-v", f"{d}/mesh-root.zone:/etc/mesh-root.zone:ro",
                 image, "-conf", "/Corefile"], capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            try:
                got = None
                for _ in range(30):
                    try:
                        got = query(port, f"mgmt.{ZONE}")
                        break
                    except (socket.timeout, ConnectionRefusedError, OSError):
                        time.sleep(0.5)
                self.assertEqual(got[:2], (NOERROR, 1), "a hosts name answers")
                self.assertEqual(query(port, f"replica-abcdefgh.{ZONE}")[:2], (NOERROR, 1), "a decoy name answers")
                self.assertEqual(query(port, "chat.partner.example")[:2], (NOERROR, 1),
                                 "an extra name outside the site's zone answers")
                self.assertEqual(query(port, f"mgmt.{ZONE}", qtype=28)[:2], (NOERROR, 0),
                                 "a type the name does not hold is NOERROR with no answer")
                for missing in (f"missing.{ZONE}", "foo.chat.partner.example", "example.org"):
                    rcode, an, ns, ttl = query(port, missing)
                    self.assertEqual((rcode, an, ns), (NXDOMAIN, 0, 1), f"{missing}: NXDOMAIN with the root SOA")
                    self.assertEqual(ttl, 30, f"{missing}: the negative TTL")
            finally:
                logs = subprocess.run(["docker", "logs", name], capture_output=True, text=True)
                subprocess.run(["docker", "rm", "-fv", name], capture_output=True)
            out = logs.stdout + logs.stderr
            self.assertNotIn("no next plugin", out)
            self.assertNotIn("[ERROR]", out)


if __name__ == "__main__":
    unittest.main()
