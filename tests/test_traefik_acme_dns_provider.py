#!/usr/bin/env python3
"""traefik-acme.yml DNS-01 provider switch: cloudflare (default) or azure.

The overlay renders the Cloudflare path exactly as before and the Azure path
with lego's `azuredns` env names; the play's pre-flight refuses an Azure site
whose service principal does not resolve; the standup driver's phase 7 asks
for the Azure client secret, not a Cloudflare token, when the site selects
azure. No host is touched: the asserts run on localhost, the driver with --plan.

Run: python3 -m unittest tests/test_traefik_acme_dns_provider.py -v
"""
import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from jinja2 import Environment, FileSystemLoader, StrictUndefined, select_autoescape
import yaml

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/traefik-acme.yml"
TEMPLATE = ROOT / "ops/ansible/templates/traefik-acme/docker-compose.acme.yml.j2"
BASE = {"acme_email": "ops@example.com", "le_dir": "/opt/mgmt-host/letsencrypt",
        "dynamic_dir": "/etc/traefik/dynamic"}
AZURE = {"acme_dns_provider": "azure", "traefik_acme_azure_tenant_id": "tenant-fixture",
         "traefik_acme_azure_client_id": "client-fixture",
         "traefik_acme_azure_subscription_id": "subscription-fixture",
         "traefik_acme_azure_resource_group": "rg-fixture"}
CF_LINES = [
    "      - --certificatesresolvers.le.acme.dnschallenge.provider=cloudflare\n",
    "      - --certificatesresolvers.le.acme.dnschallenge.resolvers=1.1.1.1:53,8.8.8.8:53\n",
    "      - CLOUDFLARE_DNS_API_TOKEN=${CLOUDFLARE_DNS_API_TOKEN:?CLOUDFLARE_DNS_API_TOKEN must be set in /opt/mgmt-host/.env}\n",
]


def render(**extra):
    # Ansible's template module: trim_blocks on, trailing newline kept.
    env = Environment(loader=FileSystemLoader(TEMPLATE.parent),
                      autoescape=select_autoescape(disabled_extensions=("yml.j2",),
                                                   default=True, default_for_string=True),
                      undefined=StrictUndefined, trim_blocks=True, keep_trailing_newline=True)
    return env.get_template(TEMPLATE.name).render(**{**BASE, **extra})


class Overlay(unittest.TestCase):
    def test_cloudflare_is_the_default_and_unchanged(self):
        default = render()
        self.assertEqual(default, render(acme_dns_provider="cloudflare"))
        for line in CF_LINES:
            self.assertIn(line, default)
        self.assertNotIn("AZURE", default)
        self.assertNotIn("azuredns", default)
        yaml.safe_load(default)

    def test_azure_renders_azuredns_and_its_env_names(self):
        out = render(**AZURE)
        doc = yaml.safe_load(out)
        cmd = doc["services"]["traefik"]["command"]
        env = dict(e.split("=", 1) for e in doc["services"]["traefik"]["environment"])
        self.assertIn("--certificatesresolvers.le.acme.dnschallenge.provider=azuredns", cmd)
        self.assertEqual(env["AZURE_AUTH_METHOD"], "env")
        self.assertEqual(env["AZURE_TENANT_ID"], "tenant-fixture")
        self.assertEqual(env["AZURE_CLIENT_ID"], "client-fixture")
        self.assertEqual(env["AZURE_SUBSCRIPTION_ID"], "subscription-fixture")
        self.assertEqual(env["AZURE_RESOURCE_GROUP"], "rg-fixture")
        # The secret is never rendered: compose interpolates it from .env.
        self.assertTrue(env["AZURE_CLIENT_SECRET"].startswith("${TRAEFIK_ACME_AZURE_CLIENT_SECRET:?"))
        self.assertNotIn("AZURE_ZONE_NAME", env)
        self.assertNotIn("AZURE_ENVIRONMENT", env)
        self.assertNotIn("CLOUDFLARE", out)

    def test_configuration_values_are_not_html_escaped(self):
        value = "fixture&value<angle>"
        doc = yaml.safe_load(render(**{**AZURE, "traefik_acme_azure_resource_group": value}))
        env = dict(e.split("=", 1) for e in doc["services"]["traefik"]["environment"])
        self.assertEqual(env["AZURE_RESOURCE_GROUP"], value)

    def test_azure_optional_zone_environment_and_resolvers(self):
        doc = yaml.safe_load(render(**AZURE, traefik_acme_azure_zone_name="example.com",
                                    traefik_acme_azure_environment="usgovernment",
                                    traefik_acme_dns_resolvers="9.9.9.9:53"))
        env = dict(e.split("=", 1) for e in doc["services"]["traefik"]["environment"])
        self.assertEqual(env["AZURE_ZONE_NAME"], "example.com")
        self.assertEqual(env["AZURE_ENVIRONMENT"], "usgovernment")
        self.assertIn("--certificatesresolvers.le.acme.dnschallenge.resolvers=9.9.9.9:53",
                      doc["services"]["traefik"]["command"])


