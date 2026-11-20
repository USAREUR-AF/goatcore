#!/usr/bin/env python3
"""Closed-LAN CoreDNS answers NXDOMAIN for a name it does not hold (roles/lan-coredns).

The role has no forwarder. CoreDNS's hosts plugin answers a name it does not hold with
SERVFAIL, because a hosts file has no SOA, and a name no server block claims gets SERVFAIL
too. A resolver that gets SERVFAIL treats the server as broken: it retries, and may fall
back to another server, which on a closed LAN is a leak or a stall. NXDOMAIN is the honest
answer: the server is authoritative for its zone, and nothing else exists.

1. The Corefile has a server block for the zone: the hosts file, falling through to a
   zone file that holds only the SOA and NS, so a missing name in the zone is NXDOMAIN
   with that SOA. Plugin order is CoreDNS's, not the file's: a `template` in the same
   block would answer before hosts, so the catch-all lives in its own block.
2. A second server block for every other name answers NXDOMAIN from a template. Nothing
   is forwarded anywhere.
3. The SOA file renders for the zone, with a 60-second negative TTL, like the hosts TTL.
4. The compose file mounts the SOA file; the role renders it and restarts on a change.
5. The role's smoke test checks the rcode (`status: NXDOMAIN`) for a missing name in the
   zone and for a name outside it, not only that no address came back.
6. With Docker (REQUIRE_DOCKER=1 makes it required): the role's pinned CoreDNS image
   answers a zone name with its address, a type it does not hold with no answer (NOERROR),
   and a missing zone name and an outside name with NXDOMAIN.

Run: python3 -m pytest tests/test_lan_coredns_nxdomain.py -v
"""
import os
import random
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

ROLE = Path(__file__).resolve().parents[1] / "ops/ansible/roles/lan-coredns"
COREFILE = ROLE / "templates/Corefile.j2"
SOA = ROLE / "templates/zone.soa.j2"
COMPOSE = ROLE / "templates/docker-compose.yml.j2"
DEFAULTS = ROLE / "defaults/main.yml"
DEPLOY = ROLE / "tasks/deploy.yml"

ZONE = "zagros.lan"
NXDOMAIN, SERVFAIL, NOERROR = 3, 2, 0


def defaults():
    return yaml.safe_load(DEFAULTS.read_text())


def render(path, **over):
    v = {
        "lan_coredns_port": 53,
        "lan_coredns_bind_ip": "192.0.2.53",
        "lan_coredns_zone_name": ZONE,
        "lan_coredns_container_name": "lan-dns",
        "lan_coredns_image": defaults()["lan_coredns_image"],
    }
    v.update(over)
    # A Corefile and a zone file, never HTML: nothing is escaped.
    env = jinja2.Environment(autoescape=jinja2.select_autoescape(default_for_string=False),
                             keep_trailing_newline=True, trim_blocks=True,  # as Ansible's template module
                             undefined=jinja2.StrictUndefined)
    return env.from_string(path.read_text()).render(**v)


def directives(text):
    """The file's lines without comments, stripped."""
    out = []
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            out.append(line)
    return out


def blocks(text):
    """{server key: [directive lines]} for each top-level server block."""
    out, key, depth = {}, None, 0
    for line in directives(text):
        if depth == 0 and line.endswith("{"):
            key = line[:-1].strip()
            out[key] = []
            depth = 1
            continue
        if line.endswith("{"):
            depth += 1
        if line == "}":
            depth -= 1
            if depth == 0:
                key = None
                continue
        out[key].append(line)
    return out


def tasks():
    return yaml.safe_load(DEPLOY.read_text())


class Corefile(unittest.TestCase):
    def test_the_zone_has_its_own_block_falling_through_to_the_soa(self):
        b = blocks(render(COREFILE))
        zone = b[f"{ZONE}:53"]
        self.assertIn("bind 127.0.0.1 192.0.2.53", zone)
        self.assertIn(f"hosts /etc/zone/{ZONE}.hosts {ZONE} {{", zone)
        self.assertIn("fallthrough", zone)
        self.assertIn(f"file /etc/zone/{ZONE}.soa {ZONE}", zone)
        hosts_at = zone.index(f"hosts /etc/zone/{ZONE}.hosts {ZONE} {{")
        self.assertLess(hosts_at, zone.index(f"file /etc/zone/{ZONE}.soa {ZONE}"))
        self.assertFalse([ln for ln in zone if ln.startswith("template")], zone)

    def test_every_other_name_is_nxdomain_from_its_own_block(self):
        b = blocks(render(COREFILE))
        self.assertEqual(sorted(b), sorted([f"{ZONE}:53", ".:53"]))
        rest = b[".:53"]
        self.assertIn("bind 127.0.0.1 192.0.2.53", rest)
        self.assertIn("template ANY ANY {", rest)
        self.assertIn("rcode NXDOMAIN", rest)
        self.assertFalse([ln for ln in rest if ln.startswith(("hosts", "file"))], rest)

    def test_nothing_is_forwarded(self):
        lines = directives(render(COREFILE))
        self.assertFalse([ln for ln in lines if ln.split()[0] in ("forward", "grpc")], lines)

    def test_the_port_comes_from_the_variable(self):
        self.assertEqual(sorted(blocks(render(COREFILE, lan_coredns_port=5300))),
                         sorted([f"{ZONE}:5300", ".:5300"]))


