#!/usr/bin/env python3
"""fenced-surface-oidc-client-exists reads its clients from the running containers'
*CLIENT_ID env. Traefik's DNS-01 solver carries AZURE_CLIENT_ID, the app ID of an Entra
service principal: a cloud credential, not a client of this goatnet's Hydra. Counting it
failed the row with "Hydra has no such client" on every site that issues certs through
Azure DNS.

Run: python3 -m unittest tests/test_fence_clients_skip_cloud_creds.py -v
"""
import importlib.util
import sys
import unittest
from pathlib import Path

SRC = Path(__file__).resolve().parents[1] / "ops/validation/probes.py"
spec = importlib.util.spec_from_file_location("acceptance_probes", SRC)
probes = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = probes  # dataclasses look their module up
spec.loader.exec_module(probes)

ISSUER = "https://oauth2.site.example.net"
DOCKER_OUT = "\n".join([
    "/traefik|AZURE_CLIENT_ID=app-id-of-dns-sp|AZURE_TENANT_ID=t|AZURE_RESOURCE_GROUP=rg",
    f"/oauth2-proxy-dashboard|OAUTH2_PROXY_OIDC_ISSUER_URL={ISSUER}|OAUTH2_PROXY_CLIENT_ID=dashboard-admin",
    "/oauth2-proxy-other|OAUTH2_PROXY_OIDC_ISSUER_URL=https://elsewhere.example.net|OAUTH2_PROXY_CLIENT_ID=foreign",
    "/netbird-dashboard|AUTH_AUTHORITY=x|AUTH_CLIENT_ID=netbird-dashboard",
]) + "\n"


class FenceClients(unittest.TestCase):
    def setUp(self):
        self._orig = probes._host_run
        probes._host_run = lambda host, cmd, timeout=30: probes._Run(0, DOCKER_OUT, "", True)

    def tearDown(self):
        probes._host_run = self._orig

    def test_azure_service_principal_is_not_an_oidc_client(self):
        clients, why = probes._fence_clients("goat@host", ISSUER)
        self.assertEqual(why, "")
        self.assertNotIn("app-id-of-dns-sp", clients)

    def test_real_fences_still_counted(self):
        clients, _ = probes._fence_clients("goat@host", ISSUER)
        self.assertEqual(sorted(clients), ["dashboard-admin", "netbird-dashboard"])


if __name__ == "__main__":
    unittest.main()
