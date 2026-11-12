"""The goatbuilder config schema is fail-closed and fits this repository.

Two things are locked here. First, the schema rejects every malformed shape it
is meant to reject — an unknown field, a converger or gate outside the
vocabulary, a soak with no duration, a rung with no environment, a stage naming
an environment nobody declared, a joiner site with no founder to join, a
peering naming a site or class that does not exist. Second, the config
generated from this repository's own sources of record (30 real components,
the live ladder) validates — the strongest test a frozen schema gets.

Run: python3 -m unittest discover -s ops/goatbuilder -p 'test_*.py' -v
Needs PyYAML + jsonschema (pip install pyyaml jsonschema).
"""
from __future__ import annotations

import copy
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import validate  # noqa: E402

_spec = importlib.util.spec_from_file_location("gen_config", os.path.join(HERE, "gen-config.py"))
gen_config = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gen_config)  # type: ignore[union-attr]


def minimal() -> dict:
    """The smallest config that validates — the goatlite shape, in effect."""
    return {
        "schema": "goatbuilder/v1",
        "preset": "lite",
        "repo": {"source": "example-org/example-repo", "image_namespace": "ghcr.io/example-org"},
        "components": {"example-service": {"declared_in": "ops/pins/fleet-pins.yaml"}},
        "environments": [{"name": "dev", "converger": "push-ansible"}],
        "promotion": {"path": [{"name": "dev", "kind": "rung", "environment": "dev"}]},
    }


def with_sites() -> dict:
    """One goatnet with a founder site and a joiner site, each with classes."""
    c = minimal()
    c["environments"][0]["sites"] = [
        {"name": "alpha", "role": "founder", "classes": {"core": {}, "edge": {}}},
        {"name": "bravo", "role": "joiner", "joins": "alpha", "classes": {"core": {}, "pod": {"converger": "pull-agent"}}},
    ]
    return c


def goatlite() -> dict:
    """The learner's file (goatlite.md §2): four facts, no environments, no promotion."""
    return {
        "schema": "goatbuilder/v1",
        "preset": "lite",
        "lite": {"name": "pasture", "host": "pasture-box.lan", "ssh_user": "ubuntu", "admin": "admin@example.org"},
        "repo": {"source": "example-org/goatnet-from-scratch", "image_namespace": "ghcr.io/example-org"},
        "components": {"netbird-mgmt": {"declared_in": "declared/pins.yaml"}},
    }


def _has_live_ladder():
    try:
        gen_config.generate()
    except SystemExit:
        return False
    return True


HAS_LIVE_LADDER = _has_live_ladder()


class MinimalShape(unittest.TestCase):
    def test_minimal_config_validates(self):
        self.assertEqual(validate.errors_for(minimal()), [])

    def test_four_field_goatlite_config_validates(self):
        self.assertEqual(validate.errors_for(goatlite()), [])

    def test_goatlite_derives_one_environment_one_site_one_rung_and_the_env_file_store(self):
        d = validate.derive_lite(goatlite())
        self.assertEqual(d["environments"], [{
            "name": "pasture", "converger": "push-ansible", "transport": "direct", "groups": ["pasture_hosts"],
            "sites": [{"name": "pasture", "role": "founder", "classes": {"core": {}}}],
        }])
        self.assertEqual(d["promotion"], {"path": [{"name": "pasture", "kind": "rung", "environment": "pasture"}]})
        self.assertEqual(d["secrets"], {"store": "env-file"})
        self.assertEqual(validate.errors_for(d), [], "the derived shape validates as a full declaration")
        self.assertNotIn("environments", goatlite(), "the input is never mutated")

    def test_goatlite_hyphenated_name_derives_an_underscored_environment(self):
        c = goatlite(); c["lite"]["name"] = "my-pasture"
        d = validate.derive_lite(c)
        self.assertEqual(d["environments"][0]["name"], "my_pasture")
        self.assertEqual(d["environments"][0]["groups"], ["my_pasture_hosts"])
        self.assertEqual(d["environments"][0]["sites"][0]["name"], "my-pasture")
        self.assertEqual(validate.errors_for(c), [])

    def test_goatlite_explicit_fields_win(self):
        c = goatlite()
        c["environments"] = [{"name": "pasture", "converger": "pull-agent"}]
        c["secrets"] = {"store": "sops"}
        d = validate.derive_lite(c)
        self.assertEqual(d["environments"][0]["converger"], "pull-agent")
        self.assertEqual(d["secrets"], {"store": "sops"})
        self.assertEqual(d["promotion"]["path"][0]["environment"], "pasture")
        self.assertEqual(validate.errors_for(c), [])

    @unittest.skipUnless(HAS_LIVE_LADDER, "ops/release/ladder.yaml has no rungs in this tree")
    def test_four_fact_goatlite_takes_repo_and_components_from_the_checkout(self):
        four = goatlite(); del four["repo"]; del four["components"]
        own = gen_config.generate()  # this repository's own config, as the checkout's
        self.assertEqual(validate.errors_for(four, checkout=own), [])
        d = validate.derive_lite(four, own)
        self.assertEqual(d["repo"], own["repo"])
        self.assertIn("microscope", d["components"])
        self.assertEqual(d["environments"][0]["name"], "pasture", "the lite: block still derives the rest")
        self.assertNotIn("repo", four, "the input is never mutated")
        explicit = validate.derive_lite(goatlite(), own)
        self.assertEqual(explicit["repo"]["source"], "example-org/goatnet-from-scratch", "an explicit block wins")
        problems = validate.errors_for(four)  # no checkout config to take them from
        self.assertEqual(problems, [f"(root): {validate.NO_CHECKOUT}"])

    def test_goatlite_optional_fields_and_the_growth_block_validate(self):
        c = goatlite()
        c["lite"].update({"zone": "pasture.home.arpa", "disk_encryption": False,
                          "data_disk": "/dev/sda", "ssh_pubkey": "~/.ssh/id_ed25519.pub",
                          "internet": {"zone": "pasture.example.org", "aws_profile": "pasture", "cloudflare_token_ref": "CLOUDFLARE_API_TOKEN_PASTURE"}})
        self.assertEqual(validate.errors_for(c), [])
        c["lite"]["data_disk"] = ""
        self.assertTrue(any("lite/data_disk" in p for p in validate.errors_for(c)))

    def test_founder_and_joiner_sites_validate(self):
        self.assertEqual(validate.errors_for(with_sites()), [])

    def test_per_class_pull_override_under_push_site_is_allowed(self):
        c = with_sites()
        self.assertEqual(c["environments"][0]["sites"][1]["classes"]["pod"]["converger"], "pull-agent")
        self.assertEqual(validate.errors_for(c), [])

    def test_peering_validates(self):
        c = with_sites()
        c["peerings"] = [{
            "with": "velmara", "environment": "dev", "site": "alpha", "edge_class": "edge",
            "tunnel": "wireguard", "trust": "spire-federation", "ingress_prefix": "from/velmara/",
        }]
        self.assertEqual(validate.errors_for(c), [])

    def test_peering_offer_transport_values_hosts_forwarder_validate(self):
        c = with_sites()
        c["peerings"] = [{
            "with": "xab", "environment": "dev", "site": "alpha", "edge_class": "edge",
            "offer": ["REL-ALL", "release/xab/**"], "tunnel": "wireguard",
            "values": {"wg_listen_port": 51825, "link_subnet": "169.254.10.2/31"},
            "hosts": {"edge": "192.0.2.20", "core": "192.0.2.10", "user": "goat"},
            "forwarder": {"endpoint": "203.0.113.7", "group": "peering_forwarders", "ansible_host": "203.0.113.7"},
        }]
        self.assertEqual(validate.errors_for(c), [])


