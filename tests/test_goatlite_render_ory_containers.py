"""A goatlite names its Ory containers as its own compose names them.

ory-narrow-repset (and postgres-encoding-migrate) wait for the Kratos and Hydra
containers by name, ory_spock_kratos_container and ory_spock_hydra_container,
defaulting to kratos-a and hydra-a. A goatlite's compose
(ops/compose/mgmt-host, project = the compose directory) sets no container_name,
so its containers are mgmt-host-kratos-1 and mgmt-host-hydra-1. A goatlite that
grew a second site (ory_spock_enabled) waited on containers that do not exist,
and join-node phase 13 stopped there (measured in a VM founder run).

Run: python3 -m pytest tests/test_goatlite_render_ory_containers.py
"""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
COMPOSE = ROOT / "ops/compose/mgmt-host/docker-compose.ory.yml"


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    return mod


render = load("goatlite_render", ROOT / "ops/scripts/goatlite-render.py")


def rendered(**lite):
    d = Path(tempfile.mkdtemp())
    pub = d / "key.pub"
    pub.write_text("placeholder: load_config only checks that the file exists\n")
    cfg = d / "goatlite.yaml"
    cfg.write_text(yaml.safe_dump({"schema": "goatbuilder/v1", "preset": "lite",
                                   "lite": {"name": "fixture", "host": "192.0.2.50", "ssh_user": "ubuntu",
                                            "admin": "admin@example.org", "ssh_pubkey": str(pub), **lite}}))
    return yaml.safe_load(render.vars_yml(render.facts(render.load_config(cfg, "fixture"))))


def compose_name(service, project="mgmt-host"):
    """The name docker compose gives a service's first container."""
    svc = yaml.safe_load(COMPOSE.read_text())["services"][service]
    return svc.get("container_name") or f"{project}-{service}-1"


class OryContainers(unittest.TestCase):
    def check(self, gv):
        self.assertEqual(gv.get("ory_spock_kratos_container"), compose_name("kratos"))
        self.assertEqual(gv.get("ory_spock_hydra_container"), compose_name("hydra"))

    def test_a_lan_goatlite_names_them_as_its_compose_does(self):
        self.check(rendered())

    def test_an_internet_goatlite_names_them_as_its_compose_does(self):
        self.check(rendered(internet={"zone": "fixture.example.org",
                                      "relays": [{"name": "a", "region": "us-east-2"}]}))


if __name__ == "__main__":
    unittest.main()
