#!/usr/bin/env python3
"""Mesh CoreDNS's outside forwarders are a role variable (roles/mesh-coredns).

1. The default list renders today's line, `forward . 1.1.1.1 8.8.8.8`, so a goatnet that
   sets nothing renders the Corefile it ran before.
2. A supplied list renders in order, on one `forward .` line.
3. An empty list renders no `forward` directive and no `fallthrough` out of the zone file:
   the resolver answers only its own names and sends no query anywhere. CoreDNS's hosts
   plugin answers a name it does not hold with SERVFAIL (it has no SOA to say NXDOMAIN).
4. The role refuses an item that is not one non-empty word (a space would make two
   forwarders out of one value).
5. With Docker (REQUIRE_DOCKER=1 makes it required): the role's pinned CoreDNS image, on an
   empty list, answers a zone name and SERVFAILs an outside name, logging no missing plugin.

Run: python3 -m pytest tests/test_mesh_coredns_forwarders.py -v
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

ROLE = Path(__file__).resolve().parents[1] / "ops/ansible/roles/mesh-coredns"
TEMPLATE = ROLE / "templates/Corefile.j2"
DEFAULTS = ROLE / "defaults/main.yml"
TASKS = ROLE / "tasks/main.yml"


def defaults():
    return yaml.safe_load(DEFAULTS.read_text())


def render(**over):
    v = {
        "mesh_coredns_port": 5353,
        "mesh_coredns_bind_ip": "100.127.0.1",
        "mesh_coredns_wg_cp0_bind_ip": "",
        "mesh_coredns_decoy_zone": "",
        "mesh_coredns_forwarders": defaults()["mesh_coredns_forwarders"],
    }
    v.update(over)
    # A Corefile, never HTML: nothing is escaped.
    env = jinja2.Environment(autoescape=jinja2.select_autoescape(default_for_string=False),
                             keep_trailing_newline=True, trim_blocks=True,  # as Ansible's template module
                             undefined=jinja2.StrictUndefined)
    env.filters["regex_escape"] = re.escape  # Ansible's filter, used by the decoy stanza
    return env.from_string(TEMPLATE.read_text()).render(**v)


def directives(text):
    """The Corefile's lines without comments, stripped."""
    out = []
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            out.append(line)
    return out


class Forwarders(unittest.TestCase):
    def test_the_default_renders_todays_forward_line(self):
        self.assertEqual(defaults()["mesh_coredns_forwarders"], ["1.1.1.1", "8.8.8.8"])
        lines = directives(render())
        self.assertIn("forward . 1.1.1.1 8.8.8.8", lines)
        self.assertIn("fallthrough", lines)

    def test_a_supplied_list_renders_in_order(self):
        lines = directives(render(mesh_coredns_forwarders=["192.0.2.53", "198.51.100.53:5300"]))
        self.assertIn("forward . 192.0.2.53 198.51.100.53:5300", lines)

    def test_an_empty_list_forwards_nothing(self):
        lines = directives(render(mesh_coredns_forwarders=[]))
        self.assertFalse([ln for ln in lines if ln.startswith("forward")], lines)
        # The zone file is the last answer: no fallthrough to a plugin that is not there.
        self.assertNotIn("fallthrough", lines)
        self.assertIn("hosts /etc/mesh-hosts {", lines)

    def test_an_empty_list_keeps_the_decoy_fallthrough(self):
        lines = directives(render(mesh_coredns_forwarders=[], mesh_coredns_decoy_zone="site.example.internal",
                                  mesh_coredns_decoy_answer_ip="100.127.0.1"))
        # The decoy templates fall through to the zone file; the zone file itself does not.
        self.assertEqual(lines.count("fallthrough"), 2, lines)

    def test_the_role_refuses_an_item_that_is_not_one_word(self):
        tasks = yaml.safe_load(TASKS.read_text())
        checks = [t for t in tasks if "mesh_coredns_forwarders" in str(t.get("ansible.builtin.assert", ""))]
        self.assertEqual(len(checks), 1, "one assert on mesh_coredns_forwarders")
        env = jinja2.Environment()
        for cond in checks[0]["ansible.builtin.assert"]["that"]:
            expr = env.compile_expression(cond)
            for good in ([], ["1.1.1.1"], ["192.0.2.53", "tls://198.51.100.53"]):
                self.assertTrue(expr(mesh_coredns_forwarders=good), (cond, good))
        bad = (["1.1.1.1 8.8.8.8"], [""], "1.1.1.1", [53])
        for b in bad:
            self.assertFalse(all(env.compile_expression(c)(mesh_coredns_forwarders=b)
                                 for c in checks[0]["ansible.builtin.assert"]["that"]), b)


def query(port, name, qtype=1):
    """One UDP DNS query to 127.0.0.1:port; returns (rcode, answer count)."""
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
    rid, flags, _, an, _, _ = struct.unpack(">HHHHHH", data[:12])
    assert rid == qid
    return flags & 0x000F, an


@unittest.skipUnless(shutil.which("docker") or os.environ.get("REQUIRE_DOCKER") == "1", "needs Docker")
class PinnedCoreDNS(unittest.TestCase):
    def test_an_empty_list_answers_the_zone_and_refuses_the_rest(self):
        image = defaults()["mesh_coredns_image"]
        port = random.randint(20000, 40000)
        with tempfile.TemporaryDirectory() as d:
            corefile = render(mesh_coredns_forwarders=[], mesh_coredns_bind_ip="", mesh_coredns_port=53)
            # In the container: listen on all of its addresses, so the published port reaches it.
            corefile = corefile.replace("bind 127.0.0.1", "bind 0.0.0.0")
            Path(d, "Corefile").write_text(corefile)
            Path(d, "mesh-hosts").write_text("100.127.0.1 mgmt.zagros.example.org\n")
            name = f"mesh-coredns-fwd-test-{os.getpid()}"
            run = subprocess.run(
                ["docker", "run", "-d", "--rm", "--name", name, "-p", f"127.0.0.1:{port}:53/udp",
                 "-v", f"{d}/Corefile:/Corefile:ro", "-v", f"{d}/mesh-hosts:/etc/mesh-hosts:ro",
                 image, "-conf", "/Corefile"], capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            try:
                got = None
                for _ in range(30):
                    try:
                        got = query(port, "mgmt.zagros.example.org")
                        break
                    except (socket.timeout, ConnectionRefusedError, OSError):
                        time.sleep(0.5)
                self.assertEqual(got, (0, 1), "the zone name answers with one A record")
                self.assertEqual(query(port, "example.com")[0], 2, "an outside name is SERVFAIL, from the zone file")
            finally:
                logs = subprocess.run(["docker", "logs", name], capture_output=True, text=True)
                subprocess.run(["docker", "rm", "-fv", name], capture_output=True)
            self.assertNotIn("no next plugin", logs.stdout + logs.stderr)


if __name__ == "__main__":
    unittest.main()