@unittest.skipUnless(shutil.which("ansible-playbook"), "needs ansible-playbook")
class PreFlight(unittest.TestCase):
    """The play's own DNS-01 asserts, lifted out and run against localhost."""
    WANTED = ("Pre-flight | DNS-01 provider", "Pre-flight | this goatnet's",
              "Pre-flight | public_mgmt_fqdn sits")

    def run_preflight(self, **group_vars):
        play = yaml.safe_load(PLAY.read_text())[0]
        tasks = [t for t in play["pre_tasks"] if t["name"].startswith(self.WANTED)]
        self.assertEqual(len(tasks), 4, "provider, Cloudflare, Azure and zone asserts")
        vars_ = {k: play["vars"][k] for k in ("cf_api_token", "acme_dns_provider")}
        with tempfile.TemporaryDirectory() as tmp:
            pb = Path(tmp) / "pb.yml"
            pb.write_text(yaml.safe_dump([{"hosts": "localhost", "gather_facts": False,
                                           "vars": vars_, "tasks": tasks}]))
            ev = Path(tmp) / "ev.json"
            ev.write_text(json.dumps(group_vars))
            env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_LOCALHOST_WARNING="False",
                       ANSIBLE_INVENTORY_UNPARSED_WARNING="False")
            return subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local",
                                   str(pb), "-e", f"@{ev}"],
                                  cwd=tmp, env=env, capture_output=True, text=True, timeout=120)

    def full_azure(self, **over):
        v = {"traefik_acme_dns_provider": "azure", "public_mgmt_fqdn": "netbird.example.com",
             "traefik_acme_azure_tenant_id": "tenant-fixture",
             "traefik_acme_azure_client_id": "client-fixture",
             "traefik_acme_azure_client_secret": "fixture-secret-value",
             "traefik_acme_azure_subscription_id": "subscription-fixture",
             "traefik_acme_azure_resource_group": "rg-fixture"}
        v.update(over)
        return v

    def test_azure_refuses_a_missing_client_secret_and_names_it(self):
        r = self.run_preflight(**self.full_azure(traefik_acme_azure_client_secret=""))
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("traefik_acme_azure_client_secret", r.stdout)
        self.assertNotIn("traefik_acme_azure_tenant_id,", r.stdout)

    def test_azure_refuses_an_undeclared_principal(self):
        r = self.run_preflight(traefik_acme_dns_provider="azure")
        self.assertNotEqual(r.returncode, 0, r.stdout)
        for name in ("tenant_id", "client_id", "client_secret", "subscription_id", "resource_group"):
            self.assertIn("traefik_acme_azure_" + name, r.stdout)

    def test_azure_with_every_value_passes_without_a_cloudflare_token(self):
        r = self.run_preflight(**self.full_azure())
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertNotIn("fixture-secret-value", r.stdout + r.stderr)

    def test_azure_refuses_a_name_outside_the_pinned_zone(self):
        r = self.run_preflight(**self.full_azure(traefik_acme_azure_zone_name="other.example"))
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("is not in", r.stdout)
        ok = self.run_preflight(**self.full_azure(traefik_acme_azure_zone_name="example.com"))
        self.assertEqual(ok.returncode, 0, ok.stdout)

    def test_cloudflare_default_still_refuses_an_empty_token(self):
        r = self.run_preflight()
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("cf_api_token is empty", r.stdout)
        self.assertEqual(self.run_preflight(cloudflare_api_token="t" * 40).returncode, 0)

    def test_an_unknown_provider_is_refused(self):
        r = self.run_preflight(traefik_acme_dns_provider="route53")
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("not supported", r.stdout)