class EdgeReservedUdp(unittest.TestCase):
    """environments[].peering_edge_reserved_udp is per environment, where the
    allocator reads it (crates/goatbuilder/src/federate/alloc.rs)."""

    def test_an_environment_declares_its_edge_reserved_udp(self):
        c = with_sites()
        c["environments"][0]["peering_edge_reserved_udp"] = []
        self.assertEqual(validate.errors_for(c), [])
        c["environments"][0]["peering_edge_reserved_udp"] = [51820, 51900]
        self.assertEqual(validate.errors_for(c), [])

    def test_a_site_does_not(self):
        c = with_sites()
        c["environments"][0]["sites"][0]["peering_edge_reserved_udp"] = []
        self.assertTrue(any("peering_edge_reserved_udp" in e for e in validate.errors_for(c)))


class PeeringDeployment(unittest.TestCase):
    def test_deployment_is_hand_or_rendered(self):
        c = with_sites()
        c["peerings"] = [{"with": "xab", "environment": "dev", "site": "alpha", "edge_class": "edge", "deployment": "rendered"}]
        self.assertEqual(validate.errors_for(c), [])
        c["peerings"][0]["deployment"] = "generated"
        self.assertTrue(any("generated" in e for e in validate.errors_for(c)))


class PeeringFederationToml(unittest.TestCase):
    def test_federation_toml_is_hand_or_rendered(self):
        c = with_sites()
        c["peerings"] = [{"with": "xab", "environment": "dev", "site": "alpha", "edge_class": "edge", "federation_toml": "rendered"}]
        self.assertEqual(validate.errors_for(c), [])
        c["peerings"][0]["federation_toml"] = "template"
        self.assertTrue(any("template" in e for e in validate.errors_for(c)))


class PeeringDirectory(unittest.TestCase):
    """environments[].peering_directory names the directory and its CA for
    goatbuilder federate (crates/goatbuilder/src/federate/declared.rs)."""

    def test_url_and_ca_validate(self):
        c = with_sites()
        c["environments"][0]["peering_directory"] = {"url": "https://directory.example.org:8443", "ca_cert": "directory-ca.pem"}
        self.assertEqual(validate.errors_for(c), [])
        c["environments"][0]["peering_directory"] = {"url": "https://directory.example.org"}
        self.assertEqual(validate.errors_for(c), [])

    def test_plain_http_no_url_unknown_field_or_a_site_is_rejected(self):
        for bad, fragment in (({"url": "http://directory.example.org"}, "url"), ({"ca_cert": "x.pem"}, "url"),
                              ({"url": "https://d.example.org", "token": "x"}, "token"),
                              ({"url": "https://d.example.org/'$(x)'"}, "url")):
            c = with_sites()
            c["environments"][0]["peering_directory"] = bad
            problems = validate.errors_for(c)
            self.assertTrue(any(fragment in p for p in problems), (bad, problems))
        c = with_sites()
        c["environments"][0]["sites"][0]["peering_directory"] = {"url": "https://d.example.org"}
        self.assertTrue(any("peering_directory" in e for e in validate.errors_for(c)))


class SecretsFederation(unittest.TestCase):
    """secrets.federation names this side's federation key paths for
    goatbuilder federate (crates/goatbuilder/src/federate/declared.rs)."""

    def test_paths_validate(self):
        c = minimal()
        c["secrets"] = {"store": "env-file", "federation": {
            "signing_key": "federation/signing.key", "ca_cert": "/srv/fed/ca.pem", "ca_key": "~/fed/ca-key.pem"}}
        self.assertEqual(validate.errors_for(c), [])

    def test_unknown_empty_or_shell_unsafe_is_rejected(self):
        for bad, fragment in (({"signing_key": "a", "seed": "b"}, "seed"), ({}, "federation"),
                              ({"ca_key": ""}, "ca_key"), ({"ca_cert": "/srv/$(x)/ca.pem"}, "ca_cert")):
            c = minimal()
            c["secrets"] = {"store": "env-file", "federation": bad}
            problems = validate.errors_for(c)
            self.assertTrue(any(fragment in p for p in problems), (bad, problems))


