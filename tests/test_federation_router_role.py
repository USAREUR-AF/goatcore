"""The standard role's rendered core has a discoverable name and label."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
ROLE = ROOT / 'ops/ansible/roles/goat-zenoh'


@unittest.skipUnless(shutil.which('ansible-playbook'), 'ansible-playbook is required')
class CoreDiscoveryContract(unittest.TestCase):
    def test_default_and_custom_stacks_render_discoverable_cores(self):
        for stack in (None, 'custom-fabric'):
            with self.subTest(stack=stack), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                dest = root / 'compose.yml'
                variables = {'site_id': 'zagros',
                             'zenoh_router_image': 'registry.example.org/zenoh:1.9.0',
                             'ansible_python_interpreter': sys.executable}
                if stack:
                    variables['zenoh_stack'] = stack
                defaults = yaml.safe_load((ROLE / 'defaults/main.yml').read_text())
                defaults.update(variables)
                play = [{'hosts': 'localhost', 'connection': 'local',
                         'gather_facts': False, 'become': False,
                         'vars': defaults,
                         'tasks': [{'ansible.builtin.template': {
                             'src': str(ROLE / 'templates/docker-compose.yml.j2'),
                             'dest': str(dest)}}]}]
                path = root / 'play.yml'
                path.write_text(yaml.safe_dump(play))
                env = {**os.environ, 'ANSIBLE_LOCAL_TEMP': str(root / 'local'),
                       'ANSIBLE_REMOTE_TEMP': str(root / 'remote'),
                       'OBJC_DISABLE_INITIALIZE_FORK_SAFETY': 'YES'}
                result = subprocess.run(['ansible-playbook', '-i', 'localhost,', str(path)],
                                        env=env, capture_output=True, text=True, timeout=45)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                services = yaml.safe_load(dest.read_text())['services']
                expected = (stack or 'zagros-zenoh') + '-router'
                self.assertIn(expected, services)
                core = services[expected]
                self.assertEqual(core['container_name'], expected)
                self.assertEqual(core['labels']['org.goat.component'], 'zenoh-core')


if __name__ == '__main__':
    unittest.main()