class Driver(unittest.TestCase):
    """goat-standup.sh phase 7, planned from a fixture tree (no host, no AWS)."""

    def phase7(self, vars_text):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            (repo / "ops/scripts").mkdir(parents=True)
            gv = repo / "ops/ansible/group_vars/acmetest_hosts"
            gv.mkdir(parents=True)
            (gv / "vars.yml").write_text(vars_text)
            shutil.copy(ROOT / "ops/scripts/goat-standup.sh", repo / "ops/scripts/goat-standup.sh")
            env = {"PATH": os.environ["PATH"], "HOME": tmp}
            r = subprocess.run(["bash", str(repo / "ops/scripts/goat-standup.sh"), "acmetest",
                                "--plan", "--profile", "internet", "--from", "7", "--to", "7"],
                               capture_output=True, text=True, env=env, timeout=60)
            self.assertEqual(r.returncode, 0, r.stderr)
            m = re.search(r"=== Phase 7 \[A\] \[internet\] public TLS certificate \(traefik-acme\) ===\n    (.*)\n",
                          r.stdout)
            self.assertIsNotNone(m, r.stdout)
            return m.group(1)

    def test_cloudflare_site_keeps_the_token_step(self):
        cmd = self.phase7("dashboard_cert_delivery_mode: acme\n")
        self.assertIn('-e cf_api_token="$(printenv CLOUDFLARE_API_TOKEN_ACMETEST)"', cmd)
        self.assertNotIn("AZURE", cmd)

    def test_azure_site_asks_for_the_client_secret_not_a_cloudflare_token(self):
        for line in ("traefik_acme_dns_provider: azure\n", 'traefik_acme_dns_provider: "azure"  # x\n'):
            cmd = self.phase7(line)
            self.assertNotIn("CLOUDFLARE", cmd)
            self.assertNotIn("cf_api_token", cmd)
            self.assertIn('test -n "$(printenv AZURE_DNS_CLIENT_SECRET_ACMETEST)" && ', cmd)
            self.assertTrue(cmd.endswith("traefik-acme.yml -e target=acmetest_hosts"), cmd)
            # The shared runner refuses it by name while the secret is unset.
            chk = subprocess.run([str(ROOT / "ops/scripts/standup-phase-run"), "--check", cmd],
                                 capture_output=True, text=True,
                                 env={"PATH": os.environ["PATH"], "HOME": tempfile.mkdtemp()})
            self.assertEqual(chk.returncode, 3, chk.stderr)
            self.assertIn("AZURE_DNS_CLIENT_SECRET_ACMETEST", chk.stderr)


class Posture(unittest.TestCase):
    def test_azure_names_are_external_never_generated(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("posture", ROOT / "ops/secrets/posture.py")
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        for name in ("AZURE_DNS_CLIENT_SECRET_SITE", "AZURE_DNS_CLIENT_ID_SITE", "SITE_AZURE_TENANT_ID"):
            self.assertTrue(mod.EXTERNAL.search(name), name)
        self.assertFalse(mod.EXTERNAL.search("SITE_PG_PASS"))


if __name__ == "__main__":
    unittest.main()