class FailClosed(unittest.TestCase):
    def assertRejected(self, config: dict, fragment: str):
        problems = validate.errors_for(config)
        self.assertTrue(problems, "expected a validation error, got none")
        self.assertTrue(any(fragment in p for p in problems), f"{fragment!r} not in {problems}")

    def test_unknown_top_level_field(self):
        c = minimal(); c["surprise"] = 1
        self.assertRejected(c, "surprise")

    def test_lite_with_neither_environments_nor_a_lite_block_is_rejected(self):
        c = goatlite(); del c["lite"]
        self.assertRejected(c, "preset lite: declare environments or a lite: block")

    def test_full_preset_still_requires_environments_and_promotion(self):
        c = goatlite(); c["preset"] = "full"
        self.assertRejected(c, "'environments' is a required property")
        self.assertRejected(c, "'promotion' is a required property")
        del c["preset"]  # absent = full
        self.assertRejected(c, "'environments' is a required property")
        self.assertRejected(c, "'promotion' is a required property")

    def test_lite_block_beside_environments_must_name_one_of_them(self):
        c = goatlite(); c["environments"] = [{"name": "dev", "converger": "push-ansible"}]
        self.assertRejected(c, "lite.name 'pasture' names no declared environment")
        c["environments"][0]["sites"] = [{"name": "pasture"}]
        self.assertEqual(validate.errors_for(c), [], "a site by that name counts")

    def test_lite_block_is_fail_closed(self):
        c = goatlite(); del c["lite"]["admin"]
        self.assertRejected(c, "admin")
        c = goatlite(); c["lite"]["admin"] = "not-an-email"
        self.assertRejected(c, "lite/admin")
        c = goatlite(); c["lite"]["name"] = "Pasture"
        self.assertRejected(c, "lite/name")
        c = goatlite(); c["lite"]["port"] = 22
        self.assertRejected(c, "port")
        c = goatlite(); c["lite"]["internet"] = {"aws_profile": "pasture"}
        self.assertRejected(c, "zone")

    def test_unknown_component_field(self):
        c = minimal(); c["components"]["example-service"]["replicas"] = 3
        self.assertRejected(c, "replicas")

    def test_wrong_schema_version(self):
        c = minimal(); c["schema"] = "goatbuilder/v2"
        self.assertRejected(c, "schema")

    def test_converger_outside_vocabulary(self):
        c = minimal(); c["environments"][0]["converger"] = "ssh-by-hand"
        self.assertRejected(c, "converger")

    def test_transport_on_one_operators_network_is_rejected(self):
        # Tailscale is the bad-day path (ADR 1114), never a transport.
        c = minimal(); c["environments"][0]["transport"] = "tailscale"
        self.assertRejected(c, "transport")
        c["environments"][0]["transport"] = "ssm"
        self.assertEqual(validate.errors_for(c), [])

    def test_gate_outside_vocabulary(self):
        c = minimal(); c["promotion"]["path"][0]["gates"] = [{"id": "vibes"}]
        self.assertRejected(c, "gates/0/id")

    def test_soak_gate_requires_min_days(self):
        c = minimal(); c["promotion"]["path"][0]["gates"] = [{"id": "soak"}]
        self.assertRejected(c, "min_days")

    def test_rung_requires_environment(self):
        c = minimal(); del c["promotion"]["path"][0]["environment"]
        self.assertRejected(c, "environment")

    def test_stage_naming_undeclared_environment(self):
        c = minimal(); c["promotion"]["path"][0]["environment"] = "prod"
        self.assertRejected(c, "undeclared environment 'prod'")

    def test_duplicate_environment_names(self):
        c = minimal(); c["environments"].append({"name": "dev", "converger": "pull-agent"})
        self.assertRejected(c, "duplicate")

    def test_mutable_image_namespace_shape(self):
        c = minimal(); c["repo"]["image_namespace"] = "ghcr.io"
        self.assertRejected(c, "image_namespace")

    def test_joiner_requires_joins(self):
        c = with_sites(); del c["environments"][0]["sites"][1]["joins"]
        self.assertRejected(c, "joins")

    def test_joiner_must_join_a_declared_founder(self):
        c = with_sites(); c["environments"][0]["sites"][1]["joins"] = "charlie"
        self.assertRejected(c, "must join a founder site")

    def test_joiner_cannot_join_a_joiner(self):
        c = with_sites()
        c["environments"][0]["sites"].append({"name": "charlie", "role": "joiner", "joins": "bravo"})
        self.assertRejected(c, "must join a founder site")

    def test_duplicate_site_names(self):
        c = with_sites(); c["environments"][0]["sites"].append({"name": "alpha"})
        self.assertRejected(c, "duplicate site")

    def test_unknown_class_field(self):
        c = with_sites(); c["environments"][0]["sites"][0]["classes"]["core"]["cpus"] = 4
        self.assertRejected(c, "cpus")

    def test_peering_to_undeclared_site(self):
        c = with_sites()
        c["peerings"] = [{"with": "velmara", "environment": "dev", "site": "delta", "edge_class": "edge"}]
        self.assertRejected(c, "undeclared site 'delta'")

    def test_peering_edge_class_must_be_declared(self):
        c = with_sites()
        c["peerings"] = [{"with": "velmara", "environment": "dev", "site": "alpha", "edge_class": "router"}]
        self.assertRejected(c, "edge_class 'router'")

    def test_peering_trust_is_spire_federation_only(self):
        c = with_sites()
        c["peerings"] = [{"with": "velmara", "environment": "dev", "site": "alpha", "edge_class": "edge", "trust": "handshake"}]
        self.assertRejected(c, "trust")

    def test_peering_unknown_value_key(self):
        c = with_sites()
        c["peerings"] = [{"with": "xab", "environment": "dev", "site": "alpha", "edge_class": "edge", "values": {"mtu": 1300}}]
        self.assertRejected(c, "mtu")

    def test_peering_offer_is_set_names_or_release_key_expressions(self):
        c = with_sites()
        c["peerings"] = [{"with": "xab", "environment": "dev", "site": "alpha", "edge_class": "edge", "offer": ["from/xaa/**"]}]
        self.assertRejected(c, "from/xaa/**")

    def test_peering_offer_is_never_empty(self):
        c = with_sites()
        c["peerings"] = [{"with": "xab", "environment": "dev", "site": "alpha", "edge_class": "edge", "offer": []}]
        self.assertRejected(c, "offer")

    def test_peering_reserved_name(self):
        c = with_sites()
        c["peerings"] = [{"with": "status", "environment": "dev", "site": "alpha", "edge_class": "edge"}]
        self.assertRejected(c, "GB-120")

    def test_peering_ingress_prefix_shape(self):
        c = with_sites()
        c["peerings"] = [{"with": "velmara", "environment": "dev", "site": "alpha", "edge_class": "edge", "ingress_prefix": "velmara/"}]
        self.assertRejected(c, "ingress_prefix")


