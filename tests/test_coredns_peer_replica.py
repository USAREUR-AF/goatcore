#!/usr/bin/env python3
"""goat-coredns.yml's nameserver list (ADR 1235). Without mesh_dns_peer_replica_group it is the
targeted host alone, as before. A joining site's run (join-node phase 13) names the founder's core
group there, and the list is then the site's own replica first and the founder's second, so a site
cut off from its founder still answers from its own replica. The default distribution is All.

Run: python3 -m unittest tests/test_coredns_peer_replica.py -v
"""
import ast
import unittest
from pathlib import Path

import jinja2
import yaml

PLAY = Path(__file__).resolve().parents[1] / "ops/ansible/goat-coredns.yml"


def play_vars():
    return yaml.safe_load(PLAY.read_text())[0]["vars"]


def servers(**facts):
    expr = play_vars()["mesh_dns_servers"]
    out = jinja2.Environment().from_string(expr).render(**facts)
    return ast.literal_eval(out.strip())


class PeerReplica(unittest.TestCase):
    hostvars = {"alpha-core-1": {"mesh_ip_a": "100.127.0.1"}, "gamma-core-1": {"mesh_ip_a": "100.127.0.3"}}
    groups = {"alpha_hosts": ["alpha-core-1"], "gamma_hosts": ["gamma-core-1"]}

    def test_alone_it_is_the_targeted_host(self):
        got = servers(inventory_hostname="alpha-core-1", mesh_ip_a="100.127.0.1",
                      mesh_dns_peer_replica_group="", groups=self.groups, hostvars=self.hostvars)
        self.assertEqual(got, [{"host": "alpha-core-1", "mesh_ip": "100.127.0.1"}])

    def test_a_joining_site_lists_its_own_replica_first_and_the_founders_second(self):
        got = servers(inventory_hostname="gamma-core-1", mesh_ip_a="100.127.0.3",
                      mesh_dns_peer_replica_group="alpha_hosts", groups=self.groups, hostvars=self.hostvars)
        self.assertEqual(got, [{"host": "gamma-core-1", "mesh_ip": "100.127.0.3"},
                               {"host": "alpha-core-1", "mesh_ip": "100.127.0.1"}])

    def test_the_default_distribution_is_all(self):
        v = play_vars()
        self.assertEqual(v["mesh_dns_peer_replica_group"], "")
        self.assertEqual(v["netbird_nameserver_distribution_groups"], ["All"])


if __name__ == "__main__":
    unittest.main()
