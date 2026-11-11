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

    def _write(self, name, text):
        path = os.path.join(self.tmp.name, name)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(text)
        return path

    def gen(self, project, posture=None):
        return gen_config.generate(self.pins, self.ladder, project, posture or self.none)

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
