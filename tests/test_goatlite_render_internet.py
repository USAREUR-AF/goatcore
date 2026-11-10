#!/usr/bin/env python3
"""A goatlite grown to the internet (docs/design/goatlite.md §6): `lite.internet`
renders an internet-correct surface, refuses what would not work on the internet,
and leaves the LAN render as it was.

The required-inputs scan proves only that a key exists; internet_problems checks
the values, and each negative control below proves it notices one wrong value.
"""
import copy
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("goatlite_render", ROOT / "ops/scripts/goatlite-render.py")
render = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = render
spec.loader.exec_module(render)

PUBKEY = tempfile.NamedTemporaryFile("w", suffix=".pub", delete=False)
PUBKEY.write("placeholder: load_config only checks that the file exists\n")
PUBKEY.close()


def config(internet=None, **lite):
    doc = {"schema": "goatbuilder/v1", "preset": "lite",
           "lite": {"name": "fixture", "host": "192.0.2.50", "ssh_user": "ubuntu",
                    "admin": "admin@example.org", "ssh_pubkey": PUBKEY.name, **lite}}
    if internet is not None:
        doc["lite"]["internet"] = internet
    f = tempfile.NamedTemporaryFile("w", suffix=".yaml", delete=False)
    yaml.safe_dump(doc, f)
    f.close()
    return Path(f.name)


def facts(internet=None, **lite):
    return render.facts(render.load_config(config(internet, **lite), "fixture"))


class Accepts(unittest.TestCase):
    def test_the_zone_alone_takes_every_default(self):
        inet = facts({"zone": "fixture.example.org"})["internet"]
        self.assertEqual(inet["acme_email"], "admin@example.org")
        self.assertEqual(inet["aws_profile"], "fixture")
        self.assertEqual([r["name"] for r in inet["relays"]], ["a", "b"])
        self.assertIsNone(inet["cloudflare_token_ref"])
        self.assertIsNone(inet["mesh_ip"])

    def test_the_store_key_phase_7_reads_is_the_default_named(self):
        inet = facts({"zone": "fixture.example.org", "cloudflare_token_ref": "CLOUDFLARE_API_TOKEN_FIXTURE"})["internet"]
        self.assertIsNone(inet["cloudflare_token_ref"])

    def test_a_reference_is_kept(self):
        inet = facts({"zone": "fixture.example.org", "cloudflare_token_ref": "op://vault/cloudflare/token"})["internet"]
        self.assertEqual(inet["cloudflare_token_ref"], "op://vault/cloudflare/token")

    def test_the_site_zone_is_the_public_apex_and_spire_keeps_the_lan_zone(self):
        f = facts({"zone": "fixture.example.org"})
        self.assertEqual(f["site_zone"], "netbird.fixture.example.org")
        self.assertEqual(f["apex"], "netbird.fixture.example.org")
        self.assertIn('spire_trust_domain: "fixture.fixture.lan"', render.spire_yml(f))


class Refuses(unittest.TestCase):
    def refused(self, internet, needle):
        with self.assertRaises(render.Refuse) as cm:
            facts(internet)
        self.assertIn(needle, str(cm.exception))

    def test_a_lan_only_zone(self):
        for zone in ("fixture.lan", "box.local", "corp.internal", "home.home.arpa"):
            self.refused({"zone": zone}, "LAN-only")

    def test_no_zone(self):
        self.refused({"acme_email": "a@example.org"}, "public DNS zone")

    def test_a_value_where_a_reference_belongs(self):
        self.refused({"zone": "fixture.example.org", "cloudflare_token_ref": "abcdef0123456789abcdef0123456789"},
                     "never a value")
        self.refused({"zone": "fixture.example.org", "cloudflare_token_ref": "MY_CF_TOKEN"}, "no other key name")

    def test_unknown_keys_and_bad_relays(self):
        self.refused({"zone": "fixture.example.org", "port": 443}, "unknown key")
        self.refused({"zone": "fixture.example.org", "relays": [{"name": "a", "region": "mars"}]}, "AWS region")
        self.refused({"zone": "fixture.example.org", "relays": [{"name": "a", "region": "us-east-2"},
                                                                {"name": "a", "region": "eu-west-1"}]}, "unique")
        self.refused({"zone": "fixture.example.org", "relays": []}, "one to four")
        self.refused({"zone": "fixture.example.org", "mesh_ip": "not-an-ip"}, "mesh IPv4")

    def test_an_empty_block(self):
        self.refused({}, "at least `zone:`")


