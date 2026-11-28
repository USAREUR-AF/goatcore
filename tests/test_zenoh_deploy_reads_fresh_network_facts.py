"""zenoh-deploy.yml publishes the router on the mesh interface's address, read
from facts. Facts are cached (ansible.cfg), and a stand-up deploys the router
minutes after it enrols the host, so the play reads network facts afresh before
anything uses the address."""
import pathlib
import unittest

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[1]
PLAY = yaml.safe_load((ROOT / "ops/ansible/zenoh-deploy.yml").read_text())[0]


class FreshFacts(unittest.TestCase):
    def test_the_address_comes_from_the_mesh_interface_fact(self):
        self.assertIn("ansible_facts['wt0']", PLAY["vars"]["goat_zenoh_listen_host"])

    def test_network_facts_are_read_first_on_every_run(self):
        first = PLAY["pre_tasks"][0]
        self.assertEqual(first["ansible.builtin.setup"], {"gather_subset": ["network"]})
        self.assertNotIn("when", first)

    def test_facts_are_cached_which_is_why(self):
        cfg = (ROOT / "ops/ansible/ansible.cfg").read_text()
        self.assertIn("fact_caching = jsonfile", cfg)


if __name__ == "__main__":
    unittest.main()
