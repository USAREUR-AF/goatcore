"""The Spock peer walk reads every management host of the tree. A host line with
no ansible_host (a cloud session-manager line) and no signal_uri must not stop it."""
import pathlib
import unittest

import jinja2

ROOT = pathlib.Path(__file__).resolve().parents[1]
ROLE = ROOT / "ops/ansible/roles/spock-mesh-peering/tasks/main.yml"
EXPR = "hostvars[item].spock_mesh_publisher_host | default((hostvars[item].signal_uri | default((hostvars[item].ansible_host | default(item)) ~ ':10000')).split(':')[0], true)"


class Obj(dict):
    __getattr__ = dict.get

    def __getattr__(self, k):
        if k in self:
            return self[k]
        raise AttributeError(k)


class PeerWalk(unittest.TestCase):
    def render(self, hv):
        env = jinja2.Environment(autoescape=True, undefined=jinja2.StrictUndefined)
        return env.from_string("{{ " + EXPR + " }}").render(hostvars={"h": Obj(hv)}, item="h")

    def test_the_role_uses_this_expression(self):
        self.assertIn(EXPR, ROLE.read_text())

    def test_a_host_with_no_address_is_entered_under_its_name(self):
        self.assertEqual(self.render({}), "h")

    def test_an_address_a_signal_uri_and_an_override_keep_their_order(self):
        self.assertEqual(self.render({"ansible_host": "192.0.2.7"}), "192.0.2.7")
        self.assertEqual(self.render({"ansible_host": "192.0.2.7", "signal_uri": "sig.example:443"}), "sig.example")
        self.assertEqual(self.render({"signal_uri": "sig.example:443", "spock_mesh_publisher_host": "100.127.0.1"}), "100.127.0.1")


if __name__ == "__main__":
    unittest.main()
