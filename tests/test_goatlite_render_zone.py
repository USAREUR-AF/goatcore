#!/usr/bin/env python3
"""A goatlite's render writes the LAN DNS zone that step 10 deploys, and the
address its server listens on.

Nothing wrote the zone, so step 10 failed with "Could not find or access
ops/dns/<name>/<zone>.hosts". Nothing set the listen address either, so
site-services.yml bound the zone's server to the default route's address: on a
box with a second network card that was the other card, and the zone was not
answered on the address its own names point at.
"""
import importlib.util
from pathlib import Path
import re
import sys
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("goatlite_render", ROOT / "ops/scripts/goatlite-render.py")
render = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = render
spec.loader.exec_module(render)

FACTS = {"name": "fixture", "zone": "fixture.lan", "host": "192.0.2.50", "host_is_ip": True}


class Zone(unittest.TestCase):
    def names(self):
        out = {}
        for line in render.zone_hosts(FACTS).splitlines():
            if line and not line.startswith("#"):
                ip, *names = line.split()
                out.update({name: ip for name in names})
        return out

    def test_names_the_plays_depend_on_resolve_to_the_host(self):
        names = self.names()
        for name in ("mgmt-fixture-01.fixture.lan", "netbird.fixture.lan",
                     "dashboard.fixture.fixture.lan", "oauth2.fixture.fixture.lan",
                     "portal.fixture.fixture.lan", "zenoh.fixture.fixture.lan"):
            self.assertEqual(names.get(name), "192.0.2.50", name)

    def test_first_line_marks_the_file_as_generated(self):
        self.assertTrue(render.zone_hosts(FACTS).startswith(render.HEADER + "\n"))

    def test_a_host_that_does_not_resolve_is_refused(self):
        with self.assertRaises(render.Refuse):
            render.zone_address({"host": "no-such-host.invalid", "host_is_ip": False})


class ListenAddress(unittest.TestCase):
    def vars_for(self, host):
        pub = tempfile.NamedTemporaryFile("w", suffix=".pub", delete=False)
        pub.write("placeholder: load_config only checks that the file exists\n")
        pub.close()
        cfg = tempfile.NamedTemporaryFile("w", suffix=".yaml", delete=False)
        yaml.safe_dump({"schema": "goatbuilder/v1", "preset": "lite",
                        "lite": {"name": "fixture", "host": host, "ssh_user": "ubuntu",
                                 "admin": "admin@example.org", "ssh_pubkey": pub.name}}, cfg)
        cfg.close()
        f = render.facts(render.load_config(Path(cfg.name), "fixture"))
        return f, yaml.safe_load(render.vars_yml(f))

    def test_the_zone_server_listens_where_the_zone_points(self):
        f, gv = self.vars_for("192.0.2.50")
        self.assertEqual(gv.get("lan_bind_ip"), "192.0.2.50")
        zone_ips = {line.split()[0] for line in render.zone_hosts(f).splitlines()
                    if line and not line.startswith("#")}
        self.assertEqual(zone_ips, {gv["lan_bind_ip"]})

    def test_a_host_named_by_hostname_listens_on_its_address(self):
        f, gv = self.vars_for("localhost")
        self.assertEqual(gv.get("lan_bind_ip"), render.zone_address(f))
        self.assertRegex(gv["lan_bind_ip"], r"^\d+\.\d+\.\d+\.\d+$")

    def test_the_play_reads_the_rendered_listen_address(self):
        play = (ROOT / "ops/ansible/site-services.yml").read_text()
        for var in ("lan_coredns_bind_ip", "lan_coredns_smoke_test_expect"):
            line = re.search(rf"^\s*{var}:.*$", play, re.M)
            self.assertIsNotNone(line, var)
            self.assertIn("lan_bind_ip", line.group(0), var)


if __name__ == "__main__":
    unittest.main()