class InternetValues(unittest.TestCase):
    F = facts({"zone": "fixture.example.org", "relays": [{"name": "a", "region": "us-east-2"},
                                                         {"name": "b", "region": "eu-west-1"}]})
    GV = yaml.safe_load(render.vars_yml(F))

    def test_the_rendered_values_are_internet_correct(self):
        self.assertEqual(render.internet_problems(self.GV, self.F), [])
        self.assertIs(self.GV["has_acme"], True)
        self.assertEqual(self.GV["site_dns_zone"], self.GV["public_mgmt_fqdn"])
        self.assertEqual(self.GV["relay_addresses"], ["rels://goat-relay-a.fixture.example.org:33073",
                                                      "rels://goat-relay-b.fixture.example.org:33073"])
        self.assertEqual(self.GV["cloudflare_api_token"], "{{ lookup('env', 'CLOUDFLARE_API_TOKEN_FIXTURE') }}")
        self.assertNotIn("stun_host", self.GV)
        self.assertNotIn("lan_dns_zone", self.GV)

    def test_each_wrong_value_is_named(self):
        cases = {
            "has_acme": False,
            "dashboard_cert_delivery_mode": "file",
            "site_dns_zone": "fixture.lan",
            "public_mgmt_fqdn": "netbird.fixture.lan",
            "cloudflare_api_token": "abcdef0123456789",
            "relay_addresses": [],
            "stun_hosts": ["192.0.2.50"],
            "portal_traefik_cert_resolver": "",
            "portal_traefik_mesh_only_middleware": "lan-mesh-allowlist@file",
            "relay_secret": "a-literal-secret-value",
            "lan_coredns_enabled": True,
            "aws_profile": "",
        }
        for key, bad in cases.items():
            gv = copy.deepcopy(self.GV)
            gv[key] = bad
            problems = render.internet_problems(gv, self.F)
            self.assertTrue(any(key in p for p in problems), f"{key}={bad!r} was not noticed: {problems}")

    def test_a_lan_setting_left_in_is_named(self):
        gv = dict(self.GV, stun_host="192.0.2.50")
        self.assertTrue(any("stun_host" in p for p in render.internet_problems(gv, self.F)))

    def test_the_mesh_address_binds_once_set(self):
        f = facts({"zone": "fixture.example.org", "mesh_ip": "198.51.100.7"})
        gv = yaml.safe_load(render.vars_yml(f))
        self.assertEqual(gv["mesh_ip_a"], "198.51.100.7")
        self.assertEqual(render.internet_problems(gv, f), [])
        zone = render.mesh_hosts(f)
        self.assertTrue(zone.startswith(render.HEADER + "\n"))
        self.assertIn("198.51.100.7  netbird.fixture.example.org", zone)
        self.assertIn("198.51.100.7  dashboard.fixture.netbird.fixture.example.org", zone)


class InternetFiles(unittest.TestCase):
    F = facts({"zone": "fixture.example.org", "relays": [{"name": "a", "region": "us-east-2"},
                                                         {"name": "c", "region": "ap-south-1"}]})

    def test_terraform_carries_what_generate_relays_reads(self):
        tf = render.terraform_tf(self.F)
        self.assertTrue(tf.startswith(render.HEADER + "\n"))
        for needle in ('output "relays"', 'output "ansible_ssm_bucket"', 'module "relay_a"', 'module "relay_c"',
                       'round_robin_dns_name    = "netbird"', "rathole_ingress_enabled = true",
                       "coturn_enabled          = true", 'source    = "../terraform/modules/goat-relay"',
                       'region              = "ap-south-1"', "rathole_secret_name", 'default     = "fixture.example.org"'):
            self.assertIn(needle, tf)

    def test_the_relay_posture_is_le_dns01(self):
        self.assertIn("fixture) echo le-dns01 ;;", render.posture_block(self.F))

    def test_the_manifest_names_the_public_router_and_no_lan_leaves(self):
        m = render.manifest_yaml(self.F)
        self.assertTrue(yaml.safe_load(m))
        self.assertNotIn("mgmt-identity", m)
        self.assertNotIn("dashboard-a", m)
        self.assertIn("zenoh.fixture.netbird.fixture.example.org", m)
        self.assertIn("name: founder", m)

    def test_the_inventory_signal_uri_is_the_public_apex(self):
        self.assertIn("signal_uri=netbird.fixture.example.org:443", render.inventory_block(self.F))


class LanUnchanged(unittest.TestCase):
    def test_a_lan_render_carries_no_internet_setting(self):
        f = facts()
        self.assertIsNone(f["internet"])
        gv = yaml.safe_load(render.vars_yml(f))
        self.assertIs(gv["has_acme"], False)
        self.assertEqual(gv["site_dns_zone"], "fixture.lan")
        self.assertEqual(gv["relay_addresses"], [])
        for key in ("acme_email", "cloudflare_api_token", "stun_hosts", "aws_profile"):
            self.assertNotIn(key, gv)
        self.assertIn("mgmt-identity", render.manifest_yaml(f))


if __name__ == "__main__":
    unittest.main()
