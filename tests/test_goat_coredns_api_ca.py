#!/usr/bin/env python3
"""goat-coredns.yml verifies the management API against the goatnet's root.

The play registers the site's NetBird nameserver group through the management
API from this machine (delegate_to: localhost). A LAN goatnet's management
serves a certificate from the goatnet's own CA, which this machine's Python
does not trust, so every call failed "CERTIFICATE_VERIFY_FAILED" (join-node
phase 13 at a joining site, in a VM founder run). The ACL playbooks verify
against the root in the repository (netbird_api_ca, default auto); this play
now does the same, with the goatnet's CA being its founder's.

The play's own module_defaults expression is evaluated here.
Run: python3 -m pytest -q tests/test_goat_coredns_api_ca.py
"""
import os
from pathlib import Path
import tempfile
import unittest

import jinja2
import yaml

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/goat-coredns.yml"
OMIT = "__omit__"


def ca_path(play_dir, **v):
    play = yaml.safe_load(PLAY.read_text())[0]
    expr = play["module_defaults"]["ansible.builtin.uri"]["ca_path"]
    env = jinja2.Environment(autoescape=jinja2.select_autoescape(default_for_string=False))
    env.tests["exists"] = os.path.exists
    return env.from_string(expr).render(playbook_dir=play_dir, omit=OMIT, **v)


class ApiCa(unittest.TestCase):
    def test_every_api_call_runs_on_this_machine(self):
        play = yaml.safe_load(PLAY.read_text())[0]
        def walk(ts):
            for t in ts or []:
                yield t
                for k in ("block", "rescue", "always"):
                    yield from walk(t.get(k))
        uris = [t for t in walk(play.get("tasks", []) + play.get("pre_tasks", [])) if "ansible.builtin.uri" in t]
        self.assertTrue(uris)
        self.assertTrue(all(t.get("delegate_to") == "localhost" for t in uris), [t.get("name") for t in uris])

    def test_the_goatnets_root_is_the_ca(self):
        with tempfile.TemporaryDirectory() as d:
            pb = Path(d) / "ops/ansible"
            pb.mkdir(parents=True)
            ca = Path(d) / "ops/enrollment/ca/zagros"
            ca.mkdir(parents=True)
            (ca / "root-cert.pem").write_text("-----BEGIN CERTIFICATE-----\n")
            self.assertEqual(ca_path(str(pb), site_id="zagros"), f"{pb}/../enrollment/ca/zagros/root-cert.pem")
            self.assertEqual(ca_path(str(pb), site_id="beta", join_founder_site="zagros"), f"{pb}/../enrollment/ca/zagros/root-cert.pem", "a joining site uses the founder's root")
            self.assertEqual(ca_path(str(pb), site_id="zagros", netbird_api_ca="public"), OMIT, "public: the system roots")
            self.assertEqual(ca_path(str(pb), site_id="other"), OMIT, "no root in the checkout: the system roots")


if __name__ == "__main__":
    unittest.main()