class ProjectValues(unittest.TestCase):
    """repo:, deploy: and secrets: come from the deployment's own files."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        d = self.tmp.name
        self.pins = self._write("pins.yaml", "components:\n  example-service:\n    var: example_service_image\n"
                                "goatnets:\n  dev:\n    members: [dev_hosts]\n")
        self.ladder = self._write("ladder.yaml", "rungs:\n  - {name: dev, goatnet: dev, status: live}\n")
        self.none = os.path.join(d, "absent.yml")
        self.federation = self._write("federation.yml", "environments: {}\npeerings: []\n")

    def _write(self, name, text):
        path = os.path.join(self.tmp.name, name)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(text)
        return path

    def gen(self, project, posture=None):
        return gen_config.generate(
            self.pins, self.ladder, project, posture or self.none, self.federation
        )

    def test_no_project_file_names_no_one(self):
        cfg = self.gen(self.none)
        self.assertEqual(cfg["repo"], gen_config.DEFAULT_REPO)
        self.assertNotIn("deploy", cfg, "undeclared deploy values are left to goatbuilder's own fallbacks")
        self.assertEqual(cfg["secrets"], {"store": "env-file"})
        self.assertEqual(validate.errors_for(cfg), [])

    def test_declared_values_are_carried_in_schema_order(self):
        project = self._write("project.yml", "deploy:\n  state_dir: ~/.pasture/state\n  ssh_user: shepherd\n"
                              "  aws_profile: pasture\nrepo:\n  image_namespace: ghcr.io/pasture\n  source: pasture/flock\n")
        posture = self._write("posture.yml", "store: 1password\nop_account: pasture.1password.com\n")
        cfg = self.gen(project, posture)
        self.assertEqual(list(cfg["repo"].items()),
                         [("source", "pasture/flock"), ("image_namespace", "ghcr.io/pasture"), ("signing", "intent-sign")])
        self.assertEqual(list(cfg["deploy"].items()),
                         [("aws_profile", "pasture"), ("ssh_user", "shepherd"), ("state_dir", "~/.pasture/state")])
        self.assertEqual(cfg["secrets"], {"store": "1password", "op_account": "pasture.1password.com"})
        self.assertEqual(list(cfg)[:4], ["schema", "preset", "repo", "deploy"])
        self.assertEqual(validate.errors_for(cfg), [])

    def test_release_source_is_carried_when_declared(self):
        project = self._write("project.yml", "repo:\n  signing: cosign\n  release_source: pasture/release\n"
                              "  image_namespace: ghcr.io/pasture\n  source: pasture/flock\n")
        cfg = self.gen(project)
        self.assertEqual(list(cfg["repo"].items()),
                         [("source", "pasture/flock"), ("image_namespace", "ghcr.io/pasture"),
                          ("release_source", "pasture/release"), ("signing", "cosign")])
        self.assertEqual(validate.errors_for(cfg), [])

    def test_this_checkouts_project_file_validates(self):
        repo, deploy = gen_config.project()
        cfg = self.gen(gen_config.PROJECT, gen_config.POSTURE)
        self.assertEqual(cfg["repo"], repo)
        self.assertEqual(cfg.get("deploy", {}), deploy)
        self.assertEqual(validate.errors_for(cfg), [])

    def test_refuses_a_repo_without_source_or_namespace(self):
        project = self._write("project.yml", "repo:\n  source: pasture/flock\n")
        with self.assertRaises(SystemExit) as e:
            self.gen(project)
        self.assertIn("image_namespace", str(e.exception))

    def test_refuses_unknown_keys(self):
        for text, word in (("repo: {source: a/b, image_namespace: ghcr.io/a}\nrepos: {}\n", "repos"),
                           ("repo: {source: a/b, image_namespace: ghcr.io/a}\ndeploy: {aws_prof: x}\n", "aws_prof")):
            with self.subTest(word=word), self.assertRaises(SystemExit) as e:
                self.gen(self._write("project.yml", text))
            self.assertIn(word, str(e.exception))


class FederationValues(unittest.TestCase):
    """federation.yml owns environment overlays and peerings."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.pins = self._write(
            "pins.yaml",
            "components:\n  example-service: {}\n"
            "goatnets:\n  dev:\n    members: [dev_hosts]\n",
        )
        self.ladder = self._write(
            "ladder.yaml", "rungs:\n  - {name: dev, goatnet: dev, status: live}\n"
        )
        self.project = self._write(
            "project.yml",
            "repo:\n  source: example-org/example-repo\n"
            "  image_namespace: ghcr.io/example-org\n",
        )
        self.posture = self._write("posture.yml", "store: env-file\n")
        self.out = os.path.join(self.tmp.name, "goatbuilder.yaml")

    def _write(self, name, text):
        path = os.path.join(self.tmp.name, name)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(text)
        return path

    def _source(self, peerings="[]", classes="{core: {}, edge: {}}", extra=""):
        return self._write(
            "federation.yml",
            "environments:\n"
            "  dev:\n"
            f"{extra}"
            "    sites:\n"
            "      - name: alpha\n"
            "        role: founder\n"
            f"        classes: {classes}\n"
            f"peerings: {peerings}\n",
        )

    def _generate(self, federation):
        return gen_config.generate(
            self.pins, self.ladder, self.project, self.posture, federation
        )

    def _read_output(self):
        with open(self.out, "rb") as fh:
            return fh.read()

    def _legacy_text(self, config):
        return "".join(
            line for line in gen_config.render(config).splitlines(keepends=True)
            if not line.startswith(gen_config.FEDERATION_DIGEST_PREFIX)
        )

    def _marked_text(self, config, marker_config):
        marker = gen_config._federation_digest(marker_config)
        return (
            gen_config.HEADER
            + f"{gen_config.FEDERATION_DIGEST_PREFIX}{marker}\n"
            + gen_config.yaml.safe_dump(
                config, sort_keys=False, default_flow_style=False, width=100
            )
        )

    def _assert_refused_unchanged(self, config, source, fragment):
        before = (self._read_output(), os.stat(self.out).st_mtime_ns)
        with self.assertRaises(SystemExit) as e:
            gen_config.write_or_check(
                config, False, self.out, self.tmp.name, source, self.posture
            )
        self.assertIn(fragment, str(e.exception))
        after = (self._read_output(), os.stat(self.out).st_mtime_ns)
        self.assertEqual(before, after)
        return e.exception

    def test_no_peering_omits_the_optional_block(self):
        cfg = self._generate(self._source())
        self.assertNotIn("peerings", cfg)
        self.assertEqual(validate.errors_for(cfg), [])

    def test_public_core_defaults_to_a_missing_federation_source(self):
        missing = os.path.join(self.tmp.name, "missing-federation.yml")
        self.assertFalse(os.path.exists(missing))
        cfg = self._generate(missing)
        self.assertNotIn("peerings", cfg)
        self.assertTrue(all("sites" not in env for env in cfg["environments"]))

    def test_one_peering_and_its_site_survive_and_validate(self):
        source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge}]"
        )
        cfg = self._generate(source)
        self.assertEqual(cfg["environments"][0]["sites"][0]["name"], "alpha")
        self.assertEqual([row["with"] for row in cfg["peerings"]], ["velmara"])
        self.assertEqual(validate.errors_for(cfg), [])

    def test_environment_declarations_survive_actual_regeneration(self):
        source = self._source(
            extra=(
                "    transport: direct\n"
                "    peering_edge_reserved_udp: [51821, 51822]\n"
                "    peering_directory:\n"
                "      url: https://directory.example.org:8443\n"
                "      ca_cert: directory-ca.pem\n"
            )
        )
        cfg = self._generate(source)
        gen_config.write_or_check(cfg, False, self.out, self.tmp.name, source)
        loaded = gen_config.yaml.safe_load(self._read_output())
        environment = loaded["environments"][0]
        self.assertEqual(environment["transport"], "direct")
        self.assertEqual(environment["peering_edge_reserved_udp"], [51821, 51822])
        self.assertEqual(
            environment["peering_directory"],
            {"url": "https://directory.example.org:8443", "ca_cert": "directory-ca.pem"},
        )
        self.assertEqual(environment["sites"][0]["name"], "alpha")
        self.assertEqual(validate.errors_for(loaded), [])

    def test_migration_carries_existing_environment_declarations(self):
        sparse_source = self._source()
        sparse = self._generate(sparse_source)
        existing = copy.deepcopy(sparse)
        existing["environments"][0]["peering_directory"] = {
            "url": "https://directory.example.org:8443",
            "ca_cert": "directory-ca.pem",
        }
        with open(self.out, "w", encoding="utf-8") as fh:
            fh.write(self._legacy_text(existing))
        self._assert_refused_unchanged(sparse, sparse_source, "peering_directory")

        complete_source = self._source(
            extra=(
                "    peering_directory:\n"
                "      url: https://directory.example.org:8443\n"
                "      ca_cert: directory-ca.pem\n"
            )
        )
        complete = self._generate(complete_source)
        gen_config.write_or_check(
            complete, False, self.out, self.tmp.name, complete_source
        )
        loaded = gen_config.yaml.safe_load(self._read_output())
        self.assertEqual(
            loaded["environments"][0]["peering_directory"],
            existing["environments"][0]["peering_directory"],
        )

    def test_generator_owned_environment_fields_are_refused(self):
        for field, value in (
            ("name", "other"),
            ("converger", "push-ansible"),
            ("groups", "[other_hosts]"),
        ):
            with self.subTest(field=field), self.assertRaises(SystemExit) as error:
                self._generate(self._source(extra=f"    {field}: {value}\n"))
            self.assertIn("GB-079", str(error.exception))
            self.assertIn("generator-owned", str(error.exception))

    def test_multiple_peerings_survive_in_declared_order(self):
        source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge}, "
            "{with: xab, environment: dev, site: alpha, edge_class: edge}]"
        )
        cfg = self._generate(source)
        self.assertEqual([row["with"] for row in cfg["peerings"]], ["velmara", "xab"])
        self.assertEqual(validate.errors_for(cfg), [])

    def test_allocator_pins_and_hosts_survive_actual_regeneration(self):
        source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge, "
            "values: {wg_listen_port: 51821}, hosts: {edge: edge-1, core: core-1}}]"
        )
        cfg = self._generate(source)
        gen_config.write_or_check(
            cfg, False, self.out, self.tmp.name, source, self.posture
        )
        loaded = gen_config.yaml.safe_load(self._read_output())
        self.assertEqual(loaded["peerings"][0]["values"]["wg_listen_port"], 51821)
        self.assertEqual(
            loaded["peerings"][0]["hosts"], {"edge": "edge-1", "core": "core-1"}
        )
        self.assertEqual(validate.errors_for(loaded), [])

    def test_render_is_deterministic_with_federation(self):
        source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge}]"
        )
        first = gen_config.render(self._generate(source))
        second = gen_config.render(self._generate(source))
        self.assertEqual(first, second)

    def test_regeneration_preserves_declarations_and_check_is_read_only(self):
        source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge}]"
        )
        cfg = self._generate(source)
        gen_config.write_or_check(cfg, False, self.out, self.tmp.name, source)
        before = (self._read_output(), os.stat(self.out).st_mtime_ns)
        gen_config.write_or_check(cfg, True, self.out, self.tmp.name, source)
        after = (self._read_output(), os.stat(self.out).st_mtime_ns)
        self.assertEqual(before, after)
        loaded = gen_config.yaml.safe_load(before[0])
        self.assertEqual(loaded["environments"][0]["sites"][0]["name"], "alpha")
        self.assertEqual(loaded["peerings"][0]["with"], "velmara")

        drifted = copy.deepcopy(cfg)
        drifted["repo"]["signing"] = "cosign"
        with self.assertRaises(SystemExit) as e:
            gen_config.write_or_check(drifted, True, self.out, self.tmp.name, source)
        self.assertIn("DRIFT", str(e.exception))
        self.assertEqual((self._read_output(), os.stat(self.out).st_mtime_ns), before)

    def test_refuses_malformed_or_undeclared_source(self):
        cases = [
            ("environments: []\npeerings: []\n", "environments:"),
            ("environments: {}\npeerings: {}\n", "peerings:"),
            ("environments: {}\npeerings: []\nsurprise: true\n", "surprise"),
            ("environments:\n  other: {sites: [{name: alpha}]}\npeerings: []\n", "undeclared environments"),
            (
                "environments:\n  dev: {sites: [{name: alpha, classes: {core: {}}}]}\n"
                "peerings: [{with: velmara, environment: dev, site: alpha, edge_class: edge}]\n",
                "edge_class",
            ),
        ]
        for i, (text, fragment) in enumerate(cases):
            with self.subTest(fragment=fragment), self.assertRaises(SystemExit) as e:
                self._generate(self._write(f"bad-{i}.yml", text))
            self.assertIn("GB-079", str(e.exception))
            self.assertIn(fragment, str(e.exception))

    def test_unhashable_site_and_peering_identifiers_are_actionable_refusals(self):
        cases = [
            (
                "environments:\n  dev: {sites: [{name: [alpha], classes: {edge: {}}}]}\npeerings: []\n",
                "environments.dev.sites[0].name",
            ),
            (
                "environments:\n  dev: {sites: [{name: alpha, classes: {edge: {}}}]}\n"
                "peerings: [{with: velmara, environment: [dev], site: alpha, edge_class: edge}]\n",
                "peerings[0].environment",
            ),
            (
                "environments:\n  dev: {sites: [{name: alpha, classes: {edge: {}}}]}\n"
                "peerings: [{with: velmara, environment: dev, site: [alpha], edge_class: edge}]\n",
                "peerings[0].site",
            ),
            (
                "environments:\n  dev: {sites: [{name: alpha, classes: {edge: {}}}]}\n"
                "peerings: [{with: [velmara], environment: dev, site: alpha, edge_class: edge}]\n",
                "peerings[0].with",
            ),
        ]
        for i, (text, field) in enumerate(cases):
            with self.subTest(field=field), self.assertRaises(SystemExit) as e:
                self._generate(self._write(f"identifier-{i}.yml", text))
            self.assertIn("GB-079", str(e.exception))
            self.assertIn(field, str(e.exception))
            self.assertIn("non-empty string", str(e.exception))

    def test_legacy_migration_refuses_same_id_site_class_loss(self):
        source = self._source(classes="{core: {}}")
        wanted = self._generate(source)
        existing = copy.deepcopy(wanted)
        existing["environments"][0]["sites"][0]["classes"]["edge"] = {}
        with open(self.out, "w", encoding="utf-8") as fh:
            fh.write(self._legacy_text(existing))
        self._assert_refused_unchanged(wanted, source, "classes.edge")

    def test_legacy_migration_refuses_changed_peering_trust_and_offer(self):
        old_source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge, "
            "trust: spire-federation, offer: [REL-ALL]}]"
        )
        existing = self._generate(old_source)
        with open(self.out, "w", encoding="utf-8") as fh:
            fh.write(self._legacy_text(existing))
        new_source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge, "
            "trust: handshake, offer: [REL-SUB]}]"
        )
        wanted = self._generate(new_source)
        error = self._assert_refused_unchanged(wanted, new_source, "trust")
        self.assertIn("offer", str(error))

    def test_legacy_migration_refuses_omitted_peering_fields(self):
        full_source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge, "
            "trust: spire-federation, offer: [REL-ALL]}]"
        )
        existing = self._generate(full_source)
        with open(self.out, "w", encoding="utf-8") as fh:
            fh.write(self._legacy_text(existing))
        sparse_source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge}]"
        )
        wanted = self._generate(sparse_source)
        error = str(self._assert_refused_unchanged(wanted, sparse_source, "trust"))
        self.assertIn("peerings[dev/velmara].trust", error)
        self.assertIn("peerings[dev/velmara].offer", error)

    def test_legacy_migration_refuses_missing_site_and_peering_ids(self):
        source = self._source()
        wanted = self._generate(source)
        existing = copy.deepcopy(wanted)
        existing["environments"][0]["sites"].append(
            {"name": "bravo", "role": "joiner", "joins": "alpha", "classes": {"edge": {}}}
        )
        existing["peerings"] = [
            {"with": "xab", "environment": "dev", "site": "alpha", "edge_class": "edge"}
        ]
        with open(self.out, "w", encoding="utf-8") as fh:
            fh.write(self._legacy_text(existing))
        error = self._assert_refused_unchanged(
            wanted, source, "environments[dev].sites['bravo']"
        )
        self.assertIn("peerings[dev/xab]", str(error))

    def test_legacy_migration_is_type_sensitive(self):
        for old, new in (("true", "1"), ("1", "1.0")):
            with self.subTest(old=old, new=new):
                old_source = self._source(extra=f"    transport: {old}\n")
                existing = self._generate(old_source)
                with open(self.out, "w", encoding="utf-8") as fh:
                    fh.write(self._legacy_text(existing))
                new_source = self._source(extra=f"    transport: {new}\n")
                wanted = self._generate(new_source)
                error = self._assert_refused_unchanged(
                    wanted, new_source, "environments[dev].transport"
                )
                self.assertIn(f"{existing['environments'][0]['transport']!r} ->", str(error))

    def test_legacy_migration_detects_lost_list_duplicates(self):
        old_source = self._source(extra="    peering_edge_reserved_udp: [51821, 51821]\n")
        existing = self._generate(old_source)
        with open(self.out, "w", encoding="utf-8") as fh:
            fh.write(self._legacy_text(existing))
        new_source = self._source(extra="    peering_edge_reserved_udp: [51821]\n")
        wanted = self._generate(new_source)
        self._assert_refused_unchanged(
            wanted, new_source, "environments[dev].peering_edge_reserved_udp"
        )

    def test_legacy_migration_requires_federation_key_paths_in_posture(self):
        source = self._source()
        complete_posture = self._write(
            "posture.yml",
            "store: env-file\nfederation:\n"
            "  signing_key: federation/signing.key\n"
            "  ca_cert: federation/ca.pem\n"
            "  ca_key: federation/ca-key.pem\n",
        )
        existing = self._generate(source)
        with open(self.out, "w", encoding="utf-8") as fh:
            fh.write(self._legacy_text(existing))

        self._write("posture.yml", "store: env-file\n")
        sparse = self._generate(source)
        error = self._assert_refused_unchanged(
            sparse, source, "secrets.federation"
        )
        self.assertIn("posture.yml", str(error))

        self._write(
            "posture.yml",
            "store: env-file\nfederation:\n"
            "  signing_key: federation/signing.key\n"
            "  ca_cert: federation/ca.pem\n"
            "  ca_key: federation/ca-key.pem\n",
        )
        adopted = self._generate(source)
        gen_config.write_or_check(
            adopted, False, self.out, self.tmp.name, source, complete_posture
        )
        self.assertIn(gen_config.FEDERATION_DIGEST_PREFIX.encode(), self._read_output())

    def test_adopted_federation_key_paths_require_explicit_posture_edit(self):
        source = self._source()
        self._write(
            "posture.yml",
            "store: env-file\nfederation:\n"
            "  signing_key: federation/signing.key\n"
            "  ca_cert: federation/ca.pem\n"
            "  ca_key: federation/ca-key.pem\n",
        )
        adopted = self._generate(source)
        gen_config.write_or_check(
            adopted, False, self.out, self.tmp.name, source, self.posture
        )

        os.unlink(self.posture)
        without_posture = self._generate(source)
        error = self._assert_refused_unchanged(
            without_posture, source, "posture.yml"
        )
        self.assertIn("Restore", str(error))

        self._write("posture.yml", "store: env-file\n")
        intentionally_removed = self._generate(source)
        gen_config.write_or_check(
            intentionally_removed,
            False,
            self.out,
            self.tmp.name,
            source,
            self.posture,
        )
        loaded = gen_config.yaml.safe_load(self._read_output())
        self.assertNotIn("federation", loaded["secrets"])

    def test_output_only_federation_secret_edit_invalidates_digest(self):
        source = self._source()
        self._write(
            "posture.yml",
            "store: env-file\nfederation:\n  signing_key: federation/signing.key\n",
        )
        adopted = self._generate(source)
        gen_config.write_or_check(
            adopted, False, self.out, self.tmp.name, source, self.posture
        )
        tampered = copy.deepcopy(adopted)
        tampered["secrets"]["federation"]["signing_key"] = "federation/other.key"
        with open(self.out, "w", encoding="utf-8") as fh:
            fh.write(self._marked_text(tampered, adopted))
        self._assert_refused_unchanged(adopted, source, "secrets.federation.signing_key")

    def test_tampered_marked_output_is_not_trusted(self):
        source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge}]"
        )
        wanted = self._generate(source)
        tampered = copy.deepcopy(wanted)
        tampered["peerings"][0]["offer"] = ["REL-ALL"]
        with open(self.out, "w", encoding="utf-8") as fh:
            fh.write(self._marked_text(tampered, wanted))
        self._assert_refused_unchanged(wanted, source, "offer")

    def test_adopted_output_allows_later_intentional_source_changes(self):
        original_source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge, "
            "trust: spire-federation, offer: [REL-ALL]}]"
        )
        original = self._generate(original_source)
        with open(self.out, "w", encoding="utf-8") as fh:
            fh.write(self._legacy_text(original))

        gen_config.write_or_check(
            original, False, self.out, self.tmp.name, original_source
        )
        adopted = self._read_output()
        self.assertIn(gen_config.FEDERATION_DIGEST_PREFIX.encode(), adopted)

        changed_source = self._source(classes="{core: {}}")
        changed = self._generate(changed_source)
        gen_config.write_or_check(
            changed, False, self.out, self.tmp.name, changed_source
        )
        self.assertEqual(self._read_output(), gen_config.render(changed).encode())

    def test_missing_source_cannot_remove_adopted_declarations(self):
        source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge}]"
        )
        adopted = self._generate(source)
        gen_config.write_or_check(adopted, False, self.out, self.tmp.name, source)
        os.unlink(source)
        without_source = self._generate(source)
        error = self._assert_refused_unchanged(
            without_source, source, "is missing"
        )
        self.assertIn("GB-079", str(error))
        self.assertIn("environments: {}", str(error))
        self.assertIn("peerings: []", str(error))

    def test_explicit_empty_source_removes_adopted_declarations(self):
        source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge}]"
        )
        adopted = self._generate(source)
        gen_config.write_or_check(adopted, False, self.out, self.tmp.name, source)
        self._write("federation.yml", "environments: {}\npeerings: []\n")
        removed = self._generate(source)
        gen_config.write_or_check(removed, False, self.out, self.tmp.name, source)
        loaded = gen_config.yaml.safe_load(self._read_output())
        self.assertNotIn("peerings", loaded)
        self.assertTrue(all("sites" not in env for env in loaded["environments"]))
        self.assertEqual(validate.errors_for(loaded), [])

    def test_incomplete_or_null_source_cannot_remove_adopted_declarations(self):
        source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge}]"
        )
        adopted = self._generate(source)
        gen_config.write_or_check(adopted, False, self.out, self.tmp.name, source)
        cases = [
            ("", "missing required top-level keys"),
            ("{}\n", "missing required top-level keys"),
            ("environments: {}\n", "'peerings'"),
            ("peerings: []\n", "'environments'"),
            ("environments: null\npeerings: []\n", "`environments:` must map"),
            ("environments: {}\npeerings: null\n", "`peerings:` must be a list"),
        ]
        for text, fragment in cases:
            before = (self._read_output(), os.stat(self.out).st_mtime_ns)
            with self.subTest(source=text), self.assertRaises(SystemExit) as e:
                self._generate(self._write("federation.yml", text))
            self.assertIn("GB-079", str(e.exception))
            self.assertIn(fragment, str(e.exception))
            self.assertIn("federation.yml", str(e.exception))
            after = (self._read_output(), os.stat(self.out).st_mtime_ns)
            self.assertEqual(before, after)

    def test_duplicate_federation_source_keys_are_refused_without_output_change(self):
        source = self._source(
            "[{with: velmara, environment: dev, site: alpha, edge_class: edge}]"
        )
        adopted = self._generate(source)
        gen_config.write_or_check(adopted, False, self.out, self.tmp.name, source)
        cases = [
            (
                "environments:\n"
                "  dev: {sites: [{name: alpha, classes: {edge: {}}}]}\n"
                "  dev: {sites: [{name: bravo, classes: {edge: {}}}]}\n"
                "peerings: []\n",
                "duplicate key 'dev'",
            ),
            (
                "environments:\n  dev: {sites: [{name: alpha, classes: {edge: {}}}]}\n"
                "peerings: []\n"
                "peerings: [{with: velmara, environment: dev, site: alpha, edge_class: edge}]\n",
                "duplicate key 'peerings'",
            ),
        ]
        for text, fragment in cases:
            before = (self._read_output(), os.stat(self.out).st_mtime_ns)
            with self.subTest(fragment=fragment), self.assertRaises(SystemExit) as e:
                self._generate(self._write("federation.yml", text))
            self.assertIn("GB-079", str(e.exception))
            self.assertIn(fragment, str(e.exception))
            self.assertIn("federation.yml", str(e.exception))
            after = (self._read_output(), os.stat(self.out).st_mtime_ns)
            self.assertEqual(before, after)

    def test_unsupported_yaml_values_are_actionable_and_leave_output_unchanged(self):
        source = self._source()
        adopted = self._generate(source)
        gen_config.write_or_check(adopted, False, self.out, self.tmp.name, source)
        cases = [
            (
                "environments:\n  dev:\n    peering_directory:\n"
                "      url: 2026-11-12\npeerings: []\n",
                "federation.environments.dev.peering_directory.url",
                "date",
            ),
            (
                "environments:\n  dev:\n    peering_directory:\n"
                "      1: one\n      url: https://directory.example.org\npeerings: []\n",
                "federation.environments.dev.peering_directory[1]",
                "mapping key",
            ),
        ]
        for text, path, guidance in cases:
            before = (self._read_output(), os.stat(self.out).st_mtime_ns)
            with self.subTest(path=path), self.assertRaises(SystemExit) as error:
                self._generate(self._write("federation.yml", text))
            message = str(error.exception)
            self.assertIn("GB-079", message)
            self.assertIn(path, message)
            self.assertIn(guidance, message)
            self.assertNotIn("Traceback", message)
            self.assertEqual(
                (self._read_output(), os.stat(self.out).st_mtime_ns), before
            )

    def test_malformed_yaml_is_catalogued_and_leaves_output_unchanged(self):
        source = self._source()
        adopted = self._generate(source)
        gen_config.write_or_check(adopted, False, self.out, self.tmp.name, source)
        before = (self._read_output(), os.stat(self.out).st_mtime_ns)
        with self.assertRaises(SystemExit) as error:
            self._generate(self._write("federation.yml", "environments: [\n"))
        message = str(error.exception)
        self.assertIn("GB-079", message)
        self.assertIn("cannot read federation declarations", message)
        self.assertNotIn("Traceback", message)
        self.assertEqual((self._read_output(), os.stat(self.out).st_mtime_ns), before)


