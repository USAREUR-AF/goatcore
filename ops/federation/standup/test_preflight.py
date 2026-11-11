"""The phase-12 dependency check fails closed without probing a real host."""
import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
import unittest.mock as mock


class DockerPreflight(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("preflight", Path(__file__).with_name("preflight.py"))
        self.preflight = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.preflight)

    def check(self, binary="docker", result=None, error=None):
        out = io.StringIO()
        with mock.patch("shutil.which", return_value=binary), \
                mock.patch("subprocess.run", return_value=result, side_effect=error) as run, \
                contextlib.redirect_stdout(out):
            self.preflight.check_local_docker()
        return out.getvalue(), run

    def test_missing_binary_is_a_named_refusal(self):
        out, run = self.check(binary=None)
        self.assertEqual(self.preflight.problems, ["docker compose plugin (phase 12)"])
        self.assertIn("no docker", out)
        run.assert_not_called()

    def test_timeout_and_failed_execution_are_refused(self):
        for error in [subprocess.TimeoutExpired("docker", 15), OSError("cannot execute")]:
            with self.subTest(error=error):
                self.preflight.problems.clear()
                self.check(error=error)
                self.assertEqual(self.preflight.problems, ["docker compose plugin (phase 12)"])

    def test_missing_plugin_is_refused(self):
        out, _ = self.check(result=subprocess.CompletedProcess([], 1, "", "missing"))
        self.assertTrue(self.preflight.problems)
        self.assertIn("Install", out)

    def test_empty_success_is_not_evidence_of_a_version(self):
        out, _ = self.check(result=subprocess.CompletedProcess([], 0, "", ""))
        self.assertTrue(self.preflight.problems)
        self.assertIn("no version", out)

    def test_verified_version_passes(self):
        out, run = self.check(result=subprocess.CompletedProcess([], 0, "Docker Compose version v2.0\n", ""))
        self.assertEqual(self.preflight.problems, [])
        self.assertIn("Docker Compose version v2.0", out)
        run.assert_called_once_with(["docker", "compose", "version"], capture_output=True,
                                    text=True, timeout=15)


class SharedInventoryNames(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("preflight_inv", Path(__file__).with_name("preflight.py"))
        self.p = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.p)

    INV = """[peering_edges:children]
b_edge
c_edge

[b_edge]
{b_edge} ansible_host=192.0.2.10

[c_edge]
{c_edge} ansible_host=192.0.2.10

[peering_cores:children]
b_core
c_core

[b_core]
{b_core} ansible_host=192.0.2.20

[c_core]
{c_core} ansible_host=192.0.2.20
"""

    def test_one_name_in_two_peering_groups_is_found_for_edge_and_core(self):
        inv = self.INV.format(b_edge="edge", c_edge="edge", b_core="core", c_core="core")
        self.assertEqual(self.p.shared_inventory_names(inv, ["c_edge", "c_core"]),
                         [("edge", ["c_edge", "b_edge"]), ("core", ["c_core", "b_core"])])

    def test_one_name_per_peering_for_the_same_machine_passes(self):
        inv = self.INV.format(b_edge="edge-b", c_edge="edge-c", b_core="core-b", c_core="core-c")
        self.assertEqual(self.p.shared_inventory_names(inv, ["c_edge", "c_core"]), [])

    def test_one_peerings_own_edge_and_core_on_one_host_is_not_a_conflict(self):
        inv = self.INV.format(b_edge="edge-b", c_edge="box-c", b_core="core-b", c_core="box-c")
        self.assertEqual(self.p.shared_inventory_names(inv, ["c_edge", "c_core"]), [])

    def test_a_group_outside_the_peering_groups_is_not_a_conflict(self):
        inv = self.INV.format(b_edge="edge-b", c_edge="edge-c", b_core="core-b", c_core="core-c")
        inv += "\n[monitoring]\nedge-c\n"
        self.assertEqual(self.p.shared_inventory_names(inv, ["c_edge", "c_core"]), [])



class EdgeCoreEndpoints(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("preflight_ep", Path(__file__).with_name("preflight.py"))
        self.p = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.p)
        self.tmp = tempfile.TemporaryDirectory()
        self.gv = Path(self.tmp.name) / "ops/ansible/group_vars/xab_edge"
        self.gv.mkdir(parents=True)

    def tearDown(self):
        self.tmp.cleanup()

    def test_the_rendered_deployment_half_is_read_beside_the_hand_written_one(self):
        (self.gv / "deployment.yml").write_text(
            'goat_zenoh_edge_core_endpoints:\n  - "tls/192.0.2.20:7447"\n')
        (self.gv / "peering-deployment.yml").write_text(
            "goat_zenoh_edge_core_endpoints:\n"
            '  - "tls/core.example.org:7447#root_ca_certificate_file=/x/our-ca.pem"\n'
            '  - "tls/192.0.2.20:7447"\n'
            'goat_fed_other_endpoint: "tls/198.51.100.9:7447"\n')
        self.assertEqual(self.p.edge_core_endpoints(self.tmp.name, "xab_edge"),
                         ["192.0.2.20", "core.example.org"])

    def test_nothing_declared_is_an_empty_list(self):
        self.assertEqual(self.p.edge_core_endpoints(self.tmp.name, "xab_edge"), [])
        self.assertEqual(self.p.edge_core_endpoints(self.tmp.name, "no_such_edge"), [])

if __name__ == "__main__":
    unittest.main()
