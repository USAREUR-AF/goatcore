#!/usr/bin/env python3
"""mgmt-host.yml on an ACME host: the DNS-01 credential follows traefik_acme_dns_provider.

mgmt-host.yml is the single full writer of /opt/mgmt-host/.env (F-471), so it must render
every ACME key traefik-acme.yml sets. For an azure site that is TRAEFIK_ACME_AZURE_CLIENT_SECRET,
not CLOUDFLARE_DNS_API_TOKEN: without it the next converge deletes the secret and Traefik loses
its resolver; and the pre-flight assert must accept the azure credential instead of refusing the
host for want of a Cloudflare token. Renders the play's own task text; no host is touched.

Run: python3 -m unittest tests/test_mgmt_host_acme_provider.py -v
"""
import unittest
from pathlib import Path

import yaml
from jinja2 import DictLoader, Environment, StrictUndefined, Undefined, select_autoescape

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/mgmt-host.yml"
ASSERT = "Assert the DNS-01 credential resolves on an ACME host"
RENDER = "Render .env from playbook vars"


def tasks():
    out = {}
    for play in yaml.safe_load(PLAY.read_text()):
        for t in play.get("tasks", []) or []:
            out[t.get("name")] = t
    return out


def env(strict=False, content=""):
    # These named fixtures model Ansible's text configuration, not HTML.
    e = Environment(loader=DictLoader({"fixture.env": content}),
                    autoescape=select_autoescape(disabled_extensions=("env",),
                                                 default=True, default_for_string=True),
                    undefined=StrictUndefined if strict else Undefined, trim_blocks=True)
    e.filters["bool"] = lambda v: str(v).lower() in ("1", "true", "yes", "on")
    e.filters["mandatory"] = lambda v: v  # Ansible filter; the fixtures supply what it guards
    return e


def check_assert(hostvars):
    """True when the play's assert passes for these hostvars."""
    t = tasks()[ASSERT]
    ctx = dict(hostvars)
    ctx["lookup"] = lambda kind, name, default="": ctx.get(name, default)
    for k, v in t["vars"].items():
        ctx[k] = env(content=v).get_template("fixture.env").render(**ctx)
    return all(env(content="{{ (" + cond + ") }}").get_template("fixture.env").render(**ctx) == "True"
               for cond in t["ansible.builtin.assert"]["that"])


def render_env(hostvars):
    t = tasks()[RENDER]
    ctx = {"_mgmt_acme_overlay": {"stat": {"exists": True}},
           "ansible_date_time": {"iso8601": "2026-01-01T00:00:00Z"}, **hostvars}
    return env(content=t["ansible.builtin.copy"]["content"]).get_template("fixture.env").render(**ctx)


AZURE = {"traefik_acme_dns_provider": "azure", "traefik_acme_azure_client_secret": "az-fixture-secret"}
CF = {"cloudflare_api_token": "cf-fixture-token"}


class AcmeProvider(unittest.TestCase):
    def test_cloudflare_default_unchanged(self):
        self.assertTrue(check_assert(CF))
        self.assertFalse(check_assert({}))
        out = render_env(CF)
        self.assertIn("CLOUDFLARE_DNS_API_TOKEN=cf-fixture-token", out)
        self.assertNotIn("TRAEFIK_ACME_AZURE_CLIENT_SECRET", out)

    def test_azure_needs_its_own_secret_not_cloudflare(self):
        self.assertTrue(check_assert(AZURE))
        self.assertFalse(check_assert({"traefik_acme_dns_provider": "azure"}))
        self.assertFalse(check_assert({"traefik_acme_dns_provider": "azure", **CF}))

    def test_azure_secret_survives_the_full_render(self):
        out = render_env(AZURE)
        self.assertIn("TRAEFIK_ACME_AZURE_CLIENT_SECRET=az-fixture-secret", out)
        self.assertIn("COMPOSE_FILE=docker-compose.yml:docker-compose.acme.yml", out)
        self.assertNotIn("CLOUDFLARE_DNS_API_TOKEN", out)

    def test_azure_render_needs_no_cloudflare_token(self):
        # Ansible fails a template on an undefined variable; only the ACME block is checked
        # strictly, the rest of the .env takes the fixtures' defaults.
        t = tasks()[RENDER]["ansible.builtin.copy"]["content"]
        block = t[t.index("{% if _mgmt_acme_overlay.stat.exists %}"):]
        out = env(strict=True, content=block).get_template("fixture.env").render(_mgmt_acme_overlay={"stat": {"exists": True}}, **AZURE)
        self.assertIn("TRAEFIK_ACME_AZURE_CLIENT_SECRET=az-fixture-secret", out)

    def test_credentials_are_not_html_escaped(self):
        secret = "fixture&value<angle>"
        out = render_env({**AZURE, "traefik_acme_azure_client_secret": secret})
        self.assertIn("TRAEFIK_ACME_AZURE_CLIENT_SECRET=" + secret + "\n", out)
        self.assertTrue(check_assert({**AZURE, "traefik_acme_azure_client_secret": secret}))

    def test_only_named_env_templates_disable_html_escaping(self):
        policy = env().autoescape
        self.assertFalse(policy("fixture.env"))
        for name in (None, "page.html", "page.xml", "unknown"):
            self.assertTrue(policy(name))
        self.assertEqual(env().from_string("{{ value }}").render(value="<&"), "&lt;&amp;")

    def test_unknown_provider_refused(self):
        self.assertFalse(check_assert({"traefik_acme_dns_provider": "route53", **CF}))


if __name__ == "__main__":
    unittest.main()
