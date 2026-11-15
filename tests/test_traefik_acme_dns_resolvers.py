"""The ACME DNS-01 resolvers of the Traefik overlay are a per-goatnet setting.

A site whose network permits DNS only to its own resolver cannot reach the
public resolvers, so every renewal fails until the overlay asks that resolver.
"""
import pathlib
import re
import unittest

from jinja2 import Environment

ROOT = pathlib.Path(__file__).resolve().parents[1]
TEMPLATE = ROOT / "ops/ansible/templates/traefik-acme/docker-compose.acme.yml.j2"


def flags(**variables):
    """Render the DNS-01 resolver lines of the overlay with the given variables."""
    text = TEMPLATE.read_text()
    start = text.index("      - --certificatesresolvers.le.acme.dnschallenge.resolvers=")
    end = text.index("    environment:", start)
    env = Environment(trim_blocks=True)
    env.filters["bool"] = lambda v: str(v).lower() in ("1", "true", "yes", "on")
    rendered = env.from_string(text[start:end]).render(**variables)
    return [line.strip()[2:] for line in rendered.splitlines() if line.strip().startswith("- --")]


class AcmeDnsResolvers(unittest.TestCase):
    def test_default_is_the_two_public_resolvers_and_no_extra_flag(self):
        self.assertEqual(
            flags(),
            ["--certificatesresolvers.le.acme.dnschallenge.resolvers=1.1.1.1:53,8.8.8.8:53"],
        )

    def test_a_declared_resolver_replaces_the_default(self):
        self.assertEqual(
            flags(acme_dns_resolvers=["192.0.2.53:53"]),
            ["--certificatesresolvers.le.acme.dnschallenge.resolvers=192.0.2.53:53"],
        )

    def test_skipping_authoritative_checks_adds_the_propagation_flag(self):
        got = flags(acme_dns_resolvers=["192.0.2.53:53"], acme_dns_skip_authoritative_checks=True)
        self.assertEqual(got, [
            "--certificatesresolvers.le.acme.dnschallenge.resolvers=192.0.2.53:53",
            "--certificatesresolvers.le.acme.dnschallenge.propagation.disableANSChecks=true",
        ])

    def test_no_resolver_address_is_written_outside_the_variable(self):
        text = TEMPLATE.read_text()
        literal = re.findall(r"dnschallenge\.resolvers=(?!\{\{)", text)
        self.assertEqual(literal, [])


if __name__ == "__main__":
    unittest.main()
