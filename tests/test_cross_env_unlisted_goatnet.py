"""A goatnet with its own relay-health box that is on no other goatnet's overview.

A new goatnet's stand-up deploys its own box and changes no other goatnet
(cross-env-apply.sh --only). `listed: false` in the roster keeps it out of the
other boxes' rendered config as well, until joining them is decided and
deployed to each goatnet through its own ladder.
"""
import importlib.util
import pathlib
import unittest
from unittest import mock

path = pathlib.Path(__file__).resolve().parents[1] / "ops/goatnets/render-cross-env.py"
spec = importlib.util.spec_from_file_location("render_cross_env", path)
rce = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rce)


def roster(new_listed):
    def g(i, **extra):
        return dict({"id": i, "label": i, "role": "dev",
                     "relay_health": {"box": i.replace("-", "_") + "_relay_health",
                                      "url": f"https://relay-health.{i}.goat.example/",
                                      "inventory": f"inventory/{i}-relay-health.ini", "terraform": f"terraform-{i}"}}, **extra)
    new = g("new-net") if new_listed is None else g("new-net", listed=new_listed)
    return {"goatnets": {"old_a": g("old-a"), "old_b": g("old-b"), "new_net": new}}


class Unlisted(unittest.TestCase):
    def render(self, doc):
        with mock.patch.object(rce, "load", return_value={"goatnets": {}}):
            return rce.render(doc)

    def test_an_unlisted_goatnet_is_in_no_other_boxes_config(self):
        out = self.render(roster(False))
        for box in ("old_a_relay_health", "old_b_relay_health"):
            v = out[box]
            self.assertNotIn("new-net", [r["id"] for r in v["relay_health_goatnets"]])
            self.assertNotIn("new-net", v["relay_health_summary_targets"])
            self.assertNotIn("new-net", v["relay_health_cross_fleet_allowed"])
        mine = out["new_net_relay_health"]
        self.assertEqual([r["id"] for r in mine["relay_health_goatnets"]], ["new-net"])
        self.assertEqual(mine["relay_health_summary_targets"], "")
        self.assertEqual(mine["relay_health_cross_fleet_allowed"], "")

    def test_the_other_boxes_render_the_same_with_or_without_it(self):
        with_new = self.render(roster(False))
        doc = roster(False)
        del doc["goatnets"]["new_net"]
        without = self.render(doc)
        for box in ("old_a_relay_health", "old_b_relay_health"):
            self.assertEqual(with_new[box], without[box])

    def test_a_listed_goatnet_is_on_every_overview(self):
        out = self.render(roster(None))
        self.assertIn("new-net", [r["id"] for r in out["old_a_relay_health"]["relay_health_goatnets"]])
        self.assertIn("old-a", out["new_net_relay_health"]["relay_health_cross_fleet_allowed"])


if __name__ == "__main__":
    unittest.main()