class GeneratorSubprocessRefusals(unittest.TestCase):
    """The executable reports catalogued refusals before touching output."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = self.tmp.name
        self.generator_dir = os.path.join(self.root, "ops", "goatbuilder")
        os.makedirs(self.generator_dir)
        os.makedirs(os.path.join(self.root, "ops", "pins"))
        os.makedirs(os.path.join(self.root, "ops", "release"))
        shutil.copy2(
            os.path.join(HERE, "gen-config.py"),
            os.path.join(self.generator_dir, "gen-config.py"),
        )
        self._write(
            "ops/pins/fleet-pins.yaml",
            "components: {}\ngoatnets:\n  dev:\n    members: [dev_hosts]\n",
        )
        self._write(
            "ops/release/ladder.yaml",
            "rungs:\n  - {name: dev, goatnet: dev, status: live}\n",
        )
        self._write(
            "ops/goatbuilder/project.yml",
            "repo:\n  source: example-org/example-repo\n"
            "  image_namespace: ghcr.io/example-org\n",
        )
        self._write("ops/goatbuilder/posture.yml", "store: env-file\n")
        self._write(
            "ops/goatbuilder/federation.yml", "environments: {}\npeerings: []\n"
        )
        self.output = os.path.join(self.generator_dir, "goatbuilder.yaml")

    def _path(self, relative):
        return os.path.join(self.root, relative)

    def _write(self, relative, text):
        path = self._path(relative)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(text)
        return path

    def _run(self):
        return subprocess.run(
            [sys.executable, os.path.join(self.generator_dir, "gen-config.py")],
            cwd=self.root,
            capture_output=True,
            text=True,
            check=False,
        )

    def _generate_output(self):
        result = self._run()
        self.assertEqual(result.returncode, 0, result.stderr)
        with open(self.output, "rb") as fh:
            return fh.read()

    def _assert_catalogued_unchanged(self, before, result, *fragments):
        self.assertEqual(result.returncode, 1, result)
        self.assertIn("GB-079", result.stderr)
        for fragment in fragments:
            self.assertIn(fragment, result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertNotIn("UnicodeDecodeError", result.stderr)
        with open(self.output, "rb") as fh:
            self.assertEqual(fh.read(), before)

    def test_legacy_federation_secrets_refusal_is_catalogued_and_atomic(self):
        self._write(
            "ops/goatbuilder/posture.yml",
            "store: env-file\nfederation:\n"
            "  signing_key: federation/signing.key\n"
            "  ca_cert: federation/ca.pem\n"
            "  ca_key: federation/ca-key.pem\n",
        )
        adopted = self._generate_output().decode("utf-8")
        legacy = "".join(
            line for line in adopted.splitlines(keepends=True)
            if not line.startswith(gen_config.FEDERATION_DIGEST_PREFIX)
        ).encode("utf-8")
        with open(self.output, "wb") as fh:
            fh.write(legacy)
        self._write("ops/goatbuilder/posture.yml", "store: env-file\n")
        self._assert_catalogued_unchanged(
            legacy, self._run(), "secrets.federation", "posture.yml"
        )

    def test_non_utf8_source_and_output_refusals_are_catalogued_and_atomic(self):
        before = self._generate_output()
        federation = self._path("ops/goatbuilder/federation.yml")
        with open(federation, "wb") as fh:
            fh.write(b"\xff\xfe\x80")
        self._assert_catalogued_unchanged(
            before, self._run(), "federation.yml", "cannot read federation declarations"
        )

        self._write("ops/goatbuilder/federation.yml", "environments: {}\npeerings: []\n")
        invalid_output = b"\xff\xfe\x80"
        with open(self.output, "wb") as fh:
            fh.write(invalid_output)
        self._assert_catalogued_unchanged(
            invalid_output, self._run(), "goatbuilder.yaml", "cannot read the generated output"
        )

    def test_duplicate_peering_identity_refusal_names_source_and_is_atomic(self):
        before = self._generate_output()
        self._write(
            "ops/goatbuilder/federation.yml",
            "environments:\n  dev:\n    sites:\n"
            "      - {name: alpha, classes: {edge: {}}}\n"
            "peerings:\n"
            "  - {with: xab, environment: dev, site: alpha, edge_class: edge}\n"
            "  - {with: xab, environment: dev, site: alpha, edge_class: edge}\n",
        )
        self._assert_catalogued_unchanged(
            before, self._run(), "federation.yml", "duplicates peering identity", "dev/xab"
        )


@unittest.skipUnless(HAS_LIVE_LADDER, "ops/release/ladder.yaml has no rungs in this tree")
class ThisRepository(unittest.TestCase):
    def test_generated_config_validates(self):
        cfg = gen_config.generate()
        self.assertEqual(validate.errors_for(cfg), [])

    def test_generated_config_carries_the_live_ladder(self):
        cfg = gen_config.generate()
        names = [s["name"] for s in cfg["promotion"]["path"]]
        self.assertEqual(names, ["dev", "preprod", "prod"])
        prod = cfg["promotion"]["path"][-1]
        self.assertIn({"id": "soak", "min_days": 1}, prod["gates"])
        self.assertIn({"id": "operator-ack"}, prod["gates"])

    def test_generated_config_keeps_ledger_names_that_differ(self):
        cfg = gen_config.generate()
        self.assertEqual(cfg["components"]["portal"]["ledger"], "goat-zenoh-portal")
        self.assertNotIn("ledger", cfg["components"]["microscope"])

    def test_externally_built_components_have_null_workflow(self):
        cfg = gen_config.generate()
        self.assertIsNone(cfg["components"]["spire"]["image_workflow"])
        self.assertEqual(cfg["components"]["microscope"]["image_workflow"], "microscope-image.yml")

    def test_render_is_deterministic(self):
        a = gen_config.render(gen_config.generate())
        b = gen_config.render(copy.deepcopy(gen_config.generate()))
        self.assertEqual(a, b)


if __name__ == "__main__":
    unittest.main()
