#!/usr/bin/env python3
"""Execute the public-bundle export and disabled phase against local fixtures."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROLE = Path(__file__).resolve().parents[1]


class DisabledSpire(unittest.TestCase):
    def run_role(self, tasks_from, enabled=False, server="", expected_error=None):
        with tempfile.TemporaryDirectory(prefix="peering-spire-") as scratch:
            root = Path(scratch)
            calls = root / "docker-calls"
            bundle = root / "ours.spiffe-bundle"
            docker = root / "docker"
            docker.write_text(r"""#!/usr/bin/env python3
import os, pathlib, sys
assert sys.argv[1:] == ['exec', 'fixture-spire', '/fixture/spire-server', 'bundle', 'show', '-format', 'spiffe']
pathlib.Path(os.environ['FIXTURE_DOCKER_CALLS']).write_text('exported\n')
print('{"fixture": "public-bundle"}')
""")
            docker.chmod(0o755)
            inventory = root / "inventory.yml"
            inventory.write_text(json.dumps({"all": {"hosts": {
                "fixture-edge": {"ansible_connection": "local"},
                "fixture-server": {"ansible_connection": "local"},
            }}}))
            playbook = root / "playbook.yml"
            playbook.write_text(json.dumps([{
                "hosts": "fixture-edge", "gather_facts": False,
                "vars": {"peering_spire_enabled": enabled, "peering_spire_server_host": server,
                         "peering_spire_container": "fixture-spire", "peering_spire_bin": "/fixture/spire-server",
                         "peering_spire_our_bundle_file": str(bundle)},
                "tasks": [{"ansible.builtin.include_role": {"name": "goat-peering-spire", "tasks_from": tasks_from}}],
            }]))
            result = subprocess.run(
                ["ansible-playbook", "-i", str(inventory), str(playbook)],
                env={**os.environ, "ANSIBLE_ROLES_PATH": str(ROLE.parent),
                     "PATH": str(root) + os.pathsep + os.environ["PATH"],
                     "FIXTURE_DOCKER_CALLS": str(calls), "ANSIBLE_NOCOLOR": "1"},
                capture_output=True, text=True, timeout=30,
            )
            output = result.stdout + result.stderr
            if expected_error:
                self.assertEqual(result.returncode, 2, output)
                self.assertIn(expected_error, output)
            else:
                self.assertEqual(result.returncode, 0, output)
            if enabled and not expected_error:
                self.assertTrue(calls.exists(), output)
                self.assertEqual(json.loads(bundle.read_text()), {"fixture": "public-bundle"})
                self.assertIn("fixture-edge -> fixture-server", output)
            else:
                self.assertFalse(calls.exists(), output)
                self.assertFalse(bundle.exists(), output)

    def test_disabled_export_with_default_empty_host(self):
        self.run_role("export")

    def test_disabled_export_does_not_contact_an_unused_server(self):
        self.run_role("export", server="unused.example.invalid")

    def test_disabled_phase13_stops_without_delegation(self):
        self.run_role("main")

    def test_enabled_export_requires_a_server(self):
        self.run_role("export", enabled=True, expected_error="Set peering_spire_server_host")

    def test_enabled_phase13_retains_its_validation(self):
        self.run_role("main", enabled=True, expected_error="goat-peering-spire needs the server's host")

    def test_enabled_export_uses_the_declared_delegate(self):
        self.run_role("export", enabled=True, server="fixture-server")


if __name__ == "__main__":
    unittest.main()
