"""acceptance_surfaces.py: phase 14's own surfaces come from the core and edge
declarations, and the environment still overrides each one."""
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
sys.path.insert(0, str(HERE))
import acceptance_surfaces as surf  # noqa: E402

ROLES = ("goat-zenoh", "goat-zenoh-edge", "goat-peering-spire")


class Surfaces(unittest.TestCase):
    def setUp(self):
        self.repo = Path(tempfile.mkdtemp(prefix="acc-surfaces-"))
        self.addCleanup(shutil.rmtree, self.repo)
        for role in ROLES:  # the roles' real defaults, as the plays read them
            d = self.repo / "ops/ansible/roles" / role / "defaults"
            d.mkdir(parents=True)
            shutil.copy(REPO / "ops/ansible/roles" / role / "defaults/main.yml", d / "main.yml")
        self.inventory(
            """
            [xab_edge]
            edge-a ansible_host=192.0.2.10 ansible_user=goat ansible_port=2222
            [xab_core]
            core-a ansible_host=192.0.2.20 ansible_user=goat
            [spire]
            spire-a ansible_host=192.0.2.30
            """)

    def inventory(self, text):
        p = self.repo / "ops/ansible/inventory/peering-edges.ini"
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(textwrap.dedent(text))

    def group_vars(self, group, text):
        d = self.repo / "ops/ansible/group_vars" / group
        d.mkdir(parents=True, exist_ok=True)
        (d / "main.yml").write_text(textwrap.dedent(text))

    def derive(self, environ=None, **kw):
        out, notes = surf.derive(str(self.repo), "xab", environ or {}, **kw)
        return dict(out), notes

    def test_loopback_rest_is_read_on_the_core_it_is_declared_on(self):
        # The role binds REST to loopback by default: read it on the core host.
        out, _ = self.derive()
        self.assertEqual(out["FED_OURS_REST"], "http://127.0.0.1:8000")
        self.assertEqual(out["FED_SSH_OURS"], "ssh://goat@192.0.2.20")

    def test_rest_bound_to_an_address_is_read_there_without_ssh(self):
        self.group_vars("xab_core", "goat_zenoh_rest_host: 0.0.0.0\ngoat_zenoh_rest_port: 18000\n")
        out, notes = self.derive()
        self.assertEqual(out["FED_OURS_REST"], "http://192.0.2.20:18000")
        self.assertNotIn("FED_SSH_OURS", out)
        self.assertTrue(any("group_vars/xab_core: goat_zenoh_rest_port" in n for n in notes), notes)

    def test_given_ssh_reads_rest_on_the_core(self):
        self.group_vars("xab_core", "goat_zenoh_rest_host: 0.0.0.0\n")
        out, _ = self.derive({"FED_SSH_OURS": "ops@198.51.100.5"})
        self.assertEqual(out["FED_OURS_REST"], "http://127.0.0.1:8000")
        self.assertNotIn("FED_SSH_OURS", out)

    def test_edge_log_is_the_edge_stack_on_the_edge_host(self):
        out, _ = self.derive()
        self.assertEqual(out["FED_EDGE_LOG_OURS"], "docker logs goat-zenoh-edge-xab")
        self.assertEqual(out["EDGE_LOG_VIA"], "ssh://goat@192.0.2.10:2222")
        self.group_vars("peering_edges", "goat_zenoh_edge_stack: edge-of-xab\nansible_become: true\n")
        out, _ = self.derive()
        self.assertEqual(out["FED_EDGE_LOG_OURS"], "sudo -n docker logs edge-of-xab")

    def test_spire_only_when_the_edge_enables_it(self):
        out, notes = self.derive()
        self.assertNotIn("FED_SPIRE_OURS", out)
        self.assertTrue(any("peering_spire_enabled is not true" in n for n in notes), notes)
        self.group_vars("xab_edge", "peering_spire_enabled: true\npeering_spire_server_host: spire-a\n")
        out, _ = self.derive()
        self.assertEqual(out["FED_SPIRE_OURS"], "docker exec spire-server /opt/spire/bin/spire-server")
        self.assertEqual(out["SPIRE_VIA"], "ssh://192.0.2.30")

    def test_spire_server_not_in_the_inventory_is_named(self):
        self.group_vars("xab_edge", "peering_spire_enabled: true\npeering_spire_server_host: nowhere\n")
        out, notes = self.derive()
        self.assertNotIn("FED_SPIRE_OURS", out)
        self.assertTrue(any("nowhere" in n and "not a host" in n for n in notes), notes)

    def test_the_environment_overrides_every_value(self):
        env = {"FED_OURS_REST": "http://203.0.113.1:8000", "FED_EDGE_LOG_OURS": "cat log",
               "FED_SPIRE_OURS": "spire-server"}
        self.group_vars("xab_edge", "peering_spire_enabled: true\npeering_spire_server_host: spire-a\n")
        out, _ = self.derive(env)
        self.assertEqual(out, {})

    def test_nothing_is_guessed(self):
        # No host in the group, or a templated value: left unset, with what was looked for.
        self.inventory("[other]\nhost-x\n")
        out, notes = self.derive()
        self.assertEqual(out, {})
        self.assertTrue(any("no host under [xab_core]" in n for n in notes), notes)
        self.assertTrue(any("no host under [xab_edge]" in n for n in notes), notes)
        self.inventory("[xab_core]\ncore-a\n[xab_edge]\nedge-a\n")
        self.group_vars("xab_core", "goat_zenoh_rest_port: \"{{ some_port }}\"\n")
        out, notes = self.derive()
        self.assertNotIn("FED_OURS_REST", out)
        self.assertTrue(any("templated" in n for n in notes), notes)

    def test_groups_follow_the_declared_edge_and_core_groups(self):
        self.inventory("[site_edge]\nedge-b ansible_host=192.0.2.11\n[site_core]\ncore-b ansible_host=192.0.2.21\n")
        self.group_vars("site_core", "goat_zenoh_rest_host: 192.0.2.21\n")
        out, _ = self.derive(edge_group="site_edge", core_group="site_core")
        self.assertEqual(out["FED_OURS_REST"], "http://192.0.2.21:8000")
        self.assertEqual(out["EDGE_LOG_VIA"], "ssh://192.0.2.11")

    def test_output_is_shell_assignments(self):
        self.group_vars("peering_edges", "goat_zenoh_edge_stack: \"edge xab; touch /nonexistent\"\n")
        env = {k: v for k, v in os.environ.items() if not k.startswith("FED_")}
        p = subprocess.run([sys.executable, str(HERE / "acceptance_surfaces.py"), "--peering", "xab",
                            "--repo", str(self.repo)], capture_output=True, text=True, env=env)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("acceptance: FED_OURS_REST derived", p.stderr)
        shown = subprocess.run(["bash", "-c", p.stdout + '\nprintf "%s|%s" "$FED_EDGE_LOG_OURS" "$EDGE_LOG_VIA"'],
                               capture_output=True, text=True)
        self.assertEqual(shown.stdout, "docker logs 'edge xab; touch /nonexistent'|ssh://goat@192.0.2.10:2222")


if __name__ == "__main__":
    unittest.main()