class ZoneFile(unittest.TestCase):
    def test_the_soa_names_the_zone_with_a_60_second_negative_ttl(self):
        lines = directives(render(SOA).replace(";", "#"))
        self.assertIn(f"$ORIGIN {ZONE}.", lines)
        soa = [ln.split() for ln in lines if " SOA " in f" {ln} "]
        self.assertEqual(len(soa), 1, lines)
        f = soa[0]
        self.assertEqual(f[:4], ["@", "60", "IN", "SOA"])
        self.assertEqual(f[4:6], [f"ns.{ZONE}.", f"hostmaster.{ZONE}."])
        self.assertEqual(f[-1], "60", "the SOA minimum is the negative-answer TTL")
        self.assertTrue([ln for ln in lines if ln.split()[:4] == ["@", "60", "IN", "NS"]], lines)


class Role(unittest.TestCase):
    def test_compose_mounts_the_soa_file(self):
        c = yaml.safe_load(render(COMPOSE))
        vols = c["services"]["coredns"]["volumes"]
        self.assertIn(f"./{ZONE}.soa:/etc/zone/{ZONE}.soa:ro", vols)

    def test_the_role_renders_the_soa_and_restarts_on_a_change(self):
        t = [x for x in tasks() if x.get("ansible.builtin.template", {}).get("src") == "zone.soa.j2"]
        self.assertEqual(len(t), 1, "one task renders zone.soa.j2")
        self.assertEqual(t[0]["ansible.builtin.template"]["dest"], "{{ lan_coredns_soa_path }}")
        self.assertEqual(t[0].get("notify"), "Restart lan-coredns")
        self.assertEqual(defaults()["lan_coredns_soa_path"],
                         "{{ lan_coredns_dir }}/{{ lan_coredns_zone_name }}.soa")

    def test_the_smoke_test_checks_the_rcode_inside_and_outside_the_zone(self):
        smoke = [x for x in tasks() if "NXDOMAIN" in str(x.get("name", ""))]
        names = sorted(str(x["name"]) for x in smoke)
        self.assertEqual(len(smoke), 2, names)
        cmds = " ".join(x["ansible.builtin.command"]["cmd"] for x in smoke)
        self.assertIn("this-name-does-not-exist.{{ lan_coredns_zone_name }}", cmds)
        self.assertIn("this-name-does-not-exist.invalid", cmds)
        for x in smoke:
            self.assertIn("+noall +comments", x["ansible.builtin.command"]["cmd"])
            self.assertIn("'status: NXDOMAIN' not in", x["failed_when"])


def query(port, name, qtype=1):
    """One UDP DNS query to 127.0.0.1:port; returns (rcode, answer count, authority count)."""
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
    rid, flags, _, an, ns, _ = struct.unpack(">HHHHHH", data[:12])
    assert rid == qid
    return flags & 0x000F, an, ns


@unittest.skipUnless(shutil.which("docker") or os.environ.get("REQUIRE_DOCKER") == "1", "needs Docker")
class PinnedCoreDNS(unittest.TestCase):
    def test_missing_names_are_nxdomain_inside_and_outside_the_zone(self):
        image = defaults()["lan_coredns_image"]
        port = random.randint(20000, 40000)
        with tempfile.TemporaryDirectory() as d:
            # In the container: listen on all of its addresses, so the published port reaches it.
            corefile = render(COREFILE, lan_coredns_bind_ip="").replace("bind 127.0.0.1 ", "bind 0.0.0.0")
            Path(d, "Corefile").write_text(corefile)
            Path(d, f"{ZONE}.hosts").write_text(f"192.0.2.10 portal.{ZONE}\n")
            Path(d, f"{ZONE}.soa").write_text(render(SOA))
            name = f"lan-coredns-nxdomain-test-{os.getpid()}"
            run = subprocess.run(
                ["docker", "run", "-d", "--rm", "--name", name, "-p", f"127.0.0.1:{port}:53/udp",
                 "-v", f"{d}/Corefile:/Corefile:ro",
                 "-v", f"{d}/{ZONE}.hosts:/etc/zone/{ZONE}.hosts:ro",
                 "-v", f"{d}/{ZONE}.soa:/etc/zone/{ZONE}.soa:ro",
                 image, "-conf", "/Corefile"], capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            try:
                got = None
                for _ in range(30):
                    try:
                        got = query(port, f"portal.{ZONE}")
                        break
                    except (socket.timeout, ConnectionRefusedError, OSError):
                        time.sleep(0.5)
                self.assertEqual(got[:2], (NOERROR, 1), "the zone name answers with one A record")
                self.assertEqual(query(port, f"portal.{ZONE}", qtype=28)[:2], (NOERROR, 0),
                                 "a type the name does not hold is NOERROR with no answer")
                rcode, _, ns = query(port, f"missing.{ZONE}")
                self.assertEqual(rcode, NXDOMAIN, "a missing zone name is NXDOMAIN, not SERVFAIL")
                self.assertEqual(ns, 1, "with the zone's SOA, so a resolver caches the answer")
                self.assertEqual(query(port, "example.org")[0], NXDOMAIN, "an outside name is NXDOMAIN")
                self.assertEqual(query(port, "10.2.0.192.in-addr.arpa", qtype=12)[0], NXDOMAIN,
                                 "a reverse name is NXDOMAIN (no_reverse)")
            finally:
                logs = subprocess.run(["docker", "logs", name], capture_output=True, text=True)
                subprocess.run(["docker", "rm", "-fv", name], capture_output=True)
            out = logs.stdout + logs.stderr
            self.assertNotIn("no next plugin", out)
            self.assertNotIn("[ERROR]", out)


if __name__ == "__main__":
    unittest.main()
